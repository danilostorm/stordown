use crate::{
    adaptive::AdaptiveLinkPool,
    control::TransferControl,
    model::{LinkConfig, ProgressCallback, TransferProgress},
    throttle::TransferThrottle,
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
    collections::{BTreeSet, HashMap, VecDeque},
    net::IpAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, SeekFrom},
    task::JoinSet,
    time::sleep,
};
use url::Url;

const DRIVE_UPLOAD_URL: &str =
    "https://www.googleapis.com/upload/drive/v3/files?uploadType=resumable&supportsAllDrives=true&fields=id,name,size,webViewLink";
const DRIVE_FILES_URL: &str = "https://www.googleapis.com/drive/v3/files";
const DRIVE_DRIVES_URL: &str = "https://www.googleapis.com/drive/v3/drives";
const DRIVE_FOLDER_MIME: &str = "application/vnd.google-apps.folder";
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
    pub max_bytes_per_second: Option<u64>,
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
            max_bytes_per_second: None,
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
    pub max_bytes_per_second: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleDriveItem {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: Option<u64>,
    pub drive_id: Option<String>,
    pub resource_key: Option<String>,
    pub can_download: bool,
    pub md5_checksum: Option<String>,
    pub is_folder: bool,
    pub is_google_workspace: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoogleDriveExportFormat {
    pub label: String,
    pub mime_type: String,
    pub extension: String,
}

#[derive(Debug, Deserialize)]
struct GoogleDriveItemListResponse {
    #[serde(default)]
    files: Vec<GoogleDriveItemResponse>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleDriveItemResponse {
    id: String,
    name: String,
    #[serde(rename = "mimeType")]
    mime_type: String,
    #[serde(default)]
    size: Option<String>,
    #[serde(rename = "driveId")]
    drive_id: Option<String>,
    #[serde(rename = "resourceKey")]
    resource_key: Option<String>,
    #[serde(default)]
    capabilities: GoogleDriveCapabilities,
    #[serde(rename = "md5Checksum")]
    md5_checksum: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GoogleDriveCapabilities {
    #[serde(rename = "canDownload", default)]
    can_download: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleDriveFolder {
    pub id: String,
    pub name: String,
    pub drive_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleSharedDrive {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct GoogleDriveFolderListResponse {
    #[serde(default)]
    files: Vec<GoogleDriveFolderResponse>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleDriveFolderResponse {
    id: String,
    name: String,
    #[serde(rename = "driveId")]
    drive_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleSharedDriveListResponse {
    #[serde(default)]
    drives: Vec<GoogleSharedDrive>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleDriveUploadResumeState {
    pub source: String,
    pub session_uri: String,
    pub confirmed_offset: u64,
    pub total_size: u64,
    pub remote_name: String,
    pub mime_type: String,
    pub parent_id: Option<String>,
    pub chunk_size: u64,
    pub completed: bool,
}

pub type DriveUploadCheckpointCallback =
    Arc<dyn Fn(GoogleDriveUploadResumeState) + Send + Sync>;

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

pub async fn list_google_drive_items(
    access_token: &str,
    parent_id: Option<&str>,
    drive_id: Option<&str>,
) -> Result<Vec<GoogleDriveItem>> {
    let client = Client::new();
    let parent = parent_id
        .filter(|value| !value.trim().is_empty())
        .or(drive_id.filter(|value| !value.trim().is_empty()))
        .unwrap_or("root");

    let query = format!(
        "'{}' in parents and trashed = false",
        escape_drive_query_literal(parent)
    );

    let mut page_token: Option<String> = None;
    let mut items = Vec::new();

    loop {
        let mut request = client
            .get(DRIVE_FILES_URL)
            .bearer_auth(access_token)
            .query(&[
                ("q", query.as_str()),
                ("spaces", "drive"),
                ("supportsAllDrives", "true"),
                ("includeItemsFromAllDrives", "true"),
                ("pageSize", "1000"),
                (
                    "fields",
                    "nextPageToken,files(id,name,mimeType,size,driveId,resourceKey,md5Checksum,capabilities(canDownload))",
                ),
                ("orderBy", "folder,name_natural"),
            ]);

        if let Some(drive_id) = drive_id.filter(|value| !value.trim().is_empty()) {
            request = request.query(&[("corpora", "drive"), ("driveId", drive_id)]);
        } else {
            request = request.query(&[("corpora", "user")]);
        }

        if let Some(token) = page_token.as_deref() {
            request = request.query(&[("pageToken", token)]);
        }

        let response = request
            .send()
            .await
            .context("failed to list Google Drive items")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Google Drive item list failed ({status}): {body}");
        }

        let page: GoogleDriveItemListResponse = response.json().await?;
        items.extend(
            page.files
                .into_iter()
                .map(|item| to_drive_item(item, None)),
        );

        page_token = page.next_page_token;
        if page_token.is_none() {
            break;
        }
    }

    Ok(items)
}

pub async fn resolve_google_drive_shared_link(
    access_token: &str,
    shared_link: &str,
) -> Result<GoogleDriveItem> {
    let (file_id, resource_key) = parse_google_drive_shared_link(shared_link)?;
    let client = Client::new();
    let endpoint = format!("{DRIVE_FILES_URL}/{file_id}");

    let mut request = client
        .get(endpoint)
        .bearer_auth(access_token)
        .query(&[
            ("supportsAllDrives", "true"),
            (
                "fields",
                "id,name,mimeType,size,driveId,resourceKey,md5Checksum,capabilities(canDownload)",
            ),
        ]);

    if let Some(resource_key) = resource_key.as_deref() {
        request = request.header(
            "X-Goog-Drive-Resource-Keys",
            format!("{file_id}/{resource_key}"),
        );
    }

    let response = request
        .send()
        .await
        .context("failed to resolve Google Drive shared link")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("Google Drive shared link lookup failed ({status}): {body}");
    }

    let item: GoogleDriveItemResponse = response.json().await?;
    Ok(to_drive_item(item, resource_key))
}

pub fn parse_google_drive_shared_link(shared_link: &str) -> Result<(String, Option<String>)> {
    let trimmed = shared_link.trim();

    if !trimmed.contains("://")
        && !trimmed.contains('/')
        && !trimmed.contains('?')
        && trimmed.len() >= 10
    {
        return Ok((trimmed.to_string(), None));
    }

    let url = Url::parse(trimmed).context("invalid Google Drive shared link")?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();

    if !(host == "drive.google.com"
        || host == "docs.google.com"
        || host == "sheets.google.com"
        || host == "slides.google.com")
    {
        bail!("link is not a recognized Google Drive/Workspace URL");
    }

    let resource_key = url
        .query_pairs()
        .find(|(key, _)| key.eq_ignore_ascii_case("resourcekey"))
        .map(|(_, value)| value.into_owned())
        .filter(|value| !value.is_empty());

    let query_id = url
        .query_pairs()
        .find(|(key, _)| key == "id")
        .map(|(_, value)| value.into_owned())
        .filter(|value| !value.is_empty());

    if let Some(file_id) = query_id {
        return Ok((file_id, resource_key));
    }

    let segments: Vec<&str> = url
        .path_segments()
        .map(|segments| segments.filter(|segment| !segment.is_empty()).collect())
        .unwrap_or_default();

    for marker in ["d", "folders"] {
        if let Some(index) = segments.iter().position(|segment| *segment == marker) {
            if let Some(file_id) = segments.get(index + 1).filter(|value| !value.is_empty()) {
                return Ok(((*file_id).to_string(), resource_key));
            }
        }
    }

    bail!("could not extract a Google Drive file or folder ID from the link")
}

fn to_drive_item(
    item: GoogleDriveItemResponse,
    fallback_resource_key: Option<String>,
) -> GoogleDriveItem {
    let is_folder = item.mime_type == DRIVE_FOLDER_MIME;
    let is_google_workspace =
        item.mime_type.starts_with("application/vnd.google-apps.") && !is_folder;

    GoogleDriveItem {
        id: item.id,
        name: item.name,
        mime_type: item.mime_type,
        size: item.size.and_then(|value| value.parse::<u64>().ok()),
        drive_id: item.drive_id,
        resource_key: item.resource_key.or(fallback_resource_key),
        can_download: item.capabilities.can_download,
        md5_checksum: item.md5_checksum,
        is_folder,
        is_google_workspace,
    }
}

pub fn google_drive_media_url(file_id: &str) -> String {
    format!(
        "https://www.googleapis.com/drive/v3/files/{}?alt=media&supportsAllDrives=true",
        file_id.trim()
    )
}


pub fn google_drive_export_formats(source_mime: &str) -> Vec<GoogleDriveExportFormat> {
    fn format(label: &str, mime_type: &str, extension: &str) -> GoogleDriveExportFormat {
        GoogleDriveExportFormat {
            label: label.to_string(),
            mime_type: mime_type.to_string(),
            extension: extension.to_string(),
        }
    }

    match source_mime {
        "application/vnd.google-apps.document" => vec![
            format(
                "Microsoft Word (.docx)",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                ".docx",
            ),
            format("PDF (.pdf)", "application/pdf", ".pdf"),
            format("Texto (.txt)", "text/plain", ".txt"),
            format("Markdown (.md)", "text/markdown", ".md"),
            format("EPUB (.epub)", "application/epub+zip", ".epub"),
        ],
        "application/vnd.google-apps.spreadsheet" => vec![
            format(
                "Microsoft Excel (.xlsx)",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                ".xlsx",
            ),
            format("PDF (.pdf)", "application/pdf", ".pdf"),
            format("CSV - primeira planilha (.csv)", "text/csv", ".csv"),
        ],
        "application/vnd.google-apps.presentation" => vec![
            format(
                "Microsoft PowerPoint (.pptx)",
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
                ".pptx",
            ),
            format("PDF (.pdf)", "application/pdf", ".pdf"),
            format("Texto (.txt)", "text/plain", ".txt"),
        ],
        "application/vnd.google-apps.drawing" => vec![
            format("PDF (.pdf)", "application/pdf", ".pdf"),
            format("PNG (.png)", "image/png", ".png"),
            format("JPEG (.jpg)", "image/jpeg", ".jpg"),
            format("SVG (.svg)", "image/svg+xml", ".svg"),
        ],
        "application/vnd.google-apps.script" => vec![
            format(
                "Apps Script JSON (.json)",
                "application/vnd.google-apps.script+json",
                ".json",
            ),
        ],
        _ => Vec::new(),
    }
}

pub fn google_drive_export_url(file_id: &str, export_mime: &str) -> Result<String> {
    let mut url = Url::parse(&format!(
        "https://www.googleapis.com/drive/v3/files/{}/export",
        file_id.trim()
    ))?;
    url.query_pairs_mut().append_pair("mimeType", export_mime);
    Ok(url.to_string())
}

pub async fn list_google_drive_folders(
    access_token: &str,
    parent_id: Option<&str>,
    drive_id: Option<&str>,
) -> Result<Vec<GoogleDriveFolder>> {
    let client = Client::new();
    let parent = parent_id
        .filter(|value| !value.trim().is_empty())
        .or(drive_id.filter(|value| !value.trim().is_empty()))
        .unwrap_or("root");

    let query = format!(
        "'{}' in parents and trashed = false and mimeType = '{}'",
        escape_drive_query_literal(parent),
        DRIVE_FOLDER_MIME
    );

    let mut page_token: Option<String> = None;
    let mut folders = Vec::new();

    loop {
        let mut request = client
            .get(DRIVE_FILES_URL)
            .bearer_auth(access_token)
            .query(&[
                ("q", query.as_str()),
                ("spaces", "drive"),
                ("supportsAllDrives", "true"),
                ("includeItemsFromAllDrives", "true"),
                ("pageSize", "1000"),
                ("fields", "nextPageToken,files(id,name,driveId)"),
                ("orderBy", "name_natural"),
            ]);

        if let Some(drive_id) = drive_id.filter(|value| !value.trim().is_empty()) {
            request = request.query(&[("corpora", "drive"), ("driveId", drive_id)]);
        } else {
            request = request.query(&[("corpora", "user")]);
        }

        if let Some(token) = page_token.as_deref() {
            request = request.query(&[("pageToken", token)]);
        }

        let response = request
            .send()
            .await
            .context("failed to list Google Drive folders")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Google Drive folder list failed ({status}): {body}");
        }

        let page: GoogleDriveFolderListResponse = response.json().await?;
        folders.extend(page.files.into_iter().map(|folder| GoogleDriveFolder {
            id: folder.id,
            name: folder.name,
            drive_id: folder.drive_id,
        }));

        page_token = page.next_page_token;
        if page_token.is_none() {
            break;
        }
    }

    Ok(folders)
}

pub async fn list_google_shared_drives(
    access_token: &str,
) -> Result<Vec<GoogleSharedDrive>> {
    let client = Client::new();
    let mut page_token: Option<String> = None;
    let mut drives = Vec::new();

    loop {
        let mut request = client
            .get(DRIVE_DRIVES_URL)
            .bearer_auth(access_token)
            .query(&[
                ("pageSize", "100"),
                ("fields", "nextPageToken,drives(id,name)"),
            ]);

        if let Some(token) = page_token.as_deref() {
            request = request.query(&[("pageToken", token)]);
        }

        let response = request
            .send()
            .await
            .context("failed to list shared Google Drives")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Google shared-drive list failed ({status}): {body}");
        }

        let page: GoogleSharedDriveListResponse = response.json().await?;
        drives.extend(page.drives);

        page_token = page.next_page_token;
        if page_token.is_none() {
            break;
        }
    }

    Ok(drives)
}

fn escape_drive_query_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
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

    let throttle = TransferThrottle::new(request.max_bytes_per_second);
    upload_google_drive_file_with_pool(
        request,
        AdaptiveLinkPool::new(vec![link]),
        throttle,
        transfer_id,
        progress,
        control,
        None,
        None,
    )
    .await
}

async fn upload_google_drive_file_with_pool(
    request: GoogleDriveUploadRequest,
    pool: AdaptiveLinkPool,
    throttle: TransferThrottle,
    transfer_id: String,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
    resume: Option<GoogleDriveUploadResumeState>,
    checkpoint_callback: Option<DriveUploadCheckpointCallback>,
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

    let source_key = request.source.to_string_lossy().to_string();
    let mut resume_offset = 0u64;
    let mut resumed = false;

    let (session_uri, session_link) = if let Some(saved) = resume
        .filter(|saved| {
            !saved.completed
                && saved.source == source_key
                && saved.total_size == total_size
                && saved.remote_name == remote_name
                && saved.mime_type == mime_type
                && saved.chunk_size == request.chunk_size
        })
    {
        match query_upload_offset_with_failover(&pool, &saved.session_uri, total_size).await {
            Ok((offset, link)) => {
                resume_offset = offset.min(total_size);
                resumed = true;
                (saved.session_uri, link)
            }
            Err(_) => create_resumable_session_with_failover(
                &pool,
                &request.access_token,
                request.parent_id.as_deref(),
                &remote_name,
                &mime_type,
                total_size,
                control.as_ref(),
            )
            .await?,
        }
    } else {
        create_resumable_session_with_failover(
            &pool,
            &request.access_token,
            request.parent_id.as_deref(),
            &remote_name,
            &mime_type,
            total_size,
            control.as_ref(),
        )
        .await?
    };

    emit_upload_checkpoint(
        checkpoint_callback.as_ref(),
        &source_key,
        &session_uri,
        resume_offset,
        total_size,
        &remote_name,
        &mime_type,
        request.parent_id.clone(),
        request.chunk_size,
        resume_offset == total_size,
    );

    emit_upload_progress(
        progress.as_ref(),
        &transfer_id,
        &remote_name,
        if resumed { "resumed" } else { "starting" },
        0,
        resume_offset,
        total_size,
        &session_link,
        resume_offset == total_size,
    );

    if resume_offset == total_size && total_size > 0 {
        return Ok(GoogleDriveUploadResult {
            id: String::new(),
            name: remote_name,
            size: total_size,
            web_view_link: None,
            bytes_uploaded: total_size,
            link_used: session_link.name,
            local_ip: session_link.local_ip,
        });
    }

    if total_size == 0 {
        return upload_empty_file(
            &pool,
            &session_uri,
            &source_key,
            &remote_name,
            &mime_type,
            request.parent_id.clone(),
            request.chunk_size,
            &transfer_id,
            progress,
            control,
            checkpoint_callback,
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
        resume_offset,
        &source_key,
        request.parent_id.clone(),
        &transfer_id,
        progress,
        control,
        throttle,
        checkpoint_callback,
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
    upload_google_drive_batch_resumable_with_control(
        request,
        transfer_id,
        progress,
        control,
        HashMap::new(),
        None,
    )
    .await
}

pub async fn upload_google_drive_batch_resumable_with_control(
    request: GoogleDriveBatchUploadRequest,
    transfer_id: String,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
    resume_sessions: HashMap<String, GoogleDriveUploadResumeState>,
    checkpoint_callback: Option<DriveUploadCheckpointCallback>,
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
    let throttle = TransferThrottle::new(request.max_bytes_per_second);
    let mut pending: VecDeque<(usize, PathBuf)> =
        request.files.into_iter().enumerate().collect();
    let mut jobs: JoinSet<Result<(usize, GoogleDriveUploadResult)>> = JoinSet::new();

    while jobs.len() < max_parallel {
        let Some((index, source)) = pending.pop_front() else {
            break;
        };

        let resume = resume_sessions
            .get(&source.to_string_lossy().to_string())
            .cloned();
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
            throttle.clone(),
            resume,
            checkpoint_callback.clone(),
        );
    }

    let mut results = Vec::new();

    while let Some(joined) = jobs.join_next().await {
        results.push(joined??);

        if let Some((index, source)) = pending.pop_front() {
            let resume = resume_sessions
                .get(&source.to_string_lossy().to_string())
                .cloned();
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
                throttle.clone(),
                resume,
                checkpoint_callback.clone(),
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
    throttle: TransferThrottle,
    resume: Option<GoogleDriveUploadResumeState>,
    checkpoint_callback: Option<DriveUploadCheckpointCallback>,
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
                max_bytes_per_second: None,
            },
            pool,
            throttle,
            transfer_id,
            progress,
            control,
            resume,
            checkpoint_callback,
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
    source: &str,
    remote_name: &str,
    mime_type: &str,
    parent_id: Option<String>,
    chunk_size: u64,
    transfer_id: &str,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
    checkpoint_callback: Option<DriveUploadCheckpointCallback>,
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
                emit_upload_checkpoint(
                    checkpoint_callback.as_ref(),
                    source,
                    session_uri,
                    0,
                    0,
                    remote_name,
                    mime_type,
                    parent_id.clone(),
                    chunk_size,
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
    initial_offset: u64,
    source_key: &str,
    parent_id: Option<String>,
    transfer_id: &str,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
    throttle: TransferThrottle,
    checkpoint_callback: Option<DriveUploadCheckpointCallback>,
) -> Result<GoogleDriveUploadResult> {
    let mut file = File::open(source).await?;
    let mut offset = initial_offset.min(total_size);
    let mut reported_offset = offset;
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

            throttle.consume(length).await;

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
                    emit_upload_checkpoint(
                        checkpoint_callback.as_ref(),
                        source_key,
                        session_uri,
                        total_size,
                        total_size,
                        remote_name,
                        mime_type,
                        parent_id.clone(),
                        chunk_size,
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
                    emit_upload_checkpoint(
                        checkpoint_callback.as_ref(),
                        source_key,
                        session_uri,
                        next_offset,
                        total_size,
                        remote_name,
                        mime_type,
                        parent_id.clone(),
                        chunk_size,
                        false,
                    );

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
                    emit_upload_checkpoint(
                        checkpoint_callback.as_ref(),
                        source_key,
                        session_uri,
                        known_offset,
                        total_size,
                        remote_name,
                        mime_type,
                        parent_id.clone(),
                        chunk_size,
                        false,
                    );

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
                    emit_upload_checkpoint(
                        checkpoint_callback.as_ref(),
                        source_key,
                        session_uri,
                        known_offset,
                        total_size,
                        remote_name,
                        mime_type,
                        parent_id.clone(),
                        chunk_size,
                        false,
                    );

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

fn emit_upload_checkpoint(
    callback: Option<&DriveUploadCheckpointCallback>,
    source: &str,
    session_uri: &str,
    confirmed_offset: u64,
    total_size: u64,
    remote_name: &str,
    mime_type: &str,
    parent_id: Option<String>,
    chunk_size: u64,
    completed: bool,
) {
    if let Some(callback) = callback {
        callback(GoogleDriveUploadResumeState {
            source: source.to_owned(),
            session_uri: session_uri.to_owned(),
            confirmed_offset,
            total_size,
            remote_name: remote_name.to_owned(),
            mime_type: mime_type.to_owned(),
            parent_id,
            chunk_size,
            completed,
        });
    }
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
    use super::{
        escape_drive_query_literal, google_drive_export_formats, google_drive_export_url,
        google_drive_media_url, next_offset_from_range, parse_google_drive_shared_link,
        validate_chunk_size,
    };
    use reqwest::header::HeaderValue;

    #[test]
    fn parses_common_drive_shared_links_and_resource_keys() {
        let (id, key) = parse_google_drive_shared_link(
            "https://drive.google.com/file/d/ABC123/view?usp=sharing&resourcekey=RK999",
        )
        .unwrap();
        assert_eq!(id, "ABC123");
        assert_eq!(key.as_deref(), Some("RK999"));

        let (id, _) = parse_google_drive_shared_link(
            "https://docs.google.com/document/d/DOC777/edit",
        )
        .unwrap();
        assert_eq!(id, "DOC777");

        let (id, _) = parse_google_drive_shared_link(
            "https://drive.google.com/drive/folders/FOLDER42?usp=sharing",
        )
        .unwrap();
        assert_eq!(id, "FOLDER42");

        let (id, _) =
            parse_google_drive_shared_link("https://drive.google.com/open?id=OPEN88").unwrap();
        assert_eq!(id, "OPEN88");
    }

    #[test]
    fn workspace_export_formats_include_office_defaults() {
        let docs = google_drive_export_formats("application/vnd.google-apps.document");
        assert_eq!(docs.first().unwrap().extension, ".docx");

        let sheets = google_drive_export_formats("application/vnd.google-apps.spreadsheet");
        assert_eq!(sheets.first().unwrap().extension, ".xlsx");

        let slides = google_drive_export_formats("application/vnd.google-apps.presentation");
        assert_eq!(slides.first().unwrap().extension, ".pptx");
    }

    #[test]
    fn export_url_encodes_mime_type() {
        let url = google_drive_export_url("abc123", "application/pdf").unwrap();
        assert!(url.contains("/abc123/export?"));
        assert!(url.contains("mimeType=application%2Fpdf"));
    }

    #[test]
    fn media_url_targets_drive_content_endpoint() {
        assert_eq!(
            google_drive_media_url("abc123"),
            "https://www.googleapis.com/drive/v3/files/abc123?alt=media&supportsAllDrives=true"
        );
    }

    #[test]
    fn drive_query_literals_are_escaped() {
        assert_eq!(escape_drive_query_literal("abc'def"), "abc\\'def");
        assert_eq!(escape_drive_query_literal("a\\b"), "a\\\\b");
    }

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
