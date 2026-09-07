//! Fixture-based deserializer tests.
//!
//! These fixtures are **real** `boxbox capture` output (2026-07-13, QUALIFY1,
//! LMP3, 24 cars) — not guesses. The asserts pin the semantics that surprised
//! us, so a future refactor can't quietly reintroduce the old wrong reading.

use pf_core::lmu::types::{
    has_virtual_energy, PitstopEstimate, PitstopEstimateTimes, RepairAndRefuel, SessionInfo,
    Standings, StrategyUsage,
};

#[test]
fn strategy_usage_is_keyed_by_driver_name() {
    let json = include_str!("fixtures/strategy_usage.json");
    let usage: StrategyUsage = serde_json::from_str(json).unwrap();
    assert_eq!(usage.0.len(), 24, "the whole field, not just us");

    let mine = usage.laps_for(Some("Finn Idagi")).expect("player laps");
    assert_eq!(mine.len(), 5);

    // fuel is the tank level REMAINING as a fraction — it falls across a stint.
    assert_eq!(mine[0].fuel, Some(0.5));
    assert!(mine[3].fuel.unwrap() < mine[0].fuel.unwrap());
    // ...and jumps back up on a refuel, which is how we detect a stop.
    assert!(mine[4].fuel.unwrap() > mine[3].fuel.unwrap());

    assert!(mine[1].pit, "the game flags pit laps for us");
}

#[test]
fn a_car_without_virtual_energy_reads_as_absent_not_empty() {
    let json = include_str!("fixtures/strategy_usage.json");
    let usage: StrategyUsage = serde_json::from_str(json).unwrap();

    // The LMP3 we recorded has no VE system: a flat 0.0 forever. Treating that
    // as "empty tank" would put a false alarm on the pit wall all race.
    let mine = usage.laps_for(Some("Finn Idagi")).unwrap();
    assert!(!has_virtual_energy(mine));

    // A Hypercar in the same field does have it.
    let hyper = usage.0.values().find(|laps| has_virtual_energy(laps));
    assert!(hyper.is_some(), "at least one car in the field runs VE");
}

#[test]
fn falling_back_to_the_only_driver_with_fuel_data_finds_us() {
    let json = include_str!("fixtures/strategy_usage.json");
    let usage: StrategyUsage = serde_json::from_str(json).unwrap();
    // Rivals expose `ve` but never `fuel`, so "has fuel" identifies the player
    // even when standings isn't answering.
    let mine = usage.laps_for(None).expect("player by fallback");
    assert!(mine.iter().any(|l| l.fuel.is_some()));
}

#[test]
fn session_info_time_remaining_is_a_subtraction() {
    let json = include_str!("fixtures/session_info.json");
    let info: SessionInfo = serde_json::from_str(json).unwrap();
    assert_eq!(info.session.as_deref(), Some("QUALIFY1"));

    let (now, end) = (
        info.current_event_time.unwrap(),
        info.end_event_time.unwrap(),
    );
    assert_eq!(info.time_remaining(), Some(end - now));
}

#[test]
fn session_info_carries_the_track_length_lap_distance_is_measured_against() {
    let json = include_str!("fixtures/session_info.json");
    let info: SessionInfo = serde_json::from_str(json).unwrap();

    // At session level `lapDistance` is the length of the lap; on a car it is
    // progress along it. Without this field a car's lapDistance cannot be
    // normalised, which is why it was added when the field was first modeled.
    assert_eq!(info.lap_distance, Some(6981.6005859375));

    // Every car in the capture is somewhere within one lap of it (negative
    // before the start line, hence the lower bound rather than zero).
    let standings: Standings =
        serde_json::from_str(include_str!("fixtures/standings.json")).unwrap();
    let length = info.lap_distance.unwrap();
    assert!(standings
        .0
        .iter()
        .filter_map(|e| e.lap_distance)
        .all(|d| d > -length && d < length));
}

#[test]
fn maximum_laps_sentinel_means_time_certain_not_four_billion_laps() {
    let json = include_str!("fixtures/session_info.json");
    let info: SessionInfo = serde_json::from_str(json).unwrap();
    assert_eq!(info.maximum_laps, Some(u32::MAX));
    assert_eq!(info.lap_limit(), None, "u32::MAX is 'no limit'");
}

