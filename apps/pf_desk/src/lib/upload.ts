import { invoke } from "@tauri-apps/api/core";

import type { SimId } from "./sims";

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

/**
 * Car names the site knows for a sim, for the car field's suggestions.
 * Fails soft to `[]` — the field is free text either way, since a car new to
 * the site may not be in the list yet.
 */
export const listCars = (sim: SimId) =>
  invoke<string[]>("list_cars", { sim }).catch(() => [] as string[]);

/** Push a local setup file to parcferme.cc as the linked user. */
export const uploadSetup = (args: {
  path: string;
  sim: SimId;
  car: string;
  track?: string;
  name?: string;
}) => invoke<UploadedSetup>("upload_setup", args);
