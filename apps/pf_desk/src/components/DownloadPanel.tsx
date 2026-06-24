import { useCallback, useEffect, useState } from "react";
import {
  detectSims,
  downloadSetup,
  type InstalledSetup,
  type SimFolder,
  type SimOverrides,
} from "../lib/download";

type Phase = "idle" | "working" | "done" | "error";

export function DownloadPanel() {
  const [sims, setSims] = useState<SimFolder[] | null>(null);
  // Applied overrides (drive detection + download) vs. in-progress text inputs.
  const [overrides, setOverrides] = useState<SimOverrides>({});
  const [drafts, setDrafts] = useState<SimOverrides>({});
  const [url, setUrl] = useState("");
  const [phase, setPhase] = useState<Phase>("idle");
  const [result, setResult] = useState<InstalledSetup | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const refreshSims = useCallback(async (ov: SimOverrides) => {
    try {
      setSims(await detectSims(Object.keys(ov).length ? ov : undefined));
    } catch {
      setSims([]);
    }
  }, []);

  useEffect(() => {
    void refreshSims(overrides);
  }, [refreshSims, overrides]);

  function applyOverride(id: string) {
    const draft = (drafts[id] ?? "").trim();
    setOverrides((prev) => {
      const next = { ...prev };
      if (draft) next[id] = draft;
      else delete next[id];
      return next;
    });
  }

  async function handleDownload() {
    setPhase("working");
    setMessage(null);
    setResult(null);
    try {
      const installed = await downloadSetup(
        url.trim(),
        Object.keys(overrides).length ? overrides : undefined,
      );
      setResult(installed);
      setPhase("done");
      // A freshly created car/track folder may now exist — re-detect.
      void refreshSims(overrides);
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
        Paste a Parc Fermé setup link to install it straight into the right sim
        folder.
      </p>

      {/* Per-sim setups folders */}
      <div className="mt-4 space-y-2">
        <p className="text-xs text-muted">Setups folders</p>
        {sims === null ? (
          <p className="text-xs text-muted">Detecting…</p>
        ) : (
          sims.map((s) => (
            <div
              key={s.id}
              className="rounded-lg bg-background/50 px-3 py-2 text-xs ring-1 ring-border"
            >
              <div className="flex items-center justify-between gap-2">
                <span className="font-medium text-foreground">{s.name}</span>
                <span
                  className={s.found ? "text-success" : "text-muted/70"}
                  title={s.found ? "Folder found" : "Folder not found on this PC"}
                >
                  {s.found ? "Found ✓" : "Not found"}
                </span>
              </div>
              {s.dir && (
                <p className="mt-0.5 break-all font-mono text-muted">{s.dir}</p>
              )}
              {!s.found && (
                <div className="mt-2 flex gap-2">
                  <input
                    value={drafts[s.id] ?? ""}
                    onChange={(e) =>
                      setDrafts((d) => ({ ...d, [s.id]: e.target.value }))
                    }
                    placeholder={s.dir ?? "Setups folder path"}
                    className="min-w-0 flex-1 rounded-md bg-background px-2 py-1 font-mono text-xs text-foreground ring-1 ring-border focus:outline-none focus:ring-primary"
                  />
                  <button
                    onClick={() => applyOverride(s.id)}
                    className="shrink-0 rounded-md px-2 py-1 text-xs text-muted ring-1 ring-border transition hover:text-foreground"
                  >
                    Use
                  </button>
                </div>
              )}
            </div>
          ))
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
          <p className="mt-0.5 text-xs text-success/80">
            {result.sim}
            {result.car ? ` · ${result.car}` : ""}
            {result.track ? ` · ${result.track}` : ""}
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
