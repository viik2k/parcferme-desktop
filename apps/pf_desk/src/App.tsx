import { useCallback, useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import logo from "./assets/logo.png";
import { ConnectPanel } from "./components/ConnectPanel";
import { DownloadPanel } from "./components/DownloadPanel";
import { OrganicLoader } from "./components/OrganicLoader";
import { SettingsPanel } from "./components/SettingsPanel";
import { SetupsPanel } from "./components/SetupsPanel";
import { SyncPanel } from "./components/SyncPanel";
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
import { syncStatus, type SyncStatus } from "./lib/sync";

const TABS = [
  { id: "setups", label: "Setups" },
  { id: "install", label: "Install" },
  { id: "push", label: "Push" },
  { id: "sync", label: "Sync" },
] as const;

type Tab = (typeof TABS)[number]["id"];

/**
 * The companion window: a status bar, four tabs, and a body — no stacked
 * cards. The real work happens in the tray and the sync engine, so the window
 * stays small and says what the automation is doing rather than presenting
 * itself as a dashboard.
 */
function App() {
  const [linked, setLinked] = useState<boolean | null>(null);
  const [user, setUser] = useState<DeviceUser | null>(null);
  const [equip, setEquip] = useState<EquipOutcome | null>(null);
  const [tab, setTab] = useState<Tab>("setups");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [version, setVersion] = useState("");

  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch(() => {});
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
      listen(OPEN_SETTINGS_EVENT, () => setSettingsOpen(true)),
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
    <div className="flex h-screen flex-col bg-background text-foreground">
      <header className="flex h-11 shrink-0 items-center gap-2 border-b border-border px-3">
        <img
          src={logo}
          alt="Parc Fermé"
          className="h-4 w-auto select-none"
          draggable={false}
        />
        <SyncBadge linked={linked} />
        <button
          onClick={() => setSettingsOpen((open) => !open)}
          aria-label="Settings"
          title="Settings"
          className={`ml-auto rounded-md px-1.5 py-1 text-sm transition hover:text-foreground ${
            settingsOpen ? "text-primary" : "text-muted"
          }`}
        >
          ⚙
        </button>
        {linked &&
          (user?.image ? (
            <img
              src={user.image}
              alt=""
              title={user.name ?? "Signed in"}
              className="h-6 w-6 rounded-full"
            />
          ) : (
            <span
              title={user?.name ?? "Signed in"}
              className="flex h-6 w-6 items-center justify-center rounded-full bg-primary/15 text-[10px] font-semibold text-primary"
            >
              {(user?.name ?? "PF").slice(0, 2).toUpperCase()}
            </span>
          ))}
      </header>

      {linked && !settingsOpen && (
        <nav className="flex shrink-0 gap-4 border-b border-border px-3">
          {TABS.map((t) => (
            <button
              key={t.id}
              onClick={() => setTab(t.id)}
              className={`-mb-px border-b py-2 text-xs font-medium transition ${
                tab === t.id
                  ? "border-primary text-foreground"
                  : "border-transparent text-muted hover:text-foreground"
              }`}
            >
              {t.label}
            </button>
          ))}
        </nav>
      )}

      <main className="flex-1 overflow-y-auto px-3 py-3">
        <UpdateBanner />

        {equip && (
          <EquipBanner
            equip={equip}
            onDismiss={() => setEquip(null)}
            onOpenSettings={() => {
              setSettingsOpen(true);
              setEquip(null);
            }}
          />
        )}

        {settingsOpen ? (
          <SettingsPanel
            linked={!!linked}
            onBack={() => setSettingsOpen(false)}
            onSignedOut={() => {
              setUser(null);
              setLinked(false);
              setSettingsOpen(false);
            }}
          />
        ) : linked === null ? (
          <div className="flex flex-col items-center gap-2 py-10 text-muted">
            <OrganicLoader size={56} label="Checking sign-in" />
            <p className="text-xs">Checking…</p>
          </div>
        ) : !linked ? (
          <ConnectPanel
            onLinked={(u) => {
              setUser(u);
              setLinked(true);
            }}
          />
        ) : tab === "setups" ? (
          <SetupsPanel />
        ) : tab === "install" ? (
          <DownloadPanel onOpenSettings={() => setSettingsOpen(true)} />
        ) : tab === "push" ? (
          <UploadPanel />
        ) : (
          <SyncPanel />
        )}
      </main>

      <footer className="flex h-7 shrink-0 items-center justify-between border-t border-border px-3 text-[10px] text-muted/70">
        <span>{version ? `v${version}` : ""}</span>
        <span>closes to the tray</span>
      </footer>
    </div>
  );
}

