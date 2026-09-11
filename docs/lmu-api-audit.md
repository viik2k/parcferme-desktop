# `lmu-api` audit — shared-memory readiness

**Date:** 2026-09-07 · **Scope:** the ported `lmu-api` code now living at
`crates/pf_core/src/lmu.rs` + `crates/pf_core/src/lmu/types.rs`, its tests at
`crates/pf_core/tests/lmu_deserialize.rs`, and the upstream crate it came from
(`C:\Users\finnh\Desktop\Repos\boxbox\lmu-api`, outside this workspace).

## Verdict

**Neither. It is not a shared-memory client of any kind.**

`lmu-api` is a **blocking HTTP client for LMU's local REST server on
`http://localhost:6397`** (`crates/pf_core/src/lmu.rs:19`), polling five JSON
endpoints (`crates/pf_core/src/lmu.rs:22-30`). It is not on the `LMU_Data` path
and not on the rF2 SMMP path.

Evidence — this returns nothing across the whole workspace:

```
grep -rniE "rFactor2SMMP|LMU_Data|OpenFileMapping|MapViewOfFile|memmap|shared_?memory|CreateFileMapping" \
  --include=*.rs --include=*.toml --include=*.ts --include=*.tsx --include=*.md crates apps docs Cargo.toml
→ NO MATCHES
```

The same grep over the whole `boxbox` repo hits only `node_modules` TypeScript
lib files and two prose lines (`boxbox/CLAUDE.md:92`,
`boxbox/boxbox-engine/src/lap.rs:5`), both of which assert *"LMU publishes no
channel feed and no shared memory"* — a belief that is now false (see
Surprises). `boxbox/DEFERRED.md:84` lists "shared-memory telemetry" as
explicitly out of scope for that phase.

No Windows FFI dependency exists to do it with: `crates/pf_core/Cargo.toml:11-19`
is `serde`, `serde_json`, `thiserror`, `log`, `ureq`, `keyring` — no `windows`,
no `winapi`, no `memmap2`.

So the framing of "swap the mapping name and adjust offsets" has nothing to
swap. Moving to `LMU_Data` is **new construction**, not a migration.

## The eight questions

| # | Question | Finding | Reference |
|---|---|---|---|
| 1 | Which mapping name(s) does it open? | **None.** No `OpenFileMappingW`, no mmap crate, no FFI at all. The only I/O is `ureq::Agent::get(...).call()` against `http://localhost:6397`. | `crates/pf_core/src/lmu.rs:19`, `:34`, `:58-60`; `crates/pf_core/Cargo.toml:11-19` |
| 2 | Hand-written or generated structs? Source of layout? Header version? | **Hand-written Rust `serde` structs for JSON** — no C layout anywhere, so the offset question is moot. Provenance is documented and unusually good: shapes were derived from **real captured responses**, not a header — `crates/pf_core/src/lmu/types.rs:1-11`, `crates/pf_core/tests/lmu_deserialize.rs:3-6` (2026-07-13 QUALIFY1, LMP3, 24 cars; a second Spa RACE1 capture 2026-07-25). Endpoint *paths* come from the game's own OpenAPI at `/swagger-schema.json`, which documents paths but no bodies (`crates/pf_core/src/lmu.rs:9-11`). **No header, no version stamp, and no game-build number is recorded anywhere** — the fixtures are dated, but the LMU build that produced them is not. Cannot be determined from the code. | as cited |
| 3 | Single copy or double buffered? Torn-frame detection? | N/A in the shared-memory sense — there is no buffer and no version counter to check. The analogous hazard **is present and unhandled**: each endpoint is a *separate HTTP request* (`crates/pf_core/src/lmu.rs:71-89`), so a caller assembling standings + strategy + fuel gets three snapshots taken at different instants, with no sequence number or timestamp to reconcile them. Nothing in the crate detects or rejects that skew. | `crates/pf_core/src/lmu.rs:71-99` |
| 4 | Polling model? Configurable or constant? | **The crate does not poll at all.** It is pull-only: five `&self` methods, one request each, no thread, no loop, no sleep, no clock. Grep for `thread::spawn\|sleep\|interval` in `crates/pf_core/src` hits only OAuth device-poll fields in `api.rs`/`auth.rs`, nothing in `lmu`. The only time constant is a **2 s request timeout** (`crates/pf_core/src/lmu.rs:46-50`), hard-coded, not configurable. Cadence is the caller's problem — and **there is currently no caller in this workspace** (only the tests reference `lmu::`). Upstream, `boxbox-app` owned the loop on a `std::thread`: standings every 200 ms, everything else every 10th tick (`boxbox/CLAUDE.md:62`, `:105-107`) — external repo, not ported. | as cited |
| 5 | Scoring, telemetry, or both? How are the rates reconciled? | The REST split does not map onto telemetry/scoring. It reads **scoring-equivalent data only** (standings, session clock, strategy usage, fuel/wear/weather/pit menu) — `crates/pf_core/src/lmu.rs:22-30`. There is **no per-frame vehicle telemetry** (see the channel table). Reconciliation: **none in the crate.** It returns each endpoint's deserialized body independently; no combined snapshot type exists here. (Upstream `boxbox-engine` built a `Snapshot` from the parts — not ported.) | `crates/pf_core/src/lmu.rs:71-89` |
| 6 | Does it allocate per read? | **Yes, twice per read, unavoidably as written.** `get_raw` calls `resp.into_string()` → a fresh `String` of the whole body (`crates/pf_core/src/lmu.rs:61`), then `get_json` runs `serde_json::from_str` into owned `Vec`/`BTreeMap`/`String` trees (`crates/pf_core/src/lmu.rs:91-99`; e.g. `types.rs:21`, `:96`). Real body sizes from the fixtures: standings **56 KB**, repair_and_refuel **21 KB**, strategy_usage **9.8 KB**. At 10 Hz that is ~560 KB/s of allocate-parse-free for standings alone, plus JSON parsing cost — not fatal, but the wrong shape for a hot relay path. No buffer reuse, no borrowed deserialization, no arena. | as cited |
| 7 | Anything touching `localhost:6397`? | **All of it.** That is the crate's entire reason to exist. Five endpoints: `/rest/strategy/usage`, `/rest/watch/sessionInfo`, `/rest/watch/standings`, `/rest/garage/UIScreen/RepairAndRefuel`, `/rest/strategy/pitstop-estimate`. Transport failure on localhost maps to a dedicated `Error::LmuNotRunning` ("game isn't up"), distinct from a network error. | `crates/pf_core/src/lmu.rs:19`, `:22-30`, `:58-69`; `crates/pf_core/src/error.rs:43`, `:83` |
| 8 | Channel coverage | See the table below. | — |

