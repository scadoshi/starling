//! Sign-in with the OAuth 2.0 device-code flow, and the saved token that keeps it.
//!
//! Nothing here prints. The event loop shows the device code on screen.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{env, fmt, fs};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::api::CLIENT;
use crate::config;

const SCOPES: &str = "User.Read Chat.ReadWrite offline_access";
const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";

/// The environment variable that overrides the client ID in `config.json`.
const CLIENT_ID_VAR: &str = "CLIENT_ID";

/// A token this close to expiry is refreshed before use, in seconds.
const EXPIRY_MARGIN_SECS: u64 = 300;

/// Added to the poll interval each time the server answers `slow_down`, per RFC 8628.
const SLOW_DOWN_SECS: u64 = 5;

/// Stands in for a secret in `Debug` output.
const REDACTED: &str = "<redacted>";

/// A device code to show the user: open `verification_uri` and enter `user_code`.
#[derive(Clone, Deserialize)]
pub(crate) struct DeviceCode {
    pub(crate) user_code: String,
    pub(crate) verification_uri: String,
    /// What the app polls with. It can be redeemed for a token, so it is never shown.
    #[serde(rename = "device_code")]
    secret: String,
    /// Seconds between polls.
    interval: u64,
}

/// Shows everything but the device code, which can be redeemed for a token.
impl fmt::Debug for DeviceCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceCode")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .field("secret", &REDACTED)
            .field("interval", &self.interval)
            .finish()
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    /// Seconds from issue.
    expires_in: u64,
    refresh_token: Option<String>,
    /// Unix seconds. Zero in a response, set before saving.
    #[serde(default)]
    expires_at: u64,
    /// The app the token was issued to, which is the app that must refresh it. Set before saving.
    #[serde(default)]
    client_id: Option<String>,
}

/// Shows the expiry and whether a refresh token is present, never the tokens.
impl fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenResponse")
            .field("access_token", &REDACTED)
            .field("token_type", &self.token_type)
            .field("expires_in", &self.expires_in)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| REDACTED),
            )
            .field("expires_at", &self.expires_at)
            .field("client_id", &self.client_id)
            .finish()
    }
}

#[derive(Debug, Deserialize)]
struct TokenErrorResponse {
    error: String,
    #[serde(default)]
    error_description: String,
}

/// Microsoft's explanation from an error body, without the trace and correlation IDs it appends, or the whole body when it is not a Microsoft error.
fn describe_error(body: &str) -> String {
    serde_json::from_str::<TokenErrorResponse>(body)
        .ok()
        .map(|e| e.error_description)
        .filter(|d| !d.is_empty())
        .map_or_else(
            || body.trim().to_owned(),
            |d| {
                d.split("Trace ID:")
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned()
            },
        )
}

/// The Azure app to sign in with: `CLIENT_ID` from the environment, else `config.json`.
pub(crate) fn client_id() -> Option<String> {
    env::var(CLIENT_ID_VAR)
        .ok()
        .filter(|id| !id.trim().is_empty())
        .or_else(|| config::load_config()?.client_id)
}

/// Whether `id` looks like an Azure client ID: a GUID such as `12345678-1234-1234-1234-123456789abc`.
pub(crate) fn is_client_id(id: &str) -> bool {
    let groups: Vec<&str> = id.split('-').collect();
    groups.iter().map(|g| g.len()).eq([8, 4, 4, 4, 12])
        && groups
            .iter()
            .all(|g| g.chars().all(|c| c.is_ascii_hexdigit()))
}

fn now_secs() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn token_path() -> Result<PathBuf> {
    Ok(config::app_dir()?.join("token.json"))
}

/// Stamps the expiry time and client ID and saves the token owner-only.
fn save_token(mut token: TokenResponse, client_id: &str) -> Result<TokenResponse> {
    token.expires_at = now_secs()?.saturating_add(token.expires_in);
    token.client_id = Some(client_id.to_owned());
    config::write_private(&token_path()?, &serde_json::to_string_pretty(&token)?)?;
    Ok(token)
}

fn load_token() -> Result<Option<TokenResponse>> {
    let path = token_path()?;
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&fs::read_to_string(path)?)?))
}

/// Forgets the signed-in account: deletes the saved token and the cached profile.
///
/// The client ID stays, so the next sign-in uses the same app.
pub(crate) fn sign_out() -> Result<()> {
    config::remove(&token_path()?)?;
    crate::api::forget_profile()
}

