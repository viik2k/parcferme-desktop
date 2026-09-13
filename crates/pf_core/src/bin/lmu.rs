//! `lmu` — read Le Mans Ultimate live and print what the relay would send.
//!
//! ```text
//! cargo run -p pf_core --bin lmu -- --probe      # one frame, then exit
//! cargo run -p pf_core --bin lmu -- --hz 20      # stream frames as JSON lines
//! ```
//!
//! Frames go to stdout as one JSON object per line, so the output pipes
//! straight into `jq`. The periodic stats line goes to stderr, so it does not
//! pollute that stream.

use std::time::{Duration, Instant};

use pf_core::lmu::{Config, Lmu};

const STATS_EVERY: Duration = Duration::from_secs(10);

fn main() {
    let mut args = std::env::args().skip(1);
    let mut probe = false;
    let mut config = Config::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--probe" => probe = true,
            "--hz" => match args.next().and_then(|v| v.parse::<f64>().ok()) {
                Some(hz) if hz > 0.0 => config.target_hz = hz,
                _ => fail("--hz needs a positive number"),
            },
            "-h" | "--help" => {
                println!("usage: lmu [--probe] [--hz N]");
                return;
            }
            other => fail(&format!("unknown argument {other:?}")),
        }
    }

    let source = match Lmu::start(config) {
        Ok(source) => source,
        // The two liveness states read differently on purpose: one is "start
        // the game", the other is a settings change plus a restart that we
        // cannot perform for the user.
        Err(e) => fail(&e.to_string()),
    };

    if probe {
        match source.next_frame_timeout(Duration::from_secs(5)) {
            Some(frame) => println!("{}", to_json_pretty(&frame)),
            None => fail("no frame within 5s — the game is running but not publishing updates"),
        }
        return;
    }

    let mut last_stats = Instant::now();
    while let Some(frame) = source.next_frame() {
        println!("{}", to_json(&frame));
        if last_stats.elapsed() >= STATS_EVERY {
            last_stats = Instant::now();
            let s = source.stats();
            eprintln!(
                "stats  events={} ({:.1}Hz) frames={} hz={:.2} lock_us(mean/max)={:.1}/{} \
                 lock_timeouts={} gaps={} rest_age={} rest_errors={}",
                s.events,
                s.event_hz,
                s.frames,
                s.hz,
                s.lock_mean_us,
                s.lock_max_us,
                s.lock_timeouts,
                s.gaps,
                s.rest_age_ms
                    .map_or("never".to_string(), |ms| format!("{ms}ms")),
                s.rest_errors,
            );
        }
    }
    eprintln!("Le Mans Ultimate exited; stopping.");
}

fn to_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

fn to_json_pretty<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

fn fail(message: &str) -> ! {
    eprintln!("lmu: {message}");
    std::process::exit(1);
}