## (8) Channel coverage from what it reads today

Availability judged against the modeled types and verified against the real
fixture key sets (`standings.json` carries 61 keys per car).

| Channel | Status | Where / how |
|---|---|---|
| Speed | **Available** | `carVelocity.velocity` (m/s) → `speed_kph()`, `types.rs:123`, `:293` |
| Throttle | **Missing** | No such key in any of the five endpoints. Not derivable. |
| Brake (pedal) | **Missing** | Same. `Wearables::brakes` (`types.rs:336`) is pad *wear*, not pedal. |
| Steering | **Missing** | Same. Not derivable. |
| Gear | **Missing** | Same. Not derivable. |
| RPM | **Missing** | Same. Not derivable. |
| Lap number | **Available** | `lapsCompleted`, `types.rs:151` |
| Current lap time | **Available** | `timeIntoLap` (seconds elapsed on the lap in progress), `types.rs:189` |
| Last lap time | **Available** | `lastLapTime`, `types.rs:153`; the `-1.0` sentinel is handled by `pace()`, `types.rs:243` |
| Sector | **Available** | `sector` ("SECTOR1"…), `types.rs:177`; splits **derivable** — the raw fields are cumulative, differenced by `sectors()`, `types.rs:255` |
| Normalised track position | **Derivable, needs one new field** | `standings.lapDistance` (metres, negative before the line), `types.rs:212`, ÷ track length. Track length *is* published as `sessionInfo.lapDistance` (6981.6 in the fixture) but is **not modeled** — `SessionInfo` carries only five fields, `types.rs:67-74`. One field to add. |
| Fuel | **Available** | Per car as a fraction: `fuelFraction`, `types.rs:156`; in litres for the player: `FuelInfo::current_fuel`/`max_fuel`, `types.rs:364-366`; per-lap history via `UsageLap::fuel`, `types.rs:51` |
| Per-corner tyre temps | **Missing** | No temperature key anywhere in the fixtures. Not derivable. |
| Per-corner tyre wear | **Available (player only)** | `Wearables::tires` — tread **remaining**, ~0.95 fresh, falling (`types.rs:337`); per-lap `UsageLap::tyres` (`types.rs:55`). Corner order FL/FR/RL/RR is **assumed, unverified**. |
| Per-corner brake temps | **Missing** | `Wearables::brakes` is wear (0.032 fresh, rising), not temperature — `types.rs:335-336`, pinned by the `wearables_brakes_are_wear_but_tyres_are_life_remaining` test. Not derivable. |
| Lateral / longitudinal G | **Derivable** | `g_forces()` projects world-frame `carAcceleration` onto `carVelocity`, `types.rs:274-291`. Caveat: **lateral is unsigned** — left from right needs the car's up-axis, which REST does not publish. |
| Position in class | **Derivable** | `position` is *overall* (`types.rs:150`); with `carClass` (`types.rs:148`), count same-class cars ahead. Class gaps are modeled directly: `timeBehindClassLeader` (`types.rs:167`), `lapsBehindClassLeader` (`types.rs:205`). |
| Gap ahead | **Available** | `timeBehindNext`, `types.rs:166`. Use `lapsBehindNext` (`types.rs:169`) to reject the lapped case, where the seconds are noise. |
| Gap behind | **Derivable** | Not published per car; read the *following* car's `timeBehindNext` out of the same array (`Standings(pub Vec<StandingEntry>)`, `types.rs:96`). |

