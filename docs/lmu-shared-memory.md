# LMU shared memory: provenance of the transcription

`crates/pf_core/src/lmu/shm.rs` reads the `LMU_Data` mapping the game publishes
itself. Every offset in it is a number we produced and can reproduce. This file
records where they came from, because a hand transcription with no paper trail
is indistinguishable from a guess.

## The header is not in this repo, on purpose

S397 ships the headers with the game at
`…\Le Mans Ultimate\Support\SharedMemoryInterface\`:

```
InternalsPlugin.hpp        # TelemInfoV01, VehicleScoringInfoV01, ScoringInfoV01
SharedMemoryInterface.hpp  # the wrappers, the lock, the mapping names
PluginObjects.hpp
```

Their header states: *"Redistribution or modification of this header is not
permitted."* So it is **never copied into this repo** — not vendored, not
committed for `bindgen`, not pasted into a comment. What we hold is a
transcription of the layout: field names, types and offsets, which is the same
thing every third-party plugin does.

## How the numbers were produced

Two independent sources agree on every value:

1. **Compiling the shipped header locally.** A throwaway `probe.cpp` includes
   the header *by absolute path from the game install* and prints
   `sizeof`/`offsetof`. MSVC 14.51, x64. Nothing about it is committed.
2. **TinyPedal's [`pyLMUSharedMemory`](https://github.com/TinyPedal/pyLMUSharedMemory)**,
   which is **MIT licensed** (checked 2026-09-07: `License.txt`, MIT, © 2021
   Tony Whitley / © 2025 Xiang) and transcribes the same header into ctypes.
   Used as a cross-check, not as the source.

The header does not compile standalone — it uses `uint8_t`, `std::optional` and
`std::exchange` without including `<cstdint>`, `<optional>` or `<utility>`. A
probe has to include those first, and needs `/std:c++17`.

## The layout

Sizes, MSVC x64. `InternalsPlugin.hpp` is under `#pragma pack(push, 4)`; the
`SharedMemory*` wrappers are declared **after** the matching `pop` and so are
default-aligned. That mixture is the single easiest thing to get wrong here.

| Struct | Size | Packing |
|---|---|---|
| `TelemVect3` | 24 | 4 |
| `TelemWheelV01` | 260 | 4 |
| `TelemInfoV01` | 1888 | 4 |
| `VehicleScoringInfoV01` | 584 | 4 |
| `ScoringInfoV01` | 548 | 4 |
| `SharedMemoryGeneric` | 332 | default |
| `SharedMemoryPathData` | 1300 | default |
| `SharedMemoryScoringData` | 126832 | default |
| `SharedMemoryTelemetryData` | 196356 | default |
| `SharedMemoryLayout` | **324824** | default |

Offsets into the mapping:

| What | Offset |
|---|---|
| `SharedMemoryObjectOut::scoring` | 1632 |
| ⤷ `scoringInfo` | +0 |
| ⤷ `scoringStreamSize` (`size_t`, after 4 bytes of padding) | +552 |
| ⤷ `vehScoringInfo[104]` | +560 |
| ⤷ `scoringStream[65536]` | +61296 |
| `SharedMemoryObjectOut::telemetry` | 128464 |
| ⤷ `activeVehicles` / `playerVehicleIdx` / `playerHasVehicle` | +0 / +1 / +2 |
| ⤷ `telemInfo[104]` (after 1 byte of padding) | +4 |

`SharedMemoryLayout` is 4 bytes larger than the sum of its members — trailing
padding for the 8-byte `size_t` inside the scoring block.

## Regenerating the golden fixture

`crates/pf_core/tests/fixtures/lmu_shm_golden.bin` (3020 bytes) is
`TelemInfoV01` + `VehicleScoringInfoV01` + `ScoringInfoV01`, filled with
recognisable values by a C++ program compiled against the shipped header. It is
**not** a capture from a running game — it proves the Rust transcription lands
on the same offsets the header does, nothing more. `tests/lmu_shm_layout.rs`
reads it back through `shm::read_struct`.

To regenerate after a game update: write a program that includes the header,
fills the three structs with known values, and `fwrite`s them in that order;
then update the expected values in the test. Compile with:

```sh
cl -nologo -EHsc -std:c++17 probe.cpp -Fe:probe.exe \
  -I"$MSVC/include" -I"$SDK/ucrt" -I"$SDK/um" -I"$SDK/shared" \
  -link -LIBPATH:"$MSVC/lib/x64" -LIBPATH:"$SDKLIB/ucrt/x64" -LIBPATH:"$SDKLIB/um/x64"
```

## Verified vs assumed

**Verified from the header text:**

- Corner order for every four-element array is **FL, FR, RL, RR**
  (`InternalsPlugin.hpp`, comment on `TelemWheelV01 mWheel[4]`). This retires
  the "order assumed, unverified" caveat the REST types carried.
