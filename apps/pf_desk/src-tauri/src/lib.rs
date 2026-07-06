//! `pf_desk` — the Tauri shell.
//!
//! Deliberately thin: it does windowing, the system tray, toasts, and log
//! plumbing, and forwards UI calls to [`pf_core`] via [`commands`]. No product
//! logic lives here.

mod commands;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, WindowEvent,
};

/// Event name the frontend listens on for the result of an equip deep link.
const EQUIP_EVENT: &str = "equip-result";
/// Emitted when the tray asks the UI to show the Settings view.
const OPEN_SETTINGS_EVENT: &str = "open-settings";
/// Emitted when auth state changed outside the UI (tray sign-out).
const AUTH_CHANGED_EVENT: &str = "auth-changed";

/// Bring the main window to the foreground, creating focus on it.
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Open a URL in the user's default browser (tray "Browse setups").
fn open_in_browser(app: &tauri::AppHandle, url: &str) {
    use tauri_plugin_opener::OpenerExt;
    if let Err(e) = app.opener().open_url(url, None::<&str>) {
        log::warn!("couldn't open browser for {url}: {e}");
    }
}

/// Show a native OS notification. Returns `Err` so the caller can fall back to
/// revealing the window — an equip must never succeed silently.
fn notify(
    app: &tauri::AppHandle,
    title: &str,
    body: &str,
) -> Result<(), tauri_plugin_notification::Error> {
    use tauri_plugin_notification::NotificationExt;
    app.notification().builder().title(title).body(body).show()
}

/// Structured file logging (M4) to
/// `%LOCALAPPDATA%\cc.parcferme.desktop\logs\pf-desk.log`, plus stdout in dev.
/// `pf_core` logs at debug for support traces. **No secrets** — tokens and
/// presigned URLs are never logged by either crate.
fn log_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    use tauri_plugin_log::{RotationStrategy, Target, TargetKind};
    tauri_plugin_log::Builder::new()
        .targets([
            Target::new(TargetKind::Stdout),
            Target::new(TargetKind::LogDir {
                file_name: Some("pf-desk".into()),
            }),
        ])
        .level(log::LevelFilter::Info)
        .level_for("pf_core", log::LevelFilter::Debug)
        .max_file_size(1_000_000)
        .rotation_strategy(RotationStrategy::KeepOne)
        .build()
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

/// Process one opened URL: ignore anything that isn't ours, otherwise run the
/// install off the UI thread and surface the outcome — a native toast on
/// success (no focus stolen from the browser or the sim), the window on error.
fn handle_equip(app: &tauri::AppHandle, url: String) {
    if !url.starts_with(&format!("{}://", pf_core::deeplink::SCHEME)) {
        return;
    }
    // Never log the URL itself — its `token` param, if present, is a secret.
    log::info!("equip deep link received");
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = tauri::async_runtime::spawn_blocking(move || commands::run_equip(&url))
            .await
            .unwrap_or_else(|e| commands::EquipOutcome::Error {
                kind: "internal".into(),
                message: e.to_string(),
            });
        match &outcome {
            commands::EquipOutcome::Installed(setup) => {
                log::info!("equip result: {:?} at {}", setup.action, setup.path);
                let (title, body) = setup.toast();
                if notify(&app, &title, &body).is_err() {
                    show_main_window(&app);
                }
            }
            commands::EquipOutcome::Error { kind, message } => {
                log::warn!("equip failed ({kind}): {message}");
                show_main_window(&app);
            }
        }
        let _ = app.emit(EQUIP_EVENT, outcome);
    });
}

/// Sign out from the tray: clear the keychain off-thread, then tell the UI.
fn tray_sign_out(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match tauri::async_runtime::spawn_blocking(pf_core::auth::sign_out).await {
            Ok(Ok(())) => {
                let _ = app.emit(AUTH_CHANGED_EVENT, ());
            }
            Ok(Err(e)) => log::warn!("tray sign-out failed: {e}"),
            Err(e) => log::warn!("tray sign-out task failed: {e}"),
        }
    });
}

/// Build the tray icon + the M4 menu: Open, Browse setups, Settings, Sign out,
/// Quit. The tray is the app's home — the window is just its dashboard.
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open ParcFerme", true, None::<&str>)?;
    let library = MenuItem::with_id(app, "library", "Browse setups", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let sign_out = MenuItem::with_id(app, "sign-out", "Sign out", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &library,
            &settings,
            &PredefinedMenuItem::separator(app)?,
            &sign_out,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("ParcFerme")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "library" => open_in_browser(
                app,
                &format!("{}/setups", pf_core::api::base_url_from_env()),
            ),
            "settings" => {
                show_main_window(app);
                let _ = app.emit(OPEN_SETTINGS_EVENT, ());
            }
            "sign-out" => tray_sign_out(app),
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
        builder = builder
            .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
                show_main_window(app);
            }))
            // Launch-at-startup (Settings toggle). Login launches pass --hidden
            // so the app wakes in the tray without flashing a window.
            .plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                Some(vec!["--hidden"]),
            ));
    }

    builder
        .plugin(log_plugin())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            setup_tray(app)?;
            setup_deep_links(app);
            // The window is created hidden (tauri.conf.json); autostart launches
            // stay in the tray, everything else (user launch, deep-link cold
            // start) shows it.
            if std::env::args().any(|arg| arg == "--hidden") {
                log::info!("started hidden (login launch) — waiting in the tray");
            } else {
                show_main_window(app.handle());
            }
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
            commands::get_settings,
            commands::save_settings,
            commands::get_autostart,
            commands::set_autostart,
            commands::open_logs_dir,
            commands::detect_sims,
            commands::download_setup,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the ParcFerme tray app")
}
