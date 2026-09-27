#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    env,
    net::IpAddr,
    path::PathBuf,
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};
use stordown_core::{
    authorize_google_drive_desktop, download_with_control, probe_links,
    refresh_google_access_token, upload_google_drive_batch_with_control, DownloadRequest,
    DownloadResult, GoogleDriveBatchUploadRequest, GoogleDriveUploadResult, LinkConfig,
    LinkProbeStatus, ProgressCallback, TransferControl,
};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

const GOOGLE_KEYRING_SERVICE: &str = "StorDown Google Drive";

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

#[derive(Default)]
struct TransferControlState {
    controls: Mutex<HashMap<String, TransferControl>>,
}

#[derive(Debug, Clone, Serialize)]
struct TransferControlStatus {
    transfer_id: String,
    paused: bool,
    cancelled: bool,
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

fn progress_emitter(app: AppHandle) -> ProgressCallback {
    Arc::new(move |progress| {
        let _ = app.emit("transfer-progress", progress);
    })
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
async fn start_download(
    url: String,
    output: String,
    connections: usize,
    bind_ips: Vec<String>,
    transfer_id: String,
    app: AppHandle,
    control_state: State<'_, TransferControlState>,
) -> Result<DownloadResult, String> {
    let links = parse_links(bind_ips)?;
    let control = register_transfer(&transfer_id, control_state.inner()).await;

    let result = download_with_control(
        DownloadRequest {
            url,
            output: PathBuf::from(output),
            connections,
            links,
            headers: HashMap::new(),
        },
        transfer_id.clone(),
        Some(progress_emitter(app)),
        Some(control),
    )
    .await
    .map_err(|error| error.to_string());

    remove_transfer(&transfer_id, control_state.inner()).await;
    result
}

#[tauri::command]
async fn start_drive_upload(
    files: Vec<String>,
    parent_id: Option<String>,
    bind_ips: Vec<String>,
    chunk_mib: u64,
    transfer_id: String,
    app: AppHandle,
    auth_state: State<'_, GoogleAuthState>,
    control_state: State<'_, TransferControlState>,
) -> Result<Vec<GoogleDriveUploadResult>, String> {
    if files.is_empty() {
        return Err("Adicione pelo menos um arquivo para upload".to_string());
    }

    let access_token = current_google_access_token(auth_state.inner()).await?;
    let chunk_size = chunk_mib
        .checked_mul(1024 * 1024)
        .ok_or_else(|| "Tamanho de bloco inválido".to_string())?;
    let control = register_transfer(&transfer_id, control_state.inner()).await;

    let result = upload_google_drive_batch_with_control(
        GoogleDriveBatchUploadRequest {
            files: files.into_iter().map(PathBuf::from).collect(),
            access_token,
            parent_id: parent_id.filter(|value| !value.trim().is_empty()),
            chunk_size,
            links: parse_links(bind_ips)?,
        },
        transfer_id.clone(),
        Some(progress_emitter(app)),
        Some(control),
    )
    .await
    .map_err(|error| error.to_string());

    remove_transfer(&transfer_id, control_state.inner()).await;
    result
}

#[tauri::command]
async fn pause_transfer(
    transfer_id: String,
    state: State<'_, TransferControlState>,
) -> Result<TransferControlStatus, String> {
    let control = get_transfer_control(&transfer_id, state.inner()).await?;
    control.pause();

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
) -> Result<TransferControlStatus, String> {
    let control = get_transfer_control(&transfer_id, state.inner()).await?;
    control.resume();

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
) -> Result<TransferControlStatus, String> {
    let control = get_transfer_control(&transfer_id, state.inner()).await?;
    control.cancel();

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
        .manage(GoogleAuthState::default())
        .manage(TransferControlState::default())
        .invoke_handler(tauri::generate_handler![
            start_download,
            start_drive_upload,
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
