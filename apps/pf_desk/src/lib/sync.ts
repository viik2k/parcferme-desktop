import { invoke } from "@tauri-apps/api/core";

/** The session being recorded right now — mirrors `pf_core::sync::Live`. */
export interface Live {
  file: string;
  startedUnix: number;
  frames: number;
  /** The game's own names; both empty until a session loads. */
  car: string;
  track: string;
}

/** The engine's last upload attempt — mirrors `pf_core::sync::LastPush`. */
export interface LastPush {
  file: string;
  atUnix: number;
  url: string | null;
  error: string | null;
}

/** One recording on disk — mirrors `pf_core::session::Session`. */
export interface Session {
  file: string;
  startedUnix: number;
  bytes: number;
}

/** Everything the Sync tab renders — mirrors `pf_core::sync::Status`. */
export interface SyncStatus {
  enabled: boolean;
  recording: Live | null;
  /** Finished recordings waiting to be pushed, newest first. */
  pending: Session[];
  lastPush: LastPush | null;
}

export const syncStatus = () => invoke<SyncStatus>("sync_status");

/** Reveal the sessions folder in Explorer. */
export const openSessionsDir = () => invoke<void>("open_sessions_dir");

/** `M:SS` for a stint clock, `H:MM:SS` once it's been a long one. */
export function elapsed(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const parts = [Math.floor(s / 3600), Math.floor((s % 3600) / 60), s % 60];
  if (parts[0] === 0) parts.shift();
  return parts
    .map((n, i) => (i === 0 ? String(n) : String(n).padStart(2, "0")))
    .join(":");
}

/** File size the way a person reads it. */
export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/** A recording's start, in the user's own locale and time zone. */
export const when = (unix: number) =>
  new Date(unix * 1000).toLocaleString(undefined, {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