Score: **9 available, 4 derivable, 5 missing.** Every missing channel is a
driver-input or thermal channel — precisely the set a 10 Hz relay exists to
carry, and precisely the set only shared memory publishes.

## Recommendation

**Keep both, feature-gated — and build the `LMU_Data` reader from scratch.**
"Swap the mapping name and adjust offsets" is not on the table (nothing to
swap). "Port properly via bindgen" is the right *shape* but has a licence
problem (below), so: hand-written `#[repr(C)]` structs transcribed from the
shipped S397 header, verified against a golden byte capture.

Why both rather than a straight replacement: shared memory does **not**
supersede REST here. `pitstop-estimate`, `pitRecommendations`, `pitMenu`, the
five-node `weatherForecast` and `pitStopTimes` are game-UI/strategy state, and
nothing in `SharedMemoryObjectOut` obviously replaces them
(`SharedMemoryInterface.hpp:219-224`). The split is clean: **shared memory for
anything that moves inside a corner, REST for anything that moves on the scale
of a lap** — the same split `boxbox-app` arrived at empirically
(`boxbox/CLAUDE.md:105-107`).

The target is real and verified on this machine
(`…\Le Mans Ultimate\Support\SharedMemoryInterface`, three headers dated
2026-07-09):

- Mapping `"LMU_Data"`, event `"LMU_Data_Event"`, plus a separate lock mapping
  `"LMU_SharedMemoryLockData"` — `SharedMemoryInterface.hpp:85-86`, `:159-169`.
- Sync is **not** rF2-style double-buffer-with-version-counters. It is *wait on
  the event → take the named lock → memcpy the struct → unlock*
  (`SharedMemoryInterface.hpp:45-56`). So the relay can be **event-driven, not
  polled**, and torn frames are prevented by the lock rather than detected after
  the fact. A simpler and better contract than SMMP's.
- Every missing channel is present: `mUnfilteredThrottle` / `mUnfilteredBrake` /
  `mUnfilteredSteering` (`InternalsPlugin.hpp:229-231`), `mGear` `:222`,
  `mEngineRPM` `:223`, per-wheel `mTemperature[3]` in kelvin `:162`, `mWear`
  `:163`, `mBrakeTemp` `:147`, `mLocalAccel` (already car-relative, so **signed**
  lateral, unlike the REST derivation) `:213`, `mLapNumber` `:205`, `mLapDist`
  `:414`, `mCurrentSector` `:267`, `mFuel` in litres `:254`, and
  `mPlace`/`mTimeBehindNext`/`mTimeBehindLeader` `:434-440`.

**Effort: roughly 3–4 focused days**, dominated by struct transcription, not by
plumbing.

| Piece | Estimate |
|---|---|
| Open mapping + event + lock, map view, RAII teardown (six `extern "system"` declarations, no new crate — or the `windows` crate if preferred) | ~0.5 day |
| Transcribe `TelemInfoV01`, `TelemWheelV01`, `TelemVect3`, `ScoringInfoV01`, `VehicleScoringInfoV01` and the five wrapper structs as `#[repr(C)]` | 1.5–2 days |
| Layout verification: `assert_eq!(size_of::<T>(), N)` against a byte dump from a live session; one golden-bytes fixture test in the style of the existing JSON ones | 0.5 day |
| Channel mapping + a `Frame` type + 10 Hz decimation from the 100 Hz event | 0.5 day |
| Feature gate (`lmu-shm`), capability probe, fallback to REST when the mapping is absent | 0.5 day |

**What would change the call:**

- *Drop the shared-memory work entirely* if the relay's frame spec turns out to
  need only timing/strategy/gaps at 10 Hz — REST already covers 13 of the 18
  channels, and 200 ms standings polling is proven upstream.
- *Go bindgen-at-build-time* if we accept a build dependency on a local LMU
  install (breaks CI on any machine without the game — probably disqualifying).
- *Go shared-memory-only* if a capture shows `scoringStream` / `ScoringInfoV01`
  actually carries the pit-menu and forecast data REST gives us; that kills the
  dual path and saves the feature gate.
- *Reconsider the whole approach* if S397 ships a versioned struct or a C API —
  the headers carry no version constant today, so we would be pinning to a build
  number we cannot read.

