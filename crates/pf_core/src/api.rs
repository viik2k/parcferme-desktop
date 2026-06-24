//! Typed HTTP client for parcferme.cc. **[M1: device auth · M2: downloads]**
//!
//! The desktop client is mostly a *consumer* of APIs that already ship on the
//! web platform. Authenticated calls present the device token (see
//! [`crate::auth`]); the server authorizes exactly as it would a browser
//! session — including the private-setup `setupShares` check (audit #2).
//!
//! M1 implements the OAuth 2.0 Device Authorization Grant (RFC 8628) client:
//! [`ApiClient::request_device_code`] then poll [`ApiClient::poll_device_token`].

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Production web origin; override with `PARCFERME_API_URL` for local dev.
/// Note the `www.` — the apex `parcferme.cc` 308-redirects here, and a 308 on
/// POST isn't followed, so we target the canonical host directly.
pub const DEFAULT_BASE_URL: &str = "https://www.parcferme.cc";
/// OAuth client identifier for the desktop tray app.
pub const CLIENT_ID: &str = "pf-desktop";
/// Device-grant type per RFC 8628.
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// A configured HTTP client bound to one parcferme.cc origin.
pub struct ApiClient {
    base_url: String,
    agent: ureq::Agent,
}

/// Response of `POST /api/device/code` (RFC 8628 §3.2).
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCodeResponse {
    /// Secret used by the device to poll for the token. Never shown to the user.
    pub device_code: String,
    /// Short human code the user types at the verification URI.
    pub user_code: String,
    /// Where the user goes to approve (e.g. `https://parcferme.cc/device`).
    pub verification_uri: String,
    /// Verification URI with `user_code` pre-filled, if the server provides it.
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    /// Lifetime of `device_code`/`user_code`, in seconds.
    pub expires_in: u64,
    /// Minimum seconds the client must wait between polls.
    pub interval: u64,
}

/// The signed-in user the device is now linked to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceUser {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
}

/// Successful token response from `POST /api/device/token`.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub user: Option<DeviceUser>,
}

/// Everything the desktop needs to install one setup file, returned by the
/// device download endpoint (`GET /api/device/setups/{uuid}/download`).
#[derive(Debug, Clone, Deserialize)]
pub struct DownloadInfo {
    /// Short-lived presigned R2 URL for the `.sto` bytes.
    pub url: String,
    /// The setup file's name, e.g. `baseline_spa.sto`. Sanitized before use.
    pub filename: String,
    /// Car display name — becomes the `setups\<car>\` subfolder.
    #[serde(default)]
    pub car: String,
    /// Setup's display name, for the success toast.
    #[serde(default)]
    pub name: Option<String>,
}

/// Outcome of one poll of the device-token endpoint.
#[derive(Debug)]
pub enum TokenPoll {
    /// The user approved; the token (and who it belongs to) is enclosed.
    Granted(TokenResponse),
    /// The user hasn't approved yet — keep polling at the current interval.
    Pending,
    /// Polling too fast — increase the interval before the next poll.
    SlowDown,
    /// The user explicitly declined.
    Denied,
    /// The `device_code` expired before approval.
    Expired,
}

#[derive(Debug, Default, Deserialize)]
struct ErrorBody {
    #[serde(default)]
    error: String,
}

impl ApiClient {
    /// Build a client for an explicit base URL.
    pub fn new(base_url: impl Into<String>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("pf-desktop/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            base_url: normalize_base(base_url.into()),
            agent,
        }
    }

