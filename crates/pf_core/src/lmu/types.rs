//! Typed LMU responses. These shapes are now **derived from real captures**
//! (2026-07-13, a QUALIFY1 session at Sebring in an LMP3) — see ASSUMPTIONS.md
//! for what's confirmed and what's still inferred.
//!
//! Kept deliberately permissive: no `deny_unknown_fields` (LMU sends far more
//! than we model), `Vec`/`BTreeMap` over fixed arrays, and every value field is
//! `Option` + `#[serde(default)]` so a shape change costs one field, not the
//! whole response.

use std::collections::BTreeMap;

use serde::Deserialize;

/// GET /rest/strategy/usage
///
/// **Keyed by driver name**, one entry per completed lap, for the *whole field*
/// — not just you. Only the player's rows carry `fuel`/`tyres`; rivals expose
/// `ve` only (that's how much the game lets you spy on them).
#[derive(Debug, Default, Deserialize)]
#[serde(transparent)]
pub struct StrategyUsage(pub BTreeMap<String, Vec<UsageLap>>);

impl StrategyUsage {
    /// The player's lap history. Prefer looking them up by name from standings;
    /// this falls back to "the only driver the game gave us fuel data for".
    pub fn laps_for(&self, driver: Option<&str>) -> Option<&[UsageLap]> {
        if let Some(name) = driver {
            if let Some(laps) = self.0.get(name) {
                return Some(laps);
            }
        }
        self.0
            .values()
            .find(|laps| laps.iter().any(|l| l.fuel.is_some()))
            .map(|v| v.as_slice())
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct UsageLap {
    /// 0-based: lap 0 is the out/formation lap.
    pub lap: u32,
    /// The game's own pit-lap flag. Real, and better than any heuristic.
    pub pit: bool,
    /// Increments on each new stint — a refuel boundary.
    pub stint: u32,
    /// Fuel **remaining** at the end of this lap, as a fraction of tank (0–1).
    /// NOT fuel used. Player only. Multiply by `FuelInfo::max_fuel` for litres.
    pub fuel: Option<f64>,
    /// Virtual energy **remaining**, fraction (0–1). Always 0.0 on cars with no
    /// VE system (GT3/LMP2/LMP3) — see `has_virtual_energy`.
    pub ve: Option<f64>,
    /// Tyre life remaining, percent, FL/FR/RL/RR (assumed order).
    pub tyres: Vec<f64>,
}

/// Does this car even have virtual energy? Hypercars do; a GT3 reports a flat
/// 0.0 forever, which must read as "not applicable", never as "empty".
pub fn has_virtual_energy(laps: &[UsageLap]) -> bool {
    laps.iter().any(|l| l.ve.is_some_and(|v| v > 0.0))
}

/// GET /rest/watch/sessionInfo — the session clock and format.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionInfo {
    /// "QUALIFY1", "RACE1", "PRACTICE1", ...
    pub session: Option<String>,
    pub game_phase: Option<u32>,
    pub current_event_time: Option<f64>,
    pub end_event_time: Option<f64>,
    /// **`u32::MAX` means "no lap limit"** (a time-certain session). Not zero.
    pub maximum_laps: Option<u32>,
    /// Track length in metres. Note the name collision: at *session* level
    /// `lapDistance` is the length of the lap, while on a car
    /// ([`StandingEntry::lap_distance`]) it is that car's progress along it.
    /// This is the denominator that turns the latter into a normalised 0–1
    /// track position. Shared memory publishes the same value as
    /// `ScoringInfoV01::mLapDist`.
    pub lap_distance: Option<f64>,
}

impl SessionInfo {
    pub fn time_remaining(&self) -> Option<f64> {
        let (end, now) = (self.end_event_time?, self.current_event_time?);
        Some((end - now).max(0.0))
    }

