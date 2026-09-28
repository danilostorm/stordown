use crate::{
    AdaptiveLinkPool, LinkConfig, ProgressCallback, TransferControl, TransferProgress,
    TransferThrottle,
};
use anyhow::{bail, Context, Result};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cmp::min,
    collections::{BTreeSet, HashSet, VecDeque},
    net::IpAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, SeekFrom},
    task::JoinSet,
    time::sleep,
};

const MAX_RELAY_RETRIES: usize = 5;
const DEFAULT_RELAY_CHUNK_SIZE: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct RelayUploadRequest {
    pub source: PathBuf,
    pub relay_url: String,
    pub token: Option<String>,
    pub chunk_size: u64,
    pub links: Vec<LinkConfig>,
    pub max_bytes_per_second: Option<u64>,
    pub session_id: Option<String>,
}

impl RelayUploadRequest {
    pub fn new(source: PathBuf, relay_url: String, links: Vec<LinkConfig>) -> Self {
        Self {
            source,
            relay_url,
            token: None,
            chunk_size: DEFAULT_RELAY_CHUNK_SIZE,
            links,
            max_bytes_per_second: None,
            session_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayCreateSessionRequest {
    pub file_name: String,
    pub total_size: u64,
    pub chunk_size: u64,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelaySession {
    pub id: String,
    pub file_name: String,
    pub total_size: u64,
    pub chunk_size: u64,
    pub total_chunks: u64,
    pub sha256: Option<String>,
    pub completed: bool,
    pub result_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelaySessionStatus {
    pub session: RelaySession,
    pub received_chunks: Vec<u64>,
    pub received_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayCompleteResponse {
    pub session_id: String,
    pub result_path: String,
    pub total_size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayUploadResult {
    pub session_id: String,
    pub result_path: String,
    pub total_size: u64,
    pub sha256: String,
    pub links_used: Vec<String>,
}

pub async fn upload_to_relay(
    request: RelayUploadRequest,
    transfer_id: String,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
) -> Result<RelayUploadResult> {
    if request.chunk_size == 0 {
        bail!("relay chunk size must be greater than zero");
    }

    let links: Vec<LinkConfig> = request
        .links
        .into_iter()
        .filter(|link| link.enabled)
        .collect();

    if links.is_empty() {
        bail!("at least one enabled network link is required");
    }

    checkpoint(control.as_ref()).await?;

    let metadata = tokio::fs::metadata(&request.source)
        .await
        .with_context(|| format!("cannot read {}", request.source.display()))?;
    if !metadata.is_file() {
        bail!("{} is not a regular file", request.source.display());
    }

    let total_size = metadata.len();
    let file_name = request
        .source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("upload.bin")
        .to_string();
    let relay_url = request.relay_url.trim_end_matches('/').to_string();
    let pool = AdaptiveLinkPool::new(links);
    let throttle = TransferThrottle::new(request.max_bytes_per_second);

    let session = match request.session_id.as_deref() {
        Some(session_id) if !session_id.trim().is_empty() => {
            let status = relay_status(
                &pool,
                &relay_url,
                request.token.as_deref(),
                session_id,
                control.as_ref(),
            )
            .await?;
            validate_resume(&status.session, &file_name, total_size, request.chunk_size)?;
            status.session
        }
        _ => {
            create_session(
                &pool,
                &relay_url,
                request.token.as_deref(),
                RelayCreateSessionRequest {
                    file_name: file_name.clone(),
                    total_size,
                    chunk_size: request.chunk_size,
                    sha256: None,
                },
                control.as_ref(),
            )
            .await?
        }
    };

    if session.completed {
        let status = relay_status(
            &pool,
            &relay_url,
            request.token.as_deref(),
            &session.id,
            control.as_ref(),
        )
        .await?;
        return Ok(RelayUploadResult {
            session_id: session.id,
            result_path: status.session.result_path.unwrap_or_default(),
            total_size,
            sha256: status.session.sha256.unwrap_or_default(),
            links_used: Vec::new(),
        });
    }

    let status = relay_status(
        &pool,
        &relay_url,
        request.token.as_deref(),
        &session.id,
        control.as_ref(),
    )
    .await?;
    let received: HashSet<u64> = status.received_chunks.iter().copied().collect();
    let transferred = Arc::new(AtomicU64::new(status.received_bytes));
    let total_chunks = session.total_chunks;

    emit_progress(
        progress.as_ref(),
        &transfer_id,
        &file_name,
        "relay-starting",
        0,
        status.received_bytes,
        total_size,
        &pool.acquire().await.link,
        false,
    );
    // release the synthetic telemetry lease immediately.
    let synthetic = pool.acquire().await;
    pool.release(&synthetic).await;

    let mut pending: VecDeque<u64> = (0..total_chunks)
        .filter(|index| !received.contains(index))
        .collect();
    let max_parallel = pool.snapshots().await.len().saturating_mul(2).clamp(1, 8);
    let mut jobs: JoinSet<Result<String>> = JoinSet::new();
    let used_links = Arc::new(tokio::sync::Mutex::new(BTreeSet::<String>::new()));

    while jobs.len() < max_parallel {
        let Some(index) = pending.pop_front() else {
            break;
        };
        spawn_chunk(
            &mut jobs,
            index,
            request.source.clone(),
            relay_url.clone(),
            request.token.clone(),
            session.clone(),
            transfer_id.clone(),
            progress.clone(),
            control.clone(),
            pool.clone(),
            throttle.clone(),
            transferred.clone(),
            used_links.clone(),
        );
    }

    while let Some(joined) = jobs.join_next().await {
        joined??;

        if let Some(index) = pending.pop_front() {
            spawn_chunk(
                &mut jobs,
                index,
                request.source.clone(),
                relay_url.clone(),
                request.token.clone(),
                session.clone(),
                transfer_id.clone(),
                progress.clone(),
                control.clone(),
                pool.clone(),
                throttle.clone(),
                transferred.clone(),
                used_links.clone(),
            );
        }
    }

    checkpoint(control.as_ref()).await?;
    let completed = complete_session(
        &pool,
        &relay_url,
        request.token.as_deref(),
        &session.id,
        control.as_ref(),
    )
    .await?;

    let links_used = used_links.lock().await.iter().cloned().collect();

    Ok(RelayUploadResult {
        session_id: completed.session_id,
        result_path: completed.result_path,
        total_size: completed.total_size,
        sha256: completed.sha256,
        links_used,
    })
}

#[allow(clippy::too_many_arguments)]
fn spawn_chunk(
    jobs: &mut JoinSet<Result<String>>,
    index: u64,
    source: PathBuf,
    relay_url: String,
    token: Option<String>,
    session: RelaySession,
    transfer_id: String,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
    pool: AdaptiveLinkPool,
    throttle: TransferThrottle,
    transferred: Arc<AtomicU64>,
    used_links: Arc<tokio::sync::Mutex<BTreeSet<String>>>,
) {
    jobs.spawn(async move {
        checkpoint(control.as_ref()).await?;

        let offset = index
            .checked_mul(session.chunk_size)
            .context("relay chunk offset overflow")?;
        let length = min(session.chunk_size, session.total_size.saturating_sub(offset));

        let mut file = File::open(&source).await?;
        file.seek(SeekFrom::Start(offset)).await?;
        let mut buffer = vec![0u8; length as usize];
        if length > 0 {
            file.read_exact(&mut buffer).await?;
        }

        let digest = hex_sha256(&buffer);
        let mut last_error = None;

        for attempt in 1..=MAX_RELAY_RETRIES {
            checkpoint(control.as_ref()).await?;
            let lease = pool.acquire().await;
            let link = lease.link.clone();
            used_links.lock().await.insert(link.name.clone());

            let client = match relay_client(link.local_ip) {
                Ok(client) => client,
                Err(error) => {
                    pool.failure(&lease).await;
                    last_error = Some(error);
                    continue;
                }
            };

            throttle.consume(length).await;
            let url = format!(
                "{}/v1/sessions/{}/chunks/{}",
                relay_url, session.id, index
            );
            let mut request = client
                .put(url)
                .header("x-stordown-chunk-sha256", &digest)
                .body(buffer.clone());
            if let Some(token) = token.as_deref().filter(|value| !value.trim().is_empty()) {
                request = request.bearer_auth(token);
            }

            match request.send().await {
                Ok(response) if response.status().is_success() => {
                    pool.success(&lease, length).await;
                    let current = transferred.fetch_add(length, Ordering::AcqRel) + length;
                    emit_progress(
                        progress.as_ref(),
                        &transfer_id,
                        &session.file_name,
                        if attempt > 1 {
                            "relay-failover-active"
                        } else {
                            "relay-transferring"
                        },
                        length,
                        current.min(session.total_size),
                        session.total_size,
                        &link,
                        false,
                    );
                    return Ok(link.name);
                }
                Ok(response)
                    if response.status().is_server_error()
                        || response.status() == StatusCode::TOO_MANY_REQUESTS
                        || response.status() == StatusCode::REQUEST_TIMEOUT =>
                {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    pool.failure(&lease).await;
                    last_error = Some(anyhow::anyhow!(
                        "relay chunk {index} failed ({status}): {body}"
                    ));
                }
                Ok(response) => {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    pool.failure(&lease).await;
                    bail!("relay chunk {index} rejected ({status}): {body}");
                }
                Err(error) => {
                    pool.failure(&lease).await;
                    last_error = Some(error.into());
                }
            }

            if attempt < MAX_RELAY_RETRIES {
                sleep(backoff(attempt)).await;
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("relay chunk upload failed")))
    });
}

async fn create_session(
    pool: &AdaptiveLinkPool,
    relay_url: &str,
    token: Option<&str>,
    body: RelayCreateSessionRequest,
    control: Option<&TransferControl>,
) -> Result<RelaySession> {
    request_with_failover(pool, control, |client| {
        let url = format!("{relay_url}/v1/sessions");
        let mut request = client.post(url).json(&body);
        if let Some(token) = token.filter(|value| !value.trim().is_empty()) {
            request = request.bearer_auth(token);
        }
        request
    })
    .await
}

async fn relay_status(
    pool: &AdaptiveLinkPool,
    relay_url: &str,
    token: Option<&str>,
    session_id: &str,
    control: Option<&TransferControl>,
) -> Result<RelaySessionStatus> {
    request_with_failover(pool, control, |client| {
        let url = format!("{relay_url}/v1/sessions/{session_id}");
        let mut request = client.get(url);
        if let Some(token) = token.filter(|value| !value.trim().is_empty()) {
            request = request.bearer_auth(token);
        }
        request
    })
    .await
}

async fn complete_session(
    pool: &AdaptiveLinkPool,
    relay_url: &str,
    token: Option<&str>,
    session_id: &str,
    control: Option<&TransferControl>,
) -> Result<RelayCompleteResponse> {
    request_with_failover(pool, control, |client| {
        let url = format!("{relay_url}/v1/sessions/{session_id}/complete");
        let mut request = client.post(url);
        if let Some(token) = token.filter(|value| !value.trim().is_empty()) {
            request = request.bearer_auth(token);
        }
        request
    })
    .await
}

async fn request_with_failover<T, F>(
    pool: &AdaptiveLinkPool,
    control: Option<&TransferControl>,
    builder: F,
) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
    F: Fn(&Client) -> reqwest::RequestBuilder,
{
    let mut last_error = None;

    for attempt in 1..=MAX_RELAY_RETRIES {
        checkpoint(control).await?;
        let lease = pool.acquire().await;
        let client = match relay_client(lease.link.local_ip) {
            Ok(client) => client,
            Err(error) => {
                pool.failure(&lease).await;
                last_error = Some(error);
                continue;
            }
        };

        match builder(&client).send().await {
            Ok(response) if response.status().is_success() => {
                let value = response.json::<T>().await?;
                pool.release(&lease).await;
                return Ok(value);
            }
            Ok(response)
                if response.status().is_server_error()
                    || response.status() == StatusCode::TOO_MANY_REQUESTS
                    || response.status() == StatusCode::REQUEST_TIMEOUT =>
            {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                pool.failure(&lease).await;
                last_error = Some(anyhow::anyhow!("relay request failed ({status}): {body}"));
            }
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                pool.failure(&lease).await;
                bail!("relay request rejected ({status}): {body}");
            }
            Err(error) => {
                pool.failure(&lease).await;
                last_error = Some(error.into());
            }
        }

        if attempt < MAX_RELAY_RETRIES {
            sleep(backoff(attempt)).await;
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("relay request failed")))
}

fn validate_resume(
    session: &RelaySession,
    file_name: &str,
    total_size: u64,
    chunk_size: u64,
) -> Result<()> {
    if session.file_name != file_name
        || session.total_size != total_size
        || session.chunk_size != chunk_size
    {
        bail!("relay session does not match the local file");
    }
    Ok(())
}

fn relay_client(local_ip: IpAddr) -> Result<Client> {
    Client::builder()
        .local_address(local_ip)
        .build()
        .context("failed to build relay client bound to local IP")
}

fn emit_progress(
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
            transfer_id: transfer_id.to_string(),
            direction: "upload".to_string(),
            item: item.to_string(),
            phase: phase.to_string(),
            bytes_delta,
            bytes_transferred,
            total_bytes: Some(total_bytes),
            link_name: link.name.clone(),
            local_ip: link.local_ip,
            completed,
        });
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

async fn checkpoint(control: Option<&TransferControl>) -> Result<()> {
    if let Some(control) = control {
        control.checkpoint().await?;
    }
    Ok(())
}

fn backoff(attempt: usize) -> Duration {
    Duration::from_millis(400 * 2u64.pow((attempt.saturating_sub(1)).min(4) as u32))
}

#[cfg(test)]
mod tests {
    use super::{validate_resume, RelaySession};

    #[test]
    fn validates_matching_resume_session() {
        let session = RelaySession {
            id: "abc".into(),
            file_name: "movie.mkv".into(),
            total_size: 1024,
            chunk_size: 256,
            total_chunks: 4,
            sha256: None,
            completed: false,
            result_path: None,
        };

        assert!(validate_resume(&session, "movie.mkv", 1024, 256).is_ok());
        assert!(validate_resume(&session, "movie.mkv", 2048, 256).is_err());
    }
}
