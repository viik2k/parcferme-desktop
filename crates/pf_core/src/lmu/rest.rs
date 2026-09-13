//! LMU's local REST server (`http://localhost:6397`) — the slow half.
//!
//! Shared memory carries everything that moves inside a corner. What is left
//! here is what the game only publishes over HTTP: the pit menu, the weather
//! forecast, the pit stop estimate and the component wear table. None of it
//! moves faster than a lap, so this runs at 1 Hz and the merged frame carries
//! the last-known values with an age attached.
//!
//! Response shapes live in [`super::types`] and come from real captures, not
//! from guesses — see `tests/lmu_deserialize.rs`.

use std::io::Read;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::types;
use crate::{Error, Result};

pub const BASE_URL: &str = "http://localhost:6397";

/// Per-request timeout.
///
/// Measured, not guessed: the game answers `curl` in ~0.21 s but takes **~2.05 s**
/// to answer this client, consistently, whatever headers or agent settings are
/// used. The old 2 s value therefore failed *every* request by a hair while the
/// endpoint looked perfectly healthy from the command line. Why the game is
/// slow for one client and not another is unexplained — see DEFERRED.md — so
/// this leaves a wide margin rather than a tight one. Nothing here is on a hot
/// path: it is a 1 Hz background poll, and liveness is the shared memory's job.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Endpoint paths.
pub mod endpoint {
    pub const STRATEGY_USAGE: &str = "/rest/strategy/usage";
    pub const SESSION_INFO: &str = "/rest/watch/sessionInfo";
    pub const STANDINGS: &str = "/rest/watch/standings";
    pub const REPAIR_AND_REFUEL: &str = "/rest/garage/UIScreen/RepairAndRefuel";
    /// Seconds of *service* the stop you'd make right now would cost —
    /// excluding pit lane transit, which is `RepairAndRefuel::pit_stop_length`.
    pub const PITSTOP_ESTIMATE: &str = "/rest/strategy/pitstop-estimate";
}

/// One forecast node, already in celsius.
#[derive(Debug, Default, Clone, Serialize)]
pub struct ForecastNode {
    pub start_time_s: f64,
    pub temperature_c: f64,
    pub rain_chance_pct: f64,
}

/// The REST-only half of a frame: last known values, converted to the units the
/// frame publishes.
///
/// Every field is optional because the endpoints that feed them 404 outside a
/// session, and a missing value must read as missing rather than as zero. The
/// age of this data is not stored here — it belongs to the frame that carries
/// it ([`super::Frame::rest_age_ms`]), so there is one source of truth for how
/// stale it is at the moment it is read.
#[derive(Debug, Default, Clone, Serialize)]
pub struct RestSnapshot {
    /// How many polls have failed since start. Non-zero out of session is normal.
    pub errors: u64,

    pub session_name: Option<String>,
    pub time_remaining_s: Option<f64>,
    pub lap_limit: Option<u32>,
    /// Metres. Shared memory publishes this too; kept as a cross-check.
    pub track_length_m: Option<f64>,

    /// 0–1 remaining. Also present per-frame in shared memory.
    pub virtual_energy: Option<f64>,
    /// Tread **remaining**, 0–1, FL/FR/RL/RR — the opposite direction to the
    /// shared-memory `m_wear`.
    pub tyre_life: Option<[f64; 4]>,
    /// Brake pad **wear**, 0–1, FL/FR/RL/RR — rises from ~0.03 when fresh.
    pub brake_wear: Option<[f64; 4]>,

    /// The in-car pit menu as armed: (row, selected label).
    pub pit_menu: Vec<(String, String)>,
    /// Seconds of service for the stop as currently armed.
    pub pit_service_s: Option<f64>,
    /// Seconds to drive the pit lane, before any service.
    pub pit_lane_s: Option<f64>,
    /// Litres the game recommends adding.
    pub pit_fuel_advice_l: Option<f64>,