    /// The scheduled lap count, or `None` for a time-certain session.
    pub fn lap_limit(&self) -> Option<u32> {
        match self.maximum_laps {
            Some(n) if n > 0 && n != u32::MAX => Some(n),
            _ => None,
        }
    }
}

/// GET /rest/watch/standings — one entry per car. Our source of lap count and
/// pace; `usage` has no lap times.
#[derive(Debug, Default, Deserialize)]
#[serde(transparent)]
pub struct Standings(pub Vec<StandingEntry>);

impl Standings {
    pub fn player(&self) -> Option<&StandingEntry> {
        self.0.iter().find(|e| e.player)
    }
}

/// A world-space point. LMU is Y-up, so a track map is the **x/z** plane —
/// `y` is elevation and gets dropped by everything that draws.
#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(default)]
pub struct Vec3 {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub z: Option<f64>,
}

/// A world-space vector that carries its own magnitude — how LMU sends
/// `carVelocity` and `carAcceleration`. `velocity` is the scalar: m/s for the
/// first, m/s^2 for the second (the name is the game's, not ours).
#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(default)]
pub struct Motion {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub z: Option<f64>,
    pub velocity: Option<f64>,
}

impl Motion {
    fn xyz(&self) -> Option<(f64, f64, f64)> {
        Some((self.x?, self.y?, self.z?))
    }
}

/// Hypercar attack mode: how many activations are left and how long the
/// current one runs. All zeros on a car with no boost system.
#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AttackMode {
    pub remaining_count: Option<u32>,
    pub time_remaining: Option<f64>,
    pub total_count: Option<u32>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StandingEntry {
    pub driver_name: Option<String>,
    /// True for the car you're driving.
    pub player: bool,
    pub car_class: Option<String>,
    pub car_number: Option<String>,
    pub position: Option<u32>,
    pub laps_completed: Option<u32>,
    /// `-1.0` until a lap is complete — use [`StandingEntry::pace`].
    pub last_lap_time: Option<f64>,
    pub best_lap_time: Option<f64>,
    pub estimated_lap_time: Option<f64>,
    pub fuel_fraction: Option<f64>,
    pub ve_fraction: Option<f64>,
    pub pitting: bool,
    pub in_garage_stall: bool,
    pub pitstops: Option<u32>,

    // ---- timing screen ----
    /// Seconds behind the overall leader / the car directly ahead. Companion
    /// `laps_behind_*` are non-zero once lapped, and then the seconds are noise.
    pub time_behind_leader: Option<f64>,
    pub time_behind_next: Option<f64>,
    pub time_behind_class_leader: Option<f64>,
    pub laps_behind_leader: Option<u32>,
    pub laps_behind_next: Option<u32>,
    /// **Cumulative from the start line, not per-sector splits** — S2 here is
    /// S1+S2 elapsed. [`StandingEntry::sectors`] does the subtraction.
    pub last_sector_time1: Option<f64>,
    pub last_sector_time2: Option<f64>,
    pub best_lap_sector_time1: Option<f64>,
    pub best_lap_sector_time2: Option<f64>,
    /// "SECTOR1" | "SECTOR2" | "SECTOR3" — which one the car is in right now.
    pub sector: Option<String>,
    /// "NONE" | "REQUEST" | "ENTERING" | "EXITING" | ... — richer than `pitting`.
    pub pit_state: Option<String>,

    // ---- telemetry ----
    /// World-frame velocity. `velocity` is the scalar speed, m/s.
    pub car_velocity: Motion,
    /// World-frame acceleration, m/s^2. Neither component means anything on
    /// its own — [`StandingEntry::g_forces`] projects it against the heading.
    pub car_acceleration: Motion,
    /// Seconds elapsed on the lap in progress. Paired with `lap_distance` this
    /// is a distance-to-time curve, which is all a live delta needs.
    pub time_into_lap: Option<f64>,
    /// Session elapsed time at which this lap started.
    pub lap_start_et: Option<f64>,

    // ---- race control ----
    /// "GREEN" | "YELLOW" | "RED" | ... Session-wide, repeated on every car.
    pub flag: Option<String>,
    pub under_yellow: bool,
    /// Outstanding penalties. Non-zero is a drive-through waiting to happen.
    pub penalties: Option<u32>,
    pub drs_active: bool,
    /// Hypercar attack mode / manufacturer boost budget.
    pub attack_mode: AttackMode,
    pub full_team_name: Option<String>,
    pub qualification: Option<u32>,
    pub finish_status: Option<String>,
    pub laps_behind_class_leader: Option<u32>,
    /// Metres along the **pit lane**, not the circuit. The only pit-lane
    /// geometry LMU publishes.
    pub pit_lap_distance: Option<f64>,

    // ---- track map ----
    /// Metres travelled along the lap. Negative before the start line.
    pub lap_distance: Option<f64>,
    /// World position. The whole reason a track map is possible.
    pub car_position: Vec3,
}

/// Treat LMU's "no time yet" sentinel as absent. Every timing field uses -1.0
/// (and 0.0 on a sector never run) for missing — read naively, a -1 s sector
/// looks like the fastest lap of the session.
fn timed(v: Option<f64>) -> Option<f64> {
    v.filter(|t| *t > 0.0)
}

/// Split the three sectors out of LMU's **cumulative** pair plus a lap time.
///
/// The API gives elapsed-at-sector-end (S2 = S1+S2), so the splits are
/// differences, and S3 only exists once the lap itself is complete. Each is
/// independently optional: a car mid-lap has S1 but no S3.
fn splits(s1_cum: Option<f64>, s2_cum: Option<f64>, lap: Option<f64>) -> [Option<f64>; 3] {
    let (s1, s2c, lap) = (timed(s1_cum), timed(s2_cum), timed(lap));
    [
        s1,
        // Guard the subtraction: out-of-order or partial data must yield None,
        // never a negative "sector time".
        s2c.zip(s1).map(|(c, a)| c - a).filter(|d| *d > 0.0),
        lap.zip(s2c).map(|(l, c)| l - c).filter(|d| *d > 0.0),
    ]
}

impl StandingEntry {
    /// Best available lap time, in seconds. LMU reports -1.0 for "no time yet",
    /// so a naive read of `last_lap_time` would poison every projection.
    pub fn pace(&self) -> Option<f64> {
        [
            self.last_lap_time,
            self.best_lap_time,
            self.estimated_lap_time,
        ]
        .into_iter()
        .flatten()
        .find(|t| *t > 0.0)
    }

