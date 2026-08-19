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
  /** Mirrors `pf_core::upload::CarSource` — how `car` was arrived at. */
  car_source: CarSource;
  /**
   * Absolute path to an iRacing garage export (`.htm`) found beside the picked
   * `.sto`, or null. The site parses setup values out of it — a `.sto` is
   * binary and yields none on its own. iRacing only; a suggestion the form
   * lets the user drop or replace.
   */
  garage_export: string | null;
}

/**
 * Where the pre-filled car name came from: read off disk (`folder`), a curated
 * alias, an exact hit on the site's list, or `matched` — the closest name the
 * matcher could find. Only `matched` is a guess, and the form says so.
 */
export type CarSource = "folder" | "alias" | "exact" | "matched";

/**
 * Mirrors `pf_core::upload::ExportStatus` — what became of the garage export.
 * Never an error: the setup uploads either way, and only its parsed values
 * depend on this.
 */
export type ExportStatus =
  | { status: "not_sent" }
  | { status: "attached" }
  | { status: "failed"; message: string };

/** Mirrors `commands::UploadedSetupDto`. */
export interface UploadedSetup {
  id: string;
  /** Absolute parcferme.cc page URL of the new setup. */
  url: string;
  export: ExportStatus;
}

/** Infer sim/car/track for a picked file (local inspection only). */
export const identifySetup = (path: string) =>
  invoke<SetupIdentity>("identify_setup", { path });

/** Mirrors `pf_core::api::SetupOptions` as returned by `setup_options`. */
export interface SetupOptions {
  cars: string[];
  tracks: string[];
  /** Valid setup types ("safe", "aggressive", …). Empty on an older server. */
  setupTypes: string[];
}

/**
 * Car/track names the site knows for a sim, for the form's suggestions.
 * Fails soft to empty lists — the fields are free text either way, since a
 * car or track new to the site may not be in the list yet.
 */
export const setupOptions = (sim: SimId) =>
  invoke<SetupOptions>("setup_options", { sim }).catch(
    () => ({ cars: [], tracks: [], setupTypes: [] }) as SetupOptions,
  );

/** Push a local setup file to parcferme.cc as the linked user. */
export const uploadSetup = (args: {
  path: string;
  sim: SimId;
  car: string;
  track?: string;
  name?: string;
  /** Subset of `SetupOptions.setupTypes`; omit to let the server default. */
  types?: string[];
  notes?: string;
  private?: boolean;
  /** iRacing garage export to attach; omit to upload the setup on its own. */
  garageExport?: string;
}) => invoke<UploadedSetup>("upload_setup", args);
