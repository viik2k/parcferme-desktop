import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  connectBegin,
  connectPoll,
  type DeviceFlow,
  type DeviceUser,
} from "../lib/auth";
import { errorHint, toCmdError } from "../lib/errors";

/** Ceiling for the poll interval so repeated `slow_down` can't back off forever. */
const MAX_POLL_SECS = 30;

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

  // `deadline` is the wall-clock ms at which the device code expires
  // (RFC 8628 §3.2 `expires_in`); the server's `expired` isn't the only exit.
  function schedulePoll(f: DeviceFlow, intervalSecs: number, deadline: number) {
    const expire = () => {
      setPhase("error");
      setMessage("The code expired. Please try again.");
    };
    const delayMs = Math.min(intervalSecs * 1000, Math.max(0, deadline - Date.now()));
    window.setTimeout(async () => {
      if (cancelled.current) return;
      if (Date.now() >= deadline) return expire();
      try {
        const res = await connectPoll(f.device_code);
        switch (res.status) {
          case "linked":
            onLinked(res.user);
            return;
          case "pending":
            schedulePoll(f, intervalSecs, deadline);
            return;
          case "slow_down":
            schedulePoll(f, Math.min(intervalSecs + 5, MAX_POLL_SECS), deadline);
            return;
          case "denied":
            setPhase("error");
            setMessage("Approval was declined. You can try again.");
            return;
          case "expired":
            expire();
            return;
        }
      } catch (e) {
        setPhase("error");
        setMessage(describe(e));
      }
    }, delayMs);
  }

  async function start() {
    setPhase("starting");
    setMessage(null);
    try {
      const f = await connectBegin();
      setFlow(f);
      setPhase("waiting");
      await openApproval(f);
      schedulePoll(f, f.interval_secs, Date.now() + f.expires_in_secs * 1000);
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
    <div className="px-1 py-6 text-center">
      <h2 className="text-sm font-semibold">Connect your account</h2>
      <p className="mx-auto mt-1 max-w-[16rem] text-xs text-muted">
        Link this device to equip setups straight from parcferme.cc.
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
            className="glass mt-4 w-full rounded-full px-3 py-2 text-xs font-medium text-foreground hover:bg-white/12"
          >
            Reopen approval page
          </button>
          <p className="mt-3 flex items-center justify-center gap-2 text-xs text-muted">
            <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-primary" />
            <span className="pf-dance mr-1.5" aria-hidden="true" />Waiting for approval…
          </p>
        </div>
      ) : (
        <button
          onClick={() => void start()}
          disabled={phase === "starting"}
          className="mt-5 w-full rounded-full bg-primary glow px-3 py-2 text-xs font-semibold text-primary-foreground hover:brightness-110 disabled:opacity-50"
        >
          {phase === "starting" ? (
          <>
            <span className="pf-dance mr-1.5" aria-hidden="true" />Requesting code…
          </>
        ) : (
          "Connect account"
        )}
        </button>
      )}

      {phase === "error" && message && (
        <p className="mt-4 rounded-xl bg-destructive/10 px-3 py-2 text-xs text-destructive ring-1 ring-destructive/30">
          {message}
        </p>
      )}
    </div>
  );
}
