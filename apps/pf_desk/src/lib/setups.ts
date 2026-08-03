import { invoke } from "@tauri-apps/api/core";

/** Which shelf to list: the user's own setups, or their teams' vaults. */
export type Scope = "mine" | "team";

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
}

/** List the setups this device may install (SERVER_CONTRACT §9). */
export const listSetups = (scope: Scope) =>
  invoke<SetupSummary[]>("list_setups", { scope });
