import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import logo from "./assets/logo.png";
import { ConnectPanel } from "./components/ConnectPanel";
import { Connected } from "./components/Connected";
import { DownloadPanel } from "./components/DownloadPanel";
import { authStatus, type DeviceUser } from "./lib/auth";
import { EQUIP_EVENT, type EquipOutcome } from "./lib/download";

function App() {
  const [linked, setLinked] = useState<boolean | null>(null);
  const [user, setUser] = useState<DeviceUser | null>(null);
  const [equip, setEquip] = useState<EquipOutcome | null>(null);

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

  // Result of an "Equip" deep link clicked on parcferme.cc (M3). The Rust side
  // does the download and emits the outcome; we surface it as a banner.
  useEffect(() => {
    const pending = listen<EquipOutcome>(EQUIP_EVENT, (e) => setEquip(e.payload));
    return () => void pending.then((unlisten) => unlisten());
  }, []);

  return (
    <main className="flex min-h-screen flex-col items-center gap-8 bg-background px-6 py-10 text-foreground">
      <header className="flex flex-col items-center text-center">
        <img src={logo} alt="Parc Fermé" className="h-12 w-auto select-none" draggable={false} />
        <p className="mt-2 text-sm text-muted">Desktop tray client</p>
        <span className="mt-3 inline-block rounded-full bg-primary/10 px-3 py-1 text-xs font-medium text-primary ring-1 ring-primary/30">
          M3 · Multi-sim downloads
        </span>
      </header>

      {equip && (
        <div className="w-full max-w-sm">
          {equip.status === "installed" ? (
            <div className="flex items-start justify-between gap-3 rounded-lg bg-success/10 px-3 py-2 text-sm text-success ring-1 ring-success/30">
              <span>
                Equipped{equip.name ? ` “${equip.name}”` : " setup"} ✓
                <span className="block text-xs text-success/80">
                  {equip.sim}
                  {equip.car ? ` · ${equip.car}` : ""}
                  {equip.track ? ` · ${equip.track}` : ""}
                </span>
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
              <span>Couldn’t equip: {equip.message}</span>
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
        {linked === null ? (
          <p className="text-center text-sm text-muted">Checking…</p>
        ) : linked ? (
          <div className="flex flex-col gap-6">
            <Connected
              user={user}
              onSignedOut={() => {
                setUser(null);
                setLinked(false);
              }}
            />
            <DownloadPanel />
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
        Close to tray · click the tray icon to reopen
      </footer>
    </main>
  );
}

export default App;
