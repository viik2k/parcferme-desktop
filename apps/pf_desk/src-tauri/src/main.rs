// Prevents an extra console window on Windows in release; keep it for dev.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    pf_desk_lib::run()
}
