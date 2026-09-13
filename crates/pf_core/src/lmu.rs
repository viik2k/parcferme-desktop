//! Le Mans Ultimate as one live source.
//!
//! The game publishes the same session through two very different channels, and
//! this module owns both so that a consumer never has to:
//!
//! - **Shared memory** (`LMU_Data`, [`shm`]) — the player's telemetry and the
//!   scoring array. Everything that moves inside a corner. The interface implies
//!   100 Hz; a real session measured **~50 Hz**, which is why frames are paced
//!   against the clock rather than counted off the event stream.
//! - **REST** (`localhost:6397`, [`rest`]) — the pit menu, weather forecast,
//!   stop estimate and component wear. Nothing that moves faster than a lap.
//!
//! [`Lmu::start`] runs a loop for each and merges them into one [`Frame`]
//! stream at a configurable rate (10 Hz by default), which is what the relay
//! agent puts on the wire. Shared-memory channels in a frame are always from
//! the instant the frame was taken; REST fields are last-known and carry their
//! own age, so a consumer can tell a gap from a current reading.
//!
//! Units are normalised here and nowhere else: **everything leaving this module
//! is celsius, seconds, litres, km/h or a 0–1 fraction.** The game's kelvin
//! never escapes.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::Result;

pub mod rest;
pub mod shm;
pub mod types;

pub use rest::{ForecastNode, RestSnapshot};

/// Standard gravity, for reporting acceleration in g.
const G: f64 = 9.806_65;

/// How long to block on the game's event before calling the interval a gap.
const EVENT_WAIT: Duration = Duration::from_secs(1);

/// How the two loops are paced.
#[derive(Debug, Clone)]
pub struct Config {
    /// Frames per second to emit. The shared-memory loop decimates the game's
    /// event stream down to this before it takes the lock.
    pub target_hz: f64,
    /// How often to poll the REST endpoints.
    pub rest_interval: Duration,
    /// REST data older than this is marked stale on the frame.
    pub rest_stale_after: Duration,
    /// Base URL for the REST half. Only worth changing in tests.
    pub rest_base_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            target_hz: 10.0,
            rest_interval: Duration::from_secs(1),
            // Two cycles' grace. A cycle is not the interval: the game takes
            // ~2 s per request (measured), so three sequential endpoints plus
            // the interval is ~7 s of wall clock. A 3 s threshold — the obvious
            // "three polls" guess — would mark healthy data stale forever.
            rest_stale_after: Duration::from_secs(15),
            rest_base_url: rest::BASE_URL.to_string(),
        }
    }
}

