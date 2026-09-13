import { useCallback, useEffect, useState } from "react";

import { OrganicLoader } from "./OrganicLoader";
import { SetupList } from "./SetupList";
import { errorHint, toCmdError, type CmdError } from "../lib/errors";
import { listSetups, type Scope, type SetupSummary } from "../lib/setups";

const SCOPES: { id: Scope; label: string }[] = [
  { id: "mine", label: "My setups" },
  { id: "team", label: "Team vault" },
];

/**
 * Browse and install without leaving the app: the shelf the website shows.
 *
 * ponytail: flat list, no search/filter/pagination — the server caps it at 100.
 * Add a filter when a real vault outgrows a scroll.
 */
export function SetupsPanel() {
  const [scope, setScope] = useState<Scope>("mine");
  const [items, setItems] = useState<SetupSummary[] | null>(null);
  const [error, setError] = useState<CmdError | null>(null);

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

  return (
    <div>
      <div className="flex items-center gap-3">
        {SCOPES.map((s) => (
          <button
            key={s.id}
            onClick={() => setScope(s.id)}
            className={`rounded-full px-3 py-1 text-[11px] font-medium ${
              scope === s.id
                ? "bg-primary text-primary-foreground glow"
                : "text-muted ring-1 ring-border hover:bg-white/8 hover:text-foreground"
            }`}
          >
            {s.label}
          </button>
        ))}
        <button
          onClick={() => void load()}
          className="ml-auto rounded-full px-2.5 py-1 text-[11px] text-muted ring-1 ring-border hover:bg-white/8 hover:text-foreground"
        >
          Refresh
        </button>
      </div>

      {error ? (
        <div className="mt-3 rounded-xl bg-destructive/10 px-3 py-2 text-xs text-destructive ring-1 ring-destructive/30">
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
        <SetupList items={items} />
      )}
    </div>
  );
}
