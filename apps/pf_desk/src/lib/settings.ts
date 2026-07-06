import { invoke } from "@tauri-apps/api/core";

/**
 * Mirrors `pf_core::settings::Settings` (camelCase on the wire; see the
 * `wire_shape_is_stable` test in settings.rs). Persisted by Rust at
 * `%APPDATA%\cc.parcferme.desktop\settings.json`.
 */
export interface Settings {
  /** Per-sim folder overrides keyed by sim id ("iracing" | "acc" | "lmu"). */
  simFolders: Record<string, string>;
  conflictPolicy: ConflictPolicy;
}

/** What a download does when the file already exists with different bytes. */
export type ConflictPolicy = "keep_both" | "overwrite";

export const getSettings = () => invoke<Settings>("get_settings");

export const saveSettings = (settings: Settings) =>
  invoke<void>("save_settings", { settings });

/** Launch-at-startup state (Windows Run key, managed by the autostart plugin). */
export const getAutostart = () => invoke<boolean>("get_autostart");

export const setAutostart = (enabled: boolean) =>
  invoke<void>("set_autostart", { enabled });

/** Reveal the app's log folder in Explorer (support flow). */
export const openLogsDir = () => invoke<void>("open_logs_dir");
