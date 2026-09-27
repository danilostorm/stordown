use crate::model::LinkConfig;
use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::task::JoinSet;

const DEFAULT_PUBLIC_IP_ENDPOINT: &str = "https://api.ipify.org?format=json";

#[derive(Debug, Clone, Serialize)]
pub struct LinkProbeResult {
    pub name: String,
    pub local_ip: String,
    pub public_ip: String,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkProbeStatus {
    pub name: String,
    pub local_ip: String,
    pub public_ip: Option<String>,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PublicIpResponse {
    ip: String,
}

pub async fn probe_link(link: &LinkConfig) -> Result<LinkProbeResult> {
    let client = Client::builder()
        .local_address(link.local_ip)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .context("failed to build route probe client")?;

    let started = Instant::now();
    let response = client
        .get(DEFAULT_PUBLIC_IP_ENDPOINT)
        .send()
        .await
        .with_context(|| format!("failed to probe {}", link.name))?
        .error_for_status()?;

    let payload: PublicIpResponse = response.json().await?;

    Ok(LinkProbeResult {
        name: link.name.clone(),
        local_ip: link.local_ip.to_string(),
        public_ip: payload.ip,
        latency_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
    })
}

pub async fn probe_links(links: Vec<LinkConfig>) -> Vec<LinkProbeStatus> {
    let mut jobs = JoinSet::new();

    for (index, link) in links.into_iter().enumerate() {
        let name = link.name.clone();
        let local_ip = link.local_ip.to_string();

        jobs.spawn(async move {
            let status = match probe_link(&link).await {
                Ok(result) => LinkProbeStatus {
                    name: result.name,
                    local_ip: result.local_ip,
                    public_ip: Some(result.public_ip),
                    latency_ms: Some(result.latency_ms),
                    error: None,
                },
                Err(error) => LinkProbeStatus {
                    name,
                    local_ip,
                    public_ip: None,
                    latency_ms: None,
                    error: Some(error.to_string()),
                },
            };

            (index, status)
        });
    }

    let mut results = Vec::new();

    while let Some(joined) = jobs.join_next().await {
        if let Ok((index, status)) = joined {
            results.push((index, status));
        }
    }

    results.sort_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, status)| status).collect()
}
