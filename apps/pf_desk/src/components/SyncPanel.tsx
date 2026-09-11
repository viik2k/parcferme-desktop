import { useCallback, useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toCmdError } from "../lib/errors";
import { getSettings, saveSettings, type Settings } from "../lib/settings";
import {
  bytes,
  elapsed,
  openSessionsDir,
  syncStatus,
  when,
  type SyncStatus,
} from "../lib/sync";

/** How often to ask Rust what the engine is doing. */
const POLL_MS = 3000;

/**
 * The Sync tab: the engine's own screen. It owns the on/off switch (Settings
 * keeps the folder and download preferences), shows the stint being recorded,
 * the upload queue, and where the last session went.
 *
 * ponytail: polled, not evented — a status read is a mutex and a directory
 * listing, and an event channel would be more plumbing than the 3 s lag costs.
 */
export function SyncPanel() {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Ticks the stint clock between polls, so the seconds count up smoothly.
  const [now, setNow] = useState(() => Date.now());

  const poll = useCallback(async () => {
    try {
      setStatus(await syncStatus());
    } catch {
      setStatus(null);
    }
  }, []);

  useEffect(() => {
    void poll();
    void getSettings().then(setSettings).catch(() => {});
    const status = window.setInterval(() => void poll(), POLL_MS);
    const clock = window.setInterval(() => setNow(Date.now()), 1000);
    return () => {
      window.clearInterval(status);
      window.clearInterval(clock);
    };
  }, [poll]);

  async function toggle() {
    if (!settings) return;
    const next = { ...settings, syncEnabled: !settings.syncEnabled };
    setSettings(next);
    try {
      await saveSettings(next);
      setError(null);
      // The engine reads settings once per cycle; show the new state as soon
      // as it has picked it up rather than guessing.
      window.setTimeout(() => void poll(), 200);
    } catch (e) {
      setSettings(settings);
      setError(toCmdError(e).message);
    }
  }

  const live = status?.recording ?? null;
  const enabled = settings?.syncEnabled ?? false;
  const queue = status?.pending ?? [];
  const last = status?.lastPush ?? null;

  return (
    <div className="divide-y divide-border">
      <label className="flex cursor-pointer items-start justify-between gap-3 pb-3 text-xs">
        <span className="text-foreground">
          Record and upload my sessions
          <span className="mt-0.5 block text-[10px] text-muted">
            Every Le Mans Ultimate session is recorded and pushed to your
            private library when you leave the track.
          </span>
        </span>
        <input
          type="checkbox"
          checked={enabled}
          disabled={!settings}
          onChange={() => void toggle()}
          className="mt-0.5 h-3.5 w-3.5 shrink-0 accent-primary"
        />
      </label>

      {/* The status bar: what the engine is doing this second. */}
      <div className="py-3">
        {live ? (
          <>
            <div className="flex items-baseline justify-between gap-2">
              <span className="flex items-center gap-1.5 text-xs font-medium text-success">
                <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-success" />
                Recording
              </span>
              <span className="font-mono text-xs text-foreground">
                {elapsed(now / 1000 - live.startedUnix)}
              </span>
            </div>
            <div className="mt-1 flex items-baseline justify-between gap-2 text-[10px] text-muted">
              <span className="truncate">
                {live.car || live.track
                  ? [live.car, live.track].filter(Boolean).join(" · ")
                  : "In the menus — no session loaded yet"}
              </span>
              <span className="shrink-0 font-mono">
                {live.frames.toLocaleString()} frames
              </span>
            </div>
          </>
        ) : (
          <div className="flex items-center gap-1.5 text-xs">
            <span
              className={`h-1.5 w-1.5 rounded-full ${enabled ? "bg-success" : "bg-muted"}`}
            />
            <span className={enabled ? "text-foreground" : "text-muted"}>
              {enabled ? "Waiting for Le Mans Ultimate" : "Sync is off"}
            </span>
          </div>
        )}
        {enabled && !live && (
          <p className="mt-1 text-[10px] text-muted/80">
            Start the game with plugins enabled and recording begins on its own.
          </p>
        )}
      </div>

      {/* The queue: finished recordings that haven't gone up yet. */}
      <div className="py-3">
        <div className="mb-1.5 flex items-center justify-between gap-2">
          <p className="text-[10px] font-medium uppercase tracking-wide text-muted/70">
            Queue{queue.length > 0 ? ` (${queue.length})` : ""}
          </p>
          <button
            onClick={() => void openSessionsDir().catch(() => undefined)}
            className="rounded-full px-2.5 py-1 text-[11px] text-muted ring-1 ring-border hover:bg-white/8 hover:text-foreground"
          >
            Open folder
          </button>
        </div>
        {queue.length === 0 ? (
          <p className="text-[10px] text-muted/80">
            Nothing waiting — recordings are deleted once the site has them.
          </p>
        ) : (
          <ul className="divide-y divide-border/60">
            {queue.map((s) => (
              <li
                key={s.file}
                className="flex items-baseline justify-between gap-2 py-1 text-[10px]"
              >
                <span className="truncate text-foreground">
                  {when(s.startedUnix)}
                </span>
                <span className="shrink-0 font-mono text-muted">
                  {bytes(s.bytes)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>

      {/* Where the last one went — the proof the automation is working. */}
      {last && (
        <div className="py-3">
          <p className="mb-1.5 text-[10px] font-medium uppercase tracking-wide text-muted/70">
            Last push
          </p>
          {last.url ? (
            <button
              onClick={() => void openUrl(last.url as string).catch(() => undefined)}
              className="text-left text-[10px] text-success underline underline-offset-2 hover:brightness-125"
            >
              Uploaded {when(last.atUnix)} ✓ — view on parcferme.cc
            </button>
          ) : (
            <p className="text-[10px] text-destructive">
              {when(last.atUnix)} — {last.error}
              <span className="mt-0.5 block text-muted">
                It stays in the queue and the engine tries again.
              </span>
            </p>
          )}
        </div>
      )}

      {error && (
        <p className="mt-3 rounded-xl bg-destructive/10 px-3 py-2 text-xs text-destructive ring-1 ring-destructive/30">
          {error}
        </p>
      )}
    </div>
  );
}
