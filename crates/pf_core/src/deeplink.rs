//! Parse `parcferme://equip?…` deep links. **[M3]**
//!
//! Treat all deep-link input as **untrusted**: the link alone grants nothing.
//! It carries a short-lived signed payload that is validated *server-side* at
//! download time (see Build Plan §6). A poll/queue fallback (`getPendingEquips`)
//! is the alternative handshake if URL-scheme registration proves fiddly.

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// A parsed `parcferme://equip` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquipRequest {
    /// The setup version to fetch.
    pub version_id: String,
    /// Opaque short-lived signed token; meaningful only to the server.
    pub token: String,
}

/// Parse and structurally validate a `parcferme://` URL into an
/// [`EquipRequest`]. Does **not** authorize anything — that happens server-side.
pub fn parse(_url: &str) -> Result<EquipRequest> {
    Err(Error::NotImplemented("deeplink::parse"))
}
