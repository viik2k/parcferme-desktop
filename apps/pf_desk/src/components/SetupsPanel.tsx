import { useCallback, useEffect, useState } from "react";

import { downloadSetup } from "../lib/download";
import { OrganicLoader } from "./OrganicLoader";
import { errorHint, toCmdError, type CmdError } from "../lib/errors";
import { listSetups, type Scope, type SetupSummary } from "../lib/setups";
import { simLabel } from "../lib/sims";

const SCOPES: { id: Scope; label: string }[] = [
  { id: "mine", label: "My setups" },
  { id: "team", label: "Team vault" },
];

/**
 * Browse and install without leaving the app: the shelf the website shows,
 * with Install wired to the same download path a pasted link or an Equip deep
 * link uses (so folder overrides and the conflict policy apply unchanged).
 *
 * ponytail: flat list, no search/filter/pagination — the server caps it at 100.
 * Add a filter when a real vault outgrows a scroll.
 */
export function SetupsPanel() {
  const [scope, setScope] = useState<Scope>("mine");
  const [items, setItems] = useState<SetupSummary[] | null>(null);
  const [error, setError] = useState<CmdError | null>(null);
  // id → installing | installed | its failure message.
  const [status, setStatus] = useState<Record<string, string>>({});

  const load = useCallback(async () => {
    setItems(null);
    setError(null);
    try {
      setItems(await listSetups(scope));
    } catch (e) {
      setError(toCmdError(e));
    }
  }, [scope]);

  useEffect(() => {
    void load();
  }, [load]);

  async function install(setup: SetupSummary) {
    setStatus((s) => ({ ...s, [setup.id]: "installing" }));
    try {
      const result = await downloadSetup(setup.id);
      setStatus((s) => ({
        ...s,
        [setup.id]:
          result.action === "already_installed" ? "Already installed ✓" : "Installed ✓",
      }));
    } catch (e) {
      setStatus((s) => ({ ...s, [setup.id]: toCmdError(e).message }));
    }
  }

  return (
    <div>
      <div className="flex items-center gap-3">
        {SCOPES.map((s) => (
          <button
            key={s.id}
            onClick={() => setScope(s.id)}
            className={`text-xs transition ${
              scope === s.id
                ? "font-medium text-primary"
                : "text-muted hover:text-foreground"
            }`}
          >
            {s.label}
          </button>
        ))}
        <button
          onClick={() => void load()}
          className="ml-auto text-xs text-muted transition hover:text-foreground"
        >
          Refresh
        </button>
      </div>

      {error ? (
        <div className="mt-3 rounded-md bg-destructive/10 px-2.5 py-1.5 text-xs text-destructive ring-1 ring-destructive/30">
          <p>{error.message}</p>
          {errorHint(error.kind) && (
            <p className="mt-1 text-[10px] text-destructive/80">
              {errorHint(error.kind)}
            </p>
          )}
        </div>
      ) : items === null ? (
        <div className="mt-3 flex justify-center py-6 text-muted">
          <OrganicLoader size={56} label="Loading setups" />
        </div>
      ) : items.length === 0 ? (
        <p className="mt-4 text-xs text-muted">
          {scope === "team"
            ? "Nothing in your team vault yet."
            : "You haven’t published any setups yet."}
        </p>
      ) : (
        <ul className="mt-1 divide-y divide-border">
          {items.map((s) => {
            const state = status[s.id];
            return (
              <li
                key={s.id}
                className="flex items-center justify-between gap-3 py-2"
              >
                <div className="min-w-0">
                  <p className="truncate text-xs">{s.name}</p>
                  <p className="truncate text-[10px] text-muted">
                    {simLabel(s.sim)}
                    {s.car ? ` · ${s.car}` : ""}
                    {s.track ? ` · ${s.track}` : ""}
                  </p>
                  {state && state !== "installing" && (
                    <p
                      className={`mt-0.5 text-[10px] ${
                        state.endsWith("✓") ? "text-success" : "text-destructive"
                      }`}
                    >
                      {state}
                    </p>
                  )}
                </div>
                <button
                  onClick={() => void install(s)}
                  disabled={state === "installing"}
                  className="shrink-0 rounded-md bg-primary px-2.5 py-1 text-[11px] font-semibold text-primary-foreground transition hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-50"
                >
                  {state === "installing" ? (
                    <>
                      <span className="pf-dance mr-1.5" aria-hidden="true" />Installing…
                    </>
                  ) : (
                    "Install"
                  )}
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
