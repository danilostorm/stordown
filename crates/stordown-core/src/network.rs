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
    pub latency_ms: u128,
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
        latency_ms: started.elapsed().as_millis(),
    })
}

pub async fn probe_links(links: Vec<LinkConfig>) -> Vec<Result<LinkProbeResult, String>> {
    let mut jobs = JoinSet::new();

    for (index, link) in links.into_iter().enumerate() {
        jobs.spawn(async move {
            let result = probe_link(&link).await.map_err(|error| error.to_string());
            (index, result)
        });
    }

    let mut results = Vec::new();

    while let Some(joined) = jobs.join_next().await {
        match joined {
            Ok((index, result)) => results.push((index, result)),
            Err(error) => results.push((usize::MAX, Err(error.to_string()))),
        }
    }

    results.sort_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}