/// One merged sample of the car and the session.
///
/// Corner-indexed arrays are **FL, FR, RL, RR** throughout, per the header's
/// own comment on `mWheel[4]`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Frame {
    /// Monotonic, gapless: consumers use it to detect drops on the wire.
    pub seq: u64,
    /// Milliseconds since the source started.
    pub t_ms: u64,

    // ---- driver inputs and drivetrain (shared memory, this instant) ----
    pub speed_kph: f64,
    /// 0–1.
    pub throttle: f64,
    /// 0–1.
    pub brake: f64,
    /// -1 (full left) to 1 (full right).
    pub steering: f64,
    /// -1 reverse, 0 neutral, 1+ forward.
    pub gear: i32,
    pub rpm: f64,

    // ---- lap and position ----
    pub lap: i32,
    /// Seconds into the current lap.
    pub lap_time_s: Option<f64>,
    /// `None` until a lap has actually been timed — the game's `-1.0` sentinel
    /// would otherwise read as the fastest lap of the session.
    pub last_lap_time_s: Option<f64>,
    /// 1, 2 or 3.
    pub sector: Option<u8>,
    /// 0–1 around the lap.
    pub track_pos: Option<f64>,
    pub in_pit_lane: bool,
    pub lap_invalidated: bool,

    // ---- consumables ----
    pub fuel_l: f64,
    pub fuel_capacity_l: f64,
    /// 0–1 remaining, on cars that run it.
    pub virtual_energy: Option<f64>,

    // ---- corners ----
    /// Celsius, the mean of the game's three tread samples per corner.
    /// `None` per corner until the car has been on track — the game publishes
    /// a flat 0 K there, which would otherwise surface as -273 °C.
    pub tyre_temp_c: [Option<f64>; 4],
    /// Tyre **life remaining**: 1.0 fresh, falling as the tyre is used. Same
    /// direction as [`RestSnapshot::tyre_life`], despite the game's field being
    /// named `mWear`.
    pub tyre_life: [f64; 4],
    /// Celsius. The game publishes kelvin here despite its header saying
    /// otherwise; the conversion happens once, at this boundary.
    pub brake_temp_c: [Option<f64>; 4],

    // ---- forces ----
    /// Positive to the right.
    pub g_lat: f64,
    /// Positive under acceleration, negative under braking.
    pub g_lon: f64,

    // ---- the field ----
    /// 1-based, overall.
    pub place: u8,
    /// 1-based within the car's own class.
    pub place_in_class: Option<u32>,
    pub car_class: String,
    /// The car as the game names it (`m_vehicle_name`), e.g. "Ferrari 499P".
    /// Empty out of session. Not a folder id — nothing on a live session maps
    /// to one — so a consumer sharing it sends this string as-is (§10).
    pub car_name: String,
    /// The track as the game names it (`ScoringInfoV01::m_track_name`), e.g.
    /// "Le Mans 24h". Empty out of session, and display-ish like `car_name`.
    pub track_name: String,
    /// Seconds to the car ahead **on track**.
    pub gap_ahead_s: Option<f64>,
    /// Seconds to the car behind on track.
    pub gap_behind_s: Option<f64>,

    // ---- the slow half ----
    // Shared, not cloned: the REST half changes once a second and the frame
    // rate is ten times that. `serde`'s `Arc` impl is behind a feature flag, so
    // the pointer is dereferenced for serialization rather than turning it on
    // across the whole workspace.
    #[serde(serialize_with = "serialize_rest")]
    pub rest: Arc<RestSnapshot>,
    /// Age of the REST data in this frame. `None` means it has never landed.
    pub rest_age_ms: Option<u64>,
    /// True when the REST half is too old to be read as current.
    pub rest_stale: bool,
}

fn serialize_rest<S: serde::Serializer>(
    rest: &Arc<RestSnapshot>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    RestSnapshot::serialize(rest, serializer)
}

/// A point-in-time read of the source's counters.
#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub elapsed_s: f64,
    /// Game update events seen (the undecimated rate).
    pub events: u64,
    /// Frames emitted.
    pub frames: u64,
    /// Measured output rate over the whole run.
    pub hz: f64,
    /// Measured rate of the game's own update event. Worth watching: it is
    /// ~50 Hz in practice, not the 100 Hz the interface implies.
    pub event_hz: f64,
    /// Mean time the game's lock was held by us, microseconds.
    pub lock_mean_us: f64,
    /// Worst single lock hold, microseconds. The number that matters: this is
    /// time the sim's own writer could have spent blocked.
    pub lock_max_us: u64,
    /// Frames dropped because the lock could not be taken in time.
    pub lock_timeouts: u64,
    /// Intervals where the game published nothing for a full second — paused,
    /// in the menus, or wedged.
    pub gaps: u64,
    pub rest_age_ms: Option<u64>,
    pub rest_errors: u64,
}

#[derive(Default)]
struct Counters {
    events: AtomicU64,
    frames: AtomicU64,
    gaps: AtomicU64,
    lock_ns_total: AtomicU64,
    lock_ns_max: AtomicU64,
    lock_timeouts: AtomicU64,
}

/// What the REST thread publishes for the frame builder to attach.
#[derive(Default)]
struct RestShared {
    snap: Arc<RestSnapshot>,
    at: Option<Instant>,
}

/// The live source. Dropping it stops both loops.
pub struct Lmu {
    frames: Receiver<Frame>,
    counters: Arc<Counters>,
    rest: Arc<Mutex<RestShared>>,
    shutdown: Arc<AtomicBool>,
    started: Instant,
    threads: Vec<JoinHandle<()>>,
}

