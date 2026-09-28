#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod transfer_store;

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env,
    net::IpAddr,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use stordown_core::{
    authorize_google_drive_desktop, download_direct_with_control, download_with_control,
    google_drive_export_formats, google_drive_export_url, google_drive_media_url,
    list_google_drive_folders, list_google_drive_items_with_resource_key,
    list_google_shared_drives, probe, probe_links,
    refresh_google_access_token, resolve_google_drive_shared_link,
    upload_google_drive_batch_resumable_with_control, DownloadRequest,
    DriveUploadCheckpointCallback,
    GoogleDriveBatchUploadRequest, GoogleDriveExportFormat, GoogleDriveFolder, GoogleDriveItem,
    GoogleDriveUploadResumeState, GoogleSharedDrive, LinkConfig, LinkProbeStatus, ProbeResult,
    ProgressCallback, TransferControl,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::{Mutex, Semaphore},
};
use transfer_store::{
    DownloadRule, DriveUploadSessionRecord, NewDownloadRule, NewDriveUploadSessionRecord,
    NewTransferRecord, TransferRecord, TransferStore,
};
use uuid::Uuid;

const GOOGLE_KEYRING_SERVICE: &str = "StorDown Google Drive";
const GOOGLE_UPLOAD_KEYRING_SERVICE: &str = "StorDown Google Drive Upload Session";
const DEFAULT_QUEUE_CONCURRENCY: usize = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NetworkInterfaceInfo {
    name: String,
    description: String,
    ipv4: String,
    gateway: Option<String>,
    link_speed: Option<String>,
    index: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct BrowserCaptureRequest {
    #[serde(rename = "type")]
    kind: String,
    url: Option<String>,
    filename: Option<String>,
    source: Option<String>,
    #[serde(default)]
    headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserCaptureResponse {
    ok: bool,
    transfer_id: Option<String>,
    status: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserIntegrationResult {
    manifest_path: String,
    native_host_path: String,
    chrome_registered: bool,
    edge_registered: bool,
}

#[derive(Debug, Clone, Serialize)]
struct DesktopDefaults {
    download_dir: String,
    interfaces: Vec<NetworkInterfaceInfo>,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserExtensionPrepared {
    extension_dir: String,
}

#[derive(Debug, Clone)]
struct GoogleSession {
    client_id: String,
    access_token: String,
    refresh_token: String,
    scope: Option<String>,
    expires_at: Instant,
}

#[derive(Default)]
struct GoogleAuthState {
    session: Mutex<Option<GoogleSession>>,
}

#[derive(Debug, Clone, Serialize)]
struct GoogleAuthStatus {
    connected: bool,
    client_id: Option<String>,
    scope: Option<String>,
    expires_in_seconds: Option<u64>,
}

#[derive(Clone, Default)]
struct TransferControlState {
    controls: Arc<Mutex<HashMap<String, TransferControl>>>,
}

#[derive(Debug, Clone, Serialize)]
struct TransferControlStatus {
    transfer_id: String,
    paused: bool,
    cancelled: bool,
}

#[derive(Clone)]
struct QueueState {
    slots: Arc<Semaphore>,
}

impl Default for QueueState {
    fn default() -> Self {
        Self {
            slots: Arc::new(Semaphore::new(DEFAULT_QUEUE_CONCURRENCY)),
        }
    }
}

async fn register_transfer(
    transfer_id: &str,
    state: &TransferControlState,
) -> TransferControl {
    let control = TransferControl::default();
    state
        .controls
        .lock()
        .await
        .insert(transfer_id.to_string(), control.clone());
    control
}

async fn remove_transfer(transfer_id: &str, state: &TransferControlState) {
    state.controls.lock().await.remove(transfer_id);
}

async fn get_transfer_control(
    transfer_id: &str,
    state: &TransferControlState,
) -> Result<TransferControl, String> {
    state
        .controls
        .lock()
        .await
        .get(transfer_id)
        .cloned()
        .ok_or_else(|| format!("Transferência não encontrada: {transfer_id}"))
}

fn progress_emitter(app: AppHandle, store: TransferStore) -> ProgressCallback {
    Arc::new(move |progress| {
        let _ = app.emit("transfer-progress", progress.clone());

        let store = store.clone();
        let transfer_id = progress.transfer_id.clone();
        let bytes_transferred = progress.bytes_transferred;
        let total_bytes = progress.total_bytes;

        tauri::async_runtime::spawn(async move {
            let _ = store
                .update_progress(&transfer_id, bytes_transferred, total_bytes)
                .await;
        });
    })
}

fn notify_transfer_list(app: &AppHandle, transfer_id: &str) {
    let _ = app.emit("transfer-list-changed", transfer_id.to_string());
}

fn parse_links(bind_ips: Vec<String>) -> Result<Vec<LinkConfig>, String> {
    if bind_ips.is_empty() {
        return Err("Informe pelo menos um IP/interface de rede".to_string());
    }

    bind_ips
        .into_iter()
        .enumerate()
        .map(|(index, raw)| {
            let local_ip: IpAddr = raw
                .parse()
                .map_err(|_| format!("IP inválido: {raw}"))?;

            Ok(LinkConfig {
                name: format!("Ethernet {}", index + 1),
                local_ip,
                enabled: true,
                weight: 1,
            })
        })
        .collect()
}

fn resolve_google_client_id(input: Option<String>) -> Result<String, String> {
    if let Some(value) = input.filter(|value| !value.trim().is_empty()) {
        return Ok(value.trim().to_string());
    }

    if let Some(value) = env::var("STORDOWN_GOOGLE_CLIENT_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(value);
    }

    if let Some(value) = option_env!("STORDOWN_GOOGLE_CLIENT_ID")
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(value.trim().to_string());
    }

    Err(
        "Google OAuth Client ID não configurado nesta build. Informe o Client ID na tela do Google Drive."
            .to_string(),
    )
}

fn credential_entry(client_id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(GOOGLE_KEYRING_SERVICE, client_id)
        .map_err(|error| format!("Falha ao acessar o armazenamento seguro do Windows: {error}"))
}

fn upload_session_entry(key: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(GOOGLE_UPLOAD_KEYRING_SERVICE, key)
        .map_err(|error| format!("Falha ao acessar sessão segura de upload: {error}"))
}

fn drive_checkpoint_emitter(
    transfer_id: String,
    store: TransferStore,
    credential_keys: Arc<HashMap<String, String>>,
) -> DriveUploadCheckpointCallback {
    Arc::new(move |checkpoint| {
        let Some(credential_key) = credential_keys.get(&checkpoint.source).cloned() else {
            return;
        };

        let transfer_id = transfer_id.clone();
        let store = store.clone();

        tauri::async_runtime::spawn(async move {
            if !checkpoint.completed {
                if let Ok(entry) = upload_session_entry(&credential_key) {
                    let _ = entry.set_password(&checkpoint.session_uri);
                }
            }

            let _ = store
                .update_drive_upload_checkpoint(
                    &transfer_id,
                    &checkpoint.source,
                    checkpoint.confirmed_offset,
                    checkpoint.total_size,
                    &checkpoint.remote_name,
                    &checkpoint.mime_type,
                    checkpoint.parent_id.as_deref(),
                    checkpoint.chunk_size,
                    checkpoint.completed,
                )
                .await;

            if checkpoint.completed {
                if let Ok(entry) = upload_session_entry(&credential_key) {
                    let _ = entry.delete_credential();
                }
            }
        });
    })
}

async fn cleanup_drive_upload_credentials(
    store: &TransferStore,
    transfer_id: &str,
) -> Result<(), String> {
    for session in store.list_drive_upload_sessions(transfer_id).await? {
        if let Ok(entry) = upload_session_entry(&session.credential_key) {
            let _ = entry.delete_credential();
        }
    }

    Ok(())
}

fn drive_resume_state(
    session: &DriveUploadSessionRecord,
) -> Option<GoogleDriveUploadResumeState> {
    if session.completed {
        return None;
    }

    let session_uri = upload_session_entry(&session.credential_key)
        .ok()?
        .get_password()
        .ok()?;

    Some(GoogleDriveUploadResumeState {
        source: session.source_path.clone(),
        session_uri,
        confirmed_offset: session.confirmed_offset,
        total_size: session.total_size,
        remote_name: session.remote_name.clone(),
        mime_type: session.mime_type.clone(),
        parent_id: session.parent_id.clone(),
        chunk_size: session.chunk_size,
        completed: session.completed,
    })
}

fn auth_status(session: Option<&GoogleSession>) -> GoogleAuthStatus {
    match session {
        Some(session) => GoogleAuthStatus {
            connected: true,
            client_id: Some(session.client_id.clone()),
            scope: session.scope.clone(),
            expires_in_seconds: Some(
                session
                    .expires_at
                    .saturating_duration_since(Instant::now())
                    .as_secs(),
            ),
        },
        None => GoogleAuthStatus {
            connected: false,
            client_id: None,
            scope: None,
            expires_in_seconds: None,
        },
    }
}

#[tauri::command]
async fn connect_google_drive(
    client_id: Option<String>,
    state: State<'_, GoogleAuthState>,
) -> Result<GoogleAuthStatus, String> {
    let client_id = resolve_google_client_id(client_id)?;
    let tokens = authorize_google_drive_desktop(&client_id)
        .await
        .map_err(|error| error.to_string())?;

    let refresh_token = match tokens.refresh_token {
        Some(token) => {
            credential_entry(&client_id)?
                .set_password(&token)
                .map_err(|error| format!("Falha ao salvar credencial Google com segurança: {error}"))?;
            token
        }
        None => credential_entry(&client_id)?
            .get_password()
            .map_err(|_| {
                "O Google não retornou refresh token e não existe uma credencial salva. Tente conectar novamente."
                    .to_string()
            })?,
    };

    let session = GoogleSession {
        client_id,
        access_token: tokens.access_token,
        refresh_token,
        scope: tokens.scope,
        expires_at: Instant::now() + Duration::from_secs(tokens.expires_in.saturating_sub(30)),
    };

    let status = auth_status(Some(&session));
    *state.session.lock().await = Some(session);
    Ok(status)
}

#[tauri::command]
async fn restore_google_drive(
    client_id: Option<String>,
    state: State<'_, GoogleAuthState>,
) -> Result<GoogleAuthStatus, String> {
    let client_id = resolve_google_client_id(client_id)?;
    let refresh_token = credential_entry(&client_id)?
        .get_password()
        .map_err(|_| "Nenhuma sessão Google Drive salva neste Windows".to_string())?;

    let tokens = refresh_google_access_token(&client_id, &refresh_token)
        .await
        .map_err(|error| error.to_string())?;

    let session = GoogleSession {
        client_id,
        access_token: tokens.access_token,
        refresh_token,
        scope: tokens.scope,
        expires_at: Instant::now() + Duration::from_secs(tokens.expires_in.saturating_sub(30)),
    };

    let status = auth_status(Some(&session));
    *state.session.lock().await = Some(session);
    Ok(status)
}

#[tauri::command]
async fn google_drive_auth_status(
    state: State<'_, GoogleAuthState>,
) -> Result<GoogleAuthStatus, String> {
    let guard = state.session.lock().await;
    Ok(auth_status(guard.as_ref()))
}

#[tauri::command]
async fn disconnect_google_drive(
    client_id: Option<String>,
    state: State<'_, GoogleAuthState>,
) -> Result<GoogleAuthStatus, String> {
    let client_id = {
        let guard = state.session.lock().await;
        guard
            .as_ref()
            .map(|session| session.client_id.clone())
            .or_else(|| client_id.filter(|value| !value.trim().is_empty()))
            .or_else(|| env::var("STORDOWN_GOOGLE_CLIENT_ID").ok())
    };

    *state.session.lock().await = None;

    if let Some(client_id) = client_id {
        if let Ok(entry) = credential_entry(&client_id) {
            let _ = entry.delete_credential();
        }
    }

    Ok(auth_status(None))
}

#[tauri::command]
async fn browse_google_drive_folders(
    parent_id: Option<String>,
    drive_id: Option<String>,
    state: State<'_, GoogleAuthState>,
) -> Result<Vec<GoogleDriveFolder>, String> {
    let access_token = current_google_access_token(state.inner()).await?;

    list_google_drive_folders(
        &access_token,
        parent_id.as_deref(),
        drive_id.as_deref(),
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn browse_google_drive_items(
    parent_id: Option<String>,
    drive_id: Option<String>,
    resource_key: Option<String>,
    state: State<'_, GoogleAuthState>,
) -> Result<Vec<GoogleDriveItem>, String> {
    let access_token = current_google_access_token(state.inner()).await?;

    list_google_drive_items_with_resource_key(
        &access_token,
        parent_id.as_deref(),
        drive_id.as_deref(),
        resource_key.as_deref(),
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn resolve_drive_shared_link(
    shared_link: String,
    state: State<'_, GoogleAuthState>,
) -> Result<GoogleDriveItem, String> {
    let access_token = current_google_access_token(state.inner()).await?;
    resolve_google_drive_shared_link(&access_token, &shared_link)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn list_google_drive_roots(
    state: State<'_, GoogleAuthState>,
) -> Result<Vec<GoogleSharedDrive>, String> {
    let access_token = current_google_access_token(state.inner()).await?;
    list_google_shared_drives(&access_token)
        .await
        .map_err(|error| error.to_string())
}

async fn current_google_access_token(state: &GoogleAuthState) -> Result<String, String> {
    let mut guard = state.session.lock().await;
    let session = guard
        .as_mut()
        .ok_or_else(|| "Conecte sua conta Google Drive antes de usar recursos de nuvem".to_string())?;

    if Instant::now() < session.expires_at {
        return Ok(session.access_token.clone());
    }

    let tokens = refresh_google_access_token(&session.client_id, &session.refresh_token)
        .await
        .map_err(|error| error.to_string())?;

    session.access_token = tokens.access_token;
    session.scope = tokens.scope.or_else(|| session.scope.clone());
    session.expires_at =
        Instant::now() + Duration::from_secs(tokens.expires_in.saturating_sub(30));

    Ok(session.access_token.clone())
}

fn mbps_to_bytes_per_second(mbps: Option<u64>) -> Option<u64> {
    mbps
        .filter(|value| *value > 0)
        .and_then(|value| value.checked_mul(1_000_000))
        .and_then(|value| value.checked_div(8))
}

fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

fn spawn_download_execution(
    url: String,
    output: String,
    connections: usize,
    bind_ips: Vec<String>,
    headers: HashMap<String, String>,
    max_bytes_per_second: Option<u64>,
    expected_sha256: Option<String>,
    transfer_id: String,
    scheduled_at: Option<i64>,
    app: AppHandle,
    queue_state: QueueState,
    control_state: TransferControlState,
    store: TransferStore,
) -> Result<(), String> {
    let links = parse_links(bind_ips)?;
    let control_state_for_task = control_state.clone();

    tauri::async_runtime::spawn(async move {
        let control = register_transfer(&transfer_id, &control_state_for_task).await;

        if let Some(start_at) = scheduled_at.filter(|value| *value > now_epoch_seconds()) {
            let wait_seconds = start_at.saturating_sub(now_epoch_seconds()) as u64;
            tokio::time::sleep(Duration::from_secs(wait_seconds)).await;

            if control.is_cancelled() {
                let _ = store.update_status(&transfer_id, "cancelled", None).await;
                notify_transfer_list(&app, &transfer_id);
                remove_transfer(&transfer_id, &control_state_for_task).await;
                return;
            }

            let _ = store.update_status(&transfer_id, "queued", None).await;
            notify_transfer_list(&app, &transfer_id);
        }

        let permit = queue_state.slots.acquire_owned().await;
        if permit.is_err() {
            let _ = store
                .update_status(&transfer_id, "failed", Some("Fila indisponível"))
                .await;
            notify_transfer_list(&app, &transfer_id);
            remove_transfer(&transfer_id, &control_state_for_task).await;
            return;
        }

        if control.is_cancelled() {
            let _ = store.update_status(&transfer_id, "cancelled", None).await;
            notify_transfer_list(&app, &transfer_id);
            remove_transfer(&transfer_id, &control_state_for_task).await;
            return;
        }

        let _ = store.update_status(&transfer_id, "running", None).await;
        notify_transfer_list(&app, &transfer_id);

        let result = download_with_control(
            DownloadRequest {
                url,
                output: PathBuf::from(output),
                connections,
                links,
                headers,
                max_bytes_per_second,
                expected_sha256,
            },
            transfer_id.clone(),
            Some(progress_emitter(app.clone(), store.clone())),
            Some(control.clone()),
        )
        .await;

        match result {
            Ok(result) => {
                let _ = store
                    .complete(&transfer_id, result.bytes_written, Some(result.bytes_written))
                    .await;
            }
            Err(_) if control.is_cancelled() => {
                let _ = store.update_status(&transfer_id, "cancelled", None).await;
            }
            Err(error) => {
                let message = error.to_string();
                let _ = store
                    .update_status(&transfer_id, "failed", Some(&message))
                    .await;
            }
        }

        notify_transfer_list(&app, &transfer_id);
        remove_transfer(&transfer_id, &control_state_for_task).await;
    });

    Ok(())
}

fn spawn_direct_download_execution(
    url: String,
    output: String,
    bind_ips: Vec<String>,
    headers: HashMap<String, String>,
    max_bytes_per_second: Option<u64>,
    transfer_id: String,
    app: AppHandle,
    queue_state: QueueState,
    control_state: TransferControlState,
    store: TransferStore,
) -> Result<(), String> {
    let links = parse_links(bind_ips)?;
    let control_state_for_task = control_state.clone();

    tauri::async_runtime::spawn(async move {
        let control = register_transfer(&transfer_id, &control_state_for_task).await;

        let permit = queue_state.slots.acquire_owned().await;
        if permit.is_err() {
            let _ = store
                .update_status(&transfer_id, "failed", Some("Fila indisponível"))
                .await;
            notify_transfer_list(&app, &transfer_id);
            remove_transfer(&transfer_id, &control_state_for_task).await;
            return;
        }

        if control.is_cancelled() {
            let _ = store.update_status(&transfer_id, "cancelled", None).await;
            notify_transfer_list(&app, &transfer_id);
            remove_transfer(&transfer_id, &control_state_for_task).await;
            return;
        }

        let _ = store.update_status(&transfer_id, "running", None).await;
        notify_transfer_list(&app, &transfer_id);

        let result = download_direct_with_control(
            DownloadRequest {
                url,
                output: PathBuf::from(output),
                connections: 1,
                links,
                headers,
                max_bytes_per_second,
                expected_sha256: None,
            },
            transfer_id.clone(),
            Some(progress_emitter(app.clone(), store.clone())),
            Some(control.clone()),
        )
        .await;

        match result {
            Ok(result) => {
                let _ = store
                    .complete(&transfer_id, result.bytes_written, Some(result.bytes_written))
                    .await;
            }
            Err(_) if control.is_cancelled() => {
                let _ = store.update_status(&transfer_id, "cancelled", None).await;
            }
            Err(error) => {
                let message = error.to_string();
                let _ = store
                    .update_status(&transfer_id, "failed", Some(&message))
                    .await;
            }
        }

        notify_transfer_list(&app, &transfer_id);
        remove_transfer(&transfer_id, &control_state_for_task).await;
    });

    Ok(())
}

async fn enqueue_download_job(
    url: String,
    output: String,
    connections: usize,
    bind_ips: Vec<String>,
    headers: HashMap<String, String>,
    max_bytes_per_second: Option<u64>,
    expected_sha256: Option<String>,
    transfer_id: String,
    scheduled_at: Option<i64>,
    app: AppHandle,
    queue_state: QueueState,
    control_state: TransferControlState,
    store: TransferStore,
) -> Result<TransferRecord, String> {
    parse_links(bind_ips.clone())?;
    let name = file_name_from_path(&output, "download");
    let normalized_schedule = scheduled_at.filter(|value| *value > now_epoch_seconds());

    let record = store
        .insert(NewTransferRecord {
            id: transfer_id.clone(),
            direction: "download".to_string(),
            name,
            source: url.clone(),
            destination: output.clone(),
            provider: "http".to_string(),
            connections,
            bind_ips: bind_ips.clone(),
            scheduled_at: normalized_schedule,
            max_bytes_per_second,
            expected_sha256: expected_sha256.clone(),
        })
        .await?;

    notify_transfer_list(&app, &transfer_id);

    spawn_download_execution(
        url,
        output,
        connections,
        bind_ips,
        headers,
        max_bytes_per_second,
        expected_sha256,
        transfer_id,
        normalized_schedule,
        app,
        queue_state,
        control_state,
        store,
    )?;

    Ok(record)
}

#[tauri::command]
async fn enqueue_download(
    url: String,
    output: String,
    connections: usize,
    bind_ips: Vec<String>,
    transfer_id: String,
    scheduled_at: Option<i64>,
    speed_limit_mbps: Option<u64>,
    expected_sha256: Option<String>,
    app: AppHandle,
    queue_state: State<'_, QueueState>,
    control_state: State<'_, TransferControlState>,
    store: State<'_, TransferStore>,
) -> Result<TransferRecord, String> {
    enqueue_download_job(
        url,
        output,
        connections,
        bind_ips,
        HashMap::new(),
        mbps_to_bytes_per_second(speed_limit_mbps),
        expected_sha256,
        transfer_id,
        scheduled_at,
        app,
        queue_state.inner().clone(),
        control_state.inner().clone(),
        store.inner().clone(),
    )
    .await
}

#[tauri::command]
fn google_drive_export_options(mime_type: String) -> Vec<GoogleDriveExportFormat> {
    google_drive_export_formats(&mime_type)
}

#[tauri::command]
async fn enqueue_drive_export(
    file_id: String,
    file_name: String,
    source_mime: String,
    export_mime: String,
    extension: String,
    resource_key: Option<String>,
    output: String,
    bind_ips: Vec<String>,
    speed_limit_mbps: Option<u64>,
    transfer_id: String,
    app: AppHandle,
    auth_state: State<'_, GoogleAuthState>,
    queue_state: State<'_, QueueState>,
    control_state: State<'_, TransferControlState>,
    store: State<'_, TransferStore>,
) -> Result<TransferRecord, String> {
    if file_id.trim().is_empty() {
        return Err("Documento Google Workspace sem ID".to_string());
    }

    let allowed = google_drive_export_formats(&source_mime);
    let selected = allowed.iter().find(|format| {
        format.mime_type == export_mime && format.extension == extension
    });

    if selected.is_none() {
        return Err("Formato de exportação não suportado para este documento".to_string());
    }

    parse_links(bind_ips.clone())?;
    let access_token = current_google_access_token(auth_state.inner()).await?;
    let export_url = google_drive_export_url(&file_id, &export_mime)
        .map_err(|error| error.to_string())?;
    let max_bytes_per_second = mbps_to_bytes_per_second(speed_limit_mbps);

    let mut headers = HashMap::new();
    headers.insert(
        "Authorization".to_string(),
        format!("Bearer {access_token}"),
    );
    if let Some(resource_key) = resource_key.filter(|value| !value.trim().is_empty()) {
        headers.insert(
            "X-Goog-Drive-Resource-Keys".to_string(),
            format!("{}/{}", file_id.trim(), resource_key.trim()),
        );
    }

    let name = file_name_from_path(&output, &format!("{file_name}{extension}"));

    let record = store
        .insert(NewTransferRecord {
            id: transfer_id.clone(),
            direction: "download".to_string(),
            name,
            source: format!(
                "google-workspace-export:{}:{}",
                file_id.trim(),
                export_mime
            ),
            destination: output.clone(),
            provider: "google_drive".to_string(),
            connections: 1,
            bind_ips: bind_ips.clone(),
            scheduled_at: None,
            max_bytes_per_second,
            expected_sha256: None,
        })
        .await?;

    notify_transfer_list(&app, &transfer_id);

    spawn_direct_download_execution(
        export_url,
        output,
        bind_ips,
        headers,
        max_bytes_per_second,
        transfer_id,
        app,
        queue_state.inner().clone(),
        control_state.inner().clone(),
        store.inner().clone(),
    )?;

    Ok(record)
}

#[tauri::command]
async fn enqueue_drive_download(
    file_id: String,
    file_name: String,
    mime_type: String,
    resource_key: Option<String>,
    output: String,
    connections: usize,
    bind_ips: Vec<String>,
    speed_limit_mbps: Option<u64>,
    transfer_id: String,
    app: AppHandle,
    auth_state: State<'_, GoogleAuthState>,
    queue_state: State<'_, QueueState>,
    control_state: State<'_, TransferControlState>,
    store: State<'_, TransferStore>,
) -> Result<TransferRecord, String> {
    if file_id.trim().is_empty() {
        return Err("Arquivo do Google Drive sem ID".to_string());
    }

    if mime_type.starts_with("application/vnd.google-apps.") {
        return Err(
            "Documentos Google Workspace precisam ser exportados; download Range direto é apenas para arquivos armazenados no Drive."
                .to_string(),
        );
    }

    parse_links(bind_ips.clone())?;
    let access_token = current_google_access_token(auth_state.inner()).await?;
    let media_url = google_drive_media_url(&file_id);
    let max_bytes_per_second = mbps_to_bytes_per_second(speed_limit_mbps);

    let mut headers = HashMap::new();
    headers.insert(
        "Authorization".to_string(),
        format!("Bearer {access_token}"),
    );
    if let Some(resource_key) = resource_key.filter(|value| !value.trim().is_empty()) {
        headers.insert(
            "X-Goog-Drive-Resource-Keys".to_string(),
            format!("{}/{}", file_id.trim(), resource_key.trim()),
        );
    }

    let record = store
        .insert(NewTransferRecord {
            id: transfer_id.clone(),
            direction: "download".to_string(),
            name: file_name,
            source: format!("google-drive:{}", file_id.trim()),
            destination: output.clone(),
            provider: "google_drive".to_string(),
            connections,
            bind_ips: bind_ips.clone(),
            scheduled_at: None,
            max_bytes_per_second,
            expected_sha256: None,
        })
        .await?;

    notify_transfer_list(&app, &transfer_id);

    spawn_download_execution(
        media_url,
        output,
        connections,
        bind_ips,
        headers,
        max_bytes_per_second,
        None,
        transfer_id,
        None,
        app,
        queue_state.inner().clone(),
        control_state.inner().clone(),
        store.inner().clone(),
    )?;

    Ok(record)
}

#[tauri::command]
async fn enqueue_drive_upload(
    files: Vec<String>,
    parent_id: Option<String>,
    bind_ips: Vec<String>,
    chunk_mib: u64,
    speed_limit_mbps: Option<u64>,
    transfer_id: String,
    app: AppHandle,
    auth_state: State<'_, GoogleAuthState>,
    queue_state: State<'_, QueueState>,
    control_state: State<'_, TransferControlState>,
    store: State<'_, TransferStore>,
) -> Result<TransferRecord, String> {
    if files.is_empty() {
        return Err("Adicione pelo menos um arquivo para upload".to_string());
    }

    let links = parse_links(bind_ips.clone())?;
    let access_token = current_google_access_token(auth_state.inner()).await?;
    let chunk_size = chunk_mib
        .checked_mul(1024 * 1024)
        .ok_or_else(|| "Tamanho de bloco inválido".to_string())?;
    let normalized_parent = parent_id.filter(|value| !value.trim().is_empty());

    let name = if files.len() == 1 {
        file_name_from_path(&files[0], "upload")
    } else {
        format!("{} arquivos para Google Drive", files.len())
    };
    let destination = normalized_parent
        .as_deref()
        .map(|value| format!("Google Drive / {value}"))
        .unwrap_or_else(|| "Google Drive / Meu Drive".to_string());

    let record = store
        .insert(NewTransferRecord {
            id: transfer_id.clone(),
            direction: "upload".to_string(),
            name,
            source: files.join("\n"),
            destination,
            provider: "google_drive".to_string(),
            connections: files.len().max(1),
            bind_ips,
            scheduled_at: None,
            max_bytes_per_second: mbps_to_bytes_per_second(speed_limit_mbps),
            expected_sha256: None,
        })
        .await?;

    let mut session_records = Vec::with_capacity(files.len());
    let mut credential_keys = HashMap::new();

    for (index, source) in files.iter().enumerate() {
        let metadata = tokio::fs::metadata(source)
            .await
            .map_err(|error| format!("Falha ao ler {source}: {error}"))?;

        if !metadata.is_file() {
            return Err(format!("{source} não é um arquivo regular"));
        }

        let credential_key = format!("{transfer_id}:{index}");
        credential_keys.insert(source.clone(), credential_key.clone());

        session_records.push(NewDriveUploadSessionRecord {
            transfer_id: transfer_id.clone(),
            source_path: source.clone(),
            credential_key,
            parent_id: normalized_parent.clone(),
            remote_name: file_name_from_path(source, "upload.bin"),
            mime_type: "application/octet-stream".to_string(),
            total_size: metadata.len(),
            chunk_size,
        });
    }

    if let Err(error) = store.insert_drive_upload_sessions(session_records).await {
        let _ = store.delete(&transfer_id).await;
        return Err(error);
    }

    let control = register_transfer(&transfer_id, control_state.inner()).await;
    let queue = queue_state.inner().clone();
    let controls = control_state.inner().clone();
    let task_store = store.inner().clone();
    let task_app = app.clone();
    let checkpoint = drive_checkpoint_emitter(
        transfer_id.clone(),
        task_store.clone(),
        Arc::new(credential_keys),
    );

    notify_transfer_list(&app, &transfer_id);

    tauri::async_runtime::spawn(async move {
        let permit = queue.slots.acquire_owned().await;
        if permit.is_err() {
            let _ = task_store
                .update_status(&transfer_id, "interrupted", Some("Fila indisponível"))
                .await;
            notify_transfer_list(&task_app, &transfer_id);
            remove_transfer(&transfer_id, &controls).await;
            return;
        }

        if control.is_cancelled() {
            let _ = task_store.update_status(&transfer_id, "cancelled", None).await;
            notify_transfer_list(&task_app, &transfer_id);
            remove_transfer(&transfer_id, &controls).await;
            return;
        }

        let _ = task_store.update_status(&transfer_id, "running", None).await;
        notify_transfer_list(&task_app, &transfer_id);

        let result = upload_google_drive_batch_resumable_with_control(
            GoogleDriveBatchUploadRequest {
                files: files.into_iter().map(PathBuf::from).collect(),
                access_token,
                parent_id: normalized_parent,
                chunk_size,
                links,
                max_bytes_per_second: mbps_to_bytes_per_second(speed_limit_mbps),
            },
            transfer_id.clone(),
            Some(progress_emitter(task_app.clone(), task_store.clone())),
            Some(control.clone()),
            HashMap::new(),
            Some(checkpoint),
        )
        .await;

        match result {
            Ok(results) => {
                let total = results.iter().map(|item| item.bytes_uploaded).sum();
                let _ = task_store.complete(&transfer_id, total, Some(total)).await;
            }
            Err(_) if control.is_cancelled() => {
                let _ = task_store.update_status(&transfer_id, "cancelled", None).await;
            }
            Err(error) => {
                let message = error.to_string();
                let _ = task_store
                    .update_status(&transfer_id, "interrupted", Some(&message))
                    .await;
            }
        }

        notify_transfer_list(&task_app, &transfer_id);
        remove_transfer(&transfer_id, &controls).await;
    });

    Ok(record)
}

#[tauri::command]
async fn resume_drive_upload(
    transfer_id: String,
    app: AppHandle,
    auth_state: State<'_, GoogleAuthState>,
    queue_state: State<'_, QueueState>,
    control_state: State<'_, TransferControlState>,
    store: State<'_, TransferStore>,
) -> Result<TransferRecord, String> {
    let record = store
        .get(&transfer_id)
        .await?
        .ok_or_else(|| "Transferência não encontrada".to_string())?;

    if record.provider != "google_drive"
        || record.direction != "upload"
        || record.status != "interrupted"
    {
        return Err("Esta transferência não é um upload do Drive interrompido".to_string());
    }

    let access_token = current_google_access_token(auth_state.inner()).await?;
    let links = parse_links(record.bind_ips.clone())?;
    let sessions = store.list_drive_upload_sessions(&transfer_id).await?;

    if sessions.is_empty() {
        return Err("Não há metadados de sessão salvos para este upload".to_string());
    }

    let pending: Vec<DriveUploadSessionRecord> =
        sessions.iter().filter(|session| !session.completed).cloned().collect();

    if pending.is_empty() {
        let total: u64 = sessions.iter().map(|session| session.total_size).sum();
        store.complete(&transfer_id, total, Some(total)).await?;
        notify_transfer_list(&app, &transfer_id);
        return store
            .get(&transfer_id)
            .await?
            .ok_or_else(|| "Transferência concluída não encontrada".to_string());
    }

    let chunk_size = pending[0].chunk_size;
    if pending.iter().any(|session| session.chunk_size != chunk_size) {
        return Err("Metadados de upload possuem tamanhos de bloco incompatíveis".to_string());
    }

    let parent_id = pending[0].parent_id.clone();
    if pending.iter().any(|session| session.parent_id != parent_id) {
        return Err("Metadados de upload possuem destinos incompatíveis".to_string());
    }

    let mut resume_sessions = HashMap::new();
    let mut credential_keys = HashMap::new();

    for session in &pending {
        credential_keys.insert(
            session.source_path.clone(),
            session.credential_key.clone(),
        );

        if let Some(resume) = drive_resume_state(session) {
            resume_sessions.insert(session.source_path.clone(), resume);
        }
    }

    let files: Vec<PathBuf> = pending
        .iter()
        .map(|session| PathBuf::from(&session.source_path))
        .collect();
    let already_completed: u64 = sessions
        .iter()
        .filter(|session| session.completed)
        .map(|session| session.total_size)
        .sum();

    let control = register_transfer(&transfer_id, control_state.inner()).await;
    let controls = control_state.inner().clone();
    let queue = queue_state.inner().clone();
    let task_store = store.inner().clone();
    let task_app = app.clone();
    let checkpoint = drive_checkpoint_emitter(
        transfer_id.clone(),
        task_store.clone(),
        Arc::new(credential_keys),
    );
    let speed_limit = record.max_bytes_per_second;

    store.update_status(&transfer_id, "queued", None).await?;
    notify_transfer_list(&app, &transfer_id);

    tauri::async_runtime::spawn(async move {
        let permit = queue.slots.acquire_owned().await;
        if permit.is_err() {
            let _ = task_store
                .update_status(&transfer_id, "interrupted", Some("Fila indisponível"))
                .await;
            notify_transfer_list(&task_app, &transfer_id);
            remove_transfer(&transfer_id, &controls).await;
            return;
        }

        if control.is_cancelled() {
            let _ = task_store.update_status(&transfer_id, "cancelled", None).await;
            notify_transfer_list(&task_app, &transfer_id);
            remove_transfer(&transfer_id, &controls).await;
            return;
        }

        let _ = task_store.update_status(&transfer_id, "running", None).await;
        notify_transfer_list(&task_app, &transfer_id);

        let result = upload_google_drive_batch_resumable_with_control(
            GoogleDriveBatchUploadRequest {
                files,
                access_token,
                parent_id,
                chunk_size,
                links,
                max_bytes_per_second: speed_limit,
            },
            transfer_id.clone(),
            Some(progress_emitter(task_app.clone(), task_store.clone())),
            Some(control.clone()),
            resume_sessions,
            Some(checkpoint),
        )
        .await;

        match result {
            Ok(results) => {
                let resumed_total: u64 =
                    results.iter().map(|item| item.bytes_uploaded).sum();
                let total = already_completed.saturating_add(resumed_total);
                let _ = task_store.complete(&transfer_id, total, Some(total)).await;
            }
            Err(_) if control.is_cancelled() => {
                let _ = task_store.update_status(&transfer_id, "cancelled", None).await;
            }
            Err(error) => {
                let message = error.to_string();
                let _ = task_store
                    .update_status(&transfer_id, "interrupted", Some(&message))
                    .await;
            }
        }

        notify_transfer_list(&task_app, &transfer_id);
        remove_transfer(&transfer_id, &controls).await;
    });

    store
        .get(&record.id)
        .await?
        .ok_or_else(|| "Transferência não encontrada após retomada".to_string())
}

#[tauri::command]
async fn list_transfers(
    limit: Option<usize>,
    store: State<'_, TransferStore>,
) -> Result<Vec<TransferRecord>, String> {
    store.list(limit.unwrap_or(250).clamp(1, 1000)).await
}

#[tauri::command]
async fn delete_transfer_history(
    transfer_id: String,
    control_state: State<'_, TransferControlState>,
    store: State<'_, TransferStore>,
) -> Result<(), String> {
    if control_state
        .controls
        .lock()
        .await
        .contains_key(&transfer_id)
    {
        return Err("Pare ou conclua a transferência antes de removê-la do histórico".to_string());
    }

    let _ = cleanup_drive_upload_credentials(store.inner(), &transfer_id).await;
    store.delete(&transfer_id).await
}

#[tauri::command]
async fn clear_finished_history(
    store: State<'_, TransferStore>,
) -> Result<usize, String> {
    if let Ok(records) = store.list(1000).await {
        for record in records {
            if matches!(record.status.as_str(), "completed" | "failed" | "cancelled") {
                let _ = cleanup_drive_upload_credentials(store.inner(), &record.id).await;
            }
        }
    }

    store.clear_finished().await
}

#[tauri::command]
async fn pause_transfer(
    transfer_id: String,
    state: State<'_, TransferControlState>,
    store: State<'_, TransferStore>,
    app: AppHandle,
) -> Result<TransferControlStatus, String> {
    let control = get_transfer_control(&transfer_id, state.inner()).await?;
    control.pause();
    store.update_status(&transfer_id, "paused", None).await?;
    notify_transfer_list(&app, &transfer_id);

    Ok(TransferControlStatus {
        transfer_id,
        paused: true,
        cancelled: control.is_cancelled(),
    })
}

#[tauri::command]
async fn resume_transfer(
    transfer_id: String,
    state: State<'_, TransferControlState>,
    store: State<'_, TransferStore>,
    app: AppHandle,
) -> Result<TransferControlStatus, String> {
    let control = get_transfer_control(&transfer_id, state.inner()).await?;
    control.resume();
    store.update_status(&transfer_id, "running", None).await?;
    notify_transfer_list(&app, &transfer_id);

    Ok(TransferControlStatus {
        transfer_id,
        paused: false,
        cancelled: control.is_cancelled(),
    })
}

#[tauri::command]
async fn cancel_transfer(
    transfer_id: String,
    state: State<'_, TransferControlState>,
    store: State<'_, TransferStore>,
    app: AppHandle,
) -> Result<TransferControlStatus, String> {
    let control = get_transfer_control(&transfer_id, state.inner()).await?;
    control.cancel();
    store.update_status(&transfer_id, "cancelled", None).await?;
    notify_transfer_list(&app, &transfer_id);

    Ok(TransferControlStatus {
        transfer_id,
        paused: false,
        cancelled: true,
    })
}

fn start_browser_capture_server(
    app: AppHandle,
    download_dir: PathBuf,
    queue_state: QueueState,
    control_state: TransferControlState,
    store: TransferStore,
) {
    tauri::async_runtime::spawn(async move {
        let listener = match TcpListener::bind("127.0.0.1:17832").await {
            Ok(listener) => listener,
            Err(error) => {
                let _ = app.emit(
                    "browser-capture-error",
                    format!("Falha ao iniciar integração do navegador: {error}"),
                );
                return;
            }
        };

        loop {
            let (socket, _) = match listener.accept().await {
                Ok(value) => value,
                Err(_) => continue,
            };

            let app = app.clone();
            let download_dir = download_dir.clone();
            let queue_state = queue_state.clone();
            let control_state = control_state.clone();
            let store = store.clone();

            tauri::async_runtime::spawn(async move {
                let (reader, mut writer) = socket.into_split();
                let mut reader = BufReader::new(reader);
                let mut line = String::new();

                let response = match reader.read_line(&mut line).await {
                    Ok(0) => BrowserCaptureResponse {
                        ok: false,
                        transfer_id: None,
                        status: None,
                        error: Some("Mensagem vazia do navegador".to_string()),
                    },
                    Ok(_) if line.len() > 64 * 1024 => BrowserCaptureResponse {
                        ok: false,
                        transfer_id: None,
                        status: None,
                        error: Some("Mensagem do navegador excede 64 KiB".to_string()),
                    },
                    Ok(_) => match serde_json::from_str::<BrowserCaptureRequest>(&line) {
                        Ok(request) => {
                            handle_browser_capture(
                                request,
                                app.clone(),
                                download_dir,
                                queue_state,
                                control_state,
                                store,
                            )
                            .await
                        }
                        Err(error) => BrowserCaptureResponse {
                            ok: false,
                            transfer_id: None,
                            status: None,
                            error: Some(format!("Mensagem inválida do navegador: {error}")),
                        },
                    },
                    Err(error) => BrowserCaptureResponse {
                        ok: false,
                        transfer_id: None,
                        status: None,
                        error: Some(format!("Falha ao ler mensagem do navegador: {error}")),
                    },
                };

                if let Ok(mut bytes) = serde_json::to_vec(&response) {
                    bytes.push(b'\n');
                    let _ = writer.write_all(&bytes).await;
                    let _ = writer.shutdown().await;
                }
            });
        }
    });
}

async fn handle_browser_capture(
    request: BrowserCaptureRequest,
    app: AppHandle,
    download_dir: PathBuf,
    queue_state: QueueState,
    control_state: TransferControlState,
    store: TransferStore,
) -> BrowserCaptureResponse {
    if request.kind == "ping" {
        return BrowserCaptureResponse {
            ok: true,
            transfer_id: None,
            status: Some("ready".to_string()),
            error: None,
        };
    }

    if request.kind != "download" {
        return BrowserCaptureResponse {
            ok: false,
            transfer_id: None,
            status: None,
            error: Some(format!("Tipo de captura não suportado: {}", request.kind)),
        };
    }

    let Some(url) = request.url.filter(|value| !value.trim().is_empty()) else {
        return BrowserCaptureResponse {
            ok: false,
            transfer_id: None,
            status: None,
            error: Some("URL ausente".to_string()),
        };
    };

    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return BrowserCaptureResponse {
            ok: false,
            transfer_id: None,
            status: None,
            error: Some("Somente URLs HTTP/HTTPS podem ser capturadas".to_string()),
        };
    }

    let interfaces = match tokio::task::spawn_blocking(discover_windows_interfaces).await {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            return BrowserCaptureResponse {
                ok: false,
                transfer_id: None,
                status: None,
                error: Some(error),
            };
        }
        Err(error) => {
            return BrowserCaptureResponse {
                ok: false,
                transfer_id: None,
                status: None,
                error: Some(format!("Falha ao detectar interfaces: {error}")),
            };
        }
    };

    let bind_ips: Vec<String> = interfaces.into_iter().map(|item| item.ipv4).collect();
    if bind_ips.is_empty() {
        return BrowserCaptureResponse {
            ok: false,
            transfer_id: None,
            status: None,
            error: Some("Nenhuma interface de rede ativa foi detectada".to_string()),
        };
    }

    let file_name = if request
        .filename
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        browser_capture_filename(&url, request.filename.as_deref())
    } else {
        match probe(&url).await {
            Ok(metadata) => metadata
                .suggested_name
                .map(|name| sanitize_windows_file_name(&name))
                .unwrap_or_else(|| browser_capture_filename(&url, None)),
            Err(_) => browser_capture_filename(&url, None),
        }
    };

    let destination_dir = match store.match_download_rule(&file_name).await {
        Ok(Some(rule)) => {
            let path = PathBuf::from(&rule.destination);
            if let Err(error) = tokio::fs::create_dir_all(&path).await {
                return BrowserCaptureResponse {
                    ok: false,
                    transfer_id: None,
                    status: None,
                    error: Some(format!(
                        "Falha ao criar pasta da categoria {}: {error}",
                        rule.name
                    )),
                };
            }
            path
        }
        Ok(None) => download_dir.clone(),
        Err(error) => {
            return BrowserCaptureResponse {
                ok: false,
                transfer_id: None,
                status: None,
                error: Some(format!("Falha ao aplicar regra de download: {error}")),
            };
        }
    };

    let output = unique_destination(&destination_dir, &file_name);
    let transfer_id = Uuid::new_v4().to_string();

    let auth_headers = sanitize_browser_headers(request.headers);

    match enqueue_download_job(
        url,
        output.to_string_lossy().to_string(),
        8,
        bind_ips,
        auth_headers,
        None,
        None,
        transfer_id.clone(),
        None,
        app.clone(),
        queue_state,
        control_state,
        store,
    )
    .await
    {
        Ok(_) => {
            let _ = app.emit(
                "browser-download-captured",
                format!(
                    "{}|{}",
                    request.source.unwrap_or_else(|| "browser".to_string()),
                    file_name
                ),
            );

            BrowserCaptureResponse {
                ok: true,
                transfer_id: Some(transfer_id),
                status: Some("queued".to_string()),
                error: None,
            }
        }
        Err(error) => BrowserCaptureResponse {
            ok: false,
            transfer_id: None,
            status: None,
            error: Some(error),
        },
    }
}

fn sanitize_browser_headers(
    headers: Option<HashMap<String, String>>,
) -> HashMap<String, String> {
    const ALLOWED: [&str; 4] = ["cookie", "referer", "origin", "user-agent"];

    headers
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(name, value)| {
            let normalized = name.trim().to_ascii_lowercase();
            let safe_name = ALLOWED.iter().any(|allowed| *allowed == normalized);
            let safe_value = !value.contains('\r')
                && !value.contains('\n')
                && value.len() <= 64 * 1024;

            if safe_name && safe_value {
                Some((normalized, value))
            } else {
                None
            }
        })
        .collect()
}

fn browser_capture_filename(url: &str, provided: Option<&str>) -> String {
    let candidate = provided
        .and_then(|value| Path::new(value).file_name())
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
            url.split('?')
                .next()
                .unwrap_or(url)
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "download.bin".to_string());

    sanitize_windows_file_name(&candidate)
}

fn sanitize_windows_file_name(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                ch
            }
        })
        .collect();

    let cleaned = cleaned.trim_matches(|ch| ch == ' ' || ch == '.');
    if cleaned.is_empty() {
        "download.bin".to_string()
    } else {
        cleaned.chars().take(180).collect()
    }
}