#[test]
fn standings_finds_the_player_and_their_pace() {
    let json = include_str!("fixtures/standings.json");
    let standings: Standings = serde_json::from_str(json).unwrap();
    assert_eq!(standings.0.len(), 24);

    let me = standings.player().expect("the player flag");
    assert_eq!(me.driver_name.as_deref(), Some("Finn Idagi"));
    assert_eq!(me.laps_completed, Some(4));

    // lastLapTime is -1.0 (in the pits, no timed lap) — pace() must skip it and
    // fall through to the best lap rather than returning a negative lap time.
    assert_eq!(me.last_lap_time, Some(-1.0));
    assert_eq!(me.pace(), me.best_lap_time);
    assert!(me.pace().unwrap() > 0.0);
}

#[test]
fn repair_and_refuel_reports_the_real_tank_capacity() {
    let json = include_str!("fixtures/repair_and_refuel.json");
    let rr: RepairAndRefuel = serde_json::from_str(json).unwrap();
    assert_eq!(rr.fuel_info.max_fuel, Some(100.0));
    assert!(rr.fuel_info.current_fuel.is_some_and(|f| f > 0.0));
}

#[test]
fn pitstop_estimate_is_flat_seconds_not_a_times_table() {
    // The shape this endpoint was guessed to have (a nested PascalCase `times`
    // object) is real but belongs to RepairAndRefuel — see the test below. The
    // endpoint itself is flat camelCase. Fixture: Spa RACE1, 2026-07-25.
    let json = include_str!("fixtures/pitstop_estimate.json");
    let est: PitstopEstimate = serde_json::from_str(json).unwrap();

    assert!(est.total.is_some_and(|t| (t - 13.9867).abs() < 1e-3));
    assert!(est.fuel.is_some_and(|f| (f - 9.7044).abs() < 1e-3));

    // `total` is the max of the concurrent components, NOT their sum: fuel and
    // VE go in at the same time, so the stop takes the longer of the two.
    // Re-adding the parts here would have invented 9.7 s of stop that isn't.
    assert_eq!(est.total, est.ve);
    assert!(est.total.unwrap() < est.fuel.unwrap() + est.ve.unwrap());

    // Unselected services read 0.0, not absent.
    assert_eq!(est.tires, Some(0.0));
    assert_eq!(est.driver_swap, Some(0.0));
}

#[test]
fn pitstop_times_parse_off_the_real_capture() {
    // The static rate card, which lives on RepairAndRefuel despite the type
    // name — real data, wrong endpoint until the Spa capture sorted it out.
    let json = include_str!("fixtures/repair_and_refuel.json");
    let body: serde_json::Value = serde_json::from_str(json).unwrap();
    let times: PitstopEstimateTimes =
        serde_json::from_value(body["pitStopTimes"]["times"].clone()).unwrap();

    assert_eq!(times.fuel_fill_rate, Some(2.8570001125335693));
    assert_eq!(times.four_tire_change, Some(12.0));
    assert_eq!(times.fuel_insert, Some(1.5));
    assert_eq!(times.driver_change, Some(25.0));

    // The camelCase virtualEnergy* family breaks the PascalCase pattern.
    assert_eq!(times.virtual_energy_fill_rate, Some(0.02500000037252903));

    // ...and not every entry is a duration: these two are flags.
    assert_eq!(times.on_the_fly_pressure, Some(false));
    assert_eq!(times.simultaneous_stop_go, Some(false));
}

#[test]
fn wearables_brakes_are_wear_but_tyres_are_life_remaining() {
    let json = include_str!("fixtures/repair_and_refuel.json");
    let rr: RepairAndRefuel = serde_json::from_str(json).unwrap();
    let w = rr.wearables.expect("wearables");

    // Fresh pads read 0.032: brake wear starts near 0 and rises.
    assert_eq!(w.brakes, vec![0.032; 4]);
    // Fresh rubber reads ~0.95: tyre tread REMAINING starts near 1 and falls
    // (the per-lap `tyres` percent in strategy/usage does the same, dropping
    // lap on lap and jumping back to 100 after a tyre change). Reading either
    // metric the wrong way round inverts every alert.
    assert!(w.tires.iter().all(|t| *t > 0.9));
}

