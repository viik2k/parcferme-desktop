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
    <div>
      <p className="text-[11px] text-muted">
        Paste a setup link — it lands in the right sim folder.
      </p>

      <div className="mt-2 flex gap-2">
        <input
        value={url}
        onChange={(e) => setUrl(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && canDownload) void handleDownload();
        }}
          placeholder="https://parcferme.cc/setups/…"
          className="min-w-0 flex-1 rounded-md bg-card px-2.5 py-1.5 text-xs text-foreground ring-1 ring-border focus:outline-none focus:ring-primary"
        />

        <button
          onClick={() => void handleDownload()}
          disabled={!canDownload}
          className="shrink-0 rounded-md bg-primary px-3 py-1.5 text-xs font-semibold text-primary-foreground transition hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {phase === "working" ? (
            <span className="pf-dance" aria-hidden="true" />
          ) : (
            "Install"
          )}
        </button>
      </div>

      {phase === "done" && result && (
        <div className="mt-3 rounded-md bg-success/10 px-2.5 py-1.5 text-xs text-success ring-1 ring-success/30">
          <p className="font-medium">
            {actionLabel(result.action)}
            {result.name ? ` — “${result.name}”` : ""} ✓
          </p>
          <p className="mt-0.5 text-[10px] text-success/80">
            {result.sim}
            {result.car ? ` · ${result.car}` : ""}
            {result.track ? ` · ${result.track}` : ""}
          </p>
          <p className="mt-0.5 break-all font-mono text-[10px] text-success/80">
            {result.path}
          </p>
          {needsTrackNote(result) && (
            <p className="mt-1 text-[10px] text-success/80">{trackNote(result)}</p>
          )}
        </div>
      )}

      {phase === "error" && error && (
        <div className="mt-3 rounded-md bg-destructive/10 px-2.5 py-1.5 text-xs text-destructive ring-1 ring-destructive/30">
          <p>{error.message}</p>
          {hint && <p className="mt-1 text-[10px] text-destructive/80">{hint}</p>}
          {isSettingsFixable(error.kind) && (
            <button
              onClick={onOpenSettings}
              className="mt-1.5 rounded px-1.5 py-0.5 text-[10px] font-medium text-destructive ring-1 ring-destructive/40 transition hover:bg-destructive/10"
            >
              Open Settings
            </button>
          )}
        </div>
      )}
    </div>
  );
}
