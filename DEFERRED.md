# DEFERRED

Things deliberately not built. One line each: what, and when it earns its place.

## LMU source (`pf_core::lmu`)

- **Full struct transcription.** Only the fields needed to reach and read the
  player's telemetry are modeled; the rest is `_pad`. Everything else in
  `TelemInfoV01` (tyre compounds, damage, ABS/TC settings, electric boost, the
  `mOri` orientation matrix) and in `ScoringInfoV01` (wetness, wind, session
  flags) stays opaque. Build when the wire protocol is settled and a consumer
  actually asks for a channel.
- **Position-based gaps.** The frame carries LMU's on-track
  `mTimeGapCarAhead`/`mTimeGapCarBehind` (verified live; note the ahead one is
  signed negative). `mTimeGapPlaceAhead`/`mTimeGapPlaceBehind` at offsets
  788/792 were sampled at 368.198 / 535.238 in a frame whose seconds-gaps were
  -1.593 / +0.367 — so they are not seconds, probably metres, and would need
  their own investigation before use. Add when the consumer needs a timing
  tower rather than a proximity warning.
- **Per-corner tyre temps as three points.** The game publishes inner / centre /
  outer per corner; the frame averages them to one number. Add when someone
  wants to see cross-tyre gradient, which is the whole reason the three exist.
- **Rivals' telemetry.** `telemInfo[]` holds up to 104 cars; we copy only the
  player's. The scoring array is already copied in full for the field. Add when
  a feature needs another car's inputs, and note it makes the locked copy
  proportionally longer.
- **`scoringStream` (64 KB).** Never copied. It is the results stream; nothing
  needs it until session results matter.
- **Capture and replay.** The old boxbox `capture`/`replay` pair recorded REST
  bodies to disk and played them back. `shm::read_struct` and
  `Snapshot::from_parts` are the seam a replay would attach to. Build when
  developing against a session you cannot rerun.
- **A pit-lane entry with the sector sign bit.** `in_pit_lane` ORs the sign bit
  of `mCurrentSector` with the scoring `mInPits`. 1,472 in-pit frames were
  observed, but the OR means the sign-bit half could be dead and nothing would
  show. Prove it or drop it.
- **Why LMU's REST server is slow for this client.** It answers `curl` in
  ~0.21 s and this client in ~2.05 s, consistently, regardless of agent
  settings, headers, `Connection`, or `accept-encoding` — every ureq variant
  tried lands at ~2.05 s. The timeout now clears it with room to spare, so this
  is a curiosity rather than a fault, but it costs ~6 s per poll cycle and
  caps how fresh the REST half can ever be. Worth an hour with a packet capture
  if the slow half ever needs to be faster.
- **Backpressure policy.** The frame channel drops when the consumer is behind,
  which is right for a live relay and wrong for a recorder. Revisit when
  something wants every frame.
- **A Tauri command for the LMU source.** Nothing in `pf_desk` calls this yet;
  the `lmu` binary is the only consumer. Wire it up when there is a UI for it,
  following the four-step recipe in CLAUDE.md.

## Not this phase, by instruction

WebSocket transport, auth for the relay, multi-sim abstraction behind a trait,
and any `TelemetryProvider`-shaped indirection. One concrete LMU type until the
wire protocol is settled.
