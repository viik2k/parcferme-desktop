import { useCallback, useEffect, useState } from "react";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { signOut } from "../lib/auth";
import { detectSims, type SimFolder } from "../lib/download";
import { toCmdError } from "../lib/errors";
import {
  getAutostart,
  getSettings,
  openLogsDir,
  saveSettings,
  setAutostart,
  type Settings,
} from "../lib/settings";

/** Section heading — the only chrome a section gets now that the cards are gone. */
function Label({ children }: { children: React.ReactNode }) {
  return (
    <p className="mb-1.5 text-[10px] font-medium uppercase tracking-wide text-muted/70">
      {children}
    </p>
  );
}

/** A labelled switch row: the shape every toggle in here shares. */
function Toggle({
  title,
  detail,
  checked,
  disabled,
  onChange,
}: {
  title: string;
  detail: string;
  checked: boolean;
  disabled?: boolean;
  onChange: () => void;
}) {
  return (
    <label className="flex cursor-pointer items-start justify-between gap-3 py-1.5 text-xs">
      <span className="text-foreground">
        {title}
        <span className="mt-0.5 block text-[10px] text-muted">{detail}</span>
      </span>
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={onChange}
        className="mt-0.5 h-3.5 w-3.5 shrink-0 accent-primary"
      />
    </label>
  );
}

/**
 * Settings: sim folders, the conflict policy, the sync engine, startup, and
 * the account. Every change saves immediately — there is no Save button to
 * forget. Sections are divided by hairlines rather than cards, so the whole
 * panel fits the companion window without scrolling much.
 */
export function SettingsPanel({
  linked,
  onBack,
  onSignedOut,
}: {
  linked: boolean;
  onBack: () => void;
  onSignedOut: () => void;
}) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [sims, setSims] = useState<SimFolder[] | null>(null);
  const [autostart, setAutostartState] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
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

  async function handleSignOut() {
    setBusy(true);
    try {
      await signOut();
      onSignedOut();
    } catch (e) {
      setError(toCmdError(e).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="divide-y divide-border">
      <div className="flex items-center justify-between pb-2">
        <h2 className="text-xs font-semibold">Settings</h2>
        <button
          onClick={onBack}
          className="text-xs text-muted transition hover:text-foreground"
        >
          Done
        </button>
      </div>

      {/* Per-sim setups folders */}
      <div className="py-3">
        <Label>Setup folders</Label>
        {sims === null ? (
          <p className="text-xs text-muted">
            <span className="pf-dance mr-1.5" aria-hidden="true" />
            Detecting…
          </p>
        ) : (
          <ul className="divide-y divide-border/60">
            {sims.map((s) => (
              <li key={s.id} className="py-1.5 text-xs">
                <div className="flex items-center gap-2">
                  <span className="font-medium text-foreground">{s.name}</span>
                  {s.overridden && (
                    <span className="text-[10px] text-primary">override</span>
                  )}
                  <span
                    className={`ml-auto text-[10px] ${
                      s.found ? "text-success" : "text-destructive/80"
                    }`}
                    title={
                      s.found
                        ? "Folder found — downloads land here"
                        : "Folder not found on this PC — pick it below"
                    }
                  >
                    {s.found ? "Found ✓" : "Not found"}
                  </span>
                  <button
                    onClick={() => void browse(s)}
                    className="text-[10px] text-muted underline-offset-2 transition hover:text-foreground hover:underline"
                  >
                    Browse…
                  </button>
                  {s.overridden && (
                    <button
                      onClick={() => void resetOverride(s.id)}
                      title="Forget the override and auto-detect again"
                      className="text-[10px] text-muted underline-offset-2 transition hover:text-foreground hover:underline"
                    >
                      Reset
                    </button>
                  )}
                </div>
                {s.dir && (
                  <p className="truncate font-mono text-[10px] text-muted/80" title={s.dir}>
                    {s.dir}
                  </p>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>

      {/* Conflict policy */}
      <div className="py-3">
        <Label>If a setup file already exists</Label>
        <div className="flex gap-1 rounded-md bg-card p-0.5 ring-1 ring-border">
          {(
            [
              { value: "keep_both" as const, label: "Keep both" },
              { value: "overwrite" as const, label: "Overwrite" },
            ]
          ).map((opt) => (
            <button
              key={opt.value}
              onClick={() =>
                settings && void persist({ ...settings, conflictPolicy: opt.value })
              }
              disabled={!settings}
              className={`flex-1 rounded px-2 py-1 text-[11px] font-medium transition ${
                settings?.conflictPolicy === opt.value
                  ? "bg-primary text-primary-foreground"
                  : "text-muted hover:text-foreground"
              }`}
            >
              {opt.label}
            </button>
          ))}
        </div>
        <p className="mt-1 text-[10px] text-muted/80">
          {settings?.conflictPolicy === "overwrite"
            ? "Replaces the existing file with the downloaded one."
            : "Saves the new one as “name (2)” — never touches your file."}
        </p>
      </div>

      {/* Startup */}
      <div className="py-3">
        <Label>Startup</Label>
        <Toggle
          title="Launch at startup"
          detail="Starts quietly in the tray, ready for Equip clicks."
          checked={autostart ?? false}
          disabled={autostart === null}
          onChange={() => void toggleAutostart()}
        />
      </div>

      {/* Account + support */}
      <div className="py-3">
        <Label>Account</Label>
        <div className="flex gap-2">
          <button
            onClick={() => void openLogsDir().catch(() => undefined)}
            className="flex-1 rounded-md px-2 py-1.5 text-[11px] text-muted ring-1 ring-border transition hover:text-foreground"
            title="Logs contain no tokens or personal data — safe to attach to a bug report"
          >
            Open logs
          </button>
          {linked && (
            <button
              onClick={() => void handleSignOut()}
              disabled={busy}
              className="flex-1 rounded-md px-2 py-1.5 text-[11px] text-muted ring-1 ring-border transition hover:text-destructive disabled:opacity-50"
            >
              Sign out
            </button>
          )}
        </div>
      </div>

      {/* Legal — the site's policy covers this app (its section on the desktop
          application describes exactly what it reads and sends). */}
      <div className="flex items-center gap-3 pt-3 text-[10px] text-muted/70">
        <button
          onClick={() => void openUrl("https://parcferme.cc/privacy").catch(() => undefined)}
          className="underline-offset-2 transition hover:text-foreground hover:underline"
        >
          Privacy Policy
        </button>
        <span aria-hidden="true">·</span>
        <button
          onClick={() => void openUrl("https://parcferme.cc/tos").catch(() => undefined)}
          className="underline-offset-2 transition hover:text-foreground hover:underline"
        >
          Terms of Service
        </button>
      </div>

      {error && (
        <p className="mt-3 rounded-md bg-destructive/10 px-2.5 py-1.5 text-xs text-destructive ring-1 ring-destructive/30">
          {error}
        </p>
      )}
    </div>
  );
}
