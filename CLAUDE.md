# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Development
pnpm install                              # install frontend deps
pnpm dev                                  # Tauri app with hot-reload UI
PARCFERME_API_URL=http://localhost:3000 pnpm dev  # point at local server

# Build
pnpm build                                # produces .msi/.exe under target/release/bundle

# Rust
pnpm lint:rust                            # cargo clippy --workspace --all-targets
pnpm fmt:rust                             # cargo fmt --all
pnpm test:rust                            # cargo test --workspace
cargo test -p pf_core <test_name>        # run a single test in pf_core

# Icons
pnpm icon apps/pf_desk/src-tauri/app-icon.png
```

## Architecture

All product logic lives in `pf_core`, a plain Rust library with no UI, no async runtime, and no Tauri dependency. The Tauri shell (`pf_desk`) is a thin wrapper for windowing, the tray icon, and IPC.

```
crates/pf_core/src/
├── lib.rs        # public API surface
├── auth.rs       # OAuth 2.0 device-authorization grant + Windows Credential Manager storage
├── api.rs        # typed blocking HTTP client for parcferme.cc (ureq, no async)
├── paths.rs      # locate Documents\iRacing\setups\
├── download.rs   # presigned-URL fetch → atomic write to disk
├── deeplink.rs   # parse parcferme:// URL schemes
└── error.rs      # PfError enum + Result<T> alias

apps/pf_desk/src-tauri/src/
├── lib.rs        # Tauri app setup: tray, window, command registration
└── commands.rs   # Tauri #[command] fns — bridges React IPC to pf_core; runs blocking work on spawn_blocking

apps/pf_desk/src/
├── App.tsx       # root component: auth state machine, panel switching
├── components/   # ConnectPanel, Connected, DownloadPanel
└── lib/          # thin TS wrappers over Tauri invoke() calls (auth.ts, download.ts)
```

### Key constraints

- `pf_core` must stay runtime-agnostic (no `async`, no `tokio`). All blocking I/O runs on Tauri's `spawn_blocking`.
- HTTP uses `ureq` (blocking). Adding `reqwest`/`tokio` to pf_core would break the library-first design.
- The Tauri frontend runs on port **1420** (fixed; required by Tauri dev server).
- Base API URL defaults to `https://parcferme.cc`; override with `PARCFERME_API_URL` env var.

## Adding a new feature

1. Implement in `pf_core` (pure Rust, no Tauri imports).
2. Expose via a `#[tauri::command]` in `apps/pf_desk/src-tauri/src/commands.rs`.
3. Register the command in `lib.rs` (`invoke_handler`).
4. Add a `invoke()`-based wrapper in `apps/pf_desk/src/lib/`.
5. Wire the UI in `apps/pf_desk/src/`.

## Server contract

`docs/SERVER_CONTRACT.md` specifies the API the server must implement for each milestone. The desktop client is built against it — do not change client-side endpoint paths or payload shapes without updating that doc. The `PARCFERME_API_URL` env var is the only knob; the client id is always `pf-desktop`.
