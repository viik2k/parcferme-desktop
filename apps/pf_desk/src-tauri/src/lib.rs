//! `pf_desk` — the Tauri shell.
//!
//! Deliberately thin: it does windowing, the system tray, and (later) toasts,
//! and forwards UI calls to [`pf_core`] via [`commands`]. No product logic
//! lives here.

mod commands;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};

/// Bring the main window to the foreground, creating focus on it.
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
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
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            setup_tray(app)?;
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running the ParcFerme tray app");
}