## Surprises worth knowing before the port

1. **The source repo's own documentation is wrong about the premise.**
   `boxbox/CLAUDE.md:92` and `boxbox/boxbox-engine/src/lap.rs:5` both state LMU
   "publishes no channel feed and no shared memory". The header shipped in the
   game install is dated 2026-07-09 and the fixtures were captured 2026-07-13 —
   so the shared memory existed *before* the captures that motivated the REST
   scraping. For the telemetry half, the REST client may have been unnecessary
   from day one. Worth correcting that line before someone else reads it as fact.
2. **The header cannot be vendored.** `SharedMemoryInterface.hpp:1-13`:
   *"Redistribution or modification of this header is not permitted."* That
   rules out committing it for bindgen and makes build-time bindgen dependent on
   the user's install. Transcribing the layout into Rust is the normal way
   around this, but it is a legal call, not a technical one — settle it before
   writing the structs.
3. **The whole payload is copied under a global lock, every frame.**
   `CopySharedMemoryObj` (`SharedMemoryInterface.hpp:229-249`) memcpys generic +
   paths + scoring + telemetry, with `vehScoringInfo[104]` and `telemInfo[104]`
   arrays plus a 64 KB `scoringStream`. That is hundreds of KB per event at up
   to 100 Hz, and the lock is shared with the game. The sample copies
   everything; we should copy only the player's `telemInfo[playerVehicleIdx]`
   and the scoring slice actually used, into a **pre-allocated** buffer.
4. **`vehScoringInfo[104]` is annotated `// MUST NOT BE MOVED!`**
   (`SharedMemoryInterface.hpp:193`) — a strong hint that offsets are
   load-bearing for other consumers, and that S397 may append but not reorder.
   Mildly reassuring for a hand transcription; still worth a size assert.
5. **The event wait wants LMU's PID.** The sample waits on the *game process
   handle* alongside the data event so it notices a crash
   (`SharedMemoryInterface.hpp:43-50`). A relay needs that too, or it blocks
   forever in `WaitForMultipleObjects` after LMU exits. Our current "is the game
   up?" signal is a 2 s HTTP timeout (`crates/pf_core/src/lmu.rs:46-50`,
   `:65-67`); the two liveness notions will need reconciling.
6. **The `LMU_Data` path needs user action we cannot perform.** Settings →
   Gameplay → Enable Plugins, then a game restart. The relay must detect the
   mapping's absence and say *that*, not "not running" — one more error kind
   beside `Error::LmuNotRunning` (`crates/pf_core/src/error.rs:43`), which per
   the IPC contract also means a hint in `apps/pf_desk/src/lib/errors.ts`.
7. **Temperature units differ across the two sources.** Shared-memory wheel
   temps are kelvin (`InternalsPlugin.hpp:162`), and so is REST `currentWeather`
   (`types.rs:482-500`), but the REST weather *forecast* is already celsius
   (`types.rs:521-530`). Three unit conventions in one frame is a bug waiting to
   happen.
8. **A modeled field is absent from its own fixture.** `FuelInfo` documents
   `currentVirtualEnergy`/`maxVirtualEnergy` as raw counters around 5.2e8
   (`types.rs:368-370`), but `repair_and_refuel.json` contains neither key, so
   the `ve_fraction` test only ever exercises the `None` branch. The claim comes
   from a Spa capture (`boxbox/ASSUMPTIONS.md`) that was not ported with the
   fixtures. Not blocking; just do not assume that path is tested.
9. **The five fixtures are two different sessions** — Sebring QUALIFY1
   2026-07-13 for most, Spa RACE1 2026-07-25 for `pitstop_estimate.json`
   (`crates/pf_core/tests/lmu_deserialize.rs:3-6` and the `pitstop_estimate`
   test comment) — and `session_info.json` names Spa while the file header
   comment says Sebring. Harmless for deserialization tests, misleading if
   anyone cross-references values between files.
10. **Nothing calls this code yet.** The only references to `pf_core::lmu`
    outside the module are in the test file. Whatever cadence and reconciliation
    policy the relay needs is unconstrained by existing callers — design it now
    rather than inheriting `boxbox-app`'s 200 ms / 10-tick split by default.

## Not determinable from the code

- **The update rate of the REST endpoints.** No timestamps in the responses and
  no capture-interval metadata in the ported fixtures. The 100 Hz / 5 Hz figures
  in the brief belong to the shared-memory path, not this one.
- **Which LMU build the fixtures came from.** Dates only, no game version.
- **Whether `LMU_Data` is populated on this machine right now** — that needs the
  game running with plugins enabled; only the headers were inspected, statically.
- **`sizeof(SharedMemoryLayout)`** — the headers were read, not compiled.
