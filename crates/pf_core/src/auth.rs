//! Device-authorization grant + OS-keychain token storage. **[M1]**
//!
//! NextAuth sessions are browser cookies; a tray app can't ride them. Instead
//! the tray runs the OAuth 2.0 Device Authorization Grant (RFC 8628): it asks
//! the server for a code, sends the user to `parcferme.cc/device` to approve
//! while logged in, and polls until the server issues a long-lived, revocable
//! device token. That token is stored in the **OS keychain** (Windows
//! Credential Manager via the `keyring` crate) — never in plaintext config,
//! never in the webview.

use crate::api::{ApiClient, DeviceUser, TokenPoll};
use crate::{Error, Result};

/// Keychain coordinates. Service matches the app's bundle identifier. One
/// account holds the secret device token; a second holds the linked user's
/// display profile so "Signed in as @user" (name + avatar) survives a restart
/// without a network round-trip. The profile is not a secret, but co-locating
/// it with the token keeps a single store and a single `sign_out` to clear.
const KEYCHAIN_SERVICE: &str = "cc.parcferme.desktop";
const KEYCHAIN_ACCOUNT: &str = "device-token";
const KEYCHAIN_USER_ACCOUNT: &str = "device-user";

/// A long-lived, server-revocable device token. Held transiently in memory; at
/// rest it lives in the OS keychain. Listed under Account → Devices on the
/// website for trust and per-device revocation.
#[derive(Clone)]
pub struct DeviceToken(String);

impl DeviceToken {
    /// The bearer value to attach to authenticated API calls.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What the user must do to link this device, returned by
/// [`begin_device_flow`]. Surface `user_code` + `verification_uri` in the
/// "Connect account" UI.
#[derive(Debug, Clone)]
pub struct DeviceFlow {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    /// Secret used to poll; do not show the user.
    pub device_code: String,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
}

/// Result of one [`poll_device_flow`] call.
#[derive(Debug)]
pub enum FlowOutcome {
    /// Approved and linked; the token is now in the keychain.
    Linked { user: Option<DeviceUser> },
    /// Not approved yet — poll again after `interval`.
    Pending,
    /// Polling too fast — back off, then poll again.
    SlowDown,
    /// The user declined.
    Denied,
    /// The flow expired before approval; start over.
    Expired,
}

/// Begin the device-authorization flow.
pub fn begin_device_flow(client: &ApiClient) -> Result<DeviceFlow> {
    let r = client.request_device_code()?;
    Ok(DeviceFlow {
        user_code: r.user_code,
        verification_uri: r.verification_uri,
        verification_uri_complete: r.verification_uri_complete,
        device_code: r.device_code,
        interval_secs: r.interval,
        expires_in_secs: r.expires_in,
    })
}

/// Poll once for approval. On success the token is persisted to the keychain as
/// a side effect, so callers only need to react to [`FlowOutcome::Linked`].
pub fn poll_device_flow(client: &ApiClient, device_code: &str) -> Result<FlowOutcome> {
    match client.poll_device_token(device_code)? {
        TokenPoll::Granted(token) => {
            store_token(&token.access_token)?;
            // Cache the profile so the linked state can render after a restart
            // (auth_status reads it back). Best-effort: a cache write failure
            // must not fail the link — the token is what makes us signed in.
            if let Some(user) = token.user.as_ref() {
                let _ = store_user(user);
            }
            Ok(FlowOutcome::Linked { user: token.user })
        }
        TokenPoll::Pending => Ok(FlowOutcome::Pending),
        TokenPoll::SlowDown => Ok(FlowOutcome::SlowDown),
        TokenPoll::Denied => Ok(FlowOutcome::Denied),
        TokenPoll::Expired => Ok(FlowOutcome::Expired),
    }
}

/// Whether this device currently holds a stored token.
pub fn is_linked() -> Result<bool> {
    Ok(current_token()?.is_some())
}

/// Load the stored device token, if this device has been linked.
pub fn current_token() -> Result<Option<DeviceToken>> {
    match entry()?.get_password() {
        Ok(secret) => Ok(Some(DeviceToken(secret))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(Error::Keychain(e.to_string())),
    }
}

/// Persist a token to the OS keychain, replacing any existing one.
pub fn store_token(token: &str) -> Result<()> {
    entry()?
        .set_password(token)
        .map_err(|e| Error::Keychain(e.to_string()))
}

/// The linked user's cached display profile, if one was stored at link time.
///
/// Used to render "Signed in as @user" on startup without a network call. A
/// corrupt or absent cache is not an error — we just have no profile to show
/// (the device is still linked as long as [`current_token`] returns a token).
pub fn cached_user() -> Result<Option<DeviceUser>> {
    match user_entry()?.get_password() {
        Ok(json) => Ok(serde_json::from_str(&json).ok()),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(Error::Keychain(e.to_string())),
    }
}

/// Persist the linked user's display profile, replacing any existing one.
pub fn store_user(user: &DeviceUser) -> Result<()> {
    let json = serde_json::to_string(user).map_err(|e| Error::Api(e.to_string()))?;
    user_entry()?
        .set_password(&json)
        .map_err(|e| Error::Keychain(e.to_string()))
}

/// Forget the stored token and cached profile (Sign out). Idempotent — succeeds
/// if either is already absent.
pub fn sign_out() -> Result<()> {
    delete_entry(entry()?)?;
    delete_entry(user_entry()?)?;
    Ok(())
}

/// Delete one keychain credential, treating "not found" as success.
fn delete_entry(entry: keyring::Entry) -> Result<()> {
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(Error::Keychain(e.to_string())),
    }
}

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .map_err(|e| Error::Keychain(e.to_string()))
}

fn user_entry() -> Result<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER_ACCOUNT)
        .map_err(|e| Error::Keychain(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The cached-profile format is just JSON; verify a user survives the
    // serialize→deserialize the keychain cache uses, independent of the OS store.
    #[test]
    fn cached_user_json_round_trips() {
        let user = DeviceUser {
            id: "u_123".into(),
            name: Some("Finn".into()),
            image: Some("https://www.parcferme.cc/api/avatars/u_123?v=1".into()),
        };
        let json = serde_json::to_string(&user).unwrap();
        let back: DeviceUser = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, user.id);
        assert_eq!(back.name, user.name);
        assert_eq!(back.image, user.image);
    }

    // Hits the real OS keychain, so it's opt-in (`cargo test -- --ignored`) to
    // keep CI hermetic. Uses a throwaway account and cleans up after itself.
    #[test]
    #[ignore = "touches the OS keychain"]
    fn token_round_trips_through_keychain() {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, "device-token-test").unwrap();
        entry.set_password("secret-123").unwrap();
        assert_eq!(entry.get_password().unwrap(), "secret-123");
        entry.delete_credential().unwrap();
        assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));
    }
}
