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

### Response shape (`GET /api/device/setups/{uuid}/download`)

```jsonc
{
  "url": "<presigned R2 GET>",   // required
  "filename": "quali_spa.json",  // required; the setup file name as saved in-sim
  "sim": "acc",                  // "iracing" | "acc" | "lmu"; omit ⇒ iracing
  "car": "ferrari_488_gt3_evo",  // the sim's INTERNAL folder id (see caveat)
  "track": "spa",                // ACC only; required for the setup to list in-game
  "name": "Quali — Spa"          // display name, for the toast (optional)
}
```

#### How the client picks the destination sim (updated 2026-07-02)

A live ACC download arrived without a usable `sim` tag and was routed to
iRacing, so the client no longer trusts the tag blindly. Resolution order
(`DownloadInfo::resolved_sim`):

1. **File extension is ground truth** — `.sto` → iracing, `.json` → acc,
   `.svm` → lmu. Each format loads in exactly one supported sim, so a
   recognized extension **wins over a contradicting `sim` tag** (logged as a
   warning).
2. The `sim` tag decides only when the extension is unrecognized. Parsing is
   tolerant (case/whitespace).
3. Neither usable → iracing (the M2 single-sim behavior).

The server **should still send a correct `sim` tag** — it is the only signal
for any future sim whose file extension isn't unique — and **must** send
`track` for **ACC and LMU** setups, which the client cannot infer (without it
the file lands one folder up and the sim won't list it in-game; the app warns
the user).

The client (`pf_core`) routes the file by the resolved sim:

| sim       | setups root                                                    | layout           |
| :-------- | :------------------------------------------------------------- | :--------------- |
| `iracing` | `Documents\iRacing\setups`                                      | `<car>\`         |
| `acc`     | `Documents\Assetto Corsa Competizione\Setups`                   | `<car>\<track>\` |
| `lmu`     | `<steam>\steamapps\common\Le Mans Ultimate\UserData\player\Settings` | `<track>\`  |

> **`car`/`track` must be the sim's internal folder ids, not display names.**
> Each sim lists a setup only when it sits in the exact folder it expects:
> iRacing keys by the car's internal folder (e.g. `ferrari296gt3`), ACC by its
> car model + track folders (e.g. `ferrari_488_gt3_evo\spa`). `cars.name` is a
> display name and won't match — the server must map `cars.simRefId` (or a new
> per-sim folder column) to the real folder id before returning it. Until then
> files land in a human-named folder and may not appear in-sim. (Carried over
> from the M2 iRacing caveat; now applies per sim.)
>
> **LMU is the exception** (verified against a live install 2026-07-25): it
> keeps rFactor 2's layout, filing setups by **track only** — there is no car
> folder in the path — and it stores `UserData` inside the **game install**
> (a Steam library), not under Documents. So for `lmu`, `track` is what places
> the file and `car` is metadata the client ignores when writing. The client
> finds the install by walking Steam's `libraryfolders.vdf`; a non-Steam or
> undetectable install falls back to the Settings folder override.

## 6. M3 "Equip" deep link (web → desktop)

The handshake is a **custom URL scheme** the desktop registers: `parcferme://`.
The website's **Equip** button opens:

```
parcferme://equip?setup=<setupId>
```

- `setup` — the setup's public UUID (same id as `/setups/<uuid>` and the §5
  download endpoint). Aliases also accepted by the client: `setupId`, `id`,
  `versionId`.
- `token` (optional, alias `sig`) — reserved for a short-lived signed payload if
  you later want link-level validation. **Not required for v1:** the client does
  not yet send it onward, and the download is already authorized by the device
  token + the §5 access check, so the link alone grants nothing.

How to emit it from the web: a plain anchor/redirect to the `parcferme://…` URL
(e.g. `window.location.href = "parcferme://equip?setup=" + id`). No new server
endpoint is needed — clicking it hands the setup id to the running tray app,
which then calls the **existing** §5 download endpoint as this device.

