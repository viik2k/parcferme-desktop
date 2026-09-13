//! Layout tests for the `LMU_Data` transcription.
//!
//! `fixtures/lmu_shm_golden.bin` is **not** a capture from a running game — it
//! is a byte dump written by a small C++ program that includes S397's own
//! shipped header (`Support\SharedMemoryInterface`) and fills three structs
//! with recognisable values. That makes it the authority the Rust transcription
//! is checked against: if a `_pad` in `shm.rs` is wrong, or S397 moves a field
//! and the dump is regenerated, these tests fail instead of the relay quietly
//! streaming garbage.
//!
//! Layout: `TelemInfoV01` (1888) + `VehicleScoringInfoV01` (584) +
//! `ScoringInfoV01` (548) = 3020 bytes, in that order.
//!
//! What these tests do **not** prove: that a field means what the header's
//! comment says. Semantics need a running game — see `docs/lmu-shared-memory.md`.

use std::mem::size_of;
use std::sync::Arc;
use std::time::Duration;

use pf_core::lmu::shm::{
    c_str, read_struct, ScoringInfoV01, Snapshot, TelemInfoV01, TelemWheelV01,
    VehicleScoringInfoV01,
};
use pf_core::lmu::{build_frame, RestSnapshot};

const GOLDEN: &[u8] = include_bytes!("fixtures/lmu_shm_golden.bin");
const OFF_TELEM: usize = 0;
const OFF_VEH: usize = 1888;
const OFF_SCORING: usize = 1888 + 584;

fn parts() -> (TelemInfoV01, VehicleScoringInfoV01, ScoringInfoV01) {
    (
        read_struct(GOLDEN, OFF_TELEM).expect("telemetry"),
        read_struct(GOLDEN, OFF_VEH).expect("vehicle scoring"),
        read_struct(GOLDEN, OFF_SCORING).expect("scoring"),
    )
}

fn golden_snapshot() -> Snapshot {
    let (telem, veh, scoring) = parts();
    Snapshot::from_parts(telem, scoring, vec![veh])
}

#[test]
fn struct_sizes_match_the_compiled_header() {
    // The four numbers the whole transcription rests on. MSVC, x64,
    // #pragma pack(4).
    assert_eq!(size_of::<TelemWheelV01>(), 260);
    assert_eq!(size_of::<TelemInfoV01>(), 1_888);
    assert_eq!(size_of::<VehicleScoringInfoV01>(), 584);
    assert_eq!(size_of::<ScoringInfoV01>(), 548);
    assert_eq!(GOLDEN.len(), 1_888 + 584 + 548);
}

#[test]
fn the_wrapper_layout_constants_agree_with_the_struct_sizes() {
    use pf_core::lmu::shm::{
        LAYOUT_SIZE, MAX_VEHICLES, OFF_SCORING as SCORING_AT, OFF_TELEMETRY, OFF_TELEM_ARRAY,
        OFF_VEH_ARRAY,
    };
    // The wrappers are default-aligned even though their members are packed(4),
    // which is exactly the trap this pins down: the 8-byte `scoringStreamSize`
    // forces four bytes of padding after the 548-byte ScoringInfoV01.
    assert_eq!(OFF_VEH_ARRAY, 548 + 4 + 8);
    assert_eq!(
        OFF_TELEMETRY,
        SCORING_AT + OFF_VEH_ARRAY + MAX_VEHICLES * size_of::<VehicleScoringInfoV01>() + 65_536
    );
    assert_eq!(
        LAYOUT_SIZE,
        OFF_TELEMETRY + OFF_TELEM_ARRAY + MAX_VEHICLES * size_of::<TelemInfoV01>() + 4
    );
}

#[test]
fn driver_inputs_decode_at_the_right_offsets() {
    let (telem, _, _) = parts();
    // Bound to locals: a reference to an 8-byte field of a packed(4) struct is
    // not allowed, and assert_eq! takes references.
    let (throttle, brake, steering) = (
        telem.m_unfiltered_throttle,
        telem.m_unfiltered_brake,
        telem.m_unfiltered_steering,
    );
    assert_eq!(throttle, 0.75);
    assert_eq!(brake, 0.25);
    assert_eq!(steering, -0.5);

    let (gear, rpm, lap) = (telem.m_gear, telem.m_engine_rpm, telem.m_lap_number);
    assert_eq!(gear, 4);
    assert_eq!(rpm, 8_250.0);
    assert_eq!(lap, 7);

    let (fuel, capacity) = (telem.m_fuel, telem.m_fuel_capacity);
    assert_eq!(fuel, 42.5);
    assert_eq!(capacity, 100.0);
}

#[test]
fn the_lmu_specific_float_extensions_are_not_doubles() {
    // mVirtualEnergy and the two gap fields are f32 where everything around
    // them is f64. Reading them as f64 would land four bytes off and produce
    // garbage that still looks like a number.
    let (telem, _, _) = parts();
    let (ve, ahead, behind) = (
        telem.m_virtual_energy,
        telem.m_time_gap_car_ahead,
        telem.m_time_gap_car_behind,
    );
    assert!((f64::from(ve) - 0.6).abs() < 1e-6);
    assert_eq!(ahead, 1.5);
    assert_eq!(behind, 2.5);
}

