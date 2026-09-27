use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::IpAddr,
    path::PathBuf,
    sync::Arc,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkConfig {
    pub name: String,
    pub local_ip: IpAddr,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_weight")]
    pub weight: u32,
}

fn default_true() -> bool {
    true
}

fn default_weight() -> u32 {
    1
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub url: String,
    pub output: PathBuf,
    pub connections: usize,
    pub links: Vec<LinkConfig>,
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProbeResult {
    pub size: Option<u64>,
    pub accepts_ranges: bool,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadResult {
    pub output: PathBuf,
    pub bytes_written: u64,
    pub segments: usize,
    pub links_used: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransferProgress {
    pub transfer_id: String,
    pub direction: String,
    pub item: String,
    pub phase: String,
    pub bytes_delta: u64,
    pub bytes_transferred: u64,
    pub total_bytes: Option<u64>,
    pub link_name: String,
    pub local_ip: IpAddr,
    pub completed: bool,
}

pub type ProgressCallback = Arc<dyn Fn(TransferProgress) + Send + Sync>;
