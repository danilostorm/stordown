#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{collections::HashMap, net::IpAddr, path::PathBuf};
use stordown_core::{download, DownloadRequest, DownloadResult, LinkConfig};

#[tauri::command]
async fn start_download(
    url: String,
    output: String,
    connections: usize,
    bind_ips: Vec<String>,
) -> Result<DownloadResult, String> {
    let mut links = Vec::new();

    for (index, raw) in bind_ips.into_iter().enumerate() {
        let local_ip: IpAddr = raw
            .parse()
            .map_err(|_| format!("IP inválido: {raw}"))?;

        links.push(LinkConfig {
            name: format!("Ethernet {}", index + 1),
            local_ip,
            enabled: true,
            weight: 1,
        });
    }

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

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![start_download])
        .run(tauri::generate_context!())
        .expect("error while running StorDown");
}
