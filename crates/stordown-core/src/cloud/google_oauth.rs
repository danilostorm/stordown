use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::{rngs::OsRng, RngCore};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};
use url::Url;

const AUTHORIZE_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
pub const DRIVE_FILE_SCOPE: &str = "https://www.googleapis.com/auth/drive.file";
pub const DRIVE_READONLY_SCOPE: &str =
    "https://www.googleapis.com/auth/drive.readonly";

fn drive_scopes() -> String {
    format!("{DRIVE_FILE_SCOPE} {DRIVE_READONLY_SCOPE}")
}

#[derive(Debug, Clone, Serialize)]
pub struct GoogleOAuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u64,
    pub scope: Option<String>,
    pub token_type: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    expires_in: u64,
    #[serde(default)]
    scope: Option<String>,
    token_type: String,
}

#[derive(Debug)]
struct OAuthCallback {
    code: String,
    state: String,
}

pub async fn authorize_google_drive_desktop(client_id: &str) -> Result<GoogleOAuthTokens> {
    if client_id.trim().is_empty() {
        bail!("Google OAuth client ID is required");
    }

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("failed to start local OAuth callback listener")?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}");

    let verifier = random_urlsafe(64);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_urlsafe(32);

    let mut auth_url = Url::parse(AUTHORIZE_ENDPOINT)?;
    auth_url
        .query_pairs_mut()
        .append_pair("client_id", client_id.trim())
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &drive_scopes())
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");

    open_system_browser(auth_url.as_str())?;

    let callback = timeout(Duration::from_secs(180), receive_callback(listener))
        .await
        .context("Google authorization timed out")??;

    if callback.state != state {
        bail!("OAuth state mismatch");
    }

    exchange_code(client_id.trim(), &callback.code, &redirect_uri, &verifier).await
}

pub async fn refresh_google_access_token(
    client_id: &str,
    refresh_token: &str,
) -> Result<GoogleOAuthTokens> {
    let client = Client::new();
    let response = client
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("client_id", client_id),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .context("failed to refresh Google access token")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("Google token refresh failed ({status}): {body}");
    }

    let response: TokenResponse = response.json().await?;
    Ok(GoogleOAuthTokens {
        access_token: response.access_token,
        refresh_token: response.refresh_token,
        expires_in: response.expires_in,
        scope: response.scope,
        token_type: response.token_type,
    })
}

async fn exchange_code(
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Result<GoogleOAuthTokens> {
    let client = Client::new();
    let response = client
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("client_id", client_id),
            ("code", code),
            ("code_verifier", verifier),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .await
        .context("failed to exchange Google authorization code")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("Google authorization code exchange failed ({status}): {body}");
    }

    let response: TokenResponse = response.json().await?;
    Ok(GoogleOAuthTokens {
        access_token: response.access_token,
        refresh_token: response.refresh_token,
        expires_in: response.expires_in,
        scope: response.scope,
        token_type: response.token_type,
    })
}

async fn receive_callback(listener: TcpListener) -> Result<OAuthCallback> {
    let (mut stream, _) = listener.accept().await?;
    let mut buffer = vec![0u8; 16 * 1024];
    let read = stream.read(&mut buffer).await?;
    let request = String::from_utf8_lossy(&buffer[..read]);
    let first_line = request.lines().next().context("invalid OAuth callback request")?;
    let target = first_line
        .split_whitespace()
        .nth(1)
        .context("OAuth callback request did not contain a target")?;

    let callback_url = Url::parse(&format!("http://127.0.0.1{target}"))?;
    let mut code = None;
    let mut state = None;
    let mut error = None;

    for (key, value) in callback_url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            _ => {}
        }
    }

    let (status_line, body) = if error.is_some() {
        (
            "HTTP/1.1 400 Bad Request",
            "<html><body><h2>StorDown</h2><p>Autorização cancelada ou recusada. Você pode fechar esta janela.</p></body></html>",
        )
    } else {
        (
            "HTTP/1.1 200 OK",
            "<html><body><h2>StorDown conectado ao Google Drive</h2><p>Você pode fechar esta janela e voltar ao StorDown.</p></body></html>",
        )
    };

    let response = format!(
        "{status_line}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.as_bytes().len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await?;

    if let Some(error) = error {
        bail!("Google authorization failed: {error}");
    }

    Ok(OAuthCallback {
        code: code.context("Google callback did not contain an authorization code")?,
        state: state.context("Google callback did not contain OAuth state")?,
    })
}

fn random_urlsafe(bytes: usize) -> String {
    let mut random = vec![0u8; bytes];
    OsRng.fill_bytes(&mut random);
    URL_SAFE_NO_PAD.encode(random)
}

#[cfg(target_os = "windows")]
fn open_system_browser(url: &str) -> Result<()> {
    let status = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .status()
        .context("failed to open the Windows browser")?;

    if !status.success() {
        bail!("Windows could not open the OAuth URL");
    }

    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn open_system_browser(_url: &str) -> Result<()> {
    bail!("automatic OAuth browser launch is currently implemented for Windows")
}

#[cfg(test)]
mod tests {
    use super::{drive_scopes, random_urlsafe, DRIVE_FILE_SCOPE, DRIVE_READONLY_SCOPE};

    #[test]
    fn google_drive_scope_set_supports_upload_and_metadata_browsing() {
        let scopes = drive_scopes();
        assert!(scopes.contains(DRIVE_FILE_SCOPE));
        assert!(scopes.contains(DRIVE_READONLY_SCOPE));
    }

    #[test]
    fn verifier_is_pkce_length_compatible() {
        let verifier = random_urlsafe(64);
        assert!(verifier.len() >= 43);
        assert!(verifier.len() <= 128);
        assert!(!verifier.contains('='));
    }
}
