import { invoke } from "@tauri-apps/api/core";

/** Mirrors `commands::InstalledSetupDto`. */
export interface InstalledSetup {
  path: string;
  /** Display name of the sim it was installed for, e.g. "iRacing". */
  sim: string;
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

/** Per-sim folder overrides, keyed by `SimFolder.id`. */
export type SimOverrides = Record<string, string>;

/**
 * Mirrors `commands::EquipOutcome` (serde-tagged on `status`) — the payload of
 * the `equip-result` event fired when a `parcferme://equip?…` deep link runs.
 */
export type EquipOutcome =
  | ({ status: "installed" } & InstalledSetup)
  | { status: "error"; message: string };

/** Tauri event name for an equip deep-link result (matches `EQUIP_EVENT` in Rust). */
export const EQUIP_EVENT = "equip-result";

/** Detect each sim's setups folder, applying any per-sim overrides. */
export const detectSims = (overrides?: SimOverrides) =>
  invoke<SimFolder[]>("detect_sims", { overrides: overrides ?? null });

/** Download + install a setup from a pasted parcferme.cc URL (or bare UUID). */
export const downloadSetup = (input: string, overrides?: SimOverrides) =>
  invoke<InstalledSetup>("download_setup", {
    input,
    overrides: overrides ?? null,
  });
