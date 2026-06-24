//! `pf_desk` — the Tauri shell.
//!
//! Deliberately thin: it does windowing, the system tray, and (later) toasts,
//! and forwards UI calls to [`pf_core`] via [`commands`]. No product logic
//! lives here.

mod commands;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, WindowEvent,
};

/// Event name the frontend listens on for the result of an equip deep link.
const EQUIP_EVENT: &str = "equip-result";

/// Bring the main window to the foreground, creating focus on it.
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Wire the `parcferme://equip?…` deep link (M3). Fires for both a cold start
/// (app launched by the link) and — via the single-instance plugin's `deep-link`
/// feature — a warm start (link opened while already running).
fn setup_deep_links(app: &tauri::App) {
    use tauri_plugin_deep_link::DeepLinkExt;

    // Point the scheme at this binary. Installed builds also get this from the
    // bundler (tauri.conf.json `plugins.deep-link`), but registering at runtime
    // makes `pnpm dev` testable without an install. Best-effort: a failure here
    // must not stop the app from starting.
    #[cfg(any(windows, target_os = "linux"))]
    {
        let _ = app.deep_link().register_all();
    }

    let handle = app.handle().clone();
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            handle_equip(&handle, url.to_string());
        }
    });
}

/// Process one opened URL: ignore anything that isn't ours, otherwise surface the
/// window and run the install off the UI thread, emitting the result.
fn handle_equip(app: &tauri::AppHandle, url: String) {
    if !url.starts_with(&format!("{}://", pf_core::deeplink::SCHEME)) {
        return;
    }
    show_main_window(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = tauri::async_runtime::spawn_blocking(move || commands::run_equip(&url))
            .await
            .unwrap_or_else(|e| commands::EquipOutcome::Error {
                message: e.to_string(),
            });
        let _ = app.emit(EQUIP_EVENT, outcome);
    });
}

/// Build the tray icon + menu (M0). Tray UX is refined in M4 (Open library,
/// Settings, Sign out, Quit).
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open ParcFerme", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("ParcFerme")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    // Reuse the bundled app icon for the tray.
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    // Single instance MUST be registered first (per the plugin's contract). Its
    // "deep-link" feature forwards a parcferme:// link opened against an already-
    // running app into the deep-link plugin; the callback just reveals the window.
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_main_window(app);
        }));
    }

    builder
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
            setup_tray(app)?;
            setup_deep_links(app);
            Ok(())
        })
        // Closing the window hides to tray instead of quitting — this is a
        // background tray utility that runs alongside iRacing.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::auth_status,
            commands::connect_begin,
            commands::connect_poll,
            commands::sign_out,
            commands::detect_sims,
            commands::download_setup,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the ParcFerme tray app");
}
