use crate::{
    adaptive::AdaptiveLinkPool,
    control::TransferControl,
    model::{
        DownloadRequest, DownloadResult, LinkConfig, ProbeResult, ProgressCallback,
        TransferProgress,
    },
    throttle::TransferThrottle,
};
use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use reqwest::{
    header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE},
    Client, StatusCode,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncReadExt, AsyncWriteExt},
    task::JoinSet,
    time::sleep,
};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
const MAX_SEGMENT_RETRIES: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PartManifest {
    url: String,
    size: u64,
    segments: usize,
}

pub async fn probe(url: &str) -> Result<ProbeResult> {
    probe_url(url, &HashMap::new()).await
}

pub async fn download(request: DownloadRequest) -> Result<DownloadResult> {
    download_with_progress(request, "download".to_string(), None).await
}

pub async fn download_with_progress(
    request: DownloadRequest,
    transfer_id: String,
    progress: Option<ProgressCallback>,
) -> Result<DownloadResult> {
    download_with_control(request, transfer_id, progress, None).await
}

pub async fn download_with_control(
    request: DownloadRequest,
    transfer_id: String,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
) -> Result<DownloadResult> {
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

    checkpoint(control.as_ref()).await?;

    let metadata = probe_url(&request.url, &request.headers).await?;

    if metadata.accepts_ranges {
        if let Some(size) = metadata.size {
            if size > 0 {
                return download_segmented(
                    request,
                    links,
                    size,
                    transfer_id,
                    progress,
                    control,
                )
                .await;
            }
        }
    }

    download_single(
        request,
        &links[0],
        metadata.size,
        transfer_id,
        progress,
        control,
    )
    .await
}

