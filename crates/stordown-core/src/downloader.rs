use crate::model::{DownloadRequest, DownloadResult, LinkConfig, ProbeResult};
use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use reqwest::{
    header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE},
    Client, StatusCode,
};
use std::{
    collections::BTreeSet,
    net::IpAddr,
    path::{Path, PathBuf},
};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncReadExt, AsyncWriteExt},
    task::JoinSet,
};

pub async fn probe(url: &str) -> Result<ProbeResult> {
    let client = Client::builder().build()?;
    let response = client.head(url).send().await?;

    let size = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());

    let accepts_ranges = response
        .headers()
        .get(ACCEPT_RANGES)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("bytes"))
        .unwrap_or(false);

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    Ok(ProbeResult {
        size,
        accepts_ranges,
        content_type,
    })
}

pub async fn download(request: DownloadRequest) -> Result<DownloadResult> {
    if request.connections == 0 {
        bail!("connections must be greater than zero");
    }

    let links: Vec<LinkConfig> = request
        .links
        .iter()
        .filter(|link| link.enabled)
        .cloned()
        .collect();

    if links.is_empty() {
        bail!("at least one enabled network link is required");
    }

    let metadata = probe_with_request_headers(&request).await?;

    if metadata.accepts_ranges {
        if let Some(size) = metadata.size {
            return download_segmented(request, links, size).await;
        }
    }

    download_single(request, &links[0]).await
}

async fn probe_with_request_headers(request: &DownloadRequest) -> Result<ProbeResult> {
    let client = Client::builder().build()?;
    let mut req = client.head(&request.url);
    for (key, value) in &request.headers {
        req = req.header(key, value);
    }

    let response = req.send().await?;
    let size = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());

    let accepts_ranges = response
        .headers()
        .get(ACCEPT_RANGES)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("bytes"))
        .unwrap_or(false);

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    Ok(ProbeResult {
        size,
        accepts_ranges,
        content_type,
    })
}

async fn download_single(request: DownloadRequest, link: &LinkConfig) -> Result<DownloadResult> {
    ensure_parent(&request.output).await?;

    let client = client_for(link.local_ip)?;
    let mut req = client.get(&request.url);
    for (key, value) in &request.headers {
        req = req.header(key, value);
    }

    let response = req.send().await?.error_for_status()?;
    let mut stream = response.bytes_stream();
    let mut file = File::create(&request.output).await?;
    let mut written = 0u64;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        written += chunk.len() as u64;
    }

    file.flush().await?;

    Ok(DownloadResult {
        output: request.output,
        bytes_written: written,
        segments: 1,
        links_used: vec![link.name.clone()],
    })
}

async fn download_segmented(
    request: DownloadRequest,
    links: Vec<LinkConfig>,
    size: u64,
) -> Result<DownloadResult> {
    ensure_parent(&request.output).await?;

    let segments = request.connections.min(size.max(1) as usize);
    let part_dir = part_dir_for(&request.output);
    fs::create_dir_all(&part_dir).await?;

    let weighted_links = expand_weighted_links(&links);
    let mut jobs = JoinSet::new();

    for index in 0..segments {
        let (start, end) = segment_bounds(size, segments, index);
        let link = weighted_links[index % weighted_links.len()].clone();
        let url = request.url.clone();
        let headers = request.headers.clone();
        let part = part_dir.join(format!("{index:05}.part"));

        jobs.spawn(async move {
            download_range(&url, &headers, &part, start, end, &link)
                .await
                .map(|bytes| (index, bytes, link.name))
        });
    }

    let mut total = 0u64;
    let mut used = BTreeSet::new();

    while let Some(result) = jobs.join_next().await {
        let (_index, bytes, link_name) = result??;
        total += bytes;
        used.insert(link_name);
    }

    if total != size {
        bail!("downloaded {total} bytes but expected {size}");
    }

    assemble_parts(&part_dir, &request.output, segments).await?;
    fs::remove_dir_all(&part_dir).await?;

    Ok(DownloadResult {
        output: request.output,
        bytes_written: total,
        segments,
        links_used: used.into_iter().collect(),
    })
}

async fn download_range(
    url: &str,
    headers: &std::collections::HashMap<String, String>,
    part_path: &Path,
    start: u64,
    end: u64,
    link: &LinkConfig,
) -> Result<u64> {
    let expected = end - start + 1;
    let client = client_for(link.local_ip)?;
    let mut req = client.get(url).header(RANGE, format!("bytes={start}-{end}"));

    for (key, value) in headers {
        req = req.header(key, value);
    }

    let response = req.send().await?;

    if response.status() != StatusCode::PARTIAL_CONTENT {
        bail!(
            "server ignored HTTP Range for {} on {} (status {})",
            link.name,
            link.local_ip,
            response.status()
        );
    }

    if response.headers().get(CONTENT_RANGE).is_none() {
        bail!("server returned 206 without Content-Range");
    }

    let mut stream = response.bytes_stream();
    let mut file = File::create(part_path).await?;
    let mut written = 0u64;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        written += chunk.len() as u64;
    }

    file.flush().await?;

    if written != expected {
        bail!(
            "segment {}-{} wrote {} bytes, expected {}",
            start,
            end,
            written,
            expected
        );
    }

    Ok(written)
}

fn client_for(local_ip: IpAddr) -> Result<Client> {
    Client::builder()
        .local_address(local_ip)
        .build()
        .context("failed to build HTTP client bound to local IP")
}

fn expand_weighted_links(links: &[LinkConfig]) -> Vec<LinkConfig> {
    let mut result = Vec::new();

    for link in links {
        for _ in 0..link.weight.max(1) {
            result.push(link.clone());
        }
    }

    result
}

fn segment_bounds(size: u64, segments: usize, index: usize) -> (u64, u64) {
    let segment_size = size / segments as u64;
    let remainder = size % segments as u64;

    let extra_before = remainder.min(index as u64);
    let start = index as u64 * segment_size + extra_before;
    let this_size = segment_size + u64::from((index as u64) < remainder);
    let end = start + this_size - 1;

    (start, end)
}

fn part_dir_for(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download");

    output.with_file_name(format!("{name}.stordown.parts"))
}

async fn ensure_parent(output: &Path) -> Result<()> {
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).await?;
        }
    }
    Ok(())
}

async fn assemble_parts(part_dir: &Path, output: &Path, segments: usize) -> Result<()> {
    let mut destination = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(output)
        .await?;

    let mut buffer = vec![0u8; 1024 * 1024];

    for index in 0..segments {
        let part = part_dir.join(format!("{index:05}.part"));
        let mut source = File::open(&part)
            .await
            .with_context(|| format!("missing segment {}", part.display()))?;

        loop {
            let read = source.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            destination.write_all(&buffer[..read]).await?;
        }
    }

    destination.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::segment_bounds;

    #[test]
    fn segment_bounds_cover_entire_file_without_overlap() {
        let size = 10u64;
        let segments = 3usize;
        assert_eq!(segment_bounds(size, segments, 0), (0, 3));
        assert_eq!(segment_bounds(size, segments, 1), (4, 6));
        assert_eq!(segment_bounds(size, segments, 2), (7, 9));
    }
}