fn unique_destination(directory: &Path, file_name: &str) -> PathBuf {
    let initial = directory.join(file_name);
    if !initial.exists() {
        return initial;
    }

    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("download");
    let extension = path.extension().and_then(|value| value.to_str());

    for index in 1..10_000 {
        let candidate_name = match extension {
            Some(extension) if !extension.is_empty() => {
                format!("{stem} ({index}).{extension}")
            }
            _ => format!("{stem} ({index})"),
        };
        let candidate = directory.join(candidate_name);

        if !candidate.exists() {
            return candidate;
        }
    }

    directory.join(format!("{stem}-{}.bin", Uuid::new_v4()))
}

fn validate_extension_id(value: &str) -> Result<String, String> {
    let value = value.trim().to_ascii_lowercase();

    if value.len() != 32 || !value.chars().all(|ch| ('a'..='p').contains(&ch)) {
        return Err(
            "ID da extensão inválido. Copie o ID de 32 caracteres mostrado em chrome://extensions ou edge://extensions."
                .to_string(),
        );
    }

    Ok(value)
}

fn native_host_source_path(app: &AppHandle) -> Result<PathBuf, String> {
    let current = env::current_exe()
        .map_err(|error| format!("Falha ao localizar o executável do StorDown: {error}"))?;
    let directory = current
        .parent()
        .ok_or_else(|| "Pasta do StorDown não encontrada".to_string())?;
    let resource_dir = app.path().resource_dir().ok();

    let mut candidates = vec![
        directory.join("stordown-native-host.exe"),
        directory.join("resources").join("stordown-native-host.exe"),
    ];

    if let Some(resource_dir) = resource_dir {
        candidates.push(resource_dir.join("stordown-native-host.exe"));
        candidates.push(resource_dir.join("target").join("release").join("stordown-native-host.exe"));
    }

    candidates.push(
        directory
            .parent()
            .map(|parent| parent.join("stordown-native-host.exe"))
            .unwrap_or_default(),
    );

    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            "stordown-native-host.exe não foi encontrado no pacote do StorDown."
                .to_string()
        })
}

