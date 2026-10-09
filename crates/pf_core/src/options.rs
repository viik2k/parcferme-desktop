//! The site's car and track name lists, for the upload form's pickers. **[M5: push]**
//!
//! [`crate::car_aliases`] fixes the folder ids we know about; this closes the
//! rest of the gap by letting the user *pick* names instead of guessing how
//! parcferme.cc spells them. New cars and tracks land on the site before they
//! reach the alias table, so the fields stay free text — this only supplies
//! suggestions.
//!
//! The endpoint is the device API's own `GET /api/device/options`
//! (SERVER_CONTRACT §7a): one authenticated call returning both lists, plain
//! JSON — no tRPC envelope to unwrap, and the server keeps the "Unknown Track"
//! catch-all row out of the suggestions.
//!
//! **Fails soft, always.** Offline, a revoked or missing token, or a slow
//! response yields empty lists, never an error — a missing datalist is a
//! smaller problem than an upload form that won't open.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::api::{ApiClient, SetupOptions};
use crate::sim::Sim;
use crate::{auth, Result};

/// Cached lists keyed by sim id. The lists change when the site seeds new
/// cars or tracks — once per app run is fresh enough, and it keeps a
/// re-opened form instant.
///
/// Only a real answer is cached. A failed fetch or an empty one (offline at
/// login, not linked yet) is retried on the next form open: this is a tray app
/// that runs for days, and a cached empty list silently switches off the car
/// matching `upload::identify` relies on (#38).
static CACHE: Mutex<Option<HashMap<&'static str, SetupOptions>>> = Mutex::new(None);

/// Car/track name lists the site knows for `sim`, for the upload form's
/// suggestions. Empty when they can't be fetched — callers must treat that as
/// "no suggestions", never as an error.
pub fn options_for(sim: Sim) -> SetupOptions {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let map = cache.get_or_insert_with(HashMap::new);
    if let Some(hit) = map.get(sim.id()) {
        return hit.clone();
    }
    let fetched = fetch(sim);
    let cache_it = worth_caching(&fetched);
    let options = fetched.unwrap_or_else(|e| {
        // Not an error path for the user — just no autocomplete this time.
        log::warn!("setup options unavailable for {}: {e}", sim.id());
        SetupOptions::default()
    });
    log::info!(
        "options for {}: {} cars, {} tracks",
        sim.id(),
        options.cars.len(),
        options.tracks.len()
    );
    if cache_it {
        map.insert(sim.id(), options.clone());
    }
    options
}

/// Forget every cached list. Called when the linked account changes (connect,
/// sign out), the two moments the answer can change identity, not just age.
pub fn clear_cache() {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// A fetch is worth keeping for the rest of the run only if it succeeded and
/// actually carried something.
fn worth_caching(fetched: &Result<SetupOptions>) -> bool {
    matches!(fetched, Ok(o) if !(o.cars.is_empty() && o.tracks.is_empty()))
}

/// One authenticated request for `sim`'s lists. Separate from the caching so
/// the failure mapping stays testable.
fn fetch(sim: Sim) -> Result<SetupOptions> {
    // No paired device, no suggestions: the upload this feeds needs the token
    // too, so the form is unusable until then anyway.
    let Some(token) = auth::current_token()? else {
        return Ok(SetupOptions::default());
    };
    ApiClient::from_env().setup_options(token.as_str(), sim)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    #[test]
    fn only_a_real_answer_is_cached() {
        let full = SetupOptions {
            cars: vec!["Ferrari 499P".into()],
            ..SetupOptions::default()
        };
        assert!(worth_caching(&Ok(full)));
        // Signed out or a site with nothing yet: retry next time.
        assert!(!worth_caching(&Ok(SetupOptions::default())));
        // Offline at login.
        assert!(!worth_caching(&Err(Error::Http("offline".into()))));
    }

    /// Hits the live site as the currently paired device, so ignored by
    /// default: run
    /// `cargo test -p pf_core options_live -- --ignored --nocapture`
    /// on a linked machine to confirm the endpoint and shapes still agree.
    /// Prints empty lists when the machine has no paired device token.
    #[test]
    #[ignore]
    fn options_live() {
        for sim in Sim::ALL {
            let options = fetch(sim).expect("live options fetch");
            println!(
                "{}: {} cars, {} tracks (first car {:?})",
                sim.id(),
                options.cars.len(),
                options.tracks.len(),
                options.cars.first()
            );
        }
    }
}