    /// Build a client from the environment (`PARCFERME_API_URL`), defaulting to
    /// production.
    pub fn from_env() -> Self {
        let base =
            std::env::var("PARCFERME_API_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        Self::new(base)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// Start the device-authorization grant: ask the server for a device/user
    /// code pair (RFC 8628 §3.1).
    pub fn request_device_code(&self) -> Result<DeviceCodeResponse> {
        let result = self
            .agent
            .post(&self.url("/api/device/code"))
            .send_json(serde_json::json!({
                "client_id": CLIENT_ID,
                "scope": "setups:download",
            }));
        match result {
            Ok(resp) => read_json(resp),
            Err(ureq::Error::Status(code, _)) => Err(Error::Api(format!(
                "server returned HTTP {code} for /api/device/code — is the ParcFermé desktop API deployed at this URL?"
            ))),
            Err(e) => Err(map_transport(e)),
        }
    }

    /// Poll the token endpoint once (RFC 8628 §3.4). Maps the standardized
    /// error codes into [`TokenPoll`] variants so the caller can drive the loop.
    pub fn poll_device_token(&self, device_code: &str) -> Result<TokenPoll> {
        let result = self
            .agent
            .post(&self.url("/api/device/token"))
            .send_json(serde_json::json!({
                "client_id": CLIENT_ID,
                "device_code": device_code,
                "grant_type": DEVICE_GRANT_TYPE,
            }));

        match result {
            Ok(resp) => {
                let mut token: TokenResponse = read_json(resp)?;
                // The web API returns custom avatars as same-origin *relative*
                // URLs (e.g. `/api/avatars/{id}?v=…`). Those resolve against the
                // webview's own origin (`tauri://localhost`), where no such route
                // exists, so the `<img>` 404s. Resolve against our API origin so
                // the avatar loads. OAuth avatars are already absolute CDN URLs
                // and pass through untouched.
                if let Some(user) = token.user.as_mut() {
                    if let Some(image) = user.image.take() {
                        user.image = Some(self.absolutize(image));
                    }
                }
                Ok(TokenPoll::Granted(token))
            }
            // RFC 8628 signals pending/slow_down/etc. as 4xx with an `error` body.
            Err(ureq::Error::Status(_, resp)) => {
                let body: ErrorBody = resp.into_json().unwrap_or_default();
                Ok(map_token_error(&body.error))
            }
            Err(e) => Err(map_transport(e)),
        }
    }

    /// Resolve a setup by its public UUID into a presigned download, presenting
    /// the device token. The server runs the *same* private/`setupShares` access
    /// check it applies to a browser session (Build Plan §6, audit #2).
    pub fn get_download(&self, setup_uuid: &str, token: &str) -> Result<DownloadInfo> {
        let result = self
            .agent
            .get(&self.url(&format!("/api/device/setups/{setup_uuid}/download")))
            .set("Authorization", &format!("Bearer {token}"))
            .call();
        match result {
            Ok(resp) => read_json(resp),
            Err(ureq::Error::Status(401, _)) => Err(Error::Api(
                "this device is no longer authorized — sign out and reconnect".into(),
            )),
            Err(ureq::Error::Status(403, _)) => {
                Err(Error::Api("you don't have access to this setup".into()))
            }
            Err(ureq::Error::Status(404, _)) => Err(Error::Api("setup not found".into())),
            Err(ureq::Error::Status(code, resp)) => {
                let body: ErrorBody = resp.into_json().unwrap_or_default();
                let detail = if body.error.is_empty() {
                    String::new()
                } else {
                    format!(": {}", body.error)
                };
                Err(Error::Api(format!("download failed (HTTP {code}){detail}")))
            }
            Err(e) => Err(map_transport(e)),
        }
    }

    /// Resolve a possibly-relative URL returned by the web API against this
    /// client's origin. Absolute `http(s)` URLs are returned unchanged.
    fn absolutize(&self, url: String) -> String {
        if url.starts_with("http://") || url.starts_with("https://") {
            url
        } else if url.starts_with('/') {
            format!("{}{url}", self.base_url)
        } else {
            format!("{}/{url}", self.base_url)
        }
    }
}

/// Map an RFC 8628 token-endpoint error code to a [`TokenPoll`].
fn map_token_error(code: &str) -> TokenPoll {
    match code {
        "authorization_pending" => TokenPoll::Pending,
        "slow_down" => TokenPoll::SlowDown,
        "access_denied" => TokenPoll::Denied,
        "expired_token" => TokenPoll::Expired,
        // Unknown codes are treated as expiry so the UI fails closed rather
        // than polling forever.
        _ => TokenPoll::Expired,
    }
}

fn map_transport(err: ureq::Error) -> Error {
    Error::Http(err.to_string())
}

/// Parse a successful response as JSON, but fail with a *clear* message when the
/// server returns something else — an HTML 404, an apex→www redirect body, or a
/// proxy error page — instead of the cryptic underlying parse error.
fn read_json<T: DeserializeOwned>(resp: ureq::Response) -> Result<T> {
    let status = resp.status();
    let content_type = resp.content_type().to_string();
    if !content_type.contains("json") {
        let snippet: String = resp
            .into_string()
            .unwrap_or_default()
            .chars()
            .take(80)
            .collect();
        return Err(Error::Api(format!(
            "expected JSON but got HTTP {status} ({content_type}). Is the desktop API deployed at this URL? Response began: {snippet:?}"
        )));
    }
    resp.into_json()
        .map_err(|e| Error::Api(format!("invalid JSON from server (HTTP {status}): {e}")))
}

/// Strip a trailing slash so `url()` can join paths uniformly.
fn normalize_base(mut base: String) -> String {
    while base.ends_with('/') {
        base.pop();
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_error_codes_map_to_outcomes() {
        assert!(matches!(map_token_error("authorization_pending"), TokenPoll::Pending));
        assert!(matches!(map_token_error("slow_down"), TokenPoll::SlowDown));
        assert!(matches!(map_token_error("access_denied"), TokenPoll::Denied));
        assert!(matches!(map_token_error("expired_token"), TokenPoll::Expired));
        // Unknown fails closed.
        assert!(matches!(map_token_error("weird"), TokenPoll::Expired));
    }

    #[test]
    fn base_url_normalized_and_joined() {
        let c = ApiClient::new("https://example.com/");
        assert_eq!(c.url("/api/device/code"), "https://example.com/api/device/code");
    }

    #[test]
    fn absolutize_resolves_relative_but_leaves_absolute() {
        let c = ApiClient::new("https://www.parcferme.cc");
        // Same-origin relative avatar URL gets the API origin prepended.
        assert_eq!(
            c.absolutize("/api/avatars/abc?v=123".to_string()),
            "https://www.parcferme.cc/api/avatars/abc?v=123"
        );
        // Relative without a leading slash still joins cleanly.
        assert_eq!(
            c.absolutize("api/avatars/abc".to_string()),
            "https://www.parcferme.cc/api/avatars/abc"
        );
        // Absolute CDN URLs (OAuth avatars) are untouched.
        let cdn = "https://cdn.discordapp.com/avatars/1/2.png".to_string();
        assert_eq!(c.absolutize(cdn.clone()), cdn);
    }
}