- Local vehicle axes: **+x out the left side, +y up, +z out the back.** Forward
  acceleration is therefore `-z` and rightward is `-x`; both are negated in
  `build_frame`.
- `mWheel[].mBrakeTemp` is **celsius**; `mWheel[].mTemperature[3]` and
  `mTireCarcassTemperature` are **kelvin**. Two units, one struct.
- `mWear` is wear **rising from 0**, the opposite direction to the REST
  `wearables.tires` "life remaining".
- `mVirtualEnergy`, `mTimeGapCarAhead`, `mTimeGapCarBehind` are LMU additions
  and are `float`, not `double`, unlike everything around them.

## What a live session corrected (2026-09-07, LMP2, 5 laps)

The layout was right first time. Three of the header's own *comments* were not.

| Field | Header says | Live data says |
|---|---|---|
| `mWheel[].mBrakeTemp` | "Celsius" | **Kelvin.** A cold car in the garage reads 293.18 — ambient 20 °C, not 293 °C — rising to ~873 (600 °C) under braking. |
| `mWheel[].mWear` | "wear (0.0-1.0, fraction of maximum)" | **Life remaining.** 1.0 fresh, falling to 0.933 over five laps. Confirmed independently: it equals REST `wearables.tires` (already pinned as life remaining) to 1.7e-8 on all four corners. |
| Telemetry rate | interface implies 100 Hz | **~50 Hz.** A stride of 10 events produced 4.5 Hz output instead of 10, so frames are now paced against the clock. |

Also observed:

- `mWheel[].mTemperature` reads a flat **0.0 kelvin** until the car has been on
  track — a sentinel, not a temperature. Converted naively it puts -273.15 °C
  on the wire; it is now `None`.
- Corner order is consistent between shared memory and REST (both FL/FR/RL/RR),
  and front tyres ran hotter than rears (126/112 vs 88/80 °C) as expected.
- Driver inputs, gear (-1..6), RPM (to 8766), speed (to 290 km/h), sector 1-3,
  `track_pos` wrapping 0→1 once per lap, and lap times (96.99 s, 102.59 s) all
  behaved. `g_lon` went negative under braking (-2.45 g) and positive under
  power (+1.56 g), confirming the axis negation.
- Lock hold: **mean 6.2 µs, max 69 µs, zero timeouts** over 2900 frames. The
  game's writer is never meaningfully blocked.

## What the multi-class session added (2026-09-07, Hypercar, 38 cars, 3 classes)

- **`mTimeGapCarAhead` is negative.** Measured `-1.593` while running 11th of
  38, with `mTimeGapCarBehind` at `+0.367` in the same frame. The two gaps are
  signed in opposite directions. Applying the lap-time `-1.0` sentinel rule to
  them nulled **every** gap-ahead across 11,493 frames while gap-behind looked
  perfect — an asymmetry is the tell. Both are now published as positive
  magnitudes, with an exact `0.0` meaning "no car to measure against".
- **`mVirtualEnergy` confirmed** on a car that has it: 1.0 → 0.754 over the
  stint, populated in every frame. (It reads a flat 0 on LMP2/GT3, which is why
  the earlier session proved nothing.)
- **`place_in_class` confirmed against a real grid:** 1-11 within class while
  1-15 overall, so the class ranking is genuinely independent of the overall
  position.
- **Game-exit detection confirmed.** Quitting the game ended the reader cleanly
  through the process-handle wait — no hang, no partial frame.
- Rates on a full grid: **47.6 Hz** event stream, **9.5 Hz** output, lock
  **mean 9.7 µs / max 376 µs**, zero lock timeouts over 12,345 frames. The max
  is up from 69 µs on an empty track because the copy scales with `mNumVehicles`
  (38 × 584 bytes), which is the intended trade.
- One frame hit -7.1 g longitudinal at 34 km/h under full brake with 2.2 g
  lateral: a low-speed impact, not an artefact. Every other frame stayed inside
  ±3 g.

**Still assumed:**

- That `mCurrentSector`'s low bits are 0/1/2 for sectors 1/2/3 and the sign bit
  is the pit lane. All three sectors and 1,472 in-pit-lane frames were observed,
  but `in_pit_lane` also ORs in the scoring `mInPits`, so the sign-bit half of
  that expression is still not independently proven.
- `mTimeGapPlaceAhead` / `mTimeGapPlaceBehind` (offsets 788/792) read **368.198**
  and **535.238** in the same frame the seconds-gaps read -1.593/+0.367. Whatever
  they are, they are not seconds — probably metres. Not modeled.
- The scoring half's own update rate.

## One deliberate deviation from the header's sample code

`SharedMemoryLock::Lock` in the shipped sample returns `true` when the wait
event fires **without** having won the compare-exchange, and leaves `waiters`
incremented. Copying that would hand the lock to two readers at once and leak a
wakeup on every contention. `shm::SharedMemoryLock::acquire` loops until the CAS
actually succeeds and always balances the counter.