fn bundled_extension_source(app: &AppHandle) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();

    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("browser-extension"));
        candidates.push(resource_dir.join("resources").join("browser-extension"));
    }

    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../browser-extension"),
    );

    candidates
        .into_iter()
        .find(|path| path.join("manifest.json").is_file())
        .ok_or_else(|| "Extensão do StorDown não foi encontrada no pacote.".to_string())
}

fn copy_extension_files(source: &Path, destination: &Path) -> Result<(), String> {
    std::fs::create_dir_all(destination)
        .map_err(|error| format!("Falha ao criar pasta da extensão: {error}"))?;

    for entry in std::fs::read_dir(source)
        .map_err(|error| format!("Falha ao ler extensão empacotada: {error}"))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();

        if path.is_file() {
            std::fs::copy(&path, destination.join(entry.file_name()))
                .map_err(|error| format!("Falha ao copiar arquivo da extensão: {error}"))?;
        }
    }

    Ok(())
}

fn register_native_host_key(key: &str, manifest_path: &Path) -> Result<(), String> {
    let output = Command::new("reg.exe")
        .args([
            "ADD",
            key,
            "/ve",
            "/t",
            "REG_SZ",
            "/d",
            &manifest_path.to_string_lossy(),
            "/f",
        ])
        .output()
        .map_err(|error| format!("Falha ao executar reg.exe: {error}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[tauri::command]
fn install_browser_integration(
    extension_id: String,
    app: AppHandle,
) -> Result<BrowserIntegrationResult, String> {
    let extension_id = validate_extension_id(&extension_id)?;
    let source = native_host_source_path(&app)?;

    let local_app_data = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "LOCALAPPDATA não está disponível neste Windows".to_string())?;
    let install_dir = local_app_data.join("StorDown").join("BrowserIntegration");
    std::fs::create_dir_all(&install_dir)
        .map_err(|error| format!("Falha ao criar pasta da integração: {error}"))?;

    let installed_host = install_dir.join("stordown-native-host.exe");
    if source != installed_host {
        std::fs::copy(&source, &installed_host)
            .map_err(|error| format!("Falha ao instalar Native Host: {error}"))?;
    }

    let manifest_path = install_dir.join("native-messaging-host.json");
    let manifest = serde_json::json!({
        "name": "cloud.hoststorm.stordown",
        "description": "StorDown browser capture bridge",
        "path": installed_host.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": [
            format!("chrome-extension://{extension_id}/")
        ]
    });

    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("Falha ao gravar manifesto do navegador: {error}"))?;

    let chrome_key = r"HKCU\Software\Google\Chrome\NativeMessagingHosts\cloud.hoststorm.stordown";
    let edge_key = r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts\cloud.hoststorm.stordown";

    let chrome_registered = register_native_host_key(chrome_key, &manifest_path).is_ok();
    let edge_registered = register_native_host_key(edge_key, &manifest_path).is_ok();

    if !chrome_registered && !edge_registered {
        return Err("Não foi possível registrar o Native Host no Chrome nem no Edge".to_string());
    }

    Ok(BrowserIntegrationResult {
        manifest_path: manifest_path.to_string_lossy().to_string(),
        native_host_path: installed_host.to_string_lossy().to_string(),
        chrome_registered,
        edge_registered,
    })
}

