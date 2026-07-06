import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  connectBegin,
  connectPoll,
  type DeviceFlow,
  type DeviceUser,
} from "../lib/auth";
import { errorHint, toCmdError } from "../lib/errors";

type Phase = "idle" | "starting" | "waiting" | "error";

/** Message + recovery hint on one line (the panel has a single error slot). */
function describe(e: unknown): string {
  const err = toCmdError(e);
  const hint = errorHint(err.kind);
  return hint ? `${err.message} — ${hint}` : err.message;
}

export function ConnectPanel({
  onLinked,
}: {
  onLinked: (user: DeviceUser | null) => void;
}) {
  const [phase, setPhase] = useState<Phase>("idle");
  const [flow, setFlow] = useState<DeviceFlow | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const cancelled = useRef(false);

  // Reset on setup, not just at ref creation: React StrictMode mounts twice
  // (setup → cleanup → setup) in dev, and the cleanup flips this to `true`. If
  // we only seeded `false` at creation, the ref would stay `true` after remount
  // and every poll tick would bail at the guard below — the flow would never
  // recognise approval. Re-seeding in setup keeps the guard correct in both
  // StrictMode's double-mount and a real unmount.
  useEffect(() => {
    cancelled.current = false;
    return () => {
      cancelled.current = true;
    };
  }, []);

  function schedulePoll(f: DeviceFlow, intervalSecs: number) {
    window.setTimeout(async () => {
      if (cancelled.current) return;
      try {
        const res = await connectPoll(f.device_code);
        switch (res.status) {
          case "linked":
            onLinked(res.user);
            return;
          case "pending":
            schedulePoll(f, intervalSecs);
            return;
          case "slow_down":
            schedulePoll(f, intervalSecs + 5);
            return;
          case "denied":
            setPhase("error");
            setMessage("Approval was declined. You can try again.");
            return;
          case "expired":
            setPhase("error");
            setMessage("The code expired. Please try again.");
            return;
        }
      } catch (e) {
        setPhase("error");
        setMessage(describe(e));
      }
    }, intervalSecs * 1000);
  }

  async function start() {
    setPhase("starting");
    setMessage(null);
    try {
      const f = await connectBegin();
      setFlow(f);
      setPhase("waiting");
      await openApproval(f);
      schedulePoll(f, f.interval_secs);
    } catch (e) {
      setPhase("error");
      setMessage(describe(e));
    }
  }

  async function openApproval(f: DeviceFlow) {
    const url = f.verification_uri_complete ?? f.verification_uri;
    try {
      await openUrl(url);
    } catch {
      // Non-fatal: the user can open the URL shown below manually.
    }
  }

  return (
    <div className="rounded-2xl bg-card p-6 ring-1 ring-border">
      <h2 className="text-base font-semibold">Connect your account</h2>
      <p className="mt-1 text-sm text-muted">
        Link this device to your Parc Fermé account to equip setups straight from
        the website.
      </p>

      {phase === "waiting" && flow ? (
        <div className="mt-5">
          <p className="text-xs text-muted">Enter this code in your browser:</p>
          <p className="mt-1 select-all text-center font-mono text-3xl font-bold tracking-[0.3em] text-primary">
            {flow.user_code}
          </p>
          <p className="mt-3 break-all text-center text-xs text-muted">
            {flow.verification_uri}
          </p>
          <button
            onClick={() => void openApproval(flow)}
            className="mt-4 w-full rounded-lg bg-card px-4 py-2 text-sm font-medium text-foreground ring-1 ring-border transition hover:bg-border/40"
          >
            Reopen approval page
          </button>
          <p className="mt-3 flex items-center justify-center gap-2 text-xs text-muted">
            <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-primary" />
            Waiting for approval…
          </p>
        </div>
      ) : (
        <button
          onClick={() => void start()}
          disabled={phase === "starting"}
          className="mt-5 w-full rounded-lg bg-primary px-4 py-2.5 text-sm font-semibold text-primary-foreground transition hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {phase === "starting" ? "Requesting code…" : "Connect account"}
        </button>
      )}

      {phase === "error" && message && (
        <p className="mt-4 rounded-lg bg-destructive/10 px-3 py-2 text-sm text-destructive ring-1 ring-destructive/30">
          {message}
        </p>
      )}
    </div>
  );
}
