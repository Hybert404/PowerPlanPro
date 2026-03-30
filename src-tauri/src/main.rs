#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod engine;
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
            std::mem::forget(h);
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

fn main() {
    if !acquire_single_instance_mutex() {
        // Another instance is already running — exit silently.
        return;
    }

    let core = Arc::new(AppCore::new().expect("failed to initialize app core"));

    tauri::Builder::default()
        .manage(Arc::clone(&core))
        .setup(move |app| {
            let handle = app.handle().clone();

            // Resolve persistent config directory and load saved settings.
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("failed to resolve app data dir: {e}"))?;
            core.set_data_dir(data_dir.clone());
            if let Some(saved) = persist::load(&data_dir) {
                core.apply_persisted_config(saved);
            }

            // Register tray-icon update callback (engine → tray, no circular dep).
            let handle_cb = handle.clone();
            core.set_plan_changed_callback(move |guid| {
                tray::update_tray_icon(&handle_cb, guid);
            });

            // Create system tray with initial plan list and active plan.
            let plans = core.list_power_plans().unwrap_or_default();
            let status = core
                .get_engine_status()
                .expect("failed to read initial engine status");
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
            set_autostart
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
