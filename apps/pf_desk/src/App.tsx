import { useCallback, useEffect, useState } from "react";
import logo from "./assets/logo.png";
import { ConnectPanel } from "./components/ConnectPanel";
import { Connected } from "./components/Connected";
import { DownloadPanel } from "./components/DownloadPanel";
import { authStatus, type DeviceUser } from "./lib/auth";

function App() {
  const [linked, setLinked] = useState<boolean | null>(null);
  const [user, setUser] = useState<DeviceUser | null>(null);

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

  return (
    <main className="flex min-h-screen flex-col items-center gap-8 bg-background px-6 py-10 text-foreground">
      <header className="flex flex-col items-center text-center">
        <img src={logo} alt="Parc Fermé" className="h-12 w-auto select-none" draggable={false} />
        <p className="mt-2 text-sm text-muted">Desktop tray client</p>
        <span className="mt-3 inline-block rounded-full bg-primary/10 px-3 py-1 text-xs font-medium text-primary ring-1 ring-primary/30">
          M1 · Device auth
        </span>
      </header>

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