pub async fn download_direct_with_control(
    request: DownloadRequest,
    transfer_id: String,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
) -> Result<DownloadResult> {
    let links: Vec<LinkConfig> = request
        .links
        .iter()
        .filter(|link| link.enabled)
        .cloned()
        .collect();

    if links.is_empty() {
        bail!("at least one enabled network link is required");
    }

    checkpoint(control.as_ref()).await?;

    download_single(
        request,
        &links[0],
        None,
        transfer_id,
        progress,
        control,
    )
    .await
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

async fn download_single(
    request: DownloadRequest,
    link: &LinkConfig,
    expected_size: Option<u64>,
    transfer_id: String,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
) -> Result<DownloadResult> {
    ensure_parent(&request.output).await?;
    checkpoint(control.as_ref()).await?;

    let client = client_for(link.local_ip)?;
    let req = request_with_headers(client.get(&request.url), &request.headers);
    let response = req.send().await?.error_for_status()?;
    let total_size = response.content_length().or(expected_size);
    let mut stream = response.bytes_stream();
    let mut file = File::create(&request.output).await?;
    let mut written = 0u64;
    let mut pending_delta = 0u64;
    let mut last_emit = Instant::now();
    let item = output_name(&request.output);
    let throttle = TransferThrottle::new(request.max_bytes_per_second);

    while let Some(chunk) = stream.next().await {
        checkpoint(control.as_ref()).await?;

        let chunk = chunk?;
        let delta = chunk.len() as u64;
        throttle.consume(delta).await;
        file.write_all(&chunk).await?;
        written += delta;
        pending_delta += delta;

        if last_emit.elapsed() >= PROGRESS_INTERVAL {
            emit_progress(
                progress.as_ref(),
                &transfer_id,
                "download",
                &item,
                "transferring",
                pending_delta,
                written,
                total_size,
                link,
                false,
            );
            pending_delta = 0;
            last_emit = Instant::now();
        }
    }

    file.flush().await?;
    drop(file);

    if pending_delta > 0 {
        emit_progress(
            progress.as_ref(),
            &transfer_id,
            "download",
            &item,
            "transferring",
            pending_delta,
            written,
            total_size,
            link,
            false,
        );
    }

    let (sha256, integrity_verified) =
        verify_sha256(&request.output, request.expected_sha256.as_deref()).await?;

    emit_progress(
        progress.as_ref(),
        &transfer_id,
        "download",
        &item,
        if integrity_verified { "verified" } else { "completed" },
        0,
        written,
        total_size.or(Some(written)),
        link,
        true,
    );

    Ok(DownloadResult {
        output: request.output,
        bytes_written: written,
        segments: 1,
        links_used: vec![link.name.clone()],
        sha256,
        integrity_verified,
    })
}

async fn download_segmented(
    request: DownloadRequest,
    links: Vec<LinkConfig>,
    size: u64,
    transfer_id: String,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
) -> Result<DownloadResult> {
    ensure_parent(&request.output).await?;

    let max_workers = size.min(usize::MAX as u64).max(1) as usize;
    let workers = request.connections.min(max_workers).max(1);
    let segments = adaptive_segment_count(size, workers);
    let part_dir = part_dir_for(&request.output);

    prepare_part_dir(&part_dir, &request.url, size, segments).await?;

    let pool = AdaptiveLinkPool::new(links.clone());
    let throttle = TransferThrottle::new(request.max_bytes_per_second);
    let aggregate = Arc::new(AtomicU64::new(0));
    let item = output_name(&request.output);
    let mut jobs: JoinSet<Result<(usize, u64, Vec<String>)>> = JoinSet::new();
    let mut next_index = 0usize;

    while next_index < segments && jobs.len() < workers {
        spawn_segment_job(
            &mut jobs,
            next_index,
            &request,
            &part_dir,
            size,
            segments,
            &transfer_id,
            &item,
            aggregate.clone(),
            progress.clone(),
            control.clone(),
            pool.clone(),
            throttle.clone(),
        );
        next_index += 1;
    }

    let mut total = 0u64;
    let mut used = BTreeSet::new();

    while let Some(result) = jobs.join_next().await {
        let (_index, bytes, link_names) = result??;
        total += bytes;
        used.extend(link_names);

        if next_index < segments {
            spawn_segment_job(
                &mut jobs,
                next_index,
                &request,
                &part_dir,
                size,
                segments,
                &transfer_id,
                &item,
                aggregate.clone(),
                progress.clone(),
                control.clone(),
                pool.clone(),
                throttle.clone(),
            );
            next_index += 1;
        }
    }

    if total != size {
        bail!("downloaded {total} bytes but expected {size}");
    }

    checkpoint(control.as_ref()).await?;
    assemble_parts(&part_dir, &request.output, segments).await?;
    let (sha256, integrity_verified) =
        verify_sha256(&request.output, request.expected_sha256.as_deref()).await?;
    fs::remove_dir_all(&part_dir).await?;

    if let Some(link) = links.first() {
        emit_progress(
            progress.as_ref(),
            &transfer_id,
            "download",
            &item,
            if integrity_verified { "verified" } else { "completed" },
            0,
            size,
            Some(size),
            link,
            true,
        );
    }

    Ok(DownloadResult {
        output: request.output,
        bytes_written: total,
        segments,
        links_used: used.into_iter().collect(),
        sha256,
        integrity_verified,
    })
}

#[allow(clippy::too_many_arguments)]
fn spawn_segment_job(
    jobs: &mut JoinSet<Result<(usize, u64, Vec<String>)>>,
    index: usize,
    request: &DownloadRequest,
    part_dir: &Path,
    size: u64,
    segments: usize,
    transfer_id: &str,
    item: &str,
    aggregate: Arc<AtomicU64>,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
    pool: AdaptiveLinkPool,
    throttle: TransferThrottle,
) {
    let (start, end) = segment_bounds(size, segments, index);
    let url = request.url.clone();
    let headers = request.headers.clone();
    let part = part_dir.join(format!("{index:05}.part"));
    let transfer_id = transfer_id.to_string();
    let item = item.to_string();

    jobs.spawn(async move {
        download_range_resumable(
            &url,
            &headers,
            &part,
            start,
            end,
            size,
            &transfer_id,
            &item,
            aggregate,
            progress,
            control,
            pool,
            throttle,
        )
        .await
        .map(|(bytes, links)| (index, bytes, links))
    });
}

#[allow(clippy::too_many_arguments)]
async fn download_range_resumable(
    url: &str,
    headers: &HashMap<String, String>,
    part_path: &Path,
    start: u64,
    end: u64,
    total_size: u64,
    transfer_id: &str,
    item: &str,
    aggregate: Arc<AtomicU64>,
    progress: Option<ProgressCallback>,
    control: Option<TransferControl>,
    pool: AdaptiveLinkPool,
    throttle: TransferThrottle,
) -> Result<(u64, Vec<String>)> {
    let expected = end - start + 1;
    let mut existing = part_len(part_path).await;
    let mut links_used = BTreeSet::new();

    if existing > expected {
        let _ = fs::remove_file(part_path).await;
        existing = 0;
    }

    if existing > 0 {
        aggregate.fetch_add(existing, Ordering::Relaxed);
    }

    if existing == expected {
        return Ok((expected, Vec::new()));
    }

    let mut attempt = 0usize;

    loop {
        checkpoint(control.as_ref()).await?;

        let current_len = part_len(part_path).await;

        if current_len == expected {
            return Ok((expected, links_used.into_iter().collect()));
        }

        if current_len > expected {
            bail!(
                "segment {}-{} contains {} bytes, expected at most {}",
                start,
                end,
                current_len,
                expected
            );
        }

        attempt += 1;
        let lease = pool.acquire().await;
        let link = lease.link.clone();
        links_used.insert(link.name.clone());

        if attempt > 1 {
            emit_progress(
                progress.as_ref(),
                transfer_id,
                "download",
                item,
                "failover",
                0,
                aggregate.load(Ordering::Relaxed),
                Some(total_size),
                &link,
                false,
            );
        } else if existing > 0 {
            emit_progress(
                progress.as_ref(),
                transfer_id,
                "download",
                item,
                "resumed",
                0,
                aggregate.load(Ordering::Relaxed),
                Some(total_size),
                &link,
                false,
            );
        }

        let client = match client_for(link.local_ip) {
            Ok(client) => client,
            Err(error) => {
                pool.failure(&lease).await;
                if attempt >= MAX_SEGMENT_RETRIES {
                    return Err(error);
                }
                continue;
            }
        };

        let resume_start = start + current_len;
        let req = request_with_headers(
            client
                .get(url)
                .header(RANGE, format!("bytes={resume_start}-{end}")),
            headers,
        );

        let response = match req.send().await {
            Ok(response) => response,
            Err(error) => {
                pool.failure(&lease).await;

                if attempt >= MAX_SEGMENT_RETRIES {
                    return Err(error).context("segment request failed after multi-WAN failover");
                }

                continue;
            }
        };

        if response.status() != StatusCode::PARTIAL_CONTENT {
            let status = response.status();
            pool.failure(&lease).await;

            if (status.is_server_error()
                || status == StatusCode::TOO_MANY_REQUESTS
                || status == StatusCode::REQUEST_TIMEOUT)
                && attempt < MAX_SEGMENT_RETRIES
            {
                continue;
            }

            bail!(
                "server ignored HTTP Range for {} on {} (status {})",
                link.name,
                link.local_ip,
                status
            );
        }

        if response.headers().get(CONTENT_RANGE).is_none() {
            pool.failure(&lease).await;

            if attempt < MAX_SEGMENT_RETRIES {
                continue;
            }

            bail!("server returned 206 without Content-Range");
        }

        let mut stream = response.bytes_stream();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(part_path)
            .await?;

        let mut pending_delta = 0u64;
        let mut attempt_bytes = 0u64;
        let mut last_emit = Instant::now();
        let mut stream_failed = false;

        while let Some(next) = stream.next().await {
            checkpoint(control.as_ref()).await?;

            match next {
                Ok(chunk) => {
                    let delta = chunk.len() as u64;
                    throttle.consume(delta).await;
                    file.write_all(&chunk).await?;
                    attempt_bytes += delta;
                    pending_delta += delta;
                    let overall = aggregate.fetch_add(delta, Ordering::Relaxed) + delta;

                    if last_emit.elapsed() >= PROGRESS_INTERVAL {
                        emit_progress(
                            progress.as_ref(),
                            transfer_id,
                            "download",
                            item,
                            if attempt > 1 { "failover-active" } else { "transferring" },
                            pending_delta,
                            overall,
                            Some(total_size),
                            &link,
                            false,
                        );
                        pending_delta = 0;
                        last_emit = Instant::now();
                    }
                }
                Err(_) => {
                    stream_failed = true;
                    break;
                }
            }
        }

        file.flush().await?;

        if pending_delta > 0 {
            emit_progress(
                progress.as_ref(),
                transfer_id,
                "download",
                item,
                if attempt > 1 { "failover-active" } else { "transferring" },
                pending_delta,
                aggregate.load(Ordering::Relaxed),
                Some(total_size),
                &link,
                false,
            );
        }

        let final_len = part_len(part_path).await;

        if final_len == expected {
            pool.success(&lease, attempt_bytes).await;
            return Ok((expected, links_used.into_iter().collect()));
        }

        pool.failure(&lease).await;

        if attempt >= MAX_SEGMENT_RETRIES {
            if stream_failed {
                bail!(
                    "segment {}-{} failed after {} multi-WAN attempts with {} of {} bytes",
                    start,
                    end,
                    MAX_SEGMENT_RETRIES,
                    final_len,
                    expected
                );
            }

            bail!(
                "segment {}-{} ended with {} bytes, expected {}",
                start,
                end,
                final_len,
                expected
            );
        }

        sleep(retry_backoff(attempt)).await;
    }
}

async fn prepare_part_dir(
    part_dir: &Path,
    url: &str,
    size: u64,
    segments: usize,
) -> Result<()> {
    let expected = PartManifest {
        url: url.to_owned(),
        size,
        segments,
    };
    let manifest_path = part_dir.join("manifest.json");

    let reusable = match fs::read_to_string(&manifest_path).await {
        Ok(raw) => serde_json::from_str::<PartManifest>(&raw)
            .map(|stored| stored == expected)
            .unwrap_or(false),
        Err(_) => false,
    };

    if part_dir.exists() && !reusable {
        fs::remove_dir_all(part_dir).await?;
    }

    fs::create_dir_all(part_dir).await?;

    if !reusable {
        fs::write(&manifest_path, serde_json::to_vec_pretty(&expected)?).await?;
    }

    Ok(())
}

async fn part_len(path: &Path) -> u64 {
    fs::metadata(path).await.map(|metadata| metadata.len()).unwrap_or(0)
}

async fn checkpoint(control: Option<&TransferControl>) -> Result<()> {
    if let Some(control) = control {
        control.checkpoint().await?;
    }

    Ok(())
}

fn retry_backoff(attempt: usize) -> Duration {
    Duration::from_millis(400 * 2u64.pow((attempt.saturating_sub(1)).min(4) as u32))
}

async fn verify_sha256(path: &Path, expected: Option<&str>) -> Result<(Option<String>, bool)> {
    let Some(expected) = expected
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok((None, false));
    };

    let expected = expected
        .trim_start_matches("sha256:")
        .trim()
        .to_ascii_lowercase();

    if expected.len() != 64 || !expected.chars().all(|ch| ch.is_ascii_hexdigit()) {
        bail!("expected SHA256 must contain exactly 64 hexadecimal characters");
    }

    let mut file = File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];

    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    let actual = format!("{:x}", hasher.finalize());

    if actual != expected {
        bail!("SHA256 mismatch: expected {expected}, got {actual}");
    }

    Ok((Some(actual), true))
}