#[test]
fn pit_recommendations_parse_off_the_real_capture() {
    let json = include_str!("fixtures/repair_and_refuel.json");
    let rr: RepairAndRefuel = serde_json::from_str(json).unwrap();

    // Keys here are lowercase/camelCase (`fuel`, `virtualEnergy`) — unlike the
    // PascalCase pitStopTimes. The zeros are real: QUALIFY1 needs nothing added.
    let rec = rr.pit_recommendations.expect("pitRecommendations");
    assert_eq!(rec.fuel, Some(0.0));
    assert_eq!(rec.virtual_energy, Some(0.0));
    // The tyre-advice key is literally "TIRES:", colon included.
    assert_eq!(rec.tires, Some(0));
}

#[test]
fn a_missing_endpoint_body_is_an_error_not_a_panic() {
    // Out of session, these endpoints return `null` or an empty body.
    assert!(serde_json::from_str::<Standings>("").is_err());
    let null: Result<StrategyUsage, _> = serde_json::from_str("null");
    assert!(null.is_err());
}

#[test]
fn sector_times_are_cumulative_and_get_differenced_into_splits() {
    let json = include_str!("fixtures/standings.json");
    let standings: Standings = serde_json::from_str(json).unwrap();
    let car = standings
        .0
        .iter()
        .find(|e| e.car_number.as_deref() == Some("23"))
        .expect("car 23");

    // The raw fields are elapsed-at-sector-end: S2 is S1+S2, not the S2 split.
    assert!(car.last_sector_time2.unwrap() > car.last_sector_time1.unwrap());

    // Split out, the three sectors must add back up to the lap.
    let [s1, s2, s3] = car.sectors();
    assert!((s1.unwrap() - 41.4427).abs() < 1e-3);
    assert!(
        (s2.unwrap() - 71.1427).abs() < 1e-3,
        "S2 split = S2cum - S1cum"
    );
    assert!((s3.unwrap() - 42.4487).abs() < 1e-3, "S3 = lap - S2cum");
    let sum: f64 = [s1, s2, s3].iter().flatten().sum();
    assert!(
        (sum - car.last_lap_time.unwrap()).abs() < 1e-6,
        "splits reconstruct the lap"
    );
}

#[test]
fn a_car_with_no_time_yet_reports_no_sectors_rather_than_negative_ones() {
    // LMU's "not set" is -1.0 (and 0.0 on a sector never run). Read naively a
    // -1 s sector is the fastest of the session — it must come back as None.
    let json = r#"[{"lastSectorTime1":-1.0,"lastSectorTime2":-1.0,"lastLapTime":-1.0,
                    "bestLapSectorTime1":0.0,"bestLapSectorTime2":0.0,"bestLapTime":-1.0}]"#;
    let standings: Standings = serde_json::from_str(json).unwrap();
    let car = &standings.0[0];
    assert_eq!(car.sectors(), [None, None, None]);
    assert_eq!(car.best_sectors(), [None, None, None]);
    assert_eq!(car.pace(), None);
}

#[test]
fn standings_carry_the_world_positions_a_track_map_needs() {
    let json = include_str!("fixtures/standings.json");
    let standings: Standings = serde_json::from_str(json).unwrap();

    // Every car in a real capture has a position and a distance along the lap:
    // together they're an ordered point cloud, i.e. a circuit.
    let located = standings
        .0
        .iter()
        .filter(|e| e.car_position.x.is_some() && e.lap_distance.is_some())
        .count();
    assert_eq!(located, 24, "all 24 cars are locatable");
}

// ------------------------------------------------- telemetry and conditions

#[test]
fn world_vectors_decompose_into_car_relative_g() {
    let json = include_str!("fixtures/standings.json");
    let standings: Standings = serde_json::from_str(json).unwrap();

    // Both vectors are world-space, so neither component is a car-relative g
    // on its own. Projecting acceleration onto the velocity vector splits it.
    let moving: Vec<_> = standings
        .0
        .iter()
        .filter(|e| e.speed_kph().is_some_and(|v| v > 20.0))
        .collect();
    assert!(!moving.is_empty(), "a real capture has cars on track");

    for car in &moving {
        let (lon, lat) = car.g_forces().expect("a moving car has a heading");
        // A racing car does not pull 6 g. A wrong frame or a bad divide would.
        assert!(
            lon.abs() < 6.0 && (0.0..6.0).contains(&lat),
            "{lon} / {lat} g is not physical"
        );
    }
}

