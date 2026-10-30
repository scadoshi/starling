//! Sign-in with the OAuth 2.0 device-code flow, and the saved token that keeps it.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::api::CLIENT;

const SCOPES: &str = "User.Read Chat.ReadWrite offline_access";
const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";

/// A token this close to expiry is refreshed before use, in seconds.
const EXPIRY_MARGIN_SECS: u64 = 300;

/// Added to the poll interval each time the server answers `slow_down`, per RFC 8628.
const SLOW_DOWN_SECS: u64 = 5;

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    interval: u64,
    /// Tells the user where to go and which code to enter.
    message: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    /// Seconds from issue.
    expires_in: u64,
    refresh_token: Option<String>,
    /// Unix seconds. Zero in a response, set before saving.
    #[serde(default)]
    expires_at: u64,
}

#[derive(Debug, Deserialize)]
struct TokenErrorResponse {
    error: String,
}

fn client_id() -> Result<String> {
    dotenvy::dotenv().ok();
    if let Ok(id) = std::env::var("CLIENT_ID") {
        return Ok(id);
    }

    crate::config::load_config()
        .and_then(|config| config.client_id)
        .with_context(|| {
            let path = crate::config::app_dir()
                .map_or_else(|_| "config.json".into(), |dir| dir.join("config.json"));
            format!(
                "No client ID set. Put your Azure app's client ID in {} or the CLIENT_ID environment variable; AZURE_SETUP.md has the steps",
                path.display()
            )
        })
}

fn now_secs() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn token_path() -> Result<PathBuf> {
    Ok(crate::config::app_dir()?.join("token.json"))
}

/// Stamps the expiry time and saves the token owner-only.
fn save_token(mut token: TokenResponse) -> Result<TokenResponse> {
    token.expires_at = now_secs()?.saturating_add(token.expires_in);
    crate::config::write_private(&token_path()?, &serde_json::to_string_pretty(&token)?)?;
    Ok(token)
}

fn load_token() -> Result<Option<TokenResponse>> {
    let path = token_path()?;
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&fs::read_to_string(path)?)?))
}

async fn start_device_flow() -> Result<DeviceCodeResponse> {
    let client_id = client_id()?;
    let response = CLIENT
        .post(DEVICE_CODE_URL)
        .form(&[("client_id", client_id.as_str()), ("scope", SCOPES)])
        .send()
        .await?;

    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        anyhow::bail!("Failed to start device flow ({status}): {text}");
    }
    serde_json::from_str(&text).with_context(|| format!("Unexpected device code response: {text}"))
}

async fn poll_for_token(device_code: &str, interval: u64) -> Result<TokenResponse> {
    let client_id = client_id()?;
    let mut interval = interval;

    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;

        let response = CLIENT
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", client_id.as_str()),
                ("device_code", device_code),
            ])
            .send()
            .await?;

        if response.status().is_success() {
            return save_token(response.json().await?);
        }

        match response.json::<TokenErrorResponse>().await?.error.as_str() {
            "authorization_pending" => {}
            "slow_down" => interval = interval.saturating_add(SLOW_DOWN_SECS),
            "authorization_declined" => anyhow::bail!("Sign-in was declined"),
            "expired_token" => anyhow::bail!("The sign-in code expired before it was used"),
            other => anyhow::bail!("Sign-in failed: {other}"),
        }
    }
}

async fn refresh_access_token(refresh_token: &str) -> Result<TokenResponse> {
    let client_id = client_id()?;
    let response = CLIENT
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id.as_str()),
            ("refresh_token", refresh_token),
            ("scope", SCOPES),
        ])
        .send()
        .await?;

    if !response.status().is_success() {
        anyhow::bail!("Failed to refresh token: {}", response.status());
    }
    save_token(response.json().await?)
}

/// An access token from the saved token, refreshed when it is near expiry, without asking the user.
pub(crate) async fn fetch_token_silently() -> Result<String> {
    let token = load_token()?.context("Not signed in")?;
    if token.expires_at > now_secs()?.saturating_add(EXPIRY_MARGIN_SECS) {
        return Ok(token.access_token);
    }
    let refresh_token = token
        .refresh_token
        .context("Token expired and has no refresh token")?;
    Ok(refresh_access_token(&refresh_token).await?.access_token)
}

/// An access token, signing in with a device code on the terminal when there is no usable saved token.
#[expect(
    clippy::print_stdout,
    reason = "the sign-in prompt prints before the TUI takes the terminal"
)]
pub(crate) async fn fetch_access_token() -> Result<String> {
    if let Ok(token) = fetch_token_silently().await {
        return Ok(token);
    }

    let device_code = start_device_flow().await?;
    println!("\n{}", device_code.message);
    println!("\nWaiting for sign-in...\n");

    let token = poll_for_token(&device_code.device_code, device_code.interval).await?;
    Ok(token.access_token)
}
