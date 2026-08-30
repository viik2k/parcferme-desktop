import { useCallback, useEffect, useState } from "react";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";
import { detectSims, type SimFolder } from "../lib/download";
import { toCmdError } from "../lib/errors";
import {
  getAutostart,
  getSettings,
  openLogsDir,
  saveSettings,
  setAutostart,
  type ConflictPolicy,
  type Settings,
} from "../lib/settings";

/**
 * M4 Settings: per-sim folder overrides (native folder picker), the
 * conflict/overwrite policy, launch-at-startup, and the support log folder.
 * Every change saves immediately — there is no Save button to forget.
 */
export function SettingsPanel({ onBack }: { onBack: () => void }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [sims, setSims] = useState<SimFolder[] | null>(null);
  const [autostart, setAutostartState] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setSettings(await getSettings());
      setSims(await detectSims());
      setError(null);
    } catch (e) {
      setError(toCmdError(e).message);
    }
    try {
      setAutostartState(await getAutostart());
    } catch {
      // Autostart is cosmetic; a plugin hiccup shouldn't block the panel.
      setAutostartState(null);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function persist(next: Settings) {
    setSettings(next);
    try {
      await saveSettings(next);
      setError(null);
      // Overrides may change which folders resolve/exist — re-detect.
      setSims(await detectSims());
    } catch (e) {
      setError(toCmdError(e).message);
    }
  }

  async function browse(sim: SimFolder) {
    const picked = await pickFolder({
      directory: true,
      defaultPath: sim.dir ?? undefined,
      title: `Choose the ${sim.name} setups folder`,
    });
    if (typeof picked === "string" && settings) {
      await persist({
        ...settings,
        simFolders: { ...settings.simFolders, [sim.id]: picked },
      });
    }
  }

  async function resetOverride(simId: string) {
    if (!settings) return;
    const simFolders = { ...settings.simFolders };
    delete simFolders[simId];
    await persist({ ...settings, simFolders });
  }

  async function setPolicy(conflictPolicy: ConflictPolicy) {
    if (!settings) return;
    await persist({ ...settings, conflictPolicy });
  }

  async function toggleAutostart() {
    if (autostart === null) return;
    const next = !autostart;
    setAutostartState(next);
    try {
      await setAutostart(next);
    } catch (e) {
      setAutostartState(!next);
      setError(toCmdError(e).message);
    }
  }

  return (
    <div className="rounded-2xl bg-card p-6 ring-1 ring-border">
      <div className="flex items-center justify-between">
        <h2 className="text-base font-semibold">Settings</h2>
        <button
          onClick={onBack}
          className="rounded-md px-2 py-1 text-xs text-muted ring-1 ring-border transition hover:text-foreground"
        >
          ← Back
        </button>
      </div>

      {/* Per-sim setups folders */}
      <div className="mt-5 space-y-2">
        <p className="text-xs font-medium text-muted">Setup folders</p>
        {sims === null ? (
          <p className="text-xs text-muted">
            <span className="pf-dance mr-1.5" aria-hidden="true" />Detecting…
          </p>
        ) : (
          sims.map((s) => (
            <div
              key={s.id}
              className="rounded-lg bg-background/50 px-3 py-2 text-xs ring-1 ring-border"
            >
              <div className="flex items-center justify-between gap-2">
                <span className="font-medium text-foreground">{s.name}</span>
                <span
                  className={s.found ? "text-success" : "text-destructive/80"}
                  title={
                    s.found
                      ? "Folder found — downloads will land here"
                      : "Folder not found on this PC — pick it below"
                  }
                >
                  {s.found ? "Found ✓" : "Not found"}
                </span>
              </div>
              {s.dir && (
                <p className="mt-0.5 break-all font-mono text-muted">{s.dir}</p>
              )}
              <div className="mt-2 flex items-center gap-2">
                <button
                  onClick={() => void browse(s)}
                  className="rounded-md px-2 py-1 text-xs text-muted ring-1 ring-border transition hover:text-foreground"
                >
                  Browse…
                </button>
                {s.overridden && (
                  <button
                    onClick={() => void resetOverride(s.id)}
                    className="rounded-md px-2 py-1 text-xs text-muted ring-1 ring-border transition hover:text-foreground"
                    title="Forget the override and auto-detect again"
                  >
                    Use detected
                  </button>
                )}
                {s.overridden && (
                  <span className="ml-auto rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary ring-1 ring-primary/30">
                    override
                  </span>
                )}
              </div>
            </div>
          ))
        )}
      </div>

      {/* Conflict policy */}
      <div className="mt-5 space-y-2">
        <p className="text-xs font-medium text-muted">
          If a setup file already exists
        </p>
        {(
          [
            {
              value: "keep_both" as const,
              label: "Keep both",
              detail: "Saves the new one as “name (2)” — never touches your file.",
            },
            {
              value: "overwrite" as const,
              label: "Overwrite",
              detail: "Replaces the existing file with the downloaded one.",
            },
          ]
        ).map((opt) => {
          const selected = settings?.conflictPolicy === opt.value;
          return (
            <button
              key={opt.value}
              onClick={() => void setPolicy(opt.value)}
              disabled={!settings}
              className={`w-full rounded-lg px-3 py-2 text-left text-xs ring-1 transition ${
                selected
                  ? "bg-primary/10 ring-primary/40"
                  : "bg-background/50 ring-border hover:ring-primary/30"
              }`}
            >
              <span
                className={`font-medium ${selected ? "text-primary" : "text-foreground"}`}
              >
                {opt.label}
                {selected ? " ✓" : ""}
              </span>
              <span className="mt-0.5 block text-muted">{opt.detail}</span>
            </button>
          );
        })}
        <p className="text-[10px] text-muted/80">
          Re-downloading an identical file never creates a copy — equips are
          safe to repeat.
        </p>
      </div>

      {/* Startup */}
      <div className="mt-5">
        <p className="text-xs font-medium text-muted">Startup</p>
        <label className="mt-2 flex cursor-pointer items-center justify-between rounded-lg bg-background/50 px-3 py-2 text-xs ring-1 ring-border">
          <span className="text-foreground">
            Launch at startup
            <span className="block text-muted">
              Starts quietly in the tray, ready for Equip clicks.
            </span>
          </span>
          <input
            type="checkbox"
            checked={autostart ?? false}
            disabled={autostart === null}
            onChange={() => void toggleAutostart()}
            className="h-4 w-4 accent-primary"
          />
        </label>
      </div>

      {/* Support */}
      <div className="mt-5">
        <p className="text-xs font-medium text-muted">Support</p>
        <button
          onClick={() => void openLogsDir().catch(() => undefined)}
          className="mt-2 w-full rounded-lg px-3 py-2 text-xs text-muted ring-1 ring-border transition hover:text-foreground"
        >
          Open logs folder
        </button>
        <p className="mt-1 text-[10px] text-muted/80">
          Logs contain no tokens or personal data — safe to attach to a bug
          report.
        </p>
      </div>

      {error && (
        <p className="mt-4 rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive ring-1 ring-destructive/30">
          {error}
        </p>
      )}
    </div>
  );
}
