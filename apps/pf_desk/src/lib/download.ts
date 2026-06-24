import { invoke } from "@tauri-apps/api/core";

/** Mirrors `commands::InstalledSetupDto`. */
export interface InstalledSetup {
  path: string;
  car: string;
  name: string | null;
}

/** Resolve + validate the setups folder (optional Settings override). */
export const setupsDir = (overrideDir?: string) =>
  invoke<string>("setups_dir", { overrideDir: overrideDir ?? null });

/** Download + install a setup from a pasted parcferme.cc URL (or bare UUID). */
export const downloadSetup = (input: string, overrideDir?: string) =>
  invoke<InstalledSetup>("download_setup", {
    input,
    overrideDir: overrideDir ?? null,
  });
