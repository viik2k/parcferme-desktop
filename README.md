# ParcFerme Desktop

A lightweight Windows **tray client** that turns clicking **Equip** on
[parcferme.cc](https://parcferme.cc) into a setup file landing in the user's
iRacing, LMU or ACC folder. Built on **Tauri 2** (Rust core + web UI).

> Status: **shipped (v1.0.x).** Device auth (OAuth 2.0 device-authorization
> grant), multi-sim manual pull/download (iRacing, ACC, LMU), setup upload, and
> the `parcferme://equip` deep link all ship in v1.0. On top of that, v1.0
> ships the **Sync Engine** (records every LMU telemetry session, pushes it, then
> deletes it), **Team Auto-Install** (opt-in pull of new team-vault setups), and
> a live **LMU telemetry source** (shared memory + REST merged
> into one frame stream). The client also has persisted Settings (per-sim folder
> overrides + conflict policy), typed error states with recovery hints, a full
> tray menu, native equip toasts, launch-at-startup, and structured file logging.
> Server endpoints are specified in
> [`docs/SERVER_CONTRACT.md`](docs/SERVER_CONTRACT.md).

## Architecture

All product logic lives in `pf_core`, a plain Rust library with no UI, no async
runtime, and no Tauri dependency. The Tauri shell (`pf_desk`) is a thin wrapper
for windowing, the tray icon, and IPC.

```
parcferme-desktop/
├── crates/
│   └── pf_core/              ← the real product: Rust library, no UI
│        ├── auth             (OAuth 2.0 device-authorization grant + OS-keychain storage)
│        ├── api              (typed blocking client for parcferme.cc endpoints)
│        ├── paths            (locate each sim's setups root)
│        ├── sim              (supported sims: iRacing, ACC, LMU folder layouts)
│        ├── download         (presigned-URL fetch → atomic, conflict-aware write)
│        ├── upload           (identify a local setup + push to the site)
│        ├── deeplink         (parse parcferme:// URL schemes)
│        ├── session          (telemetry sessions: record, summarize, gzip + share)
│        ├── sync             (Sync Engine: daemon — record every LMU session, push, delete)
│        ├── team_sync        (Team Auto-Install: pull new team-vault setups, opt-in)
│        ├── lmu              (Le Mans Ultimate live source: shared memory + REST)
│        ├── car_aliases      (curated iRacing folder-id → site car-name map)
│        ├── car_match        (match a folder id against the site's live car list)
│        ├── options          (cached car/track name lists for upload pickers)
│        └── settings         (persisted per-sim folder overrides + conflict policy)
├── apps/
│   └── pf_desk/              ← Tauri shell: tray, window, settings, toasts
│        ├── src/             (React + TS + Tailwind + Vite UI)
│        └── src-tauri/       (commands.rs → calls pf_core; tray; updater)
└── packages/
    └── ui/                   ← optional shared design system w/ the web app
```

## Prerequisites

- [Rust](https://rustup.rs/) (stable) + the MSVC toolchain
- [Node.js](https://nodejs.org/) 20+ and [pnpm](https://pnpm.io/) 9+
- [WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/)
  (preinstalled on Windows 11)

## Development

```bash
pnpm install            # install frontend deps
pnpm dev                # run the Tauri app in dev (hot-reloads the UI)
```

Point the client at a local web server during development:

```bash
PARCFERME_API_URL=http://localhost:3000 pnpm dev
```

## Build

```bash
pnpm build              # produces an unsigned .msi / .exe under target/release/bundle
```

## Regenerating icons

```bash
pnpm icon apps/pf_desk/src-tauri/app-icon.png
```

## Roadmap

The M0→M6 build plan is complete and v1.0 has shipped. In-flight work on
`staging`: **auto-sync** (setups saved in-sim reach the site with no button
press) and a team-sync polling backoff. Open design questions live in the GitHub
issues (session-boundary recording, sync-queue retention, in-app diff).