#[tauri::command]
fn prepare_browser_extension(app: AppHandle) -> Result<BrowserExtensionPrepared, String> {
    let source = bundled_extension_source(&app)?;
    let local_app_data = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "LOCALAPPDATA não está disponível neste Windows".to_string())?;
    let destination = local_app_data
        .join("StorDown")
        .join("BrowserExtension");

    copy_extension_files(&source, &destination)?;

    Command::new("explorer.exe")
        .arg(&destination)
        .spawn()
        .map_err(|error| format!("Extensão preparada, mas não foi possível abrir a pasta: {error}"))?;

    Ok(BrowserExtensionPrepared {
        extension_dir: destination.to_string_lossy().to_string(),
    })
}

#[tauri::command]
fn get_desktop_defaults(app: AppHandle) -> Result<DesktopDefaults, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Falha ao localizar AppData: {error}"))?;
    let download_dir = app
        .path()
        .download_dir()
        .unwrap_or_else(|_| app_data.join("Downloads"));
    std::fs::create_dir_all(&download_dir)
        .map_err(|error| format!("Falha ao preparar a pasta de Downloads: {error}"))?;

    Ok(DesktopDefaults {
        download_dir: download_dir.to_string_lossy().to_string(),
        interfaces: discover_windows_interfaces().unwrap_or_default(),
    })
}

