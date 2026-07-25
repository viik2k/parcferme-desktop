// Thin wrapper over the Tauri updater/process plugins. The app polls the signed
// latest.json on GitHub Releases (tauri.conf.json → plugins.updater.endpoints),
// downloads the newer installer, applies it, then relaunches. Kept in lib/ like
// the other invoke() wrappers so components never import the plugins directly.
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type AvailableUpdate = {
  /** Version offered by the manifest, e.g. "0.2.1". */
  version: string;
  /** Version currently running. */
  currentVersion: string;
  /** Release notes from the manifest, if any. */
  notes?: string;
  /**
   * Download the new installer and apply it. Reports 0–100 progress; resolves
   * once the update is staged — the caller then relaunches to finish.
   */
  download: (onProgress?: (percent: number) => void) => Promise<void>;
};

/**
 * Ask the configured endpoint whether a newer signed release exists. Returns
 * null when up to date. Throws only on a genuine updater failure (bad manifest,
 * signature mismatch); callers treat network/offline errors as "no update".
 */
export async function checkForUpdate(): Promise<AvailableUpdate | null> {
  const update = await check();
  if (!update) return null;

  return {
    version: update.version,
    currentVersion: update.currentVersion,
    notes: update.body,
    download: async (onProgress) => {
      let total = 0;
      let received = 0;
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? 0;
            break;
          case "Progress":
            received += event.data.chunkLength;
            if (total > 0) onProgress?.(Math.min(100, Math.round((received / total) * 100)));
            break;
          case "Finished":
            onProgress?.(100);
            break;
        }
      });
    },
  };
}

export { relaunch };
