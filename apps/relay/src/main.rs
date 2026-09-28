use anyhow::{Context, Result};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    routing::{get, post, put},
    Json, Router,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    env,
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use stordown_core::{
    RelayCompleteResponse, RelayCreateSessionRequest, RelaySession, RelaySessionStatus,
};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
};
use uuid::Uuid;

#[derive(Clone)]
struct RelayState {
    root: Arc<PathBuf>,
    token: Option<Arc<String>>,
}

type ApiError = (StatusCode, String);
type ApiResult<T> = Result<T, ApiError>;

#[tokio::main]
async fn main() -> Result<()> {
    let bind = env::var("STORDOWN_RELAY_BIND").unwrap_or_else(|_| "0.0.0.0:17834".to_string());
    let root = env::var("STORDOWN_RELAY_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./stordown-relay-data"));
    let token = env::var("STORDOWN_RELAY_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(Arc::new);

    fs::create_dir_all(root.join("sessions")).await?;
    fs::create_dir_all(root.join("completed")).await?;

    let state = RelayState {
        root: Arc::new(root),
        token,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/sessions", post(create_session))
        .route("/v1/sessions/{id}", get(session_status))
        .route("/v1/sessions/{id}/chunks/{index}", put(upload_chunk))
        .route("/v1/sessions/{id}/complete", post(complete_session))
        .with_state(state);

    let address: SocketAddr = bind
        .parse()
        .with_context(|| format!("invalid STORDOWN_RELAY_BIND: {bind}"))?;
    let listener = tokio::net::TcpListener::bind(address).await?;

    println!("StorDown Relay listening on http://{address}");
    if env::var("STORDOWN_RELAY_TOKEN").is_err() {
        eprintln!("WARNING: STORDOWN_RELAY_TOKEN is not configured; relay is unauthenticated");
    }

    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({
        "ok": true,
        "service": "stordown-relay",
        "protocol": 1
    }))
}

async fn create_session(
    State(state): State<RelayState>,
    headers: HeaderMap,
    Json(request): Json<RelayCreateSessionRequest>,
) -> ApiResult<Json<RelaySession>> {
    authorize(&state, &headers)?;

    if request.file_name.trim().is_empty() {
        return Err(bad_request("file_name is required"));
    }
    if request.chunk_size == 0 {
        return Err(bad_request("chunk_size must be greater than zero"));
    }

    let id = Uuid::new_v4().to_string();
    let session = RelaySession {
        id: id.clone(),
        file_name: safe_file_name(&request.file_name),
        total_size: request.total_size,
        chunk_size: request.chunk_size,
        total_chunks: chunk_count(request.total_size, request.chunk_size),
        sha256: request
            .sha256
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty()),
        completed: false,
        result_path: None,
    };

    let dir = session_dir(&state, &id);
    fs::create_dir_all(dir.join("chunks"))
        .await
        .map_err(internal_error)?;
    save_session(&state, &session).await?;

    Ok(Json(session))
}

async fn session_status(
    State(state): State<RelayState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Json<RelaySessionStatus>> {
    authorize(&state, &headers)?;
    let session = load_session(&state, &id).await?;
    let (received_chunks, received_bytes) = scan_chunks(&state, &session).await?;

    Ok(Json(RelaySessionStatus {
        session,
        received_chunks,
        received_bytes,
    }))
}

async fn upload_chunk(
    State(state): State<RelayState>,
    headers: HeaderMap,
    Path((id, index)): Path<(String, u64)>,
    body: Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    authorize(&state, &headers)?;
    let session = load_session(&state, &id).await?;

    if session.completed {
        return Err((StatusCode::CONFLICT, "session is already completed".to_string()));
    }
    if index >= session.total_chunks {
        return Err(bad_request("chunk index is out of range"));
    }

    let expected = expected_chunk_size(&session, index);
    if body.len() as u64 != expected {
        return Err(bad_request(&format!(
            "chunk size mismatch: expected {expected}, got {}",
            body.len()
        )));
    }

    if let Some(expected_hash) = headers
        .get("x-stordown-chunk-sha256")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
    {
        let actual = sha256_bytes(&body);
        if !actual.eq_ignore_ascii_case(expected_hash.trim()) {
            return Err(bad_request("chunk SHA256 mismatch"));
        }
    }

    let chunks = session_dir(&state, &id).join("chunks");
    fs::create_dir_all(&chunks).await.map_err(internal_error)?;
    let final_path = chunks.join(format!("{index:020}.part"));
    let temp_path = chunks.join(format!("{index:020}.{}.tmp", Uuid::new_v4()));

    fs::write(&temp_path, &body).await.map_err(internal_error)?;
    if fs::try_exists(&final_path).await.map_err(internal_error)? {
        fs::remove_file(&final_path).await.map_err(internal_error)?;
    }
    fs::rename(&temp_path, &final_path).await.map_err(internal_error)?;

    Ok(Json(json!({
        "ok": true,
        "session_id": id,
        "chunk": index,
        "bytes": body.len()
    })))
}

async fn complete_session(
    State(state): State<RelayState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Json<RelayCompleteResponse>> {
    authorize(&state, &headers)?;
    let mut session = load_session(&state, &id).await?;

    if session.completed {
        let result_path = session
            .result_path
            .clone()
            .ok_or_else(|| internal_message("completed session is missing result_path"))?;
        let hash = session.sha256.clone().unwrap_or_default();
        return Ok(Json(RelayCompleteResponse {
            session_id: session.id,
            result_path,
            total_size: session.total_size,
            sha256: hash,
        }));
    }

    let (received_chunks, received_bytes) = scan_chunks(&state, &session).await?;
    if received_chunks.len() as u64 != session.total_chunks || received_bytes != session.total_size {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "session is incomplete: {}/{} chunks, {}/{} bytes",
                received_chunks.len(),
                session.total_chunks,
                received_bytes,
                session.total_size
            ),
        ));
    }

    let completed_dir = state.root.join("completed");
    fs::create_dir_all(&completed_dir)
        .await
        .map_err(internal_error)?;
    let result_path = completed_dir.join(format!("{}-{}", session.id, session.file_name));
    let temp_path = completed_dir.join(format!("{}.assembling", session.id));
    let mut output = fs::File::create(&temp_path).await.map_err(internal_error)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];

    for index in 0..session.total_chunks {
        let path = session_dir(&state, &session.id)
            .join("chunks")
            .join(format!("{index:020}.part"));
        let mut input = fs::File::open(&path).await.map_err(internal_error)?;

        loop {
            let count = input.read(&mut buffer).await.map_err(internal_error)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
            output
                .write_all(&buffer[..count])
                .await
                .map_err(internal_error)?;
        }
    }

    output.flush().await.map_err(internal_error)?;
    drop(output);

    let digest = format!("{:x}", hasher.finalize());
    if let Some(expected) = session.sha256.as_deref() {
        if !digest.eq_ignore_ascii_case(expected) {
            let _ = fs::remove_file(&temp_path).await;
            return Err(bad_request(&format!(
                "final SHA256 mismatch: expected {expected}, got {digest}"
            )));
        }
    }

    if fs::try_exists(&result_path).await.map_err(internal_error)? {
        fs::remove_file(&result_path).await.map_err(internal_error)?;
    }
    fs::rename(&temp_path, &result_path)
        .await
        .map_err(internal_error)?;

    session.completed = true;
    session.sha256 = Some(digest.clone());
    session.result_path = Some(result_path.to_string_lossy().to_string());
    save_session(&state, &session).await?;

    Ok(Json(RelayCompleteResponse {
        session_id: session.id,
        result_path: session.result_path.unwrap_or_default(),
        total_size: session.total_size,
        sha256: digest,
    }))
}

