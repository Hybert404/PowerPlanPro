use crate::engine::AppCore;
use crate::power::PowerPlan;
use std::sync::Arc;
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

const GUID_BALANCED: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";
const GUID_POWER_SAVER: &str = "a1841308-3541-4fab-bc81-f71556f20b4a";
const GUID_HIGH_PERF: &str = "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c";
const GUID_ULTRA_PERF: &str = "e9a42b02-d5df-448d-aa00-03f14749eb61";

static TRANSPARENT_PIXEL: [u8; 4] = [0, 0, 0, 0];

pub fn icon_filename_for_guid(guid: &str) -> &'static str {
    let lower = guid.to_lowercase();
    if lower == GUID_BALANCED {
        "plan-balanced.ico"
    } else if lower == GUID_POWER_SAVER {
        "plan-power-saver.ico"
    } else if lower == GUID_HIGH_PERF || lower == GUID_ULTRA_PERF {
        "plan-high-performance.ico"
    } else {
        "plan-unknown.ico"
    }
}

pub fn static_plan_label(guid: &str) -> &'static str {
    let lower = guid.to_lowercase();
    if lower == GUID_BALANCED {
        "Balanced"
    } else if lower == GUID_POWER_SAVER {
        "Power Saver"
    } else if lower == GUID_HIGH_PERF || lower == GUID_ULTRA_PERF {
        "High Performance"
    } else {
        "Custom Plan"
    }
}

fn try_load_icon(app: &AppHandle, guid: &str) -> Option<Image<'static>> {
    let path = app
        .path()
        .resource_dir()
        .ok()?
        .join("icons")
        .join("tray")
        .join(icon_filename_for_guid(guid));

    let img = image::open(&path).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Some(Image::new_owned(img.into_raw(), width, height))
}

pub fn create_tray(
    app: &AppHandle,
    core: Arc<AppCore>,
    plans: &[PowerPlan],
    active_guid: &str,
) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?;
    let pause_item = MenuItem::with_id(app, "pause_15", "Pause Rule Engine for 15 min", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit Power Plan Pro", true, None::<&str>)?;

    // Build plan-switch items; collect so they outlive the builder.
    let plan_items: Vec<MenuItem<tauri::Wry>> = plans
        .iter()
        .map(|plan| {
            MenuItem::with_id(
                app,
                format!("plan:{}", plan.guid),
                &plan.name,
                true,
                None::<&str>,
            )
        })
        .collect::<tauri::Result<_>>()?;

    let mut builder = MenuBuilder::new(app).item(&show_item).separator();
    for item in &plan_items {
        builder = builder.item(item);
    }
    let menu = builder.separator().item(&pause_item).item(&quit_item).build()?;

    let icon = try_load_icon(app, active_guid)
        .or_else(|| app.default_window_icon().cloned())
        .unwrap_or_else(|| Image::new(&TRANSPARENT_PIXEL, 1, 1));

    TrayIconBuilder::with_id("main_tray")
        .icon(icon)
        .tooltip(&format!(
            "Power Plan Pro \u{2014} {}",
            static_plan_label(active_guid)
        ))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "pause_15" => {
                let _ = core.pause_engine(15);
            }
            "quit" => app.exit(0),
            id => {
                if let Some(guid) = id.strip_prefix("plan:") {
                    let _ = core.set_power_plan(guid.to_string());
                    // tray icon/tooltip updated via the on_plan_changed callback
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Some(window) = tray.app_handle().get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}

/// Called from the on_plan_changed callback registered in main.rs.
pub fn update_tray_icon(app: &AppHandle, guid: &str) {
    let Some(tray) = app.tray_by_id("main_tray") else {
        log::warn!("tray icon not found, cannot update");
        return;
    };
    if let Some(icon) = try_load_icon(app, guid) {
        let _ = tray.set_icon(Some(icon));
    } else {
        log::warn!("failed to load tray icon for plan {guid}, keeping previous icon");
    }
    let label = static_plan_label(guid);
    let _ = tray.set_tooltip(Some(&format!("Power Plan Pro \u{2014} {label}")));
}

