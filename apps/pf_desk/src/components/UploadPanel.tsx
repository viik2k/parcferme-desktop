import { useEffect, useState } from "react";
import { open as pickFile } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { errorHint, toCmdError, type CmdError } from "../lib/errors";
import { SIM_OPTIONS, simLayout, type SimId } from "../lib/sims";
import {
  identifySetup,
  setupOptions,
  uploadSetup,
  type SetupOptions,
  type UploadedSetup,
} from "../lib/upload";

type Phase = "idle" | "working" | "done" | "error";

/**
 * Push a local setup to parcferme.cc (M5). Picking a file pre-fills sim, car
 * and track when it sits inside a sim's setups folder; every field stays
 * editable because inference is a guess, not an authority.
 */
export function UploadPanel() {
  const [path, setPath] = useState<string | null>(null);
  const [filename, setFilename] = useState("");
  const [sim, setSim] = useState<SimId>("iracing");
  const [car, setCar] = useState("");
  const [track, setTrack] = useState("");
  const [name, setName] = useState("");
  const [phase, setPhase] = useState<Phase>("idle");
  const [result, setResult] = useState<UploadedSetup | null>(null);
  const [error, setError] = useState<CmdError | null>(null);
  const [options, setOptions] = useState<SetupOptions>({
    cars: [],
    tracks: [],
  });

  // Suggestions for the car/track fields, so the user picks the site's
  // spelling instead of guessing it. Cached per sim in the core; empty when
  // offline or unpaired.
  useEffect(() => {
    let live = true;
    void setupOptions(sim).then((o) => live && setOptions(o));
    return () => {
      live = false;
    };
  }, [sim]);

  async function handlePick() {
    const picked = await pickFile({
      title: "Choose a setup file",
      filters: [{ name: "Setup files", extensions: ["sto", "json", "svm"] }],
    });
    if (typeof picked !== "string") return;
    setPath(picked);
    setResult(null);
    setError(null);
    setPhase("idle");
    try {
      const id = await identifySetup(picked);
      setFilename(id.filename);
      if (id.sim) setSim(id.sim);
      setCar(id.car ?? "");
      setTrack(id.track ?? "");
    } catch {
      // Inference failing must not block a manual upload.
      setFilename(picked.split(/[\\/]/).pop() ?? picked);
    }
  }

  async function handleUpload() {
    if (!path) return;
    setPhase("working");
    setError(null);
    setResult(null);
    try {
      setResult(
        await uploadSetup({
          path,
          sim,
          car: car.trim(),
          // Only ever send what the form is actually showing — switching sims
          // after picking a file would otherwise submit a hidden field.
          track: (showTrack && track.trim()) || undefined,
          name: name.trim() || undefined,
        }),
      );
      setPhase("done");
    } catch (e) {
      setError(toCmdError(e));
      setPhase("error");
    }
  }

  // The car names the setup for the site whatever the sim's folders look like;
  // a track is *required* only where the sim files by track (ACC, LMU), since
  // without it the server can't place the setup. iRacing still gets an optional
  // track field — the server parks a trackless upload on "Unknown Track", which
  // is worth avoiding when the user already knows where the lap was set.
  const layout = simLayout(sim);
  const showTrack = layout.track || sim === "iracing";
  const canUpload =
    !!path &&
    car.trim().length > 0 &&
    (!layout.track || track.trim().length > 0) &&
    phase !== "working";
  const hint = error ? errorHint(error.kind) : null;
  const fieldClass =
    "mt-1 w-full rounded-lg bg-background px-3 py-2 text-sm text-foreground ring-1 ring-border focus:outline-none focus:ring-primary";

  return (
    <div className="rounded-2xl bg-card p-6 ring-1 ring-border">
      <h2 className="text-base font-semibold">Push a setup</h2>
      <p className="mt-1 text-sm text-muted">
        Share a setup from your sim folder to parcferme.cc. Picking a file from
        the setups folder fills in the car and track for you.
        {options.cars.length > 0 && " Start typing to pick from the site's list."}
      </p>

      <button
        onClick={() => void handlePick()}
        className="mt-4 w-full rounded-lg px-3 py-2 text-sm text-muted ring-1 ring-border transition hover:text-foreground"
      >
        {filename ? filename : "Choose a setup file…"}
      </button>

      {path && (
        <div className="mt-3 space-y-3 text-xs">
          <label className="block">
            <span className="font-medium text-muted">Sim</span>
            <select
              value={sim}
              onChange={(e) => setSim(e.target.value as SimId)}
              className={fieldClass}
            >
              {SIM_OPTIONS.map((o) => (
                <option key={o.id} value={o.id}>
                  {o.label}
                </option>
              ))}
            </select>
          </label>

          <label className="block">
            {/* Not "car folder": what goes to the site is its car name, which
                the alias table fills in for folder ids that abbreviate it. */}
            <span className="font-medium text-muted">
              Car{car ? "" : " — required"}
            </span>
            <input
              value={car}
              onChange={(e) => setCar(e.target.value)}
              list="pf-known-cars"
              placeholder="e.g. Ferrari 296 GT3"
              className={fieldClass}
            />
            {/* Suggestions only — a car the site has just added won't be
                listed yet, so arbitrary text stays valid. */}
            <datalist id="pf-known-cars">
              {options.cars.map((c) => (
                <option key={c} value={c} />
              ))}
            </datalist>
          </label>

          {showTrack && (
            <label className="block">
              {/* Not "track folder": the server resolves site names and folder
                  ids alike, and the suggestions are site names. */}
              <span className="font-medium text-muted">
                {layout.track
                  ? `Track${track ? "" : " — required"}`
                  : "Track on the site (optional)"}
              </span>
              <input
                value={track}
                onChange={(e) => setTrack(e.target.value)}
                list="pf-known-tracks"
                placeholder={
                  sim === "lmu"
                    ? "e.g. Fuji"
                    : sim === "acc"
                      ? "e.g. spa"
                      : "e.g. Watkins Glen"
                }
                className={fieldClass}
              />
              <datalist id="pf-known-tracks">
                {options.tracks.map((t) => (
                  <option key={t} value={t} />
                ))}
              </datalist>
            </label>
          )}

          <label className="block">
            <span className="font-medium text-muted">
              Display name (optional)
            </span>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Quali — Spa 2.18,9"
              className={fieldClass}
            />
          </label>

          <button
            onClick={() => void handleUpload()}
            disabled={!canUpload}
            className="w-full rounded-lg bg-primary px-4 py-2.5 text-sm font-semibold text-primary-foreground transition hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {phase === "working" ? "Uploading…" : "Upload to Parc Fermé"}
          </button>
        </div>
      )}

      {phase === "done" && result && (
        <div className="mt-4 rounded-lg bg-success/10 px-3 py-2 text-sm text-success ring-1 ring-success/30">
          <p className="font-medium">Uploaded ✓</p>
          <button
            onClick={() => void openUrl(result.url)}
            className="mt-1 break-all text-left text-xs text-success/80 underline transition hover:text-success"
          >
            {result.url}
          </button>
        </div>
      )}

      {phase === "error" && error && (
        <div className="mt-4 rounded-lg bg-destructive/10 px-3 py-2 text-sm text-destructive ring-1 ring-destructive/30">
          <p>{error.message}</p>
          {hint && <p className="mt-1 text-xs text-destructive/80">{hint}</p>}
        </div>
      )}
    </div>
  );
}
