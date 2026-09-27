use crate::model::{DownloadRequest, DownloadResult, LinkConfig, ProbeResult};
use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use reqwest::{
    header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE},
    Client, StatusCode,
};
use std::{
    collections::{BTreeSet, HashMap},
    net::IpAddr,
    path::{Path, PathBuf},
};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncReadExt, AsyncWriteExt},
    task::JoinSet,
};

pub async fn probe(url: &str) -> Result<ProbeResult> {
    probe_url(url, &HashMap::new()).await
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

    let metadata = probe_url(&request.url, &request.headers).await?;

    if metadata.accepts_ranges {
        if let Some(size) = metadata.size {
            if size > 0 {
                return download_segmented(request, links, size).await;
            }
        }
    }

    download_single(request, &links[0]).await
}

async fn probe_url(url: &str, headers: &HashMap<String, String>) -> Result<ProbeResult> {
    let client = Client::builder().build()?;

    if let Ok(response) = request_with_headers(client.head(url), headers).send().await {
        if response.status().is_success() {
            let head_probe = probe_from_headers(response.headers(), false);

            if head_probe.accepts_ranges && head_probe.size.is_some() {
                return Ok(head_probe);
            }
        }
    }

    // Many CDNs omit Accept-Ranges on HEAD even though byte ranges work.
    // A one-byte request is a more reliable capability probe.
    let response = request_with_headers(client.get(url).header(RANGE, "bytes=0-0"), headers)
        .send()
        .await?;

    if response.status() == StatusCode::PARTIAL_CONTENT {
        let mut probe = probe_from_headers(response.headers(), true);

        if probe.size.is_none() {
            probe.size = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(total_from_content_range);
        }

        return Ok(probe);
    }

    let mut probe = probe_from_headers(response.headers(), false);
    probe.accepts_ranges = false;
    Ok(probe)
}

fn probe_from_headers(headers: &reqwest::header::HeaderMap, partial: bool) -> ProbeResult {
    let size = headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());

    let accepts_ranges = partial
        || headers
            .get(ACCEPT_RANGES)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.eq_ignore_ascii_case("bytes"))
            .unwrap_or(false);

    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    ProbeResult {
        size,
        accepts_ranges,
        content_type,
    }
}

fn total_from_content_range(value: &str) -> Option<u64> {
    let (_, total) = value.rsplit_once('/')?;
    if total == "*" {
        return None;
    }
    total.parse().ok()
}

fn request_with_headers(
    mut builder: reqwest::RequestBuilder,
    headers: &HashMap<String, String>,
) -> reqwest::RequestBuilder {
    for (key, value) in headers {
        builder = builder.header(key.as_str(), value.as_str());
    }
    builder
}

async fn download_single(request: DownloadRequest, link: &LinkConfig) -> Result<DownloadResult> {
    ensure_parent(&request.output).await?;

    let client = client_for(link.local_ip)?;
    let req = request_with_headers(client.get(&request.url), &request.headers);
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
    headers: &HashMap<String, String>,
    part_path: &Path,
    start: u64,
    end: u64,
    link: &LinkConfig,
) -> Result<u64> {
    let expected = end - start + 1;
    let client = client_for(link.local_ip)?;
    let req = request_with_headers(
        client.get(url).header(RANGE, format!("bytes={start}-{end}")),
        headers,
    );

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
    use super::{segment_bounds, total_from_content_range};

    #[test]
    fn segment_bounds_cover_entire_file_without_overlap() {
        let size = 10u64;
        let segments = 3usize;
        assert_eq!(segment_bounds(size, segments, 0), (0, 3));
        assert_eq!(segment_bounds(size, segments, 1), (4, 6));
        assert_eq!(segment_bounds(size, segments, 2), (7, 9));
    }

    #[test]
    fn parses_total_size_from_content_range() {
        assert_eq!(total_from_content_range("bytes 0-0/12345"), Some(12345));
        assert_eq!(total_from_content_range("bytes 0-0/*"), None);
    }
}
