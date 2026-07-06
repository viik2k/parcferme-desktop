import { useState } from "react";
import { signOut, type DeviceUser } from "../lib/auth";
import { toCmdError } from "../lib/errors";

export function Connected({
  user,
  onSignedOut,
}: {
  user: DeviceUser | null;
  onSignedOut: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSignOut() {
    setBusy(true);
    setError(null);
    try {
      await signOut();
      onSignedOut();
    } catch (e) {
      setError(toCmdError(e).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="rounded-2xl bg-card p-6 ring-1 ring-border">
      <div className="flex items-center gap-3">
        {user?.image ? (
          <img src={user.image} alt="" className="h-10 w-10 rounded-full" />
        ) : (
          <span className="flex h-10 w-10 items-center justify-center rounded-full bg-primary/15 text-sm font-semibold text-primary">
            {(user?.name ?? "PF").slice(0, 2).toUpperCase()}
          </span>
        )}
        <div>
          <p className="text-sm font-semibold text-success">Device linked</p>
          <p className="text-xs text-muted">
            {user?.name ? `Signed in as ${user.name}` : "This device is connected"}
          </p>
        </div>
      </div>

      <p className="mt-4 text-sm text-muted">
        You can close this window — it stays in your tray, ready to equip setups.
      </p>

      <button
        onClick={() => void handleSignOut()}
        disabled={busy}
        className="mt-5 w-full rounded-lg bg-card px-4 py-2 text-sm font-medium text-foreground ring-1 ring-border transition hover:bg-border/40 disabled:opacity-50"
      >
        Sign out
      </button>

      {error && (
        <p className="mt-3 rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive ring-1 ring-destructive/30">
          {error}
        </p>
      )}
    </div>
  );
}
