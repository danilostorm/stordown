#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::IpAddr, path::PathBuf, process::Command};
use stordown_core::{
    download, probe_links, upload_google_drive_batch, DownloadRequest, DownloadResult,
    GoogleDriveBatchUploadRequest, GoogleDriveUploadResult, LinkConfig, LinkProbeStatus,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NetworkInterfaceInfo {
    name: String,
    description: String,
    ipv4: String,
    gateway: Option<String>,
    link_speed: Option<String>,
    index: u32,
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

#[tauri::command]
async fn start_download(
    url: String,
    output: String,
    connections: usize,
    bind_ips: Vec<String>,
) -> Result<DownloadResult, String> {
    let links = parse_links(bind_ips)?;

    download(DownloadRequest {
        url,
        output: PathBuf::from(output),
        connections,
        links,
        headers: HashMap::new(),
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn start_drive_upload(
    files: Vec<String>,
    access_token: String,
    parent_id: Option<String>,
    bind_ips: Vec<String>,
    chunk_mib: u64,
) -> Result<Vec<GoogleDriveUploadResult>, String> {
    if files.is_empty() {
        return Err("Adicione pelo menos um arquivo para upload".to_string());
    }

    if access_token.trim().is_empty() {
        return Err("Token Google Drive ausente".to_string());
    }

    let chunk_size = chunk_mib
        .checked_mul(1024 * 1024)
        .ok_or_else(|| "Tamanho de bloco inválido".to_string())?;

    upload_google_drive_batch(GoogleDriveBatchUploadRequest {
        files: files.into_iter().map(PathBuf::from).collect(),
        access_token,
        parent_id: parent_id.filter(|value| !value.trim().is_empty()),
        chunk_size,
        links: parse_links(bind_ips)?,
    })
    .await
    .map_err(|error| error.to_string())
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
        .invoke_handler(tauri::generate_handler![
            start_download,
            start_drive_upload,
            test_routes,
            list_network_interfaces
        ])
        .run(tauri::generate_context!())
        .expect("error while running StorDown");
}