/// Asks Microsoft for a device code to show the user.
pub(crate) async fn start_device_flow(client_id: &str) -> Result<DeviceCode> {
    let response = CLIENT
        .post(DEVICE_CODE_URL)
        .form(&[("client_id", client_id), ("scope", SCOPES)])
        .send()
        .await?;

    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        anyhow::bail!(
            "Could not start sign-in ({status}): {}",
            describe_error(&text)
        );
    }
    serde_json::from_str(&text).with_context(|| format!("Unexpected device code response: {text}"))
}

/// Waits until the user enters the code, then saves the token.
pub(crate) async fn poll_for_token(client_id: &str, code: &DeviceCode) -> Result<()> {
    let mut interval = code.interval;

    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;

        let response = CLIENT
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", client_id),
                ("device_code", code.secret.as_str()),
            ])
            .send()
            .await?;

        if response.status().is_success() {
            save_token(response.json().await?, client_id)?;
            return Ok(());
        }

        let body = response.text().await?;
        let error = serde_json::from_str::<TokenErrorResponse>(&body)
            .map(|e| e.error)
            .unwrap_or_default();
        match error.as_str() {
            "authorization_pending" => {}
            "slow_down" => interval = interval.saturating_add(SLOW_DOWN_SECS),
            "authorization_declined" => anyhow::bail!("Sign-in was declined"),
            "expired_token" => anyhow::bail!("The sign-in code expired before it was used"),
            _ => anyhow::bail!("Sign-in failed: {}", describe_error(&body)),
        }
    }
}

async fn refresh_access_token(refresh_token: &str, client_id: &str) -> Result<TokenResponse> {
    let response = CLIENT
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
            ("scope", SCOPES),
        ])
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!(
            "Could not refresh the sign-in ({status}): {}",
            describe_error(&body)
        );
    }
    save_token(response.json().await?, client_id)
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
    let client_id = token
        .client_id
        .or_else(client_id)
        .context("No client ID to refresh the token with")?;
    Ok(refresh_access_token(&refresh_token, &client_id)
        .await?
        .access_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_hides_secrets() {
        let token = TokenResponse {
            access_token: "hunter2-access".to_owned(),
            token_type: "Bearer".to_owned(),
            expires_in: 3600,
            refresh_token: Some("hunter2-refresh".to_owned()),
            expires_at: 0,
            client_id: None,
        };
        let device = DeviceCode {
            user_code: "ABCD1234".to_owned(),
            verification_uri: "https://microsoft.com/devicelogin".to_owned(),
            secret: "hunter2-device".to_owned(),
            interval: 5,
        };
        let shown = format!("{token:?} {device:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(
            shown.contains("Bearer") && shown.contains("ABCD1234"),
            "{shown}"
        );
    }

    #[test]
    fn device_code_parses_from_microsoft_json() {
        let code: DeviceCode = serde_json::from_str(
            r#"{"user_code":"FJ9WRZ8LK","device_code":"DAQABAAEAAAD","verification_uri":"https://microsoft.com/devicelogin",
                "expires_in":900,"interval":5,"message":"To sign in, use a web browser"}"#,
        )
        .unwrap();
        assert_eq!(code.user_code, "FJ9WRZ8LK");
        assert_eq!(code.interval, 5);
    }

    #[test]
    fn is_client_id_accepts_only_guids() {
        assert!(is_client_id("12345678-1234-1234-1234-123456789abc"));
        assert!(is_client_id("ABCDEF01-2345-6789-ABCD-EF0123456789"));
        assert!(!is_client_id("your-client-id-here"));
        assert!(!is_client_id("12345678-1234-1234-1234-123456789abg"));
        assert!(!is_client_id("12345678123412341234123456789abc"));
        assert!(!is_client_id(""));
    }

    #[test]
    fn tokens_saved_before_the_client_id_field_still_load() {
        let token: TokenResponse = serde_json::from_str(
            r#"{"access_token":"a","token_type":"Bearer","expires_in":1,"refresh_token":null,"expires_at":5}"#,
        )
        .unwrap();
        assert!(token.client_id.is_none());
    }

    #[test]
    fn describe_error_keeps_microsofts_reason_and_drops_the_ids() {
        let body = r#"{"error":"invalid_request","error_description":"AADSTS50059: No tenant-identifying information found.\r\nTrace ID: 15bf22c9\r\nCorrelation ID: 728af49b","error_codes":[50059]}"#;
        assert_eq!(
            describe_error(body),
            "AADSTS50059: No tenant-identifying information found."
        );
        assert_eq!(describe_error(" plain failure "), "plain failure");
    }
}
