#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{collections::HashMap, net::IpAddr, path::PathBuf};
use stordown_core::{
    download, upload_google_drive_batch, DownloadRequest, DownloadResult,
    GoogleDriveBatchUploadRequest, GoogleDriveUploadResult, LinkConfig,
};

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

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            start_download,
            start_drive_upload
        ])
        .run(tauri::generate_context!())
        .expect("error while running StorDown");
}