/**
 * What the automation is doing, in the one line the bar has room for: the sync
 * engine when it has something to say, otherwise the link state.
 *
 * ponytail: polls on a timer rather than an event — the status is two cheap
 * reads and nothing else needs a channel back from Rust.
 */
function SyncBadge({ linked }: { linked: boolean | null }) {
  const [sync, setSync] = useState<SyncStatus | null>(null);

  useEffect(() => {
    const tick = () =>
      syncStatus()
        .then(setSync)
        .catch(() => setSync(null));
    void tick();
    const timer = window.setInterval(tick, 5000);
    return () => window.clearInterval(timer);
  }, []);

  const queued = sync?.pending.length ?? 0;
  const [dot, text] =
    linked === false
      ? ["bg-muted", "Not signed in"]
      : sync?.recording
        ? ["bg-success animate-pulse", "Recording"]
        : queued > 0
          ? ["bg-primary", `${queued} to upload`]
          : sync?.enabled
            ? ["bg-success", "Sync on"]
            : ["bg-muted", "Idle"];

  return (
    <span className="flex items-center gap-1.5 text-[11px] text-muted">
      <span className={`h-1.5 w-1.5 rounded-full ${dot}`} />
      {text}
    </span>
  );
}

/** One-line result of an equip deep link, with its recovery affordance. */
function EquipBanner({
  equip,
  onDismiss,
  onOpenSettings,
}: {
  equip: EquipOutcome;
  onDismiss: () => void;
  onOpenSettings: () => void;
}) {
  const ok = equip.status === "installed";
  const tone = ok
    ? "bg-success/10 text-success ring-success/30"
    : "bg-destructive/10 text-destructive ring-destructive/30";

  return (
    <div
      className={`mb-3 flex items-start justify-between gap-2 rounded-md px-2.5 py-1.5 text-xs ring-1 ${tone}`}
    >
      <span className="min-w-0">
        {ok ? (
          <>
            {actionLabel(equip.action)}
            {equip.name ? ` — “${equip.name}”` : ""} ✓
            <span className="block truncate text-[10px] opacity-80">
              {equip.sim}
              {equip.car ? ` · ${equip.car}` : ""}
              {equip.track ? ` · ${equip.track}` : ""}
            </span>
            {needsTrackNote(equip) && (
              <span className="block text-[10px] opacity-80">
                {trackNote(equip)}
              </span>
            )}
          </>
        ) : (
          <>
            Couldn’t equip: {equip.message}
            {errorHint(equip.kind) && (
              <span className="block text-[10px] opacity-80">
                {errorHint(equip.kind)}
              </span>
            )}
            {isSettingsFixable(equip.kind) && (
              <button
                onClick={onOpenSettings}
                className="mt-1 rounded px-1.5 py-0.5 text-[10px] font-medium ring-1 ring-destructive/40 transition hover:bg-destructive/10"
              >
                Open Settings
              </button>
            )}
          </>
        )}
      </span>
      <button
        onClick={onDismiss}
        aria-label="Dismiss"
        className="shrink-0 opacity-70 hover:opacity-100"
      >
        ✕
      </button>
    </div>
  );
}

export default App;