    /// Last lap's three sector splits, in seconds.
    pub fn sectors(&self) -> [Option<f64>; 3] {
        splits(
            self.last_sector_time1,
            self.last_sector_time2,
            self.last_lap_time,
        )
    }

    /// Longitudinal and lateral acceleration, in g.
    ///
    /// LMU publishes both vectors in **world space**, so no single component
    /// is a car-relative g on its own. The velocity vector *is* the heading,
    /// though, so projecting acceleration onto it splits the two: the parallel
    /// part is longitudinal (positive accelerating, negative braking) and
    /// what's left over is lateral.
    ///
    /// Lateral comes back **unsigned** — telling left from right needs the
    /// car's up-axis, which the API doesn't publish. Magnitude is what a
    /// g-g plot needs anyway.
    pub fn g_forces(&self) -> Option<(f64, f64)> {
        const G: f64 = 9.80665;
        let (vx, vy, vz) = self.car_velocity.xyz()?;
        let (ax, ay, az) = self.car_acceleration.xyz()?;
        let speed = (vx * vx + vy * vy + vz * vz).sqrt();
        // Below walking pace the heading is noise, and dividing by it would
        // manufacture enormous g out of a stationary car.
        if speed < 1.0 {
            return None;
        }
        let lon = (ax * vx + ay * vy + az * vz) / speed;
        let total_sq = ax * ax + ay * ay + az * az;
        // Clamped at zero: floating point can leave the difference a hair
        // negative when acceleration is purely longitudinal.
        let lat = (total_sq - lon * lon).max(0.0).sqrt();
        Some((lon / G, lat / G))
    }

    /// Scalar speed in km/h.
    pub fn speed_kph(&self) -> Option<f64> {
        self.car_velocity.velocity.map(|v| v * 3.6)
    }

    /// The splits of this car's best lap.
    pub fn best_sectors(&self) -> [Option<f64>; 3] {
        splits(
            self.best_lap_sector_time1,
            self.best_lap_sector_time2,
            self.best_lap_time,
        )
    }
}

/// GET /rest/garage/UIScreen/RepairAndRefuel — 404s in the menus, live in a
/// session. The only place that reports the **real tank capacity**.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RepairAndRefuel {
    pub fuel_info: FuelInfo,
    pub pit_recommendations: Option<PitRecommendations>,
    pub wearables: Option<Wearables>,
    /// Pit lane transit time — half of what a stop costs.
    pub pit_stop_length: Option<PitStopLength>,
    /// Track and air temperature, rain, cloud — live conditions.
    pub current_weather: Option<CurrentWeather>,
    /// Five forecast nodes covering the rest of the session.
    pub weather_forecast: Option<WeatherForecast>,
    /// The pit menu exactly as the in-car overlay shows it: what's armed for
    /// the next stop.
    pub pit_menu: Option<PitMenuWrapper>,
    pub session_time: Option<SessionTime>,
    // Also carries: pitMenu, pitStopTimes, weatherForecast, teamInfo.
    // Unmodeled — see DEFERRED.md.
}

/// Corner wear, FL/FR/RL/RR (order assumed). **The two metrics run in
/// opposite directions**: `brakes` is wear — 0.032 on fresh pads, rising
/// toward 1; `tires` is tread **remaining** — ~0.95 on fresh rubber, falling
/// toward 0 (the same signal as per-lap [`UsageLap::tyres`], as a fraction).
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct Wearables {
    pub brakes: Vec<f64>,
    pub tires: Vec<f64>,
    /// Per-corner suspension damage, 0 = undamaged.
    pub suspension: Vec<f64>,
}