#[tauri::command]
async fn inspect_download_url(url: String) -> Result<ProbeResult, String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err("Informe uma URL".to_string());
    }

    probe(trimmed).await.map_err(|error| error.to_string())
}

#[tauri::command]
fn pick_download_folder() -> Result<Option<String>, String> {
    Ok(rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.to_string_lossy().to_string()))
}

#[tauri::command]
fn open_browser_extensions(browser: String) -> Result<(), String> {
    let (program, url) = match browser.to_ascii_lowercase().as_str() {
        "edge" => ("msedge.exe", "edge://extensions/"),
        _ => ("chrome.exe", "chrome://extensions/"),
    };

    Command::new(program)
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Não foi possível abrir {program}: {error}"))
}

#[tauri::command]
async fn list_download_rules(
    store: State<'_, TransferStore>,
) -> Result<Vec<DownloadRule>, String> {
    store.list_download_rules().await
}

#[tauri::command]
async fn save_download_rule(
    id: Option<i64>,
    name: String,
    extensions: Vec<String>,
    destination: String,
    enabled: bool,
    priority: i64,
    store: State<'_, TransferStore>,
) -> Result<DownloadRule, String> {
    if name.trim().is_empty() {
        return Err("Informe um nome para a categoria".to_string());
    }

    if destination.trim().is_empty() {
        return Err("Informe uma pasta de destino".to_string());
    }

    if extensions.is_empty() {
        return Err("Informe pelo menos uma extensão".to_string());
    }

    tokio::fs::create_dir_all(destination.trim())
        .await
        .map_err(|error| format!("Falha ao preparar pasta de destino: {error}"))?;

    store
        .upsert_download_rule(
            id,
            NewDownloadRule {
                name,
                extensions,
                destination,
                enabled,
                priority,
            },
        )
        .await
}

