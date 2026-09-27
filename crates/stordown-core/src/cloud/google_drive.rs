use crate::{
    adaptive::AdaptiveLinkPool,
    control::TransferControl,
    model::{LinkConfig, ProgressCallback, TransferProgress},
};
use anyhow::{bail, Context, Result};
use reqwest::{
    header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, LOCATION, RANGE},
    redirect::Policy,
    Client, StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    cmp::min,
    collections::{BTreeSet, VecDeque},
    net::IpAddr,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, SeekFrom},
    task::JoinSet,
    time::sleep,
};

const DRIVE_UPLOAD_URL: &str =
    "https://www.googleapis.com/upload/drive/v3/files?uploadType=resumable&supportsAllDrives=true&fields=id,name,size,webViewLink";
const DRIVE_CHUNK_GRANULARITY: u64 = 256 * 1024;
const DEFAULT_CHUNK_SIZE: u64 = 8 * 1024 * 1024;
const MAX_CHUNK_RETRIES: usize = 5;

#[derive(Debug, Clone)]
pub struct GoogleDriveUploadRequest {
    pub source: PathBuf,
    pub access_token: String,
    pub parent_id: Option<String>,
    pub remote_name: Option<String>,
    pub mime_type: Option<String>,
    pub chunk_size: u64,
}

