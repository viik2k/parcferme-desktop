import { useState } from "react";
import {
  actionLabel,
  downloadSetup,
  needsTrackNote,
  trackNote,
  type InstalledSetup,
} from "../lib/download";
import { errorHint, isSettingsFixable, toCmdError, type CmdError } from "../lib/errors";

type Phase = "idle" | "working" | "done" | "error";

/**
 * Manual pull: paste a setup link, get the file in the right sim folder.
 * Folder overrides + the conflict policy come from persisted Settings (M4) —
 * the same ones the Equip deep link uses.
 */
export function DownloadPanel({ onOpenSettings }: { onOpenSettings: () => void }) {
  const [url, setUrl] = useState("");
  const [phase, setPhase] = useState<Phase>("idle");
  const [result, setResult] = useState<InstalledSetup | null>(null);
  const [error, setError] = useState<CmdError | null>(null);

  async function handleDownload() {
    setPhase("working");
    setError(null);
    setResult(null);
    try {
      setResult(await downloadSetup(url.trim()));
      setPhase("done");
    } catch (e) {
      setError(toCmdError(e));
      setPhase("error");
    }
  }

  const canDownload = url.trim().length > 0 && phase !== "working";
  const hint = error ? errorHint(error.kind) : null;

  return (
    <div className="rounded-2xl bg-card p-6 ring-1 ring-border">
      <h2 className="text-base font-semibold">Download a setup</h2>
      <p className="mt-1 text-sm text-muted">
        Paste a Parc Fermé setup link to install it straight into the right sim
        folder.
      </p>

      <input
        value={url}
        onChange={(e) => setUrl(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && canDownload) void handleDownload();
        }}
        placeholder="https://parcferme.cc/setups/…"
        className="mt-4 w-full rounded-lg bg-background px-3 py-2 text-sm text-foreground ring-1 ring-border focus:outline-none focus:ring-primary"
      />

      <button
        onClick={() => void handleDownload()}
        disabled={!canDownload}
        className="mt-3 w-full rounded-lg bg-primary px-4 py-2.5 text-sm font-semibold text-primary-foreground transition hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-50"
      >
        {phase === "working" ? "Downloading…" : "Download"}
      </button>

      {phase === "done" && result && (
        <div className="mt-4 rounded-lg bg-success/10 px-3 py-2 text-sm text-success ring-1 ring-success/30">
          <p className="font-medium">
            {actionLabel(result.action)}
            {result.name ? ` — “${result.name}”` : ""} ✓
          </p>
          <p className="mt-0.5 text-xs text-success/80">
            {result.sim}
            {result.car ? ` · ${result.car}` : ""}
            {result.track ? ` · ${result.track}` : ""}
          </p>
          <p className="mt-0.5 break-all font-mono text-xs text-success/80">
            {result.path}
          </p>
          {needsTrackNote(result) && (
            <p className="mt-1 text-xs text-success/80">{trackNote(result)}</p>
          )}
        </div>
      )}

      {phase === "error" && error && (
        <div className="mt-4 rounded-lg bg-destructive/10 px-3 py-2 text-sm text-destructive ring-1 ring-destructive/30">
          <p>{error.message}</p>
          {hint && <p className="mt-1 text-xs text-destructive/80">{hint}</p>}
          {isSettingsFixable(error.kind) && (
            <button
              onClick={onOpenSettings}
              className="mt-2 rounded-md px-2 py-1 text-xs font-medium text-destructive ring-1 ring-destructive/40 transition hover:bg-destructive/10"
            >
              Open Settings
            </button>
          )}
        </div>
      )}
    </div>
  );
}