fn authorize(state: &RelayState, headers: &HeaderMap) -> ApiResult<()> {
    let Some(expected) = state.token.as_deref() else {
        return Ok(());
    };

    let actual = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();

    if actual == expected.as_str() {
        Ok(())
    } else {
        Err((StatusCode::UNAUTHORIZED, "invalid relay token".to_string()))
    }
}

async fn load_session(state: &RelayState, id: &str) -> ApiResult<RelaySession> {
    if !valid_session_id(id) {
        return Err(bad_request("invalid session id"));
    }

    let bytes = fs::read(session_dir(state, id).join("session.json"))
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                (StatusCode::NOT_FOUND, "relay session not found".to_string())
            } else {
                internal_error(error)
            }
        })?;

    serde_json::from_slice(&bytes).map_err(internal_error)
}

async fn save_session(state: &RelayState, session: &RelaySession) -> ApiResult<()> {
    let dir = session_dir(state, &session.id);
    fs::create_dir_all(&dir).await.map_err(internal_error)?;
    let bytes = serde_json::to_vec_pretty(session).map_err(internal_error)?;
    let temp = dir.join("session.json.tmp");
    fs::write(&temp, bytes).await.map_err(internal_error)?;
    let target = dir.join("session.json");
    if fs::try_exists(&target).await.map_err(internal_error)? {
        fs::remove_file(&target).await.map_err(internal_error)?;
    }
    fs::rename(temp, target).await.map_err(internal_error)?;
    Ok(())
}

async fn scan_chunks(
    state: &RelayState,
    session: &RelaySession,
) -> ApiResult<(Vec<u64>, u64)> {
    let chunks_dir = session_dir(state, &session.id).join("chunks");
    let mut received = Vec::new();
    let mut bytes = 0u64;

    if !fs::try_exists(&chunks_dir).await.map_err(internal_error)? {
        return Ok((received, bytes));
    }

    let mut entries = fs::read_dir(chunks_dir).await.map_err(internal_error)?;
    while let Some(entry) = entries.next_entry().await.map_err(internal_error)? {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(raw_index) = name.strip_suffix(".part") else {
            continue;
        };
        let Ok(index) = raw_index.parse::<u64>() else {
            continue;
        };
        if index >= session.total_chunks {
            continue;
        }

        let metadata = entry.metadata().await.map_err(internal_error)?;
        if metadata.len() == expected_chunk_size(session, index) {
            received.push(index);
            bytes = bytes.saturating_add(metadata.len());
        }
    }

    received.sort_unstable();
    Ok((received, bytes))
}

fn session_dir(state: &RelayState, id: &str) -> PathBuf {
    state.root.join("sessions").join(id)
}

fn expected_chunk_size(session: &RelaySession, index: u64) -> u64 {
    let offset = index.saturating_mul(session.chunk_size);
    session
        .chunk_size
        .min(session.total_size.saturating_sub(offset))
}

fn chunk_count(total_size: u64, chunk_size: u64) -> u64 {
    if total_size == 0 {
        0
    } else {
        (total_size + chunk_size - 1) / chunk_size
    }
}

fn safe_file_name(raw: &str) -> String {
    let name = FsPath::new(raw)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("upload.bin");

    let cleaned: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | ' ') {
                ch
            } else {
                '_'
            }
        })
        .collect();

    if cleaned.trim().is_empty() {
        "upload.bin".to_string()
    } else {
        cleaned
    }
}

fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn bad_request(message: &str) -> ApiError {
    (StatusCode::BAD_REQUEST, message.to_string())
}

fn internal_message(message: &str) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, message.to_string())
}

fn internal_error(error: impl std::fmt::Display) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

#[allow(dead_code)]
fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