Desktop behaviour (`pf_core::deeplink::parse` → `download::install_from_equip_link`):
parse + UUID-validate → reveal window → download via §5 → emit an `equip-result`
event the UI shows as a toast. Not-signed-in / no-access / network failures come
back as a clear error in the same toast. Cold start (app launched by the link)
and warm start (already running) are both handled.

## 7. M5 push endpoint — upload a setup from the desktop

The reverse of §5: the desktop pushes a local setup file and the server creates
a setup owned by the device token's linked user.

### `POST /api/device/setups/upload`

- `Authorization: Bearer <device token>` — same resolver as §3.
- `Content-Type: application/octet-stream`, body = the raw setup file bytes
  (client refuses files over 2 MB; enforce a server-side limit too).
- Metadata rides in the **query string** (no multipart):

| param      | required | meaning                                                      |
| :--------- | :------- | :----------------------------------------------------------- |
| `filename` | yes      | file name as on disk, e.g. `quali_spa.json`                  |
| `sim`      | yes      | `"iracing"` \| `"acc"` \| `"lmu"`                             |
| `car`      | yes      | the sim's **internal car folder id** as found on disk         |
| `track`    | ACC, LMU | internal track folder id (see the per-sim layouts in §5)      |
| `name`     | no       | display name typed by the user                               |
| `types`    | no       | comma-separated setup types from §7a's `setupTypes`          |
| `notes`    | no       | free text description, max 5000 chars                        |
| `private`  | no       | `"true"` makes the setup owner-only; anything else is public |

