import { invoke } from "@tauri-apps/api/core";

/** Stable sim ids, matching `pf_core::Sim::id()`. */
export type SimId = "iracing" | "acc" | "lmu";

export const SIM_OPTIONS: { id: SimId; label: string }[] = [
  { id: "iracing", label: "iRacing" },
  { id: "acc", label: "Assetto Corsa Competizione" },
  { id: "lmu", label: "Le Mans Ultimate" },
];

/**
 * Mirrors `pf_core::upload::SetupIdentity` — what the app inferred about a
 * picked file. Everything is a suggestion; the form lets the user correct it.
 */
export interface SetupIdentity {
  filename: string;
  sim: SimId | null;
  car: string | null;
  track: string | null;
}

/** Mirrors `commands::UploadedSetupDto`. */
export interface UploadedSetup {
  id: string;
  /** Absolute parcferme.cc page URL of the new setup. */
  url: string;
}

/** Infer sim/car/track for a picked file (local inspection only). */
export const identifySetup = (path: string) =>
  invoke<SetupIdentity>("identify_setup", { path });

/** Push a local setup file to parcferme.cc as the linked user. */
export const uploadSetup = (args: {
  path: string;
  sim: SimId;
  car: string;
  track?: string;
  name?: string;
}) => invoke<UploadedSetup>("upload_setup", args);