#[tauri::command]
async fn delete_download_rule(
    id: i64,
    store: State<'_, TransferStore>,
) -> Result<(), String> {
    store.delete_download_rule(id).await
}

#[tauri::command]
fn pick_download_rule_folder() -> Result<Option<String>, String> {
    Ok(rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.to_string_lossy().to_string()))
}

#[tauri::command]
fn pick_download_destination(suggested_name: Option<String>) -> Result<Option<String>, String> {
    let mut dialog = rfd::FileDialog::new();

    if let Some(name) = suggested_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let safe_name = Path::new(name)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("download.bin");
        dialog = dialog.set_file_name(safe_name);
    }

    Ok(dialog
        .save_file()
        .map(|path| path.to_string_lossy().to_string()))
}

#[tauri::command]
fn pick_upload_files() -> Result<Vec<String>, String> {
    Ok(rfd::FileDialog::new()
        .pick_files()
        .unwrap_or_default()
        .into_iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect())
}

#[tauri::command]
async fn test_routes(bind_ips: Vec<String>) -> Result<Vec<LinkProbeStatus>, String> {
    Ok(probe_links(parse_links(bind_ips)?).await)
}

#[tauri::command]
fn list_network_interfaces() -> Result<Vec<NetworkInterfaceInfo>, String> {
    discover_windows_interfaces()
}