#[test]
fn a_stationary_car_has_no_heading_to_project_against() {
    let json = include_str!("fixtures/standings.json");
    let standings: Standings = serde_json::from_str(json).unwrap();

    // Dividing by a near-zero speed would manufacture enormous g out of a
    // parked car; g_forces() declines instead.
    let parked = standings
        .0
        .iter()
        .find(|e| e.speed_kph().is_some_and(|v| v < 1.0));
    if let Some(car) = parked {
        assert_eq!(car.g_forces(), None);
    }
}

#[test]
fn a_lap_in_progress_is_a_distance_and_a_time() {
    let json = include_str!("fixtures/standings.json");
    let standings: Standings = serde_json::from_str(json).unwrap();

    // These two together are the whole basis of the live delta: without both,
    // there is no distance-to-time curve to compare laps with.
    let traceable = standings
        .0
        .iter()
        .filter(|e| e.lap_distance.is_some() && e.time_into_lap.is_some())
        .count();
    assert_eq!(traceable, 24);
}

#[test]
fn weather_is_kelvin_and_the_forecast_is_not() {
    let json = include_str!("fixtures/repair_and_refuel.json");
    let rr: RepairAndRefuel = serde_json::from_str(json).unwrap();

    let now = rr.current_weather.expect("live conditions");
    // Raw kelvin would put ~290 on a screen labelled °C.
    assert!(
        now.ambient_temp_kelvin.unwrap() > 250.0,
        "raw field is kelvin"
    );
    let air = now.air_c().unwrap();
    assert!(
        (-20.0..60.0).contains(&air),
        "{air} C is not a race weekend"
    );
    assert!(
        now.track_c().unwrap() > air,
        "tarmac runs hotter than the air"
    );

    // The forecast, by contrast, is already celsius — and arrives as parallel
    // arrays that only mean anything zipped by index.
    let nodes = rr.weather_forecast.expect("forecast").nodes;
    assert!(!nodes.start_time.is_empty());
    assert_eq!(nodes.temperature.len(), nodes.start_time.len());
    assert_eq!(nodes.rain_chance.len(), nodes.start_time.len());
    assert!(
        nodes.temperature.iter().all(|t| (-20.0..60.0).contains(t)),
        "already celsius"
    );
}

#[test]
fn the_pit_menu_resolves_its_index_to_a_label() {
    let json = include_str!("fixtures/repair_and_refuel.json");
    let rr: RepairAndRefuel = serde_json::from_str(json).unwrap();

    let menu = rr.pit_menu.expect("pit menu").pit_menu;
    assert!(!menu.is_empty(), "the menu the driver sees in-car");

    // `currentSetting` is an index into `settings`; the label is the only part
    // that carries meaning ("New Medium", "3.0 deg (-3)").
    let tires = menu
        .iter()
        .find(|i| i.name.as_deref() == Some("TIRES:"))
        .expect("every car can change tyres");
    assert!(
        tires.selected().is_some(),
        "the armed setting resolves to text"
    );

    // An out-of-range index must read as unknown, not panic or wrap.
    let bogus = pf_core::lmu::types::PitMenuItem {
        current_setting: Some(99),
        ..tires.clone()
    };
    assert_eq!(bogus.selected(), None);
}

#[test]
fn virtual_energy_counters_only_mean_anything_as_a_ratio() {
    let json = include_str!("fixtures/repair_and_refuel.json");
    let rr: RepairAndRefuel = serde_json::from_str(json).unwrap();

    // fuelInfo reports VE as raw counts in the hundreds of millions. On a car
    // with no VE system max is 0, and the ratio must decline rather than
    // divide by zero.
    match rr.fuel_info.ve_fraction() {
        Some(f) => assert!((0.0..=1.0).contains(&f), "{f} is not a fraction"),
        None => assert!(rr.fuel_info.max_virtual_energy.unwrap_or(0.0) == 0.0),
    }
}
