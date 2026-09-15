#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod engine;
mod logging;
mod metrics;
mod persist;
mod power;
mod tray;

use engine::{AppCore, EngineStatus, RuleConfig};
use metrics::UsageSnapshot;
use power::PowerPlan;
use std::sync::Arc;
use tauri::{Manager, State};

/// Returns false if another instance is already running.
/// The mutex handle is intentionally leaked so it stays alive for the process lifetime.
fn acquire_single_instance_mutex() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
    use windows::Win32::System::Threading::CreateMutexW;
    let name: Vec<u16> = "Local\\PowerPlanPro_SingleInstance\0"
        .encode_utf16()
        .collect();
    let handle = unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr())) };
    match handle {
        Ok(h) => {
            let already = unsafe { windows::Win32::Foundation::GetLastError() }
                == ERROR_ALREADY_EXISTS;
            if already {
                // Don't close — just signal the caller to exit.
                let _ = unsafe { windows::Win32::Foundation::CloseHandle(h) };
                return false;
            }
            // Leak intentionally: mutex must stay held for the whole process lifetime.
            // HANDLE is Copy, so mem::forget is a no-op; `_` is sufficient to suppress the warning.
            let _ = h;
            true
        }
        Err(_) => true, // If we can't create the mutex, allow startup anyway.
    }
}

#[tauri::command]
fn get_usage_snapshot(state: State<'_, Arc<AppCore>>) -> Result<UsageSnapshot, String> {
    state.inner().get_usage_snapshot()
}

#[tauri::command]
fn list_power_plans(state: State<'_, Arc<AppCore>>) -> Result<Vec<PowerPlan>, String> {
    state.inner().list_power_plans()
}

#[tauri::command]
fn set_power_plan(state: State<'_, Arc<AppCore>>, plan_guid: String) -> Result<(), String> {
    state.inner().set_power_plan(plan_guid)
}

#[tauri::command]
fn get_rule_config(state: State<'_, Arc<AppCore>>) -> Result<RuleConfig, String> {
    state.inner().get_rule_config()
}

#[tauri::command]
fn set_rule_config(state: State<'_, Arc<AppCore>>, config: RuleConfig) -> Result<(), String> {
    state.inner().set_rule_config(config)
}

#[tauri::command]
fn get_engine_status(state: State<'_, Arc<AppCore>>) -> Result<EngineStatus, String> {
    state.inner().get_engine_status()
}

#[tauri::command]
fn get_autostart() -> bool {
    autostart::get_autostart()
}

#[tauri::command]
fn set_autostart(_app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("failed to resolve exe path: {e}"))?;
    autostart::set_autostart(enabled, &exe)
}

#[tauri::command]
fn pause_engine(state: State<'_, Arc<AppCore>>, minutes: u32) -> Result<(), String> {
    state.inner().pause_engine(minutes)
}

#[tauri::command]
fn unpause_engine(state: State<'_, Arc<AppCore>>) -> Result<(), String> {
    state.inner().unpause_engine()
}

fn main() {
    if !acquire_single_instance_mutex() {
        // Another instance is already running — exit silently.
        return;
    }

    let core = match AppCore::new() {
        Ok(core) => Arc::new(core),
        Err(e) => {
            log::error!("failed to initialize app core: {e}");
            eprintln!("failed to initialize app core: {e}");
            return;
        }
    };

    tauri::Builder::default()
        .manage(Arc::clone(&core))
        .setup(move |app| {
            let handle = app.handle().clone();

            // Resolve persistent config directory and load saved settings.
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("failed to resolve app data dir: {e}"))?;

            // Initialize logging to file (after resolving data_dir).
            logging::init(&data_dir);

            log::info!("Power Plan Pro starting up");
            log::info!("data directory: {}", data_dir.display());

            core.set_data_dir(data_dir.clone());
            if let Some(saved) = persist::load(&data_dir) {
                log::info!("loaded persisted config from disk");
                core.apply_persisted_config(saved);
            }

            // Register tray-icon update callback (engine → tray, no circular dep).
            let handle_cb = handle.clone();
            core.set_plan_changed_callback(move |guid| {
                tray::update_tray_icon(&handle_cb, guid);
            });

            // Create system tray with initial plan list and active plan.
            let plans = core.list_power_plans().unwrap_or_default();
            let status = core.get_engine_status().unwrap_or_default();
            tray::create_tray(&handle, Arc::clone(&core), &plans, &status.active_plan_guid)?;

            // Hide to tray instead of quitting when the window's close button is clicked.
            if let Some(window) = app.get_webview_window("main") {
                let window_for_close = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = window_for_close.hide();
                    }
                });
            }

            // Start the 1-second background rule-engine loop.
            core.start_background();

            // In release builds start minimised to tray; devs see the window normally.
            #[cfg(not(debug_assertions))]
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }

            log::info!("app setup complete");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_usage_snapshot,
            list_power_plans,
            set_power_plan,
            get_rule_config,
            set_rule_config,
            get_engine_status,
            get_autostart,
            set_autostart,
            pause_engine,
            unpause_engine
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
