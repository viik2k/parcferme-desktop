import { useState } from "react";

import { downloadSetup } from "../lib/download";
import { toCmdError } from "../lib/errors";
import { type SetupSummary } from "../lib/setups";
import { simLabel } from "../lib/sims";

/**
 * A shelf of setups with Install wired to the same download path a pasted link
 * or an Equip deep link uses (so folder overrides and the conflict policy apply
 * unchanged). Shared by the Setups tab and the Install tab's browser.
 */
export function SetupList({ items }: { items: SetupSummary[] }) {
  // id → installing | installed | its failure message.
  const [status, setStatus] = useState<Record<string, string>>({});

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
    <ul className="mt-1 divide-y divide-border">
      {items.map((s) => {
        const state = status[s.id];
        return (
          <li key={s.id} className="flex items-center justify-between gap-3 py-2">
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
              className="shrink-0 rounded-full bg-primary glow px-3 py-1 text-[11px] font-semibold text-primary-foreground hover:brightness-110 disabled:opacity-50"
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
  );
}
