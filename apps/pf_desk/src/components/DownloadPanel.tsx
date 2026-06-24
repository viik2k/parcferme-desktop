import { useCallback, useEffect, useState } from "react";
import {
  downloadSetup,
  setupsDir,
  type InstalledSetup,
} from "../lib/download";

type Phase = "idle" | "working" | "done" | "error";

export function DownloadPanel() {
  const [dir, setDir] = useState<string | null>(null);
  const [dirError, setDirError] = useState<string | null>(null);
  const [override, setOverride] = useState("");
  const [url, setUrl] = useState("");
  const [phase, setPhase] = useState<Phase>("idle");
  const [result, setResult] = useState<InstalledSetup | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const refreshDir = useCallback(async (ov?: string) => {
    try {
      setDir(await setupsDir(ov));
      setDirError(null);
    } catch (e) {
      setDir(null);
      setDirError(String(e));
    }
  }, []);

  useEffect(() => {
    void refreshDir();
  }, [refreshDir]);

  async function handleDownload() {
    setPhase("working");
    setMessage(null);
    setResult(null);
    try {
      const installed = await downloadSetup(url.trim(), override.trim() || undefined);
      setResult(installed);
      setPhase("done");
    } catch (e) {
      setMessage(String(e));
      setPhase("error");
    }
  }

  const canDownload = url.trim().length > 0 && phase !== "working";

  return (
    <div className="rounded-2xl bg-card p-6 ring-1 ring-border">
      <h2 className="text-base font-semibold">Download a setup</h2>
      <p className="mt-1 text-sm text-muted">
        Paste a Parc Fermé setup link to install it straight into your iRacing
        folder.
      </p>

      {/* Setups folder status */}
      <div className="mt-4 rounded-lg bg-background/50 px-3 py-2 text-xs ring-1 ring-border">
        <p className="text-muted">Setups folder</p>
        {dir ? (
          <p className="mt-0.5 break-all font-mono text-foreground">{dir}</p>
        ) : (
          <p className="mt-0.5 text-destructive">
            {dirError ?? "Locating…"}
          </p>
        )}
        {dirError && (
          <div className="mt-2 flex gap-2">
            <input
              value={override}
              onChange={(e) => setOverride(e.target.value)}
              placeholder={String.raw`C:\Users\you\Documents\iRacing\setups`}
              className="min-w-0 flex-1 rounded-md bg-background px-2 py-1 font-mono text-xs text-foreground ring-1 ring-border focus:outline-none focus:ring-primary"
            />
            <button
              onClick={() => void refreshDir(override.trim() || undefined)}
              className="shrink-0 rounded-md px-2 py-1 text-xs text-muted ring-1 ring-border transition hover:text-foreground"
            >
              Use
            </button>
          </div>
        )}
      </div>

      <input
        value={url}
        onChange={(e) => setUrl(e.target.value)}
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
            Installed{result.name ? ` “${result.name}”` : ""} ✓
          </p>
          <p className="mt-0.5 break-all font-mono text-xs text-success/80">
            {result.path}
          </p>
        </div>
      )}

      {phase === "error" && message && (
        <p className="mt-4 rounded-lg bg-destructive/10 px-3 py-2 text-sm text-destructive ring-1 ring-destructive/30">
          {message}
        </p>
      )}
    </div>
  );
}