`types` is a **multi-select** (parc-ferme#77): `setups.tags` is an array, so
`types=aggressive,qualifying` is one setup carrying both. Omitting it entirely
leaves the tagging to the server (currently `["safe"]`), which is what clients
predating this param do. An unknown value is a `422` naming the valid list —
the server never silently drops one, since a dropped type is invisible to the
user until they open the site.

`track` is **required for ACC and LMU** — the client refuses the upload without
one rather than sending metadata the server has to reject. For LMU the client
can only infer the track (its layout has no car folder), so `car` there is
typed by the user and won't necessarily match a folder id. iRacing uploads may
omit `track` (the server parks them on "Unknown Track"); the form offers it as
an optional field so the owner doesn't have to fix it on the web afterwards.

#### `car` may arrive as a display name (client behaviour, added 2026-07-25)

The contract is unchanged — every field is still untrusted free text — but the
client no longer always sends a raw folder id. The server resolves `car` by
normalized equality, which cannot bridge a folder id that **abbreviates** its
car: iRacing's `mercedesw13` never equals `mercedesamgw13eperformance`, so those
uploads 422'd and the user had to guess the site's exact wording.

`pf_core::car_aliases` holds a curated, **exceptions-only** map of folder id →
exact Parc Fermé car name, applied both when the form pre-fills from disk and
once more just before the request. Consequences for the server:

- `car` is either an on-disk folder id (as before) or a car name copied from
  the §7a suggestions. Normalized equality already accepts both, so nothing changes.
- Cars whose folder id already normalize-matches are deliberately absent from
  the map and still arrive as folder ids.
- Anything the map doesn't know passes through untouched, so the
  `422 unknown car "<value>"` message stays the user's guide — keep returning it
  verbatim, and keep it JSON.

The map's right-hand side is validated against the live car list by an ignored
test (`aliases_match_the_live_site`). **Renaming a seeded car breaks those
aliases** — run that test after any `cars` reseed.

The form reads §7a's options endpoint for its car/track suggestions, failing
soft to free text when the site (or the device token) is unavailable.

#### `car` may also be matched against §7a's list (client behaviour, added 2026-08-01)

The map above is compiled into the binary, so every newly seeded car whose
folder id abbreviates it needed a client release before it could be uploaded.
`pf_core::car_match` removes that dependency: when the form pre-fills from disk
and the folder id isn't a curated exception, the client matches it against
§7a's `cars` list (exact → containment → subsequence → bounded edit distance)
and pre-fills the winning name. Still nothing new for the server, but worth
knowing:

- The matcher **only runs on pre-fill**, never on submit. What the form shows
  is what ships, so the user always sees and can correct a match. `car` values
  the server receives are still just folder ids or names from §7a.
- It resolves only when a **single** name is clearly closest; ties and weak
  matches leave the folder id in place, so `422 unknown car` remains reachable
  and must keep working.
- A curated alias still wins over a match, **except** when the alias target is
  absent from the live `cars` list — the client reads that as a server-side
  rename and re-matches. Renaming a seeded car is therefore recoverable
  without a client release, but §7a must keep listing every car that uploads
  may name.

`car`/`track` are the same internal folder ids §5 must *emit* — here the client
*reads them off disk* (extension → sim; position under the sim's setups folder →
car/track), so this is the reverse of the `cars.simRefId` mapping: resolve the
folder id to the matching cars/tracks row where possible, and keep the raw value
either way so nothing is lost when there's no match. The user can edit all
fields before uploading, so treat them as untrusted input (validate/limit
lengths, rate-limit with `checkRateLimit`).

Response `200/201`:

```jsonc
{
  "id": "<setup uuid>",                          // required
  "url": "https://parcferme.cc/setups/<uuid>"    // optional; client synthesizes it from `id` if absent
}
```

Errors: `401` bad/revoked token · `403` uploads not permitted for this user ·
`413` too large · `422` invalid metadata. The client maps 401 to its reconnect
hint and surfaces the rest verbatim, so a JSON `{ "error": "…" }` body with a
human-readable message is worth returning.

## 7a. Picker suggestions — car/track names and setup types

The upload form offers the site's own spelling as autocomplete suggestions, so
users pick instead of guess (the alias table above only covers known folder-id
exceptions). One authenticated call feeds every picker:

### `GET /api/device/options?sim=<iracing|acc|lmu>`

- `Authorization: Bearer <device token>` — same resolver as §3.
- `sim` is required; anything else → `400 {"error":"sim must be one of: …"}`.

Response `200`:

```jsonc
{
  "cars": ["Ferrari 296 GT3", "…"],                 // display names, sorted
  "tracks": ["Spa-Francorchamps", "…"],             // display names, sorted
  "setupTypes": ["safe", "aggressive", "qualifying", "endurance"]
}
```

- **Names only.** The client suggests; the §7 upload endpoint still resolves
  every value to a row itself (normalized equality + the folder maps), so a
  stale or partial list can never corrupt an upload.
- `setupTypes` is the same list the website's upload form renders and is
  sim-independent — it rides here so the client never hardcodes it. It is the
  one list the client must *not* treat as suggestions: §7's `types` rejects
  anything outside it. An empty or absent list means an older server; render no
  type picker and let §7 apply its default.
- `tracks` excludes the `"Unknown Track"` catch-all row §7 parks trackless
  iRacing uploads on — it is a parking spot, not a suggestion.
- Errors are JSON like the rest of the device API: `400` bad sim, `401`
  bad/revoked token, `429` rate-limited, `500` otherwise.
- The client caches per sim for the app's run and degrades any failure to
  empty lists (no datalist), never an error in the form.

## 8. Release download (website "Download the app" button)

CI attaches **stable-named** installers to every `v*` GitHub Release alongside
the versioned ones. The website's download button should link:

```
https://github.com/viik2k/parcferme-desktop/releases/latest/download/ParcFerme-Setup.msi
```

(`ParcFerme-Setup.exe` for the NSIS build.) `releases/latest/download/…` always
resolves to the newest **published** release — drafts stay invisible, so
publishing the draft release is the whole "ship" step.

## Client knobs

- Base URL: `PARCFERME_API_URL` env (defaults to `https://parcferme.cc`).
- Client id: `pf-desktop`. Scope: `setups:download`.
- Deep-link scheme: `parcferme://equip?setup=<uuid>`.
