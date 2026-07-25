//! The site's car list, for the upload form's car picker. **[M5: push]**
//!
//! [`crate::car_aliases`] fixes the folder ids we know about; this closes the
//! rest of the gap by letting the user *pick* a car instead of guessing how
//! parcferme.cc spells it. New cars land on the site before they reach the
//! alias table, so the field stays free text — this only supplies suggestions.
//!
//! The endpoint is the website's own public tRPC query, so there is no new
//! server work and no auth: `GET /api/trpc/cars.getAll?batch=1&input=…`, whose
//! response is superjson-wrapped as
//! `[{"result":{"data":{"json":[{ id, sim, name, … }]}}}]`.
//!
//! **Fails soft, always.** Offline, a shape change, or a slow response yields
//! an empty list, never an error — a missing datalist is a smaller problem
//! than an upload form that won't open.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Deserialize;

use crate::api::base_url_from_env;
use crate::sim::Sim;

/// Cached lists keyed by sim id. The list changes when the site seeds new cars
/// — once per app run is fresh enough, and it keeps a re-opened form instant.
static CACHE: Mutex<Option<HashMap<&'static str, Vec<String>>>> = Mutex::new(None);

/// One car as the site knows it. Only `name` reaches the UI; `sim` is used to
/// drop anything the server didn't filter.
#[derive(Debug, Deserialize)]
struct Car {
    #[serde(default)]
    sim: Option<String>,
    name: String,
}

/// The superjson envelope, described only as deeply as we read it.
#[derive(Debug, Deserialize)]
struct Envelope {
    result: EnvelopeResult,
}

#[derive(Debug, Deserialize)]
struct EnvelopeResult {
    data: EnvelopeData,
}

#[derive(Debug, Deserialize)]
struct EnvelopeData {
    json: Vec<Car>,
}

/// Car names the site knows for `sim`, sorted, for the upload form's
/// suggestions. Empty when the list can't be fetched — callers must treat that
/// as "no suggestions", never as an error.
pub fn names_for(sim: Sim) -> Vec<String> {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let map = cache.get_or_insert_with(HashMap::new);
    if let Some(hit) = map.get(sim.id()) {
        return hit.clone();
    }
    let names = fetch(sim).unwrap_or_else(|e| {
        // Not an error path for the user — just no autocomplete this run.
        log::warn!("car list unavailable for {}: {e}", sim.id());
        Vec::new()
    });
    log::info!("car list for {}: {} names", sim.id(), names.len());
    map.insert(sim.id(), names.clone());
    names
}

/// One request for `sim`'s cars. Separate from the caching so the parsing is
/// testable without a network.
fn fetch(sim: Sim) -> Result<Vec<String>, String> {
    let input = format!(r#"{{"0":{{"json":{{"sim":"{}"}}}}}}"#, sim.id());
    let body = ureq::get(&format!("{}/api/trpc/cars.getAll", base_url_from_env()))
        .query("batch", "1")
        .query("input", &input)
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    parse(&body, sim)
}

/// Pull the car names for `sim` out of a superjson tRPC batch response.
fn parse(body: &str, sim: Sim) -> Result<Vec<String>, String> {
    let batch: Vec<Envelope> = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let cars = batch
        .into_iter()
        .next()
        .ok_or_else(|| "empty tRPC batch".to_string())?
        .result
        .data
        .json;

    let mut names: Vec<String> = cars
        .into_iter()
        // The server filters by sim, but a mixed list must not offer an ACC car
        // as an iRacing suggestion; an untagged row is kept rather than dropped.
        .filter(|c| {
            c.sim
                .as_deref()
                .and_then(Sim::from_id)
                .is_none_or(|s| s == sim)
        })
        .map(|c| c.name)
        .filter(|n| !n.trim().is_empty())
        .collect();
    names.sort_unstable();
    names.dedup();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed real response from `cars.getAll?input={"0":{"json":{"sim":"iracing"}}}`.
    const SAMPLE: &str = r#"[{"result":{"data":{"json":[
        {"id":182,"sim":"iracing","simRefId":null,"name":"ARCA Chevrolet SS"},
        {"id":126,"sim":"iracing","simRefId":null,"name":"Acura ARX-06 GTP"},
        {"id":999,"sim":"acc","simRefId":null,"name":"Ferrari 488 GT3 Evo"},
        {"id":998,"name":"Untagged Car"}
    ]}}}]"#;

    #[test]
    fn parses_the_superjson_envelope_and_filters_by_sim() {
        let names = parse(SAMPLE, Sim::IRacing).unwrap();
        // Sorted, ACC row dropped, untagged row kept.
        assert_eq!(
            names,
            vec![
                "ARCA Chevrolet SS".to_string(),
                "Acura ARX-06 GTP".to_string(),
                "Untagged Car".to_string(),
            ]
        );
        // The same body read as ACC keeps only the ACC row (plus untagged).
        assert_eq!(
            parse(SAMPLE, Sim::Acc).unwrap(),
            vec![
                "Ferrari 488 GT3 Evo".to_string(),
                "Untagged Car".to_string()
            ]
        );
    }

    /// Hits the live site, so ignored by default: run
    /// `cargo test -p pf_core cars_getall_live -- --ignored --nocapture`
    /// to confirm the endpoint, the query encoding and the envelope still agree.
    #[test]
    #[ignore]
    fn cars_getall_live() {
        for sim in Sim::ALL {
            let names = fetch(sim).expect("live cars.getAll");
            println!(
                "{}: {} cars, first {:?}",
                sim.id(),
                names.len(),
                names.first()
            );
            assert!(!names.is_empty(), "{} returned no cars", sim.id());
        }
    }

    #[test]
    fn a_changed_shape_is_an_error_not_a_panic() {
        // Every one of these must surface as Err so `names_for` degrades to an
        // empty suggestion list instead of taking the form down.
        for body in [
            "",
            "null",
            "[]",
            r#"{"result":{}}"#,
            r#"[{"result":{"data":{}}}]"#,
            r#"[{"result":{"data":{"json":"nope"}}}]"#,
        ] {
            assert!(parse(body, Sim::IRacing).is_err(), "{body:?} should error");
        }
    }
}
