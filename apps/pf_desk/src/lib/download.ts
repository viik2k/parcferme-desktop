import { invoke } from "@tauri-apps/api/core";

/**
 * Mirrors `pf_core::download::InstallAction` — how the file landed on disk.
 * Words the success toast: fresh install, replaced, kept alongside as a
 * numbered copy, or already there byte-for-byte (idempotent re-equip).
 */
export type InstallAction =
  | "installed"
  | "replaced"
  | "kept_both"
  | "already_installed";

/** Mirrors `commands::InstalledSetupDto`. */
export interface InstalledSetup {
  path: string;
  action: InstallAction;
  /** Display name of the sim it was installed for, e.g. "iRacing". */
  sim: string;
  /** Stable sim id: "iracing" | "acc" | "lmu". */
  sim_id: string;
  car: string;
  /** Track subfolder (ACC); null for sims that don't nest by track. */
  track: string | null;
  name: string | null;
}

/** Mirrors `commands::SimFolderDto`. One per supported sim. */
export interface SimFolder {
  /** Stable id used as the override-map key: "iracing" | "acc" | "lmu". */
  id: string;
  /** Display name, e.g. "Assetto Corsa Competizione". */
  name: string;
  /** Resolved folder (override or detected default); null if unresolvable. */
  dir: string | null;
  /** Whether `dir` exists on disk. */
  found: boolean;
  /** Whether `dir` came from a Settings override. */
  overridden: boolean;
}

/**
 * Mirrors `commands::EquipOutcome` (serde-tagged on `status`) — the payload of
 * the `equip-result` event fired when a `parcferme://equip?…` deep link runs.
 */
export type EquipOutcome =
  | ({ status: "installed" } & InstalledSetup)
  | { status: "error"; kind: string; message: string };

/** Tauri event name for an equip deep-link result (matches `EQUIP_EVENT` in Rust). */
export const EQUIP_EVENT = "equip-result";
/** Tray → UI: show the Settings view (matches `OPEN_SETTINGS_EVENT` in Rust). */
export const OPEN_SETTINGS_EVENT = "open-settings";
/** Auth changed outside the UI, e.g. tray sign-out (matches `AUTH_CHANGED_EVENT`). */
export const AUTH_CHANGED_EVENT = "auth-changed";

/** Detect each sim's setups folder, applying the persisted overrides. */
export const detectSims = () => invoke<SimFolder[]>("detect_sims");

/**
 * Download + install a setup from a pasted parcferme.cc URL (or bare UUID),
 * using the persisted Settings (folder overrides + conflict policy).
 */
export const downloadSetup = (input: string) =>
  invoke<InstalledSetup>("download_setup", { input });

/**
 * ACC only lists a setup in-game when it sits in `<car>\<track>\`. If the
 * server sent no track, we still install the file (under `<car>\`) but the
 * user deserves a heads-up on where it went and why it may not show up.
 */
export function needsTrackNote(s: InstalledSetup): boolean {
  return s.sim_id === "acc" && !s.track;
}

export const TRACK_NOTE =
  "No track info came with this setup, so ACC may not list it in-game. " +
  "Move it into the matching track folder if it doesn't show up.";

/** Success-toast copy per install action (shared by banner + download panel). */
export function actionLabel(action: InstallAction): string {
  switch (action) {
    case "installed":
      return "Installed";
    case "replaced":
      return "Updated (replaced the previous file)";
    case "kept_both":
      return "Installed as a copy (kept your existing file)";
    case "already_installed":
      return "Already installed — you're up to date";
  }
}