impl GoogleDriveUploadRequest {
    pub fn new(source: PathBuf, access_token: String) -> Self {
        Self {
            source,
            access_token,
            parent_id: None,
            remote_name: None,
            mime_type: None,
            chunk_size: DEFAULT_CHUNK_SIZE,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GoogleDriveBatchUploadRequest {
    pub files: Vec<PathBuf>,
    pub access_token: String,
    pub parent_id: Option<String>,
    pub chunk_size: u64,
    pub links: Vec<LinkConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleDriveUploadResult {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub web_view_link: Option<String>,
    pub bytes_uploaded: u64,
    pub link_used: String,
    pub local_ip: IpAddr,
}

#[derive(Debug, Deserialize)]
struct DriveFileResponse {
    id: String,
    name: String,
    #[serde(default)]
    size: Option<String>,
    #[serde(rename = "webViewLink")]
    web_view_link: Option<String>,
}

pub async fn upload_google_drive_file(
    request: GoogleDriveUploadRequest,
    link: LinkConfig,
) -> Result<GoogleDriveUploadResult> {
    upload_google_drive_file_with_progress(
        request,
        link,
        "drive-upload".to_string(),
        None,
    )
    .await
}

pub async fn upload_google_drive_file_with_progress(
    request: GoogleDriveUploadRequest,
    link: LinkConfig,
    transfer_id: String,
    progress: Option<ProgressCallback>,
) -> Result<GoogleDriveUploadResult> {
    upload_google_drive_file_with_control(request, link, transfer_id, progress, None).await
}

pub async fn upload_google_drive_file_with_control(
    request: GoogleDriveUploadRequest,
    link: LinkConfig,
    transfer_id: String,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
) -> Result<GoogleDriveUploadResult> {
    if !link.enabled {
        bail!("selected link {} is disabled", link.name);
    }

    upload_google_drive_file_with_pool(
        request,
        AdaptiveLinkPool::new(vec![link]),
        transfer_id,
        progress,
        control,
    )
    .await
}

async fn upload_google_drive_file_with_pool(
    request: GoogleDriveUploadRequest,
    pool: AdaptiveLinkPool,
    transfer_id: String,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
) -> Result<GoogleDriveUploadResult> {
    validate_chunk_size(request.chunk_size)?;
    checkpoint(control.as_ref()).await?;

    let metadata = tokio::fs::metadata(&request.source)
        .await
        .with_context(|| format!("cannot read {}", request.source.display()))?;

    if !metadata.is_file() {
        bail!("{} is not a regular file", request.source.display());
    }

    let total_size = metadata.len();
    let remote_name = request
        .remote_name
        .clone()
        .unwrap_or_else(|| file_name(&request.source));
    let mime_type = request
        .mime_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let (session_uri, session_link) = create_resumable_session_with_failover(
        &pool,
        &request.access_token,
        request.parent_id.as_deref(),
        &remote_name,
        &mime_type,
        total_size,
        control.as_ref(),
    )
    .await?;

    emit_upload_progress(
        progress.as_ref(),
        &transfer_id,
        &remote_name,
        "starting",
        0,
        0,
        total_size,
        &session_link,
        false,
    );

    if total_size == 0 {
        return upload_empty_file(
            &pool,
            &session_uri,
            &remote_name,
            &transfer_id,
            progress,
            control,
        )
        .await;
    }

    upload_chunks_adaptive(
        &pool,
        &session_uri,
        &request.source,
        &remote_name,
        &mime_type,
        total_size,
        request.chunk_size,
        &transfer_id,
        progress,
        control,
    )
    .await
}

pub async fn upload_google_drive_batch(
    request: GoogleDriveBatchUploadRequest,
) -> Result<Vec<GoogleDriveUploadResult>> {
    upload_google_drive_batch_with_progress(
        request,
        "drive-batch".to_string(),
        None,
    )
    .await
}

pub async fn upload_google_drive_batch_with_progress(
    request: GoogleDriveBatchUploadRequest,
    transfer_id: String,
    progress: Option<ProgressCallback>,
) -> Result<Vec<GoogleDriveUploadResult>> {
    upload_google_drive_batch_with_control(request, transfer_id, progress, None).await
}

pub async fn upload_google_drive_batch_with_control(
    request: GoogleDriveBatchUploadRequest,
    transfer_id: String,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
) -> Result<Vec<GoogleDriveUploadResult>> {
    validate_chunk_size(request.chunk_size)?;
    checkpoint(control.as_ref()).await?;

    let enabled_links: Vec<LinkConfig> = request
        .links
        .into_iter()
        .filter(|link| link.enabled)
        .collect();

    if enabled_links.is_empty() {
        bail!("at least one enabled link is required");
    }

    if request.files.is_empty() {
        return Ok(Vec::new());
    }

    let max_parallel = enabled_links.len().saturating_mul(2).clamp(1, 8);
    let pool = AdaptiveLinkPool::new(enabled_links);
    let mut pending: VecDeque<(usize, PathBuf)> =
        request.files.into_iter().enumerate().collect();
    let mut jobs: JoinSet<Result<(usize, GoogleDriveUploadResult)>> = JoinSet::new();

    while jobs.len() < max_parallel {
        let Some((index, source)) = pending.pop_front() else {
            break;
        };

        spawn_drive_upload_job(
            &mut jobs,
            index,
            source,
            request.access_token.clone(),
            request.parent_id.clone(),
            request.chunk_size,
            transfer_id.clone(),
            progress.clone(),
            control.clone(),
            pool.clone(),
        );
    }

    let mut results = Vec::new();

    while let Some(joined) = jobs.join_next().await {
        results.push(joined??);

        if let Some((index, source)) = pending.pop_front() {
            spawn_drive_upload_job(
                &mut jobs,
                index,
                source,
                request.access_token.clone(),
                request.parent_id.clone(),
                request.chunk_size,
                transfer_id.clone(),
                progress.clone(),
                control.clone(),
                pool.clone(),
            );
        }
    }

    results.sort_by_key(|(index, _)| *index);
    Ok(results.into_iter().map(|(_, result)| result).collect())
}

#[allow(clippy::too_many_arguments)]
fn spawn_drive_upload_job(
    jobs: &mut JoinSet<Result<(usize, GoogleDriveUploadResult)>>,
    index: usize,
    source: PathBuf,
    access_token: String,
    parent_id: Option<String>,
    chunk_size: u64,
    transfer_id: String,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
    pool: AdaptiveLinkPool,
) {
    jobs.spawn(async move {
        let result = upload_google_drive_file_with_pool(
            GoogleDriveUploadRequest {
                source,
                access_token,
                parent_id,
                remote_name: None,
                mime_type: None,
                chunk_size,
            },
            pool,
            transfer_id,
            progress,
            control,
        )
        .await?;

        Ok::<_, anyhow::Error>((index, result))
    });
}

async fn create_resumable_session(
    client: &Client,
    access_token: &str,
    parent_id: Option<&str>,
    remote_name: &str,
    mime_type: &str,
    total_size: u64,
) -> Result<String> {
    let mut metadata = json!({ "name": remote_name });

    if let Some(parent_id) = parent_id.filter(|value| !value.trim().is_empty()) {
        metadata["parents"] = json!([parent_id]);
    }

    let response = client
        .post(DRIVE_UPLOAD_URL)
        .bearer_auth(access_token)
        .header("X-Upload-Content-Type", mime_type)
        .header("X-Upload-Content-Length", total_size.to_string())
        .json(&metadata)
        .send()
        .await
        .context("failed to create Google Drive resumable upload session")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("Google Drive session creation failed ({status}): {body}");
    }

    response
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .context("Google Drive did not return a resumable session Location")
}

#[allow(clippy::too_many_arguments)]
async fn create_resumable_session_with_failover(
    pool: &AdaptiveLinkPool,
    access_token: &str,
    parent_id: Option<&str>,
    remote_name: &str,
    mime_type: &str,
    total_size: u64,
    control: Option<&TransferControl>,
) -> Result<(String, LinkConfig)> {
    let mut last_error = None;

    for attempt in 1..=MAX_CHUNK_RETRIES {
        checkpoint(control).await?;
        let lease = pool.acquire().await;
        let link = lease.link.clone();

        let client = match drive_client(link.local_ip) {
            Ok(client) => client,
            Err(error) => {
                pool.failure(&lease).await;
                last_error = Some(error);
                if attempt < MAX_CHUNK_RETRIES {
                    sleep(backoff(attempt)).await;
                }
                continue;
            }
        };

        match create_resumable_session(
            &client,
            access_token,
            parent_id,
            remote_name,
            mime_type,
            total_size,
        )
        .await
        {
            Ok(session_uri) => {
                pool.release(&lease).await;
                return Ok((session_uri, link));
            }
            Err(error) => {
                pool.failure(&lease).await;
                last_error = Some(error);
                if attempt < MAX_CHUNK_RETRIES {
                    sleep(backoff(attempt)).await;
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("unable to create Drive upload session")))
}

async fn upload_empty_file(
    pool: &AdaptiveLinkPool,
    session_uri: &str,
    remote_name: &str,
    transfer_id: &str,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
) -> Result<GoogleDriveUploadResult> {
    let mut last_error = None;

    for attempt in 1..=MAX_CHUNK_RETRIES {
        checkpoint(control.as_ref()).await?;
        let lease = pool.acquire().await;
        let link = lease.link.clone();

        let client = match drive_client(link.local_ip) {
            Ok(client) => client,
            Err(error) => {
                pool.failure(&lease).await;
                last_error = Some(error);
                continue;
            }
        };

        match client
            .put(session_uri)
            .header(CONTENT_LENGTH, "0")
            .header(CONTENT_RANGE, "bytes */0")
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                let file: DriveFileResponse = response.json().await?;
                pool.success(&lease, 0).await;

                emit_upload_progress(
                    progress.as_ref(),
                    transfer_id,
                    remote_name,
                    "completed",
                    0,
                    0,
                    0,
                    &link,
                    true,
                );

                return Ok(to_result(file, 0, &link));
            }
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                pool.failure(&lease).await;
                last_error = Some(anyhow::anyhow!(
                    "Google Drive empty upload failed ({status}): {body}"
                ));
            }
            Err(error) => {
                pool.failure(&lease).await;
                last_error = Some(error.into());
            }
        }

        if attempt < MAX_CHUNK_RETRIES {
            sleep(backoff(attempt)).await;
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Google Drive empty upload failed")))
}

#[allow(clippy::too_many_arguments)]
async fn upload_chunks_adaptive(
    pool: &AdaptiveLinkPool,
    session_uri: &str,
    source: &Path,
    remote_name: &str,
    mime_type: &str,
    total_size: u64,
    chunk_size: u64,
    transfer_id: &str,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
) -> Result<GoogleDriveUploadResult> {
    let mut file = File::open(source).await?;
    let mut offset = 0u64;
    let mut reported_offset = 0u64;
    let mut used_links = BTreeSet::new();

    while offset < total_size {
        checkpoint(control.as_ref()).await?;

        let length = min(chunk_size, total_size - offset);
        file.seek(SeekFrom::Start(offset)).await?;

        let mut buffer = vec![0u8; length as usize];
        file.read_exact(&mut buffer).await?;

        let end = offset + length - 1;
        let mut attempt = 0usize;

        loop {
            checkpoint(control.as_ref()).await?;
            attempt += 1;

            let lease = pool.acquire().await;
            let link = lease.link.clone();
            used_links.insert(link.name.clone());

            if attempt > 1 {
                emit_upload_progress(
                    progress.as_ref(),
                    transfer_id,
                    remote_name,
                    "failover",
                    0,
                    reported_offset,
                    total_size,
                    &link,
                    false,
                );
            }

            let client = match drive_client(link.local_ip) {
                Ok(client) => client,
                Err(error) => {
                    pool.failure(&lease).await;

                    if attempt >= MAX_CHUNK_RETRIES {
                        return Err(error);
                    }

                    continue;
                }
            };

            let response = client
                .put(session_uri)
                .header(CONTENT_TYPE, mime_type)
                .header(CONTENT_LENGTH, length.to_string())
                .header(
                    CONTENT_RANGE,
                    format!("bytes {offset}-{end}/{total_size}"),
                )
                .body(buffer.clone())
                .send()
                .await;

            match response {
                Ok(response)
                    if response.status() == StatusCode::OK
                        || response.status() == StatusCode::CREATED =>
                {
                    let uploaded: DriveFileResponse = response
                        .json()
                        .await
                        .context("invalid Google Drive completion response")?;

                    let accepted = total_size.saturating_sub(offset).min(length);
                    pool.success(&lease, accepted).await;

                    let delta = total_size.saturating_sub(reported_offset);
                    emit_upload_progress(
                        progress.as_ref(),
                        transfer_id,
                        remote_name,
                        "completed",
                        delta,
                        total_size,
                        total_size,
                        &link,
                        true,
                    );

                    let mut result = to_result(uploaded, total_size, &link);
                    if used_links.len() > 1 {
                        result.link_used =
                            used_links.iter().cloned().collect::<Vec<_>>().join(" + ");
                    }
                    return Ok(result);
                }
                Ok(response) if response.status().as_u16() == 308 => {
                    let next_offset =
                        next_offset_from_range(response.headers().get(RANGE), end + 1);
                    let accepted = next_offset.saturating_sub(offset).min(length);
                    pool.success(&lease, accepted).await;

                    let delta = next_offset.saturating_sub(reported_offset);
                    if delta > 0 {
                        emit_upload_progress(
                            progress.as_ref(),
                            transfer_id,
                            remote_name,
                            if attempt > 1 {
                                "failover-active"
                            } else {
                                "transferring"
                            },
                            delta,
                            next_offset,
                            total_size,
                            &link,
                            false,
                        );
                        reported_offset = next_offset;
                    }

                    offset = next_offset;
                    break;
                }
                Ok(response)
                    if response.status().is_server_error()
                        || response.status() == StatusCode::TOO_MANY_REQUESTS
                        || response.status() == StatusCode::REQUEST_TIMEOUT =>
                {
                    let status = response.status();
                    pool.failure(&lease).await;

                    if attempt >= MAX_CHUNK_RETRIES {
                        let body = response.text().await.unwrap_or_default();
                        bail!("Google Drive upload failed after failover ({status}): {body}");
                    }

                    let (known_offset, probe_link) =
                        query_upload_offset_with_failover(pool, session_uri, total_size)
                            .await
                            .unwrap_or((offset, link.clone()));

                    if known_offset > reported_offset {
                        let delta = known_offset - reported_offset;
                        emit_upload_progress(
                            progress.as_ref(),
                            transfer_id,
                            remote_name,
                            "failover-recovered",
                            delta,
                            known_offset,
                            total_size,
                            &probe_link,
                            false,
                        );
                        reported_offset = known_offset;
                    }

                    if known_offset != offset {
                        offset = known_offset;
                        break;
                    }

                    sleep(backoff(attempt)).await;
                }
                Ok(response) => {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    pool.failure(&lease).await;
                    bail!("Google Drive upload failed ({status}): {body}");
                }
                Err(error) => {
                    pool.failure(&lease).await;

                    if attempt >= MAX_CHUNK_RETRIES {
                        return Err(error)
                            .context("Google Drive chunk upload failed after multi-WAN failover");
                    }

                    let (known_offset, probe_link) =
                        query_upload_offset_with_failover(pool, session_uri, total_size)
                            .await
                            .unwrap_or((offset, link.clone()));

                    if known_offset > reported_offset {
                        let delta = known_offset - reported_offset;
                        emit_upload_progress(
                            progress.as_ref(),
                            transfer_id,
                            remote_name,
                            "failover-recovered",
                            delta,
                            known_offset,
                            total_size,
                            &probe_link,
                            false,
                        );
                        reported_offset = known_offset;
                    }

                    if known_offset != offset {
                        offset = known_offset;
                        break;
                    }

                    sleep(backoff(attempt)).await;
                }
            }
        }
    }

    bail!("Google Drive upload ended without a completion response")
}

async fn query_upload_offset(client: &Client, session_uri: &str, total_size: u64) -> Result<u64> {
    let response = client
        .put(session_uri)
        .header(CONTENT_LENGTH, "0")
        .header(CONTENT_RANGE, format!("bytes */{total_size}"))
        .send()
        .await?;

    if response.status().as_u16() == 308 {
        return Ok(next_offset_from_range(response.headers().get(RANGE), 0));
    }

    if response.status() == StatusCode::OK || response.status() == StatusCode::CREATED {
        return Ok(total_size);
    }

    bail!("unable to query Google Drive upload status: {}", response.status())
}

async fn query_upload_offset_with_failover(
    pool: &AdaptiveLinkPool,
    session_uri: &str,
    total_size: u64,
) -> Result<(u64, LinkConfig)> {
    let mut last_error = None;

    for _ in 0..MAX_CHUNK_RETRIES {
        let lease = pool.acquire().await;
        let link = lease.link.clone();

        let client = match drive_client(link.local_ip) {
            Ok(client) => client,
            Err(error) => {
                pool.failure(&lease).await;
                last_error = Some(error);
                continue;
            }
        };

        match query_upload_offset(&client, session_uri, total_size).await {
            Ok(offset) => {
                pool.release(&lease).await;
                return Ok((offset, link));
            }
            Err(error) => {
                pool.failure(&lease).await;
                last_error = Some(error);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("unable to query Drive upload offset")))
}

async fn checkpoint(control: Option<&TransferControl>) -> Result<()> {
    if let Some(control) = control {
        control.checkpoint().await?;
    }

    Ok(())
}

fn emit_upload_progress(
    callback: Option<&ProgressCallback>,
    transfer_id: &str,
    item: &str,
    phase: &str,
    bytes_delta: u64,
    bytes_transferred: u64,
    total_bytes: u64,
    link: &LinkConfig,
    completed: bool,
) {
    if let Some(callback) = callback {
        callback(TransferProgress {
            transfer_id: transfer_id.to_owned(),
            direction: "upload".to_string(),
            item: item.to_owned(),
            phase: phase.to_owned(),
            bytes_delta,
            bytes_transferred,
            total_bytes: Some(total_bytes),
            link_name: link.name.clone(),
            local_ip: link.local_ip,
            completed,
        });
    }
}

fn next_offset_from_range(range: Option<&reqwest::header::HeaderValue>, fallback: u64) -> u64 {
    range
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.rsplit_once('-'))
        .and_then(|(_, end)| end.parse::<u64>().ok())
        .map(|end| end.saturating_add(1))
        .unwrap_or(fallback)
}

fn drive_client(local_ip: IpAddr) -> Result<Client> {
    Client::builder()
        .local_address(local_ip)
        .redirect(Policy::none())
        .build()
        .context("failed to build Google Drive client bound to local IP")
}

fn validate_chunk_size(chunk_size: u64) -> Result<()> {
    if chunk_size == 0 || chunk_size % DRIVE_CHUNK_GRANULARITY != 0 {
        bail!("Google Drive chunk size must be a non-zero multiple of 256 KiB");
    }

    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("upload.bin")
        .to_owned()
}

fn to_result(
    file: DriveFileResponse,
    bytes_uploaded: u64,
    link: &LinkConfig,
) -> GoogleDriveUploadResult {
    GoogleDriveUploadResult {
        id: file.id,
        name: file.name,
        size: file
            .size
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(bytes_uploaded),
        web_view_link: file.web_view_link,
        bytes_uploaded,
        link_used: link.name.clone(),
        local_ip: link.local_ip,
    }
}

fn backoff(attempt: usize) -> Duration {
    Duration::from_millis(500 * 2u64.pow((attempt.saturating_sub(1)).min(4) as u32))
}

#[cfg(test)]
mod tests {
    use super::{next_offset_from_range, validate_chunk_size};
    use reqwest::header::HeaderValue;

    #[test]
    fn drive_chunk_size_must_use_256_kib_granularity() {
        assert!(validate_chunk_size(8 * 1024 * 1024).is_ok());
        assert!(validate_chunk_size(1024 * 1024).is_ok());
        assert!(validate_chunk_size(12345).is_err());
    }

    #[test]
    fn parses_drive_resume_range() {
        let header = HeaderValue::from_static("bytes=0-524287");
        assert_eq!(next_offset_from_range(Some(&header), 0), 524288);
    }
}