/// The game's own per-stop advice, from `pitRecommendations` in
/// RepairAndRefuel.
///
/// Unlike the PascalCase `pitStopTimes`, these keys are lowercase/camelCase in
/// the real capture (`fuel`, `virtualEnergy`) — plus a `"TIRES:"`-style family,
/// colon included, carrying 0/1 tyre-change advice. `fuel` is litres; the
/// `virtualEnergy` unit is unverified (only ever seen as 0, a no-VE car).
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct PitRecommendations {
    pub fuel: Option<f64>,
    #[serde(rename = "virtualEnergy")]
    pub virtual_energy: Option<f64>,
    /// 0/1 advice on whether to change tyres. The key is literally `"TIRES:"`.
    #[serde(rename = "TIRES:")]
    pub tires: Option<i64>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FuelInfo {
    /// Litres.
    pub current_fuel: Option<f64>,
    /// Litres. The tank capacity we were previously hardcoding to 100.
    pub max_fuel: Option<f64>,
    pub current_battery: Option<f64>,
    pub max_battery: Option<f64>,
    /// Raw VE counters (~5.2e8 of ~7.1e8), not a percentage. Only the ratio
    /// means anything, and only when `max` is non-zero — see `ve_fraction`.
    pub current_virtual_energy: Option<f64>,
    pub max_virtual_energy: Option<f64>,
}

impl FuelInfo {
    /// Virtual energy remaining as a 0–1 fraction, or `None` on a car with no
    /// VE system (where `max` is 0 and the ratio would divide by zero).
    pub fn ve_fraction(&self) -> Option<f64> {
        let max = self.max_virtual_energy.filter(|m| *m > 0.0)?;
        Some(self.current_virtual_energy? / max)
    }
}

/// The static rate card: `pitStopTimes.times` on **RepairAndRefuel**.
///
/// Misleading name kept for now — it was written believing this was the shape
/// of `/rest/strategy/pitstop-estimate`. It isn't; that endpoint is flat (see
/// [`PitstopEstimate`]). These are the constants behind it: fill rates and
/// fixed action costs (`FourTireChange` 12 s, `DriverChange` 25 s), not a live
/// estimate.
///
/// Real keys are PascalCase except the camelCase `virtualEnergy*` family (hence
/// the per-field renames), and two entries aren't durations but flags.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PitstopEstimateTimes {
    pub brake_change: Option<f64>,
    pub brake_time_concurrent: Option<f64>,
    pub driver_change: Option<f64>,
    pub driver_concurrent: Option<f64>,
    pub driver_damage: Option<f64>,
    pub driver_random: Option<f64>,
    pub fender_flare_adjust: Option<f64>,
    pub fix_aero_damage: Option<f64>,
    pub fix_all_damage: Option<f64>,
    pub fix_random_delay: Option<f64>,
    pub fix_time_concurrent: Option<f64>,
    pub four_tire_change: Option<f64>,
    pub front_wing_adjust: Option<f64>,
    pub front_wing_replace: Option<f64>,
    /// Litres per second.
    pub fuel_fill_rate: Option<f64>,
    pub fuel_insert: Option<f64>,
    pub fuel_random_delay: Option<f64>,
    pub fuel_remove: Option<f64>,
    pub fuel_time_concurrent: Option<f64>,
    /// Flag, not a duration: pressures adjustable without a tyre change.
    pub on_the_fly_pressure: Option<bool>,
    pub pressure_change: Option<f64>,
    pub radiator_change: Option<f64>,
    pub random_brake_delay: Option<f64>,
    pub random_tire_delay: Option<f64>,
    pub rear_wing_adjust: Option<f64>,
    pub rear_wing_replace: Option<f64>,
    /// Flag, not a duration.
    pub simultaneous_stop_go: Option<bool>,
    pub spring_rubber_change: Option<f64>,
    pub tire_time_concurrent: Option<f64>,
    pub track_bar_change: Option<f64>,
    pub two_tire_change: Option<f64>,
    pub wedge_change: Option<f64>,
    #[serde(rename = "virtualEnergyFillRate")]
    pub virtual_energy_fill_rate: Option<f64>,
    #[serde(rename = "virtualEnergyInsert")]
    pub virtual_energy_insert: Option<f64>,
    #[serde(rename = "virtualEnergyRandomDelay")]
    pub virtual_energy_random_delay: Option<f64>,
    #[serde(rename = "virtualEnergyRemove")]
    pub virtual_energy_remove: Option<f64>,
    #[serde(rename = "virtualEnergyTimeConcurrent")]
    pub virtual_energy_time_concurrent: Option<f64>,
}

