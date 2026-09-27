use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::IpAddr, path::PathBuf};

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
