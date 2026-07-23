import { useEffect, useState } from "react";
import { checkForUpdate, relaunch, type AvailableUpdate } from "../lib/updater";

type State =
  | { kind: "hidden" }
  | { kind: "available"; update: AvailableUpdate }
  | { kind: "downloading"; version: string; percent: number }
  | { kind: "ready"; version: string }
  | { kind: "error"; message: string };

/**
 * Checks for a newer signed release on mount and, when one exists, offers a
 * one-click download + restart. A failed check (offline, unsigned dev build,
 * no endpoint) stays silent — updating is never allowed to block the app.
 */
export function UpdateBanner() {
  const [state, setState] = useState<State>({ kind: "hidden" });

  useEffect(() => {
    let cancelled = false;
    checkForUpdate()
      .then((update) => {
        if (!cancelled && update) setState({ kind: "available", update });
      })
      .catch(() => {
        /* offline / unsigned build / no manifest — nothing to offer */
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function install(update: AvailableUpdate) {
    setState({ kind: "downloading", version: update.version, percent: 0 });
    try {
      await update.download((percent) =>
        setState({ kind: "downloading", version: update.version, percent }),
      );
      setState({ kind: "ready", version: update.version });
    } catch (e) {
      setState({
        kind: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  }

  if (state.kind === "hidden") return null;

  return (
    <div className="w-full max-w-sm">
      <div className="flex items-center justify-between gap-3 rounded-lg bg-primary/10 px-3 py-2 text-sm text-primary ring-1 ring-primary/30">
        {state.kind === "available" && (
          <>
            <span>
              Update available — <span className="font-medium">v{state.update.version}</span>
            </span>
            <button
              onClick={() => install(state.update)}
              className="shrink-0 rounded-md px-2 py-1 text-xs font-medium ring-1 ring-primary/40 transition hover:bg-primary/10"
            >
              Update
            </button>
          </>
        )}

        {state.kind === "downloading" && (
          <span>
            Downloading v{state.version}… {state.percent}%
          </span>
        )}

        {state.kind === "ready" && (
          <>
            <span>
              v{state.version} ready to install
            </span>
            <button
              onClick={() => void relaunch()}
              className="shrink-0 rounded-md px-2 py-1 text-xs font-medium ring-1 ring-primary/40 transition hover:bg-primary/10"
            >
              Restart
            </button>
          </>
        )}

        {state.kind === "error" && (
          <span className="text-destructive">Update failed: {state.message}</span>
        )}
      </div>
    </div>
  );
}
