import { invoke } from "@tauri-apps/api/core";

/**
 * Which shelf to list: the user's own setups, their teams' vaults, or the
 * public feed (newest first, with the setup of the week flagged).
 */
export type Scope = "mine" | "team" | "browse";

/** Mirrors `pf_core::api::SetupSummary`. `car`/`track` are display names. */
export interface SetupSummary {
  /** Public setup UUID — pass straight to `downloadSetup` to install it. */
  id: string;
  name: string;
  /** "iracing" | "acc" | "lmu"; null on a row the server didn't tag. */
  sim: string | null;
  car: string;
  track: string | null;
  /** ISO 8601, or null on a server that predates the field. */
  updatedAt: string | null;
  /** Setup of the week — only ever set on `browse` rows. */
  featured: boolean;
}

/** List the setups this device may install (SERVER_CONTRACT §9). */
export const listSetups = (scope: Scope) =>
  invoke<SetupSummary[]>("list_setups", { scope });
