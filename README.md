# ParcFerme Desktop

A lightweight Windows **tray client** that turns clicking **Equip** on
[parcferme.cc](https://parcferme.cc) into a setup file landing in the user's
iRacing folder. Built on **Tauri 2** (Rust core + web UI).

> Status: **M1 — Device auth (desktop side).** Tray app boots with a tray icon;
> `pf_core` runs the OAuth 2.0 device-authorization grant and stores the device
> token in Windows Credential Manager. The matching server endpoints are
> specified in [`docs/SERVER_CONTRACT.md`](docs/SERVER_CONTRACT.md).

## Architecture

All product logic lives in `pf_core`, a plain Rust library with no UI. The
Tauri shell (`pf_desk`) only does windowing, the tray, and toasts. A future CLI
or push-daemon becomes new functions in `pf_core`, not a new app.

```
parcferme-desktop/
├── crates/
│   └── pf_core/              ← the real product: Rust library, no UI
│        ├── auth             (device-token grant, OS-keychain storage)   [M1]
│        ├── api              (typed client for parcferme.cc endpoints)   [M2]
│        ├── paths            (locate Documents\iRacing\setups\, …)       [M2]
│        ├── download         (presigned-URL fetch → atomic write)        [M2]
│        └── deeplink         (parse parcferme://equip?… payloads)        [M3]
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

See `ParcFerme Desktop App — Build Plan` (M0 skeleton → M6 launch) and §8 for
the post-v1 roadmap (Push, Auto-sync, Team Sync, in-app diff).