#[test]
fn wheels_decode_in_order_and_carry_two_different_temperature_units() {
    let (telem, _, _) = parts();
    let wheels = telem.m_wheel;
    for (i, wheel) in wheels.iter().enumerate() {
        let brake = wheel.m_brake_temp;
        let life = wheel.m_wear;
        let temps = wheel.m_temperature;
        // Values were written as 300+i / 0.9-0.1i / 350+10i+s, so a swapped or
        // shifted corner shows up immediately. Both temperature fields are
        // kelvin in practice, whatever the header comments say.
        assert_eq!(brake, 300.0 + i as f64);
        assert!((life - (0.9 - 0.1 * i as f64)).abs() < 1e-9);
        assert_eq!(temps[0], 350.0 + 10.0 * i as f64);
        assert_eq!(temps[2], 352.0 + 10.0 * i as f64);
    }
}

#[test]
fn scoring_carries_the_track_length_that_normalises_lap_distance() {
    let (_, veh, scoring) = parts();
    let (track_len, cars) = (scoring.m_lap_dist, scoring.m_num_vehicles);
    assert_eq!(track_len, 6_981.6);
    assert_eq!(cars, 24);
    assert_eq!(c_str(&scoring.m_track_name), "Circuit de Spa-Francorchamps");

    let dist = veh.m_lap_dist;
    assert_eq!(dist, 3_490.8);
    assert_eq!(c_str(&veh.m_driver_name), "Finn Idagi");
    assert_eq!(c_str(&veh.m_vehicle_class), "LMP3");
    let place = veh.m_place;
    assert_eq!(place, 11);
}

#[test]
fn a_frame_reports_forces_in_the_conventional_directions() {
    let frame = build_frame(
        0,
        Duration::ZERO,
        &golden_snapshot(),
        Arc::new(RestSnapshot::default()),
        None,
        Duration::from_secs(3),
    );

    // Local axes are +x out the LEFT and +z out the BACK. The fixture's
    // acceleration is (-3.0, 0.1, +9.80665): pushed to the right, decelerating.
    // Read raw, the signs would be inverted on both.
    assert!(
        (frame.g_lon + 1.0).abs() < 1e-9,
        "braking must be negative g_lon"
    );
    assert!(
        (frame.g_lat - 3.0 / 9.806_65).abs() < 1e-9,
        "positive g_lat is to the right"
    );

    // Speed is the magnitude of the local velocity vector, so the -z forward
    // convention cannot make it negative.
    let expected = (1.0f64 + 0.25 + 3025.0).sqrt() * 3.6;
    assert!((frame.speed_kph - expected).abs() < 1e-9);
    assert!(frame.speed_kph > 0.0);
}

#[test]
fn a_frame_normalises_units_and_positions() {
    let frame = build_frame(
        7,
        Duration::from_millis(1_500),
        &golden_snapshot(),
        Arc::new(RestSnapshot::default()),
        None,
        Duration::from_secs(3),
    );
    assert_eq!(frame.seq, 7);
    assert_eq!(frame.t_ms, 1_500);

    // 3490.8 of 6981.6 metres.
    assert!((frame.track_pos.unwrap() - 0.5).abs() < 1e-12);
    // mCurrentSector is zero-based; frames publish 1-3.
    assert_eq!(frame.sector, Some(2));
    assert!(!frame.in_pit_lane);

    // Kelvin must not escape: the tread samples were 350/351/352 K...
    assert!((frame.tyre_temp_c[0].unwrap() - (351.0 - 273.15)).abs() < 1e-9);
    // ...and brake temp is kelvin too. The header calls it Celsius, but a live
    // session reads 293.18 on a cold car sitting in the garage — ambient in
    // kelvin, not 293 °C. Trusting the comment put 293 °C on the wire.
    assert!((frame.brake_temp_c[0].unwrap() - (300.0 - 273.15)).abs() < 1e-9);

    // mWear is life REMAINING despite its name: 1.0 fresh, falling with use
    // (measured 1.0 → 0.933 over five laps). Read as wear it inverts.
    assert!((frame.tyre_life[0] - 0.9).abs() < 1e-9);
    assert!(
        frame.tyre_life[0] > frame.tyre_life[3],
        "corner 0 was written most-fresh"
    );

    assert_eq!(frame.lap, 7);
    assert_eq!(frame.last_lap_time_s, Some(105.5));
    assert_eq!(frame.lap_time_s, Some(30.5));
    assert_eq!(frame.place, 11);
    assert_eq!(frame.car_class, "LMP3");
    assert_eq!(frame.gap_ahead_s, Some(1.5));
    assert_eq!(frame.gap_behind_s, Some(2.5));
}

#[test]
fn rest_fields_that_never_landed_are_marked_stale_not_current() {
    let frame = build_frame(
        0,
        Duration::ZERO,
        &golden_snapshot(),
        Arc::new(RestSnapshot::default()),
        None,
        Duration::from_secs(3),
    );
    // The whole point of the marker: a consumer must not read the defaults as
    // a live 0 °C track and an empty pit menu.
    assert!(frame.rest_stale);
    assert_eq!(frame.rest_age_ms, None);
    assert_eq!(frame.rest.air_c, None);
}

#[test]
fn a_short_buffer_declines_rather_than_reading_past_the_end() {
    assert!(read_struct::<TelemInfoV01>(&GOLDEN[..100], 0).is_none());
    assert!(read_struct::<ScoringInfoV01>(GOLDEN, GOLDEN.len() - 4).is_none());
    // The last struct in the dump must still fit exactly.
    assert!(read_struct::<ScoringInfoV01>(GOLDEN, OFF_SCORING).is_some());
}