fn emit_progress(
    callback: Option<&ProgressCallback>,
    transfer_id: &str,
    direction: &str,
    item: &str,
    phase: &str,
    bytes_delta: u64,
    bytes_transferred: u64,
    total_bytes: Option<u64>,
    link: &LinkConfig,
    completed: bool,
) {
    if let Some(callback) = callback {
        callback(TransferProgress {
            transfer_id: transfer_id.to_owned(),
            direction: direction.to_owned(),
            item: item.to_owned(),
            phase: phase.to_owned(),
            bytes_delta,
            bytes_transferred,
            total_bytes,
            link_name: link.name.clone(),
            local_ip: link.local_ip,
            completed,
        });
    }
}

fn client_for(local_ip: IpAddr) -> Result<Client> {
    Client::builder()
        .local_address(local_ip)
        .build()
        .context("failed to build HTTP client bound to local IP")
}

fn adaptive_segment_count(size: u64, workers: usize) -> usize {
    const TARGET_MIN_SEGMENT: u64 = 8 * 1024 * 1024;
    const WAVES: usize = 16;

    let max_possible = size.min(usize::MAX as u64) as usize;
    let workers = workers.max(1).min(max_possible.max(1));
    let desired = workers.saturating_mul(WAVES).max(workers);
    let by_size = size
        .saturating_add(TARGET_MIN_SEGMENT - 1)
        .checked_div(TARGET_MIN_SEGMENT)
        .unwrap_or(1)
        .max(workers as u64)
        .min(usize::MAX as u64) as usize;

    desired.min(by_size).min(max_possible.max(1)).max(workers)
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

fn output_name(output: &Path) -> String {
    output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download")
        .to_owned()
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
    use super::{adaptive_segment_count, segment_bounds, total_from_content_range, PartManifest};

    #[test]
    fn segment_bounds_cover_entire_file_without_overlap() {
        let size = 10u64;
        let segments = 3usize;
        assert_eq!(segment_bounds(size, segments, 0), (0, 3));
        assert_eq!(segment_bounds(size, segments, 1), (4, 6));
        assert_eq!(segment_bounds(size, segments, 2), (7, 9));
    }

    #[test]
    fn adaptive_segmentation_creates_multiple_waves() {
        let gib = 1024u64 * 1024 * 1024;
        assert_eq!(adaptive_segment_count(gib, 8), 128);
        assert!(adaptive_segment_count(64 * 1024 * 1024, 8) >= 8);
    }

    #[test]
    fn parses_total_size_from_content_range() {
        assert_eq!(total_from_content_range("bytes 0-0/12345"), Some(12345));
        assert_eq!(total_from_content_range("bytes 0-0/*"), None);
    }

    #[test]
    fn part_manifest_detects_incompatible_resume() {
        let a = PartManifest {
            url: "https://example.test/file".to_string(),
            size: 100,
            segments: 8,
        };
        let b = PartManifest {
            url: "https://example.test/file".to_string(),
            size: 100,
            segments: 4,
        };

        assert_ne!(a, b);
    }
}