/// GET /rest/strategy/pitstop-estimate — **flat and camelCase**, seconds of
/// *service* for the stop you'd make right now, given what the pit menu has
/// selected. Confirmed against a 338-sample RACE1 capture (2026-07-25, Spa).
///
/// Live, not static: `fuel` and `ve` grow through a stint as there's more to
/// put back in. `tires`/`driverSwap` stay 0.0 until you select them.
///
/// This is **not** the cost of pitting. It excludes pit lane transit — that's
/// `RepairAndRefuel::pit_stop_length`. Service alone reads ~2 s early in a
/// stint; the stop is nearer 13 s. See [`PitstopEstimateTimes`] for the
/// separate static-rate table, which despite the name lives on RepairAndRefuel.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PitstopEstimate {
    /// The game's own sum of the components below. Prefer it to re-adding them.
    pub total: Option<f64>,
    pub fuel: Option<f64>,
    pub ve: Option<f64>,
    pub tires: Option<f64>,
    pub brakes: Option<f64>,
    pub brake_ducts: Option<f64>,
    pub damage: Option<f64>,
    pub driver_swap: Option<f64>,
    pub penalties: Option<f64>,
}

/// `pitStopLength` on RepairAndRefuel: seconds to drive the pit lane at the
/// limiter, before any service. The other half of what a stop actually costs.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PitStopLength {
    pub time_in_seconds: Option<f64>,
}

/// `currentWeather` on RepairAndRefuel. Temperatures are **kelvin**; the
/// helpers convert, because a pit wall showing 299 K is a pit wall nobody reads.
#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CurrentWeather {
    pub air_pressure: Option<f64>,
    pub ambient_temp_kelvin: Option<f64>,
    /// 0–1.
    pub cloud_coverage: Option<f64>,
    /// 0–1.
    pub humidity: Option<f64>,
    /// 0–1; falls away at dusk. The night-race signal.
    pub light_level: Option<f64>,
    /// 0–1. `raining` has only ever been seen equal to it.
    pub rain_intensity: Option<f64>,
    pub raining: Option<f64>,
    pub track_temp_kelvin: Option<f64>,
}

fn celsius(kelvin: Option<f64>) -> Option<f64> {
    kelvin.filter(|k| *k > 0.0).map(|k| k - 273.15)
}

impl CurrentWeather {
    pub fn air_c(&self) -> Option<f64> {
        celsius(self.ambient_temp_kelvin)
    }
    pub fn track_c(&self) -> Option<f64> {
        celsius(self.track_temp_kelvin)
    }
}

/// `weatherForecast` — parallel arrays, one entry per forecast node, keyed
/// PascalCase. Five nodes in every capture so far, but nothing guarantees it,
/// so this stays `Vec` and the reader zips.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct WeatherForecast {
    pub nodes: ForecastNodes,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ForecastNodes {
    pub start_time: Vec<f64>,
    pub duration: Vec<f64>,
    /// Celsius already — unlike `CurrentWeather`, which is kelvin.
    pub temperature: Vec<f64>,
    /// Percent.
    pub humidity: Vec<f64>,
    /// Percent.
    pub rain_chance: Vec<f64>,
    /// Sky index; the game's own cloud enum, meaning unconfirmed.
    pub sky: Vec<f64>,
    pub wind_speed: Vec<f64>,
    pub wind_direction: Vec<f64>,
}

/// LMU nests the menu one level deep: `pitMenu.pitMenu`.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PitMenuWrapper {
    pub pit_menu: Vec<PitMenuItem>,
}

/// One row of the in-car pit menu. `current_setting` indexes `settings` — the
/// text is the only thing that carries meaning ("New Medium", "3.0 deg (-3)"),
/// so the index is resolved rather than shown.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PitMenuItem {
    /// Includes the trailing colon, as the game sends it: `"FUEL RATIO:"`.
    pub name: Option<String>,
    pub current_setting: Option<usize>,
    pub settings: Vec<PitMenuSetting>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct PitMenuSetting {
    pub text: Option<String>,
}

impl PitMenuItem {
    /// The armed setting's label. `None` when the index is out of range —
    /// which is what a menu row with no choices looks like.
    pub fn selected(&self) -> Option<&str> {
        self.settings.get(self.current_setting?)?.text.as_deref()
    }
}

/// `sessionTime` on RepairAndRefuel: seconds since midnight of the in-sim
/// clock. The only source of "is it dark yet" in a 24 h race.
#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionTime {
    pub time_of_day: Option<f64>,
}
