#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use cc_collector::{start_server, view, AppState};
use installer::{merge_hooks_for_event, merge_statusline, remove_tool_entries, HOOK_EVENTS};
use std::fs;
use std::path::PathBuf;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};

struct AppStateHolder(AppState);

mod installer;

fn forwarder_exe_path() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("release")
        .join("cc-forward.exe")
        .to_string_lossy()
        .replace('\\', "/")
}

fn settings_path() -> PathBuf {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    PathBuf::from(home).join(".claude").join("settings.json")
}

fn forwarder_config_path() -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_default();
    PathBuf::from(appdata).join("cc-task-state").join("forwarder.json")
}

fn read_settings() -> serde_json::Value {
    fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::json!({}))
}

fn read_forwarder_config() -> serde_json::Value {
    fs::read_to_string(forwarder_config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::json!({}))
}

fn write_settings(v: &serde_json::Value) -> std::io::Result<()> {
    let p = settings_path();
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Ok(prev) = fs::read(&p) {
        let _ = fs::write(format!("{}.cc-task-state.bak", p.to_string_lossy()), prev);
    }
    fs::write(&p, serde_json::to_string_pretty(v)?)
}

fn write_forwarder_config(v: &serde_json::Value) -> std::io::Result<()> {
    let p = forwarder_config_path();
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&p, serde_json::to_string_pretty(v)?)
}

fn install_config() -> std::io::Result<()> {
    let exe = forwarder_exe_path();
    let mut settings = read_settings();
    // Preserve a pre-existing user statusline so the forwarder can chain it.
    let existing = settings
        .get("statusLine")
        .and_then(|s| s.get("command"))
        .and_then(|c| c.as_str())
        .filter(|c| !c.contains("cc-forward.exe"))
        .map(|s| s.to_string());
    write_forwarder_config(&serde_json::json!({
        "port": 7331,
        "chain_command": existing,
    }))?;
    merge_statusline(&mut settings, &exe);
    for ev in HOOK_EVENTS {
        merge_hooks_for_event(&mut settings, ev, &exe);
    }
    write_settings(&settings)
}

fn uninstall_config() -> std::io::Result<()> {
    let exe = forwarder_exe_path();
    let mut settings = read_settings();
    // Restore the user's original statusline if we chained one.
    let chain = read_forwarder_config()
        .get("chain_command")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    remove_tool_entries(&mut settings, &exe);
    if let Some(cmd) = chain {
        settings["statusLine"] = serde_json::json!({ "type": "command", "command": cmd });
    }
    write_settings(&settings)?;
    let _ = fs::remove_file(forwarder_config_path());
    Ok(())
}

#[tauri::command]
fn get_sessions(state: tauri::State<'_, AppStateHolder>) -> Vec<cc_collector::SessionView> {
    view(&state.0.records)
}

#[tauri::command]
fn install_hooks() -> Result<String, String> {
    install_config().map(|_| "installed".into()).map_err(|e| e.to_string())
}

#[tauri::command]
fn uninstall_hooks() -> Result<String, String> {
    uninstall_config().map(|_| "uninstalled".into()).map_err(|e| e.to_string())
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

fn main() {
    let app_state = AppState::default();
    let _server_handle = start_server(AppState { records: app_state.records.clone() }, 7331);

    tauri::Builder::default()
        .manage(AppStateHolder(app_state))
        .setup(|app| {
            let toggle = MenuItem::with_id(app, "toggle", "Show/Hide", true, None::<&str>)?;
            let install = MenuItem::with_id(app, "install", "Install CC config", true, None::<&str>)?;
            let uninstall = MenuItem::with_id(app, "uninstall", "Uninstall CC config", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle, &install, &uninstall, &quit])?;
            TrayIconBuilder::new()
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = if w.is_visible().unwrap_or(false) { w.hide() } else { w.show() };
                        }
                    }
                    "install" => { let _ = install_config(); }
                    "uninstall" => { let _ = uninstall_config(); }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_sessions, install_hooks, uninstall_hooks, quit_app])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