fn file_name_from_path(raw: &str, fallback: &str) -> String {
    Path::new(raw)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback)
        .to_string()
}

#[cfg(target_os = "windows")]
fn discover_windows_interfaces() -> Result<Vec<NetworkInterfaceInfo>, String> {
    const SCRIPT: &str = r#"
$items = Get-NetAdapter -Physical |
  Where-Object { $_.Status -eq 'Up' } |
  ForEach-Object {
    $adapter = $_
    $config = Get-NetIPConfiguration -InterfaceIndex $adapter.ifIndex
    $ip = $config.IPv4Address | Select-Object -First 1
    if ($null -ne $ip) {
      [PSCustomObject]@{
        name = $adapter.Name
        description = $adapter.InterfaceDescription
        ipv4 = $ip.IPAddress
        gateway = if ($null -ne $config.IPv4DefaultGateway) { $config.IPv4DefaultGateway.NextHop } else { $null }
        link_speed = $adapter.LinkSpeed
        index = [int]$adapter.ifIndex
      }
    }
  }
ConvertTo-Json -Compress -InputObject @($items)
"#;

    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            SCRIPT,
        ])
        .output()
        .map_err(|error| format!("Falha ao executar PowerShell: {error}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();

    if trimmed.is_empty() || trimmed == "null" {
        return Ok(Vec::new());
    }

    serde_json::from_str(trimmed)
        .map_err(|error| format!("Falha ao interpretar interfaces do Windows: {error}"))
}

#[cfg(not(target_os = "windows"))]
fn discover_windows_interfaces() -> Result<Vec<NetworkInterfaceInfo>, String> {
    Err("A detecção automática de interfaces está disponível no Windows nesta versão".to_string())
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let app_data = app
                .path()
                .app_data_dir()
                .map_err(|error| format!("Falha ao localizar AppData do StorDown: {error}"))?;
            let download_dir = app
                .path()
                .download_dir()
                .unwrap_or_else(|_| app_data.join("Downloads"));
            std::fs::create_dir_all(&download_dir)?;

            let store = TransferStore::open(&app_data.join("stordown.sqlite3"))
                .map_err(|error| Box::<dyn std::error::Error>::from(std::io::Error::other(error)))?;
            let queue_state = QueueState::default();
            let control_state = TransferControlState::default();

            app.manage(store.clone());
            app.manage(queue_state.clone());
            app.manage(control_state.clone());
            app.manage(GoogleAuthState::default());

            let restore_app = app.handle().clone();
            let restore_store = store.clone();
            let restore_queue = queue_state.clone();
            let restore_controls = control_state.clone();

            tauri::async_runtime::spawn(async move {
                if let Ok(records) = restore_store.list_scheduled().await {
                    for record in records {
                        if record.direction != "download" || record.provider != "http" {
                            continue;
                        }

                        let _ = spawn_download_execution(
                            record.source,
                            record.destination,
                            record.connections,
                            record.bind_ips,
                            HashMap::new(),
                            record.max_bytes_per_second,
                            record.expected_sha256,
                            record.id,
                            record.scheduled_at,
                            restore_app.clone(),
                            restore_queue.clone(),
                            restore_controls.clone(),
                            restore_store.clone(),
                        );
                    }
                }
            });

            start_browser_capture_server(
                app.handle().clone(),
                download_dir,
                queue_state,
                control_state,
                store,
            );

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            enqueue_download,
            enqueue_drive_export,
            enqueue_drive_download,
            enqueue_drive_upload,
            resume_drive_upload,
            list_transfers,
            delete_transfer_history,
            clear_finished_history,
            install_browser_integration,
            prepare_browser_extension,
            open_browser_extensions,
            get_desktop_defaults,
            inspect_download_url,
            list_download_rules,
            save_download_rule,
            delete_download_rule,
            pick_download_rule_folder,
            pause_transfer,
            resume_transfer,
            cancel_transfer,
            pick_download_destination,
            pick_download_folder,
            pick_upload_files,
            test_routes,
            list_network_interfaces,
            connect_google_drive,
            restore_google_drive,
            google_drive_auth_status,
            browse_google_drive_folders,
            browse_google_drive_items,
            resolve_drive_shared_link,
            google_drive_export_options,
            list_google_drive_roots,
            disconnect_google_drive
        ])
        .run(tauri::generate_context!())
        .expect("error while running StorDown");
}
