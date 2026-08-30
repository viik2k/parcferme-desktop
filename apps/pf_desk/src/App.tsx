import { useCallback, useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import logo from "./assets/logo.png";
import { ConnectPanel } from "./components/ConnectPanel";
import { Connected } from "./components/Connected";
import { DownloadPanel } from "./components/DownloadPanel";
import { OrganicLoader } from "./components/OrganicLoader";
import { SettingsPanel } from "./components/SettingsPanel";
import { SetupsPanel } from "./components/SetupsPanel";
import { UpdateBanner } from "./components/UpdateBanner";
import { UploadPanel } from "./components/UploadPanel";
import { authStatus, type DeviceUser } from "./lib/auth";
import {
  actionLabel,
  AUTH_CHANGED_EVENT,
  EQUIP_EVENT,
  needsTrackNote,
  OPEN_SETTINGS_EVENT,
  trackNote,
  type EquipOutcome,
} from "./lib/download";
import { errorHint, isSettingsFixable } from "./lib/errors";

type View = "home" | "settings";

function App() {
  const [linked, setLinked] = useState<boolean | null>(null);
  const [user, setUser] = useState<DeviceUser | null>(null);
  const [equip, setEquip] = useState<EquipOutcome | null>(null);
  const [view, setView] = useState<View>("home");
  const [version, setVersion] = useState("");

  useEffect(() => {
    getVersion().then(setVersion).catch(() => {});
  }, []);

  const refresh = useCallback(async () => {
    try {
      const status = await authStatus();
      setLinked(status.linked);
      setUser(status.user);
    } catch {
      setLinked(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Tray + deep-link events from the Rust shell: an equip finished, the tray's
  // Settings item was clicked, or auth changed outside the UI (tray sign-out).
  useEffect(() => {
    const subs = [
      listen<EquipOutcome>(EQUIP_EVENT, (e) => setEquip(e.payload)),
      listen(OPEN_SETTINGS_EVENT, () => setView("settings")),
      listen(AUTH_CHANGED_EVENT, () => void refresh()),
    ];
    return () => {
      for (const sub of subs) void sub.then((unlisten) => unlisten());
    };
  }, [refresh]);

  // Success banners retire themselves; errors — and successes carrying an
  // actionable note (ACC missing track) — stay until dismissed.
  useEffect(() => {
    if (equip?.status !== "installed" || needsTrackNote(equip)) return;
    const timer = window.setTimeout(() => setEquip(null), 8000);
    return () => window.clearTimeout(timer);
  }, [equip]);

  return (
    <main className="flex min-h-screen flex-col items-center gap-8 bg-background px-6 py-10 text-foreground">
      <header className="relative w-full max-w-sm">
        <div className="flex flex-col items-center text-center">
          <img src={logo} alt="Parc Fermé" className="h-12 w-auto select-none" draggable={false} />
          <p className="mt-2 text-sm text-muted">
            Click Equip on parcferme.cc — the setup lands in your sim.
          </p>
        </div>
        <button
          onClick={() => setView(view === "settings" ? "home" : "settings")}
          aria-label="Settings"
          title="Settings"
          className={`absolute right-0 top-0 rounded-lg p-2 text-lg ring-1 ring-border transition hover:text-foreground ${
            view === "settings" ? "text-primary" : "text-muted"
          }`}
        >
          ⚙
        </button>
      </header>

      <UpdateBanner />

      {equip && (
        <div className="w-full max-w-sm">
          {equip.status === "installed" ? (
            <div className="flex items-start justify-between gap-3 rounded-lg bg-success/10 px-3 py-2 text-sm text-success ring-1 ring-success/30">
              <span>
                {actionLabel(equip.action)}
                {equip.name ? ` — “${equip.name}”` : ""} ✓
                <span className="block text-xs text-success/80">
                  {equip.sim}
                  {equip.car ? ` · ${equip.car}` : ""}
                  {equip.track ? ` · ${equip.track}` : ""}
                </span>
                {needsTrackNote(equip) && (
                  <span className="mt-1 block text-xs text-success/80">
                    {trackNote(equip)}
                  </span>
                )}
              </span>
              <button
                onClick={() => setEquip(null)}
                aria-label="Dismiss"
                className="shrink-0 text-success/70 transition hover:text-success"
              >
                ✕
              </button>
            </div>
          ) : (
            <div className="flex items-start justify-between gap-3 rounded-lg bg-destructive/10 px-3 py-2 text-sm text-destructive ring-1 ring-destructive/30">
              <span>
                Couldn’t equip: {equip.message}
                {errorHint(equip.kind) && (
                  <span className="block text-xs text-destructive/80">
                    {errorHint(equip.kind)}
                  </span>
                )}
                {isSettingsFixable(equip.kind) && (
                  <button
                    onClick={() => {
                      setView("settings");
                      setEquip(null);
                    }}
                    className="mt-1.5 block rounded-md px-2 py-1 text-xs font-medium text-destructive ring-1 ring-destructive/40 transition hover:bg-destructive/10"
                  >
                    Open Settings
                  </button>
                )}
              </span>
              <button
                onClick={() => setEquip(null)}
                aria-label="Dismiss"
                className="shrink-0 text-destructive/70 transition hover:text-destructive"
              >
                ✕
              </button>
            </div>
          )}
        </div>
      )}

      <section className="w-full max-w-sm">
        {view === "settings" ? (
          <SettingsPanel onBack={() => setView("home")} />
        ) : linked === null ? (
          <div className="flex flex-col items-center gap-2 py-8 text-muted">
            <OrganicLoader size={64} label="Checking sign-in" />
            <p className="text-sm">Checking…</p>
          </div>
        ) : linked ? (
          <div className="flex flex-col gap-6">
            <Connected
              user={user}
              onSignedOut={() => {
                setUser(null);
                setLinked(false);
              }}
            />
            <SetupsPanel />
            <DownloadPanel onOpenSettings={() => setView("settings")} />
            <UploadPanel />
          </div>
        ) : (
          <ConnectPanel
            onLinked={(u) => {
              setUser(u);
              setLinked(true);
            }}
          />
        )}
      </section>

      <footer className="mt-auto text-xs text-muted/70">
        {version ? `v${version} · ` : ""}closes to the tray — click the tray
        icon to reopen
      </footer>
    </main>
  );
}

export default App;
