import { invoke } from "@tauri-apps/api/core";

/** Event carrying one merged frame from the Rust source (10 Hz by default). */
export const FRAME_EVENT = "telemetry-frame";
/** Emitted when the source stopped on its own — the game exited. */
export const TELEMETRY_ENDED_EVENT = "telemetry-ended";

/** The REST half of a frame — the slow channel (pit menu, weather, wear). */
export interface Rest {
  errors: number;
  session_name: string | null;
  time_remaining_s: number | null;
  lap_limit: number | null;
  track_length_m: number | null;
  virtual_energy: number | null;
  tyre_life: [number, number, number, number] | null;
  brake_wear: [number, number, number, number] | null;
  /** The in-car pit menu as armed: [row, selected value]. */
  pit_menu: [string, string][];
  pit_service_s: number | null;
  pit_lane_s: number | null;
  pit_fuel_advice_l: number | null;
  air_c: number | null;
  track_c: number | null;
  rain: number | null;
  light: number | null;
  time_of_day_s: number | null;
}

/**
 * Mirrors `pf_core::lmu::Frame` — snake_case, because that struct carries no
 * serde rename and the recorded JSONL must stay byte-identical to what the UI
 * sees. Corner arrays are FL, FR, RL, RR throughout.
 */
export interface Frame {
  seq: number;
  t_ms: number;
  speed_kph: number;
  throttle: number;
  brake: number;
  steering: number;
  gear: number;
  rpm: number;
  lap: number;
  lap_time_s: number | null;
  last_lap_time_s: number | null;
  sector: number | null;
  track_pos: number | null;
  in_pit_lane: boolean;
  lap_invalidated: boolean;
  fuel_l: number;
  fuel_capacity_l: number;
  virtual_energy: number | null;
  tyre_temp_c: (number | null)[];
  tyre_life: number[];
  brake_temp_c: (number | null)[];
  g_lat: number;
  g_lon: number;
  place: number;
  place_in_class: number | null;
  car_class: string;
  gap_ahead_s: number | null;
  gap_behind_s: number | null;
  rest: Rest;
  rest_age_ms: number | null;
  rest_stale: boolean;
}

export interface TelemetryStatus {
  running: boolean;
  /** File name of the session being recorded, when recording. */
  recording: string | null;
}

/** One recorded session on disk — mirrors `pf_core::session::Session`. */
export interface Session {
  file: string;
  startedUnix: number;
  bytes: number;
}

/** Open the LMU source and start streaming frames. Idempotent. */
export const telemetryStart = (record: boolean) =>
  invoke<TelemetryStatus>("telemetry_start", { record });

export const telemetryStop = () => invoke<void>("telemetry_stop");

export const telemetryRunning = () => invoke<boolean>("telemetry_running");

export const telemetrySessions = () => invoke<Session[]>("telemetry_sessions");

export const openSessionsDir = () => invoke<void>("open_sessions_dir");

/** A shared session on the site — mirrors `pf_core::api::UploadResult`. */
export interface Shared {
  id: string;
  url: string;
}

/**
 * Share a recording on parcferme.cc (SERVER_CONTRACT §10). Slow: it scans the
 * file, gzips it, and uploads. `file` is a name from `telemetrySessions`.
 */
export const shareSession = (file: string, isPrivate = false) =>
  invoke<Shared>("share_session", { file, private: isPrivate });

/** Open the always-on-top dash window (reused if already open). */
export const openDash = () => invoke<void>("open_dash");

/** `M:SS.mmm`, the way a lap time is read. */
export function lapTime(s: number | null | undefined): string {
  if (s == null) return "—";
  const m = Math.floor(s / 60);
  const rest = s - m * 60;
  return `${m}:${rest.toFixed(3).padStart(6, "0")}`;
}

/** Gear as the driver sees it: R, N, 1…8. */
export const gearLabel = (g: number) => (g < 0 ? "R" : g === 0 ? "N" : String(g));
