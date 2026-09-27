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
    time::{Duration, Instant},
};
use stordown_core::{
    authorize_google_drive_desktop, download_with_control, probe_links,
    refresh_google_access_token, upload_google_drive_batch_with_control, DownloadRequest,
    GoogleDriveBatchUploadRequest, LinkConfig, LinkProbeStatus, ProgressCallback, TransferControl,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{Mutex, Semaphore};
use transfer_store::{NewTransferRecord, TransferRecord, TransferStore};

const GOOGLE_KEYRING_SERVICE: &str = "StorDown Google Drive";
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

    env::var("STORDOWN_GOOGLE_CLIENT_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            "Google OAuth Client ID não configurado. Defina STORDOWN_GOOGLE_CLIENT_ID ou informe o Client ID na tela de desenvolvimento."
                .to_string()
        })
}

fn credential_entry(client_id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(GOOGLE_KEYRING_SERVICE, client_id)
        .map_err(|error| format!("Falha ao acessar o armazenamento seguro do Windows: {error}"))
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

async fn current_google_access_token(state: &GoogleAuthState) -> Result<String, String> {
    let mut guard = state.session.lock().await;
    let session = guard
        .as_mut()
        .ok_or_else(|| "Conecte sua conta Google Drive antes de iniciar o upload".to_string())?;

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

#[tauri::command]
async fn enqueue_download(
    url: String,
    output: String,
    connections: usize,
    bind_ips: Vec<String>,
    transfer_id: String,
    app: AppHandle,
    queue_state: State<'_, QueueState>,
    control_state: State<'_, TransferControlState>,
    store: State<'_, TransferStore>,
) -> Result<TransferRecord, String> {
    let links = parse_links(bind_ips.clone())?;
    let name = file_name_from_path(&output, "download");
    let record = store
        .insert(NewTransferRecord {
            id: transfer_id.clone(),
            direction: "download".to_string(),
            name,
            source: url.clone(),
            destination: output.clone(),
            provider: "http".to_string(),
            connections,
            bind_ips,
        })
        .await?;

    let control = register_transfer(&transfer_id, control_state.inner()).await;
    let queue = queue_state.inner().clone();
    let controls = control_state.inner().clone();
    let store = store.inner().clone();
    let task_store = store.clone();
    let task_app = app.clone();

    notify_transfer_list(&app, &transfer_id);

    tauri::async_runtime::spawn(async move {
        let permit = queue.slots.acquire_owned().await;
        if permit.is_err() {
            let _ = task_store
                .update_status(&transfer_id, "failed", Some("Fila indisponível"))
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

        let result = download_with_control(
            DownloadRequest {
                url,
                output: PathBuf::from(output),
                connections,
                links,
                headers: HashMap::new(),
            },
            transfer_id.clone(),
            Some(progress_emitter(task_app.clone(), task_store.clone())),
            Some(control.clone()),
        )
        .await;

        match result {
            Ok(result) => {
                let _ = task_store
                    .complete(&transfer_id, result.bytes_written, Some(result.bytes_written))
                    .await;
            }
            Err(error) if control.is_cancelled() => {
                let _ = task_store.update_status(&transfer_id, "cancelled", None).await;
            }
            Err(error) => {
                let message = error.to_string();
                let _ = task_store
                    .update_status(&transfer_id, "failed", Some(&message))
                    .await;
            }
        }

        notify_transfer_list(&task_app, &transfer_id);
        remove_transfer(&transfer_id, &controls).await;
    });

    Ok(record)
}

#[tauri::command]
async fn enqueue_drive_upload(
    files: Vec<String>,
    parent_id: Option<String>,
    bind_ips: Vec<String>,
    chunk_mib: u64,
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

    let name = if files.len() == 1 {
        file_name_from_path(&files[0], "upload")
    } else {
        format!("{} arquivos para Google Drive", files.len())
    };
    let destination = parent_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
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
        })
        .await?;

    let control = register_transfer(&transfer_id, control_state.inner()).await;
    let queue = queue_state.inner().clone();
    let controls = control_state.inner().clone();
    let task_store = store.inner().clone();
    let task_app = app.clone();

    notify_transfer_list(&app, &transfer_id);

    tauri::async_runtime::spawn(async move {
        let permit = queue.slots.acquire_owned().await;
        if permit.is_err() {
            let _ = task_store
                .update_status(&transfer_id, "failed", Some("Fila indisponível"))
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

        let result = upload_google_drive_batch_with_control(
            GoogleDriveBatchUploadRequest {
                files: files.into_iter().map(PathBuf::from).collect(),
                access_token,
                parent_id: parent_id.filter(|value| !value.trim().is_empty()),
                chunk_size,
                links,
            },
            transfer_id.clone(),
            Some(progress_emitter(task_app.clone(), task_store.clone())),
            Some(control.clone()),
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
                    .update_status(&transfer_id, "failed", Some(&message))
                    .await;
            }
        }

        notify_transfer_list(&task_app, &transfer_id);
        remove_transfer(&transfer_id, &controls).await;
    });

    Ok(record)
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

    store.delete(&transfer_id).await
}

#[tauri::command]
async fn clear_finished_history(
    store: State<'_, TransferStore>,
) -> Result<usize, String> {
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
            let store = TransferStore::open(&app_data.join("stordown.sqlite3"))
                .map_err(|error| Box::<dyn std::error::Error>::from(std::io::Error::other(error)))?;

            app.manage(store);
            Ok(())
        })
        .manage(GoogleAuthState::default())
        .manage(TransferControlState::default())
        .manage(QueueState::default())
        .invoke_handler(tauri::generate_handler![
            enqueue_download,
            enqueue_drive_upload,
            list_transfers,
            delete_transfer_history,
            clear_finished_history,
            pause_transfer,
            resume_transfer,
            cancel_transfer,
            test_routes,
            list_network_interfaces,
            connect_google_drive,
            restore_google_drive,
            google_drive_auth_status,
            disconnect_google_drive
        ])
        .run(tauri::generate_context!())
        .expect("error while running StorDown");
}