impl Lmu {
    /// Open the game's shared memory and start both loops.
    ///
    /// Fails immediately — before any thread is spawned — when the game is down
    /// ([`crate::Error::LmuNotRunning`]) or is up without its plugin interface
    /// ([`crate::Error::LmuPluginsDisabled`]). Those are the only two liveness
    /// states, and the shared-memory side is the one that decides them.
    pub fn start(config: Config) -> Result<Self> {
        let reader = shm::Reader::open()?;

        let counters = Arc::new(Counters::default());
        let rest = Arc::new(Mutex::new(RestShared::default()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let started = Instant::now();
        // A small queue, and the producer drops rather than blocks: a slow
        // consumer must never stall the reader thread into holding the game's
        // lock longer or falling behind the event stream.
        let (tx, frames) = sync_channel(64);

        let threads = vec![
            spawn_rest(&config, Arc::clone(&rest), Arc::clone(&shutdown)),
            spawn_shm(
                &config,
                reader,
                tx,
                Arc::clone(&counters),
                Arc::clone(&rest),
                Arc::clone(&shutdown),
                started,
            ),
        ];

        Ok(Lmu {
            frames,
            counters,
            rest,
            shutdown,
            started,
            threads,
        })
    }

    /// The next frame, or `None` once the game has exited and the source has
    /// shut itself down.
    pub fn next_frame(&self) -> Option<Frame> {
        self.frames.recv().ok()
    }

    /// The next frame, giving up after `timeout`.
    pub fn next_frame_timeout(&self, timeout: Duration) -> Option<Frame> {
        self.frames.recv_timeout(timeout).ok()
    }

    /// False once the game has exited and the loops have stopped.
    ///
    /// A consumer polling with [`Lmu::next_frame_timeout`] cannot tell a quiet
    /// session (menus, paused) from a dead one — both are `None` — but a dead
    /// source returns `None` *immediately*, so a loop that doesn't check this
    /// spins. Check it on every empty poll.
    pub fn is_running(&self) -> bool {
        !self.shutdown.load(Ordering::Relaxed)
    }

    pub fn stats(&self) -> Stats {
        let c = &self.counters;
        let frames = c.frames.load(Ordering::Relaxed);
        let events = c.events.load(Ordering::Relaxed);
        let elapsed_s = self.started.elapsed().as_secs_f64();
        let (rest_age_ms, rest_errors) = {
            let shared = self.rest.lock().unwrap_or_else(|e| e.into_inner());
            (
                shared.at.map(|t| t.elapsed().as_millis() as u64),
                shared.snap.errors,
            )
        };
        Stats {
            elapsed_s,
            events,
            frames,
            hz: if elapsed_s > 0.0 {
                frames as f64 / elapsed_s
            } else {
                0.0
            },
            event_hz: if elapsed_s > 0.0 {
                events as f64 / elapsed_s
            } else {
                0.0
            },
            lock_mean_us: if frames > 0 {
                c.lock_ns_total.load(Ordering::Relaxed) as f64 / frames as f64 / 1_000.0
            } else {
                0.0
            },
            lock_max_us: c.lock_ns_max.load(Ordering::Relaxed) / 1_000,
            lock_timeouts: c.lock_timeouts.load(Ordering::Relaxed),
            gaps: c.gaps.load(Ordering::Relaxed),
            rest_age_ms,
            rest_errors,
        }
    }
}

impl Drop for Lmu {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

fn spawn_rest(
    config: &Config,
    shared: Arc<Mutex<RestShared>>,
    shutdown: Arc<AtomicBool>,
) -> JoinHandle<()> {
    let interval = config.rest_interval;
    let base = config.rest_base_url.clone();
    std::thread::Builder::new()
        .name("lmu-rest".into())
        .spawn(move || {
            let mut client = rest::Client::with_base(base);
            while !shutdown.load(Ordering::Relaxed) {
                let landed = client.poll();
                {
                    // Publish every pass so the error count is visible even when
                    // nothing lands — a REST half that is failing every request
                    // must not look identical to one that has never been asked.
                    let mut guard = shared.lock().unwrap_or_else(|e| e.into_inner());
                    guard.snap = Arc::new(client.state().clone());
                    // ...but only re-stamp the age on success, or a session
                    // spent entirely in the menus would look permanently fresh.
                    if landed {
                        guard.at = Some(Instant::now());
                    }
                }
                // Sleep in slices so shutdown is prompt without a second
                // signalling mechanism.
                let deadline = Instant::now() + interval;
                while Instant::now() < deadline && !shutdown.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        })
        .expect("spawn lmu-rest")
}

#[allow(clippy::too_many_arguments)]
fn spawn_shm(
    config: &Config,
    mut reader: shm::Reader,
    tx: SyncSender<Frame>,
    counters: Arc<Counters>,
    rest: Arc<Mutex<RestShared>>,
    shutdown: Arc<AtomicBool>,
    started: Instant,
) -> JoinHandle<()> {
    let period = frame_period(config.target_hz);
    let stale_after = config.rest_stale_after;
    std::thread::Builder::new()
        .name("lmu-shm".into())
        .spawn(move || {
            let mut seq = 0u64;
            let mut next_due = Instant::now();
            while !shutdown.load(Ordering::Relaxed) {
                match reader.wait(EVENT_WAIT) {
                    shm::Wake::GameExited => break,
                    shm::Wake::Timeout => {
                        counters.gaps.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    shm::Wake::Event => {}
                }
                counters.events.fetch_add(1, Ordering::Relaxed);
                // The decimation happens here, before the lock, so a 10 Hz
                // target costs the game 10 lock acquisitions a second whatever
                // rate it publishes at.
                if !frame_is_due(&mut next_due, Instant::now(), period) {
                    continue;
                }

                let held = match reader.read() {
                    Ok(held) => held,
                    Err(_) => {
                        counters.lock_timeouts.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                };
                let ns = held.as_nanos() as u64;
                counters.lock_ns_total.fetch_add(ns, Ordering::Relaxed);
                counters.lock_ns_max.fetch_max(ns, Ordering::Relaxed);

                let (snap, at) = {
                    let guard = rest.lock().unwrap_or_else(|e| e.into_inner());
                    (Arc::clone(&guard.snap), guard.at)
                };
                let frame = build_frame(
                    seq,
                    started.elapsed(),
                    reader.snapshot(),
                    snap,
                    at,
                    stale_after,
                );
                seq += 1;
                counters.frames.fetch_add(1, Ordering::Relaxed);
                // Drop rather than block — see the channel comment in `start`.
                if tx.try_send(frame).is_err() {
                    log::debug!("lmu frame dropped: consumer is behind");
                }
            }
            // The game exiting ends this loop, and this is the only thread that
            // can see that. Raise the flag so the REST thread stops polling a
            // localhost server that is gone, and so consumers can tell a dead
            // source from a quiet one.
            shutdown.store(true, Ordering::Relaxed);
        })
        .expect("spawn lmu-shm")
}

/// The interval between emitted frames.
fn frame_period(target_hz: f64) -> Duration {
    // A nonsense rate must not divide by zero or ask for a frame every 0 ns.
    Duration::from_secs_f64(1.0 / target_hz.clamp(0.1, 1_000.0))
}

/// Is a frame due? Advances the schedule when it is.
///
/// Deliberately *not* "every Nth event". The first live session measured the
/// game's event stream at ~50 Hz, not the 100 Hz its interface implies, so a
/// stride of 10 produced 4.5 Hz output instead of the requested 10 — an
/// event-count stride silently inherits whatever rate the game feels like
/// publishing at. Pacing against the clock hits the requested rate exactly and
/// keeps the promise that actually matters: one lock acquisition per frame.
fn frame_is_due(next_due: &mut Instant, now: Instant, period: Duration) -> bool {
    if now < *next_due {
        return false;
    }
    // Never try to catch up a backlog: after a pause, resume a full period from
    // now rather than firing a burst of frames for time that has already gone.
    // Keeping the scheduled time when we are on cadence avoids drifting slow.
    let scheduled = *next_due + period;
    *next_due = if scheduled <= now {
        now + period
    } else {
        scheduled
    };
    true
}

/// LMU's "no time yet" sentinel is `-1.0`; read naively it is the fastest lap
/// of the session. Same rule as [`types::StandingEntry::pace`].
fn timed(v: f64) -> Option<f64> {
    (v > 0.0).then_some(v)
}

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().sum::<f64>() / v.len() as f64
}

/// One of LMU's on-track gaps, as a positive number of seconds.
///
/// The game signs the two in opposite directions: the car **ahead** comes back
/// negative (measured 2026-09-07: `-1.593` while running 11th of 38), the car
/// behind positive. A gap is a magnitude, so both are published positive.
///
/// Reusing [`timed`] here — the `-1.0` sentinel rule that is right for lap
/// times — silently nulled every gap-ahead in an 11,000 frame session while
/// gap-behind worked perfectly, which is what made the bug visible at all.
/// An exact `0.0` means "no car to measure against", not "alongside".
fn gap(raw: f32) -> Option<f64> {
    let seconds = f64::from(raw);
    (seconds != 0.0).then_some(seconds.abs())
}

/// Kelvin to celsius, treating the game's unpopulated `0.0` as absent.
///
/// Zero kelvin is not a temperature a race car reaches; it is what LMU writes
/// into the wheel arrays before the car has run. Passing it through would put
/// -273 °C on a pit wall.
fn celsius(kelvin: f64) -> Option<f64> {
    (kelvin > 0.0).then_some(kelvin - 273.15)
}

/// Merge one shared-memory snapshot and the last-known REST state into a frame.
///
/// Split out of the reader loop so it can be tested against the golden byte
/// fixture without a running game.
pub fn build_frame(
    seq: u64,
    t: Duration,
    snap: &shm::Snapshot,
    rest: Arc<RestSnapshot>,
    rest_at: Option<Instant>,
    stale_after: Duration,
) -> Frame {
    let telem = &snap.telem;
    let vel = telem.m_local_vel;
    let accel = telem.m_local_accel;
    let speed_kph = (vel[0] * vel[0] + vel[1] * vel[1] + vel[2] * vel[2]).sqrt() * 3.6;

    // Local vehicle axes are +x out the LEFT side and +z out the BACK, so both
    // useful components are negated to reach the usual "+right, +forward"
    // convention. Getting this backwards silently mirrors every g trace.
    let g_lat = -accel[0] / G;
    let g_lon = -accel[2] / G;

    let raw_sector = telem.m_current_sector;
    let sector = u8::try_from((raw_sector & 0x7FFF_FFFF) + 1)
        .ok()
        .filter(|s| (1..=3).contains(s));

    let mut tyre_temp_c = [None; 4];
    let mut tyre_life = [0.0; 4];
    let mut brake_temp_c = [None; 4];
    for (i, wheel) in telem.m_wheel.iter().enumerate() {
        let temps = wheel.m_temperature;
        tyre_temp_c[i] = celsius(mean(&temps));
        tyre_life[i] = wheel.m_wear;
        // Kelvin too, whatever the header's comment claims.
        brake_temp_c[i] = celsius(wheel.m_brake_temp);
    }

    let player = snap.player_scoring();
    let track_length = snap.scoring.m_lap_dist;
    let track_pos = player
        .map(|p| p.m_lap_dist)
        .filter(|_| track_length > 0.0)
        // Negative before the start line, so wrap rather than clamp: a car
        // 100 m before the line is at 0.98 of the previous lap, not at 0.
        .map(|d| (d / track_length).rem_euclid(1.0));

    let car_class = player
        .map(|p| shm::c_str(&p.m_vehicle_class))
        .unwrap_or_default();
    // What the session *is*, as opposed to what it's doing: the only place a
    // recording learns its car and track, since neither channel publishes an
    // id and `RestSnapshot::session_name` is the session type ("PRACTICE1").
    let car_name = player
        .map(|p| shm::c_str(&p.m_vehicle_name))
        .unwrap_or_default();
    let track_name = shm::c_str(&snap.scoring.m_track_name);
    let place = player.map(|p| p.m_place).unwrap_or(0);
    let place_in_class = player.map(|me| {
        let mine = me.m_vehicle_class;
        snap.vehicles()
            .iter()
            .filter(|v| v.m_vehicle_class == mine && v.m_place < me.m_place)
            .count() as u32
            + 1
    });

    let rest_age = rest_at.map(|t| t.elapsed());
    Frame {
        seq,
        t_ms: t.as_millis() as u64,
        speed_kph,
        throttle: telem.m_unfiltered_throttle,
        brake: telem.m_unfiltered_brake,
        steering: telem.m_unfiltered_steering,
        gear: telem.m_gear,
        rpm: telem.m_engine_rpm,
        lap: telem.m_lap_number,
        lap_time_s: player.and_then(|p| timed(p.m_time_into_lap)),
        last_lap_time_s: player.and_then(|p| timed(p.m_last_lap_time)),
        sector,
        track_pos,
        // The pit lane lives in the sign bit of the sector field.
        in_pit_lane: raw_sector < 0 || player.is_some_and(|p| p.m_in_pits != 0),
        lap_invalidated: telem.m_lap_invalidated != 0,
        fuel_l: telem.m_fuel,
        fuel_capacity_l: telem.m_fuel_capacity,
        virtual_energy: {
            let ve = f64::from(telem.m_virtual_energy);
            (ve > 0.0).then_some(ve)
        },
        tyre_temp_c,
        tyre_life,
        brake_temp_c,
        g_lat,
        g_lon,
        place,
        place_in_class,
        car_class,
        car_name,
        track_name,
        // On-track gaps, published per frame by LMU itself.
        gap_ahead_s: gap(telem.m_time_gap_car_ahead),
        gap_behind_s: gap(telem.m_time_gap_car_behind),
        rest,
        rest_age_ms: rest_age.map(|a| a.as_millis() as u64),
        // Never having polled is stale too: the consumer must not read the
        // defaults as live zeroes.
        rest_stale: rest_age.is_none_or(|a| a > stale_after),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_pacing_is_independent_of_the_games_event_rate() {
        let period = frame_period(10.0);
        assert_eq!(period, Duration::from_millis(100));

        // Feed events at 50 Hz (the rate a real session actually publishes at)
        // and 10 Hz must still come out — the bug that shipped the first time
        // was a stride of 10 against an assumed 100 Hz, which gave 4.5 Hz.
        let start = Instant::now();
        let mut next_due = start;
        let mut frames = 0;
        for tick in 0..50 {
            let now = start + Duration::from_millis(20 * tick);
            if frame_is_due(&mut next_due, now, period) {
                frames += 1;
            }
        }
        assert_eq!(
            frames, 10,
            "one second of 50 Hz events must yield 10 frames"
        );
    }

    #[test]
    fn a_stall_does_not_fire_a_burst_of_catch_up_frames() {
        let period = frame_period(10.0);
        let start = Instant::now();
        let mut next_due = start;
        assert!(frame_is_due(&mut next_due, start, period));
        // The game was paused for five seconds: that is 50 missed slots, and
        // replaying them would put 50 identical frames on the wire at once.
        let after = start + Duration::from_secs(5);
        assert!(frame_is_due(&mut next_due, after, period));
        assert!(!frame_is_due(&mut next_due, after, period));
        assert!(frame_is_due(&mut next_due, after + period, period));
    }

    #[test]
    fn nonsense_rates_cannot_divide_by_zero() {
        assert!(frame_period(0.0) <= Duration::from_secs(10));
        assert!(frame_period(-5.0) <= Duration::from_secs(10));
        assert!(frame_period(f64::MAX) >= Duration::from_micros(1));
    }

    #[test]
    fn the_gap_ahead_is_negative_and_must_not_be_read_as_a_sentinel() {
        // Measured live: ahead -1.5, behind +0.25, both real cars. Running
        // these through `timed` (the lap-time rule) nulled every gap-ahead in
        // an 11,000 frame session.
        assert_eq!(gap(-1.5), Some(1.5));
        assert_eq!(gap(0.25), Some(0.25));
        // Exactly zero is the game saying there is no car to measure against.
        assert_eq!(gap(0.0), None);
        assert_eq!(gap(-0.0), None);
        // ...and the lap-time rule really would have dropped it.
        assert_eq!(timed(-1.5), None);
    }

    #[test]
    fn the_no_time_sentinel_reads_as_absent() {
        assert_eq!(timed(-1.0), None);
        assert_eq!(timed(0.0), None);
        assert_eq!(timed(93.2), Some(93.2));
    }
}
