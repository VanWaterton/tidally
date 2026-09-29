//! OAuth2 device-code login against auth.tidal.com.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::API_BASE;

const AUTH_BASE: &str = "https://auth.tidal.com/v1/oauth2";
const SCOPE: &str = "r_usr w_usr w_sub";

#[derive(Debug, Clone)]
pub struct Credentials {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: u64,
}

impl DeviceCode {
    pub fn link(&self) -> String {
        let uri = self
            .verification_uri_complete
            .as_deref()
            .unwrap_or(&self.verification_uri);
        if uri.starts_with("http") {
            uri.to_string()
        } else {
            format!("https://{uri}")
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix timestamp (seconds) when the access token expires.
    pub expires_at: u64,
}

impl Tokens {
    pub fn expires_soon(&self) -> bool {
        now() + 60 >= self.expires_at
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// The client these tokens were issued to. Tokens only work with that client, so a
    /// different configured client means signing in again. Empty for older session files.
    #[serde(default)]
    pub client_id: String,
    pub tokens: Tokens,
    pub user_id: u64,
    pub country_code: String,
}

impl Session {
    pub fn load(path: &Path) -> Option<Self> {
        let text = fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("writing {}", path.display()))?;
        file.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
        Ok(())
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
}

#[derive(Deserialize)]
struct TokenError {
    error: String,
    error_description: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionInfo {
    user_id: u64,
    country_code: String,
}

pub async fn request_device_code(
    http: &reqwest::Client,
    creds: &Credentials,
) -> Result<DeviceCode> {
    let resp = http
        .post(format!("{AUTH_BASE}/device_authorization"))
        .form(&[("client_id", creds.client_id.as_str()), ("scope", SCOPE)])
        .send()
        .await?;
    if !resp.status().is_success() {
        bail!("device authorization failed: HTTP {}", resp.status());
    }
    Ok(resp.json().await?)
}

/// Polls until the user approves the device, then resolves the full session.
pub async fn poll_for_session(
    http: &reqwest::Client,
    creds: &Credentials,
    code: &DeviceCode,
) -> Result<Session> {
    let mut interval = Duration::from_secs(code.interval.max(1));
    let deadline = now() + code.expires_in;
    loop {
        tokio::time::sleep(interval).await;
        if now() > deadline {
            bail!("login code expired");
        }
        let resp = http
            .post(format!("{AUTH_BASE}/token"))
            .form(&[
                ("client_id", creds.client_id.as_str()),
                ("client_secret", creds.client_secret.as_str()),
                ("device_code", code.device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("scope", SCOPE),
            ])
            .send()
            .await?;
        if resp.status().is_success() {
            let token: TokenResponse = resp.json().await?;
            let tokens = Tokens {
                expires_at: now() + token.expires_in,
                refresh_token: token
                    .refresh_token
                    .context("login response had no refresh token")?,
                access_token: token.access_token,
            };
            return fetch_session(http, &creds.client_id, tokens).await;
        }
        let err: TokenError = resp.json().await.context("unexpected token response")?;
        match err.error.as_str() {
            "authorization_pending" => {}
            "slow_down" => interval += Duration::from_secs(1),
            _ => bail!(
                "login failed: {}",
                err.error_description.unwrap_or(err.error)
            ),
        }
    }
}

pub async fn refresh(http: &reqwest::Client, creds: &Credentials, old: &Tokens) -> Result<Tokens> {
    let resp = http
        .post(format!("{AUTH_BASE}/token"))
        .form(&[
            ("client_id", creds.client_id.as_str()),
            ("client_secret", creds.client_secret.as_str()),
            ("refresh_token", old.refresh_token.as_str()),
            ("grant_type", "refresh_token"),
            ("scope", SCOPE),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        bail!(
            "token refresh failed (HTTP {}); run with --logout and sign in again",
            resp.status()
        );
    }
    let token: TokenResponse = resp.json().await?;
    Ok(Tokens {
        access_token: token.access_token,
        refresh_token: token
            .refresh_token
            .unwrap_or_else(|| old.refresh_token.clone()),
        expires_at: now() + token.expires_in,
    })
}

async fn fetch_session(http: &reqwest::Client, client_id: &str, tokens: Tokens) -> Result<Session> {
    let info: SessionInfo = http
        .get(format!("{API_BASE}/sessions"))
        .bearer_auth(&tokens.access_token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(Session {
        client_id: client_id.to_string(),
        tokens,
        user_id: info.user_id,
        country_code: info.country_code,
    })
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
