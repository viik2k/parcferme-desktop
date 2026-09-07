import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";

import { errorHint, toCmdError, type CmdError } from "../lib/errors";
import {
  TELEMETRY_ENDED_EVENT,
  openDash,
  openSessionsDir,
  shareSession,
  telemetryRunning,
  telemetrySessions,
  telemetryStart,
  telemetryStop,
  type Session,
} from "../lib/telemetry";

/** Per-session share state, keyed by file name. */
type ShareState =
  | { kind: "sharing" }
  | { kind: "shared"; url: string }
  | { kind: "error"; message: string };

/**
 * The free-tier telemetry client: read Le Mans Ultimate live, show it on the
 * dash, and keep every session on disk.
 *
 * Recording is the point: a session file is what Share pushes to parcferme.cc
 * (SERVER_CONTRACT §10), and "Open folder" is there for everything the site
 * isn't — replaying, keeping, or handing the .jsonl to someone directly.
 */
export function TelemetryPanel() {
  const [running, setRunning] = useState(false);
  const [record, setRecord] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CmdError | null>(null);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [shares, setShares] = useState<Record<string, ShareState>>({});

  const refresh = useCallback(async () => {
    setRunning(await telemetryRunning().catch(() => false));
    setSessions(await telemetrySessions().catch(() => []));
  }, []);

  useEffect(() => {
    void refresh();
    // The source also stops on its own when the game closes.
    const sub = listen(TELEMETRY_ENDED_EVENT, () => void refresh());
    return () => void sub.then((un) => un());
  }, [refresh]);

  async function start() {
    setBusy(true);
    setError(null);
    try {
      await telemetryStart(record);
      setRunning(true);
      await openDash();
    } catch (e) {
      setError(toCmdError(e));
    } finally {
      setBusy(false);
      void refresh();
    }
  }

  async function stop() {
    setBusy(true);
    try {
      await telemetryStop();
    } finally {
      setRunning(false);
      setBusy(false);
      // The recorder flushes per frame, so the file is already complete — but
      // its size only settles once the pump has stopped.
      window.setTimeout(() => void refresh(), 300);
    }
  }

  async function share(file: string) {
    setShares((s) => ({ ...s, [file]: { kind: "sharing" } }));
    try {
      const { url } = await shareSession(file);
      setShares((s) => ({ ...s, [file]: { kind: "shared", url } }));
    } catch (e) {
      setShares((s) => ({ ...s, [file]: { kind: "error", message: toCmdError(e).message } }));
    }
  }

  return (
    <div className="rounded-2xl bg-card p-6 ring-1 ring-border">
      <div className="flex items-baseline justify-between gap-3">
        <h2 className="text-base font-semibold">Telemetry</h2>
        <span className="text-xs text-muted">Le Mans Ultimate</span>
      </div>
      <p className="mt-1 text-sm text-muted">
        Live dash while you drive, every session saved on your own machine, and
        one click to share it.
      </p>

      <label className="mt-4 flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={record}
          disabled={running}
          onChange={(e) => setRecord(e.target.checked)}
          className="accent-primary"
        />
        Record this session to disk
      </label>

      <div className="mt-4 flex gap-2">
        <button
          onClick={running ? stop : start}
          disabled={busy}
          className="flex-1 rounded-lg bg-primary px-3 py-2 text-sm font-medium text-primary-foreground transition hover:opacity-90 disabled:opacity-50"
        >
          {busy ? "…" : running ? "Stop" : "Start telemetry"}
        </button>
        <button
          onClick={() => void openDash()}
          className="rounded-lg px-3 py-2 text-sm ring-1 ring-border transition hover:text-foreground"
        >
          Open dash
        </button>
      </div>

      {error && (
        <p className="mt-3 text-sm text-destructive">
          {error.message}
          {errorHint(error.kind) && (
            <span className="block text-xs text-destructive/80">{errorHint(error.kind)}</span>
          )}
        </p>
      )}

      <div className="mt-5 flex items-baseline justify-between">
        <h3 className="text-sm font-medium">Recorded sessions</h3>
        <button
          onClick={() => void openSessionsDir()}
          className="text-xs text-muted underline-offset-2 transition hover:text-foreground hover:underline"
        >
          Open folder
        </button>
      </div>

      {sessions.length === 0 ? (
        <p className="mt-2 text-xs text-muted">
          Nothing recorded yet. Sessions land here once you drive with recording on.
        </p>
      ) : (
        <ul className="mt-2 space-y-1.5 text-xs">
          {/* ponytail: newest ten. The list is a receipt, not a browser — the
              folder button is there for the rest. */}
          {sessions.slice(0, 10).map((s) => {
            const share_ = shares[s.file];
            return (
              <li key={s.file} className="flex flex-wrap items-center justify-between gap-2">
                <span className="truncate tabular-nums text-muted">
                  {new Date(s.startedUnix * 1000).toLocaleString()}
                </span>
                <span className="flex shrink-0 items-center gap-2">
                  <span className="tabular-nums text-muted">
                    {(s.bytes / 1e6).toFixed(1)} MB
                  </span>
                  {share_?.kind === "shared" ? (
                    <button
                      onClick={() => void openUrl(share_.url)}
                      className="text-success underline-offset-2 hover:underline"
                    >
                      View ↗
                    </button>
                  ) : (
                    <button
                      onClick={() => void share(s.file)}
                      disabled={share_?.kind === "sharing"}
                      className="rounded-md px-2 py-0.5 ring-1 ring-border transition hover:text-foreground disabled:opacity-50"
                    >
                      {share_?.kind === "sharing" ? "Sharing…" : "Share"}
                    </button>
                  )}
                </span>
                {share_?.kind === "error" && (
                  <span className="basis-full text-destructive">{share_.message}</span>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
