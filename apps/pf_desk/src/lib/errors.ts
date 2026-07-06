/**
 * Structured errors crossing the IPC boundary (M4 unhappy paths).
 *
 * Rust commands reject with `commands::CmdError` `{ kind, message }`, where
 * `kind` is `pf_core::Error::kind()`. Keep the hint map in sync with that
 * enum — a new kind without a hint simply shows its message, which is fine.
 */

export interface CmdError {
  kind: string;
  message: string;
}

/** Normalize whatever a `catch` produced into a `CmdError`. */
export function toCmdError(e: unknown): CmdError {
  if (typeof e === "object" && e !== null && "message" in e) {
    const { kind, message } = e as { kind?: unknown; message: unknown };
    return {
      kind: typeof kind === "string" ? kind : "internal",
      message: String(message),
    };
  }
  return { kind: "internal", message: String(e) };
}

/**
 * A one-line recovery hint for an error kind, or null when the message
 * already says everything. Wording is user-facing.
 */
export function errorHint(kind: string): string | null {
  switch (kind) {
    case "not_linked":
      return "Connect your account, then try again.";
    case "device_revoked":
      return "Sign out here, then connect the device again.";
    case "access_denied":
      return "Ask the owner to share it with you on parcferme.cc.";
    case "setups_dir_not_found":
      return "Point ParcFerme at the right folder in Settings.";
    case "network":
      return "Check your connection and try again in a moment.";
    case "invalid_link":
      return "Copy the link from a setup page, e.g. parcferme.cc/setups/…";
    default:
      return null;
  }
}

/** Whether the Settings view is the right place to fix this error. */
export function isSettingsFixable(kind: string): boolean {
  return kind === "setups_dir_not_found";
}