    /// Celsius.
    pub air_c: Option<f64>,
    /// Celsius.
    pub track_c: Option<f64>,
    /// 0–1.
    pub rain: Option<f64>,
    /// 0–1; falls away at dusk.
    pub light: Option<f64>,
    /// Seconds since midnight of the in-sim clock.
    pub time_of_day_s: Option<f64>,
    pub forecast: Vec<ForecastNode>,
}

/// A blocking client for one LMU origin, with a reusable body buffer.
pub struct Client {
    agent: ureq::Agent,
    base: String,
    /// Reused across polls: the standings body alone is ~56 KB, and this runs
    /// forever.
    buf: Vec<u8>,
    state: RestSnapshot,
}

impl Client {
    pub fn new() -> Self {
        Self::with_base(BASE_URL)
    }

    pub fn with_base(base: impl Into<String>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("pf-desktop/", env!("CARGO_PKG_VERSION")))
            .build();
        Client {
            agent,
            base: base.into(),
            buf: Vec::with_capacity(64 * 1024),
            state: RestSnapshot::default(),
        }
    }

    /// Fetch an endpoint into the reusable buffer and return it as a slice.
    fn get_raw(&mut self, endpoint: &str) -> Result<&[u8]> {
        let url = format!("{}{endpoint}", self.base);
        let resp = match self.agent.get(&url).call() {
            Ok(resp) => resp,
            Err(ureq::Error::Status(code, _)) => {
                return Err(Error::Api(format!(
                    "LMU returned HTTP {code} for {endpoint}"
                )))
            }
            // Transport failure on localhost means the game's web server is not
            // there. Liveness is decided by the shared-memory side, so this is
            // reported as a plain poll error rather than a second opinion on
            // whether the game is up.
            Err(e) => return Err(Error::Http(e.to_string())),
        };
        self.buf.clear();
        resp.into_reader()
            .take(4 * 1024 * 1024)
            .read_to_end(&mut self.buf)
            .map_err(Error::Io)?;
        Ok(&self.buf)
    }

    fn get_json<T: DeserializeOwned>(&mut self, endpoint: &str) -> Result<T> {
        let body = self.get_raw(endpoint)?;
        serde_json::from_slice(body).map_err(|e| {
            // The body is the only way to fix a fixture after LMU changes a
            // shape, and it is game telemetry, not a secret.
            log::debug!("{endpoint} unexpected shape: {e}");
            Error::Api(format!("LMU {endpoint} returned an unexpected shape: {e}"))
        })
    }

    pub fn strategy_usage(&mut self) -> Result<types::StrategyUsage> {
        self.get_json(endpoint::STRATEGY_USAGE)
    }

    pub fn repair_and_refuel(&mut self) -> Result<types::RepairAndRefuel> {
        self.get_json(endpoint::REPAIR_AND_REFUEL)
    }

    pub fn session_info(&mut self) -> Result<types::SessionInfo> {
        self.get_json(endpoint::SESSION_INFO)
    }

    pub fn standings(&mut self) -> Result<types::Standings> {
        self.get_json(endpoint::STANDINGS)
    }

    pub fn pitstop_estimate(&mut self) -> Result<types::PitstopEstimate> {
        self.get_json(endpoint::PITSTOP_ESTIMATE)
    }

    /// One pass over the endpoints, folding what came back into the last-known
    /// state. Returns whether anything landed, so the caller only re-stamps the
    /// snapshot's age when it actually got fresh data.
    ///
    /// A failing endpoint leaves its fields alone: `RepairAndRefuel` 404s in
    /// the menus, and losing the whole snapshot over that would be worse than
    /// carrying one stale weather reading.
    pub fn poll(&mut self) -> bool {
        let mut any = false;

        if let Ok(info) = self.session_info() {
            self.state.session_name = info.session.clone();
            self.state.time_remaining_s = info.time_remaining();
            self.state.lap_limit = info.lap_limit();
            self.state.track_length_m = info.lap_distance;
            any = true;
        } else {
            self.state.errors += 1;
        }

        match self.repair_and_refuel() {
            Ok(rr) => {
                self.state.virtual_energy = rr.fuel_info.ve_fraction();
                self.state.pit_lane_s = rr.pit_stop_length.and_then(|p| p.time_in_seconds);
                self.state.pit_fuel_advice_l = rr.pit_recommendations.and_then(|r| r.fuel);
                self.state.time_of_day_s = rr.session_time.and_then(|s| s.time_of_day);
                if let Some(w) = rr.wearables {
                    self.state.tyre_life = corners(&w.tires);
                    self.state.brake_wear = corners(&w.brakes);
                }
                if let Some(now) = rr.current_weather {
                    // Kelvin never leaves this module.
                    self.state.air_c = now.air_c();
                    self.state.track_c = now.track_c();
                    self.state.rain = now.rain_intensity;
                    self.state.light = now.light_level;
                }
                if let Some(f) = rr.weather_forecast {
                    let n = f.nodes;
                    // Parallel arrays that only mean anything zipped by index.
                    self.state.forecast = n
                        .start_time
                        .iter()
                        .zip(n.temperature.iter())
                        .zip(n.rain_chance.iter())
                        .map(
                            |((&start_time_s, &temperature_c), &rain_chance_pct)| ForecastNode {
                                start_time_s,
                                // Already celsius here, unlike currentWeather.
                                temperature_c,
                                rain_chance_pct,
                            },
                        )
                        .collect();
                }
                if let Some(menu) = rr.pit_menu {
                    self.state.pit_menu.clear();
                    self.state
                        .pit_menu
                        .extend(menu.pit_menu.iter().filter_map(|item| {
                            Some((item.name.clone()?, item.selected()?.to_string()))
                        }));
                }
                any = true;
            }
            Err(_) => self.state.errors += 1,
        }

        match self.pitstop_estimate() {
            Ok(est) => {
                self.state.pit_service_s = est.total;
                any = true;
            }
            Err(_) => self.state.errors += 1,
        }

        any
    }

    /// The last-known values. Only meaningful once [`Client::poll`] has
    /// returned `true` at least once.
    pub fn state(&self) -> &RestSnapshot {
        &self.state
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

/// A four-corner array, or `None` if the game sent a different shape.
fn corners(v: &[f64]) -> Option<[f64; 4]> {
    v.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corner_arrays_reject_the_wrong_arity_rather_than_padding() {
        assert_eq!(corners(&[1.0, 2.0, 3.0, 4.0]), Some([1.0, 2.0, 3.0, 4.0]));
        assert_eq!(corners(&[1.0, 2.0, 3.0]), None);
        assert_eq!(corners(&[]), None);
    }

    /// Diagnostic against a running game: prints which endpoints answer and
    /// what each one failed with. Ignored by default like the other live tests.
    ///
    /// `cargo test -p pf_core --lib -- --ignored --nocapture rest_live`
    #[test]
    #[ignore = "needs Le Mans Ultimate running in a session"]
    fn rest_live() {
        let mut client = Client::new();
        macro_rules! probe {
            ($name:literal, $call:expr) => {
                match $call {
                    Ok(v) => println!("{:20} ok  {:?}", $name, v),
                    Err(e) => println!("{:20} ERR {e}", $name),
                }
            };
        }
        // Timing matters as much as success here: the server takes ~2.05 s to
        // answer this client while curl gets the same body in ~0.21 s, which is
        // what REQUEST_TIMEOUT has to clear.
        let started = std::time::Instant::now();
        probe!("sessionInfo", client.session_info());
        println!("sessionInfo took {:?}", started.elapsed());
        probe!("pitstop-estimate", client.pitstop_estimate());
        probe!(
            "RepairAndRefuel",
            client.repair_and_refuel().map(|r| r.fuel_info)
        );
        println!("poll landed: {}", client.poll());
        println!("state: {:?}", client.state());
    }

    #[test]
    fn an_unpolled_client_reports_absent_values_not_zeroes() {
        // A frame built before the first poll lands must carry None, not a
        // plausible-looking 0 °C.
        let state = Client::new().state().clone();
        assert_eq!(state.air_c, None);
        assert_eq!(state.pit_service_s, None);
        assert!(state.pit_menu.is_empty());
    }
}
