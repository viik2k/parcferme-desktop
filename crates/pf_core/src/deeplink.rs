//! Parse `parcferme://equip?…` deep links. **[M3]**
//!
//! Treat all deep-link input as **untrusted**: the link alone grants nothing.
//! The actual download is authorized by this device's stored bearer token and
//! re-runs the server's full private/`setupShares` access check (audit #2), so a
//! crafted link can't fetch a setup the linked user can't already see. The
//! optional `token` is reserved for server-side signed-payload validation if the
//! web app later adds it; v1 doesn't depend on it. See Build Plan §6.

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// The custom URL scheme registered for the desktop app.
pub const SCHEME: &str = "parcferme";

/// A parsed `parcferme://equip` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquipRequest {
    /// The setup to fetch — the same UUID the download endpoint takes.
    pub setup_id: String,
    /// Optional short-lived signed payload; meaningful only to the server.
    /// `None` when the link omits it (v1 doesn't require it).
    pub token: Option<String>,
}

/// Parse and structurally validate a `parcferme://equip?…` URL into an
/// [`EquipRequest`]. Does **not** authorize anything — that happens server-side
/// at download time.
///
/// Accepted shapes (case-insensitive scheme/action, params order-independent):
/// - `parcferme://equip?setup=<uuid>`
/// - `parcferme://equip?setup=<uuid>&token=<opaque>`
///
/// The setup id is strictly UUID-validated (via [`crate::download::extract_setup_uuid`],
/// which also accepts a full `…/setups/<uuid>` URL value) so a malformed link
/// fails fast instead of firing a doomed request.
pub fn parse(url: &str) -> Result<EquipRequest> {
    // Schemes are case-insensitive (RFC 3986); match without lowercasing the
    // whole URL, which would corrupt a case-sensitive token. `get(..n)` keeps the
    // slice on a char boundary so a unicode-prefixed string can't panic.
    let trimmed = url.trim();
    let prefix = format!("{SCHEME}://");
    let rest = trimmed
        .get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(&prefix))
        .map(|_| &trimmed[prefix.len()..])
        .ok_or_else(|| Error::Api(format!("not a {SCHEME}:// link: {url:?}")))?;

    let (action, query) = match rest.split_once('?') {
        Some((a, q)) => (a, q),
        None => (rest, ""),
    };
    let action = action.trim_end_matches('/');
    if !action.eq_ignore_ascii_case("equip") {
        return Err(Error::Api(format!(
            "unsupported {SCHEME}:// action: {action:?} (expected \"equip\")"
        )));
    }

    let mut setup_raw: Option<String> = None;
    let mut token: Option<String> = None;
    for pair in query.split('&').filter(|s| !s.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decode(value);
        match key {
            // Accept a few spellings so a small web-side naming choice can't
            // silently break the handshake.
            "setup" | "setupId" | "versionId" | "id" => setup_raw = Some(value),
            "token" | "sig" if !value.is_empty() => token = Some(value),
            _ => {}
        }
    }

    let setup_id = setup_raw
        .as_deref()
        .and_then(crate::download::extract_setup_uuid)
        .ok_or_else(|| Error::Api("equip link is missing a valid setup id".into()))?;

    Ok(EquipRequest { setup_id, token })
}

/// Minimal `%XX` percent-decoding for query values. Setup ids are UUIDs and
/// tokens are URL-safe (base64url / JWT), so this rarely fires — but a value
/// could be encoded, and decoding wrong would corrupt the token. Invalid escapes
/// are left verbatim. `+` is **not** treated as a space (that's form-encoding,
/// not generic URI syntax, and would mangle a token).
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "3f8a1c2d-1234-4abc-89ef-0123456789ab";

    #[test]
    fn parses_setup_only() {
        let req = parse(&format!("parcferme://equip?setup={UUID}")).unwrap();
        assert_eq!(req.setup_id, UUID);
        assert_eq!(req.token, None);
    }

    #[test]
    fn parses_setup_and_token_order_independent() {
        let req = parse(&format!("parcferme://equip?token=abc.def&setup={UUID}")).unwrap();
        assert_eq!(req.setup_id, UUID);
        assert_eq!(req.token.as_deref(), Some("abc.def"));
    }

    #[test]
    fn accepts_scheme_action_case_insensitively_and_trailing_slash() {
        assert!(parse(&format!("PARCFERME://Equip/?id={UUID}")).is_ok());
    }

    #[test]
    fn percent_decodes_values() {
        // A token that arrived percent-encoded is restored intact.
        let req = parse(&format!("parcferme://equip?setup={UUID}&token=a%2Bb%3Dc")).unwrap();
        assert_eq!(req.token.as_deref(), Some("a+b=c"));
    }

    #[test]
    fn rejects_wrong_scheme_action_and_bad_id() {
        assert!(parse(&format!("https://parcferme.cc/setups/{UUID}")).is_err());
        assert!(parse(&format!("parcferme://share?setup={UUID}")).is_err());
        assert!(parse("parcferme://equip?setup=not-a-uuid").is_err());
        // Missing the setup param entirely.
        assert!(parse("parcferme://equip").is_err());
    }
}
