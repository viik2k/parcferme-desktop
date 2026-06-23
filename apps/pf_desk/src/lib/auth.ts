import { invoke } from "@tauri-apps/api/core";

/** Mirrors `pf_core::api::DeviceUser`. */
export interface DeviceUser {
  id: string;
  name: string | null;
  image: string | null;
}

export interface AuthStatus {
  linked: boolean;
}

/** Mirrors `commands::DeviceFlowDto`. */
export interface DeviceFlow {
  user_code: string;
  verification_uri: string;
  verification_uri_complete: string | null;
  device_code: string;
  interval_secs: number;
  expires_in_secs: number;
}

/** Mirrors `commands::PollDto` (serde-tagged on `status`). */
export type PollResult =
  | { status: "linked"; user: DeviceUser | null }
  | { status: "pending" }
  | { status: "slow_down" }
  | { status: "denied" }
  | { status: "expired" };

export const authStatus = () => invoke<AuthStatus>("auth_status");

export const connectBegin = () => invoke<DeviceFlow>("connect_begin");

export const connectPoll = (deviceCode: string) =>
  invoke<PollResult>("connect_poll", { deviceCode });

export const signOut = () => invoke<void>("sign_out");
