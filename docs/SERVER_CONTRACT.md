# Server contract — M1 device authorization

The desktop client (`pf_core::auth` + `pf_core::api`) is built against the OAuth
2.0 **Device Authorization Grant** (RFC 8628). This document specifies what the
`parc-ferme` web app must add so the tray app can authenticate. It is grounded
in the existing repo conventions (verified June 2026):

- Tables via `createTable` → physical names prefixed `parc-ferme_`.
- User ids are `varchar(255)` UUIDs (`users.id`).
- Access checks: reuse `assertSetupAccess(db, setupId, callerId)` from
  `src/server/api/setup-access.ts` — it already closes audit #2.
- Presigned URLs: `generatePresignedUrl(fileKey)` from `src/server/r2.ts`.
- Rate limiting: `checkRateLimit(moderateLimiter, key)` from `src/server/rate-limit.ts`.

## 1. New endpoints (App Router route handlers, no session required)

### `POST /api/device/code`
Starts a flow. Request `{ client_id: "pf-desktop", scope?: string }`.
Response (RFC 8628 §3.2):
```jsonc
{
  "device_code": "<opaque secret>",
  "user_code": "WDJB-MJHT",
  "verification_uri": "https://parcferme.cc/device",
  "verification_uri_complete": "https://parcferme.cc/device?code=WDJB-MJHT",
  "expires_in": 900,
  "interval": 5
}
```
Persists a `deviceAuthRequests` row (status `pending`, expiry now+15m).

### `POST /api/device/token`
Polled by the client. Request
`{ client_id, device_code, grant_type: "urn:ietf:params:oauth:grant-type:device_code" }`.
- **Approved** → `200 { access_token, token_type: "bearer", user: { id, name, image } }`.
  Issues the token: insert a `deviceTokens` row storing a **hash** of the token
  (never the raw value), return the raw token once.
- **Not yet** → `400 { error: "authorization_pending" }`.
- Polled too fast → `400 { error: "slow_down" }`.
- Declined → `400 { error: "access_denied" }`.
- Expired → `400 { error: "expired_token" }`.

The client already maps each of these (`pf_core::api::map_token_error`).

### Web approval page `GET /device`
Logged-in page where the user confirms `user_code` (pre-filled from
`verification_uri_complete`). On approve, flips the `deviceAuthRequests` row to
`approved` and binds it to `session.user.id`; lets them name the device.

## 2. New tables (`src/server/db/schema.ts`)

```ts
export const deviceAuthRequests = createTable("device_auth_request", (d) => ({
  id: d.varchar({ length: 255 }).primaryKey().$defaultFn(() => crypto.randomUUID()),
  deviceCode: d.varchar({ length: 255 }).notNull().unique(),
  userCode: d.varchar({ length: 16 }).notNull().unique(),
  status: d.varchar({ length: 16 }).notNull().default("pending"), // pending|approved|denied
  userId: d.varchar({ length: 255 }).references(() => users.id, { onDelete: "cascade" }),
  expiresAt: d.timestamp({ withTimezone: true }).notNull(),
  createdAt: d.timestamp({ withTimezone: true }).defaultNow().notNull(),
}));

export const deviceTokens = createTable("device_token", (d) => ({
  id: d.varchar({ length: 255 }).primaryKey().$defaultFn(() => crypto.randomUUID()),
  userId: d.varchar({ length: 255 }).notNull().references(() => users.id, { onDelete: "cascade" }),
  tokenHash: d.varchar({ length: 255 }).notNull().unique(), // sha256 of the raw token
  name: d.varchar({ length: 255 }),                          // e.g. "Finn's PC"
  lastUsedAt: d.timestamp({ withTimezone: true }),
  revokedAt: d.timestamp({ withTimezone: true }),            // null = active
  createdAt: d.timestamp({ withTimezone: true }).defaultNow().notNull(),
}));
```
Then `drizzle-kit generate` + migrate.

## 3. Bearer auth for desktop callers

Add a resolver: given `Authorization: Bearer <token>`, hash it, look up an
unrevoked `deviceTokens` row → `userId`; bump `lastUsedAt`. Wire it into the
tRPC context (`createTRPCContext`) so a device-token request produces the same
`ctx.session.user.id` a browser session would — every existing access check
then applies unchanged.

## 4. Account → Devices management (tRPC `devices` router)

`list` (id, name, lastUsedAt, createdAt) and `revoke(id)` (set `revokedAt`).
Surfaces trust + revocation; also a visible Pro-tier surface later.

## 5. M2 download endpoint (next milestone, listed for continuity)

`getDownloadUrl(versionId)` honoring the device token: resolve token → userId,
then reuse the **exact** body of `downloadSetupVersion` (look up version →
`fileKey`/`setupId`, `assertSetupAccess`, increment counters, log,
`generatePresignedUrl`). No new access logic — that is what keeps audit #2 closed
for the non-browser caller.

## Client knobs

- Base URL: `PARCFERME_API_URL` env (defaults to `https://parcferme.cc`).
- Client id: `pf-desktop`. Scope: `setups:download`.
