use std::path::Path;

const APP_NAME: &str = "Power Plan Pro";
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

pub fn get_autostart() -> bool {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(key) = hkcu.open_subkey(RUN_KEY) else {
        return false;
    };
    key.get_value::<String, _>(APP_NAME).is_ok()
}

pub fn set_autostart(enabled: bool, exe_path: &Path) -> Result<(), String> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(RUN_KEY)
        .map_err(|e| format!("failed to open autostart registry key: {e}"))?;
    if enabled {
        let value = format!("\"{}\"", exe_path.to_string_lossy());
        key.set_value(APP_NAME, &value)
            .map_err(|e| format!("failed to write autostart registry value: {e}"))?;
    } else {
        // Ignore NotFound  already disabled.
        let _ = key.delete_value(APP_NAME);
    }
    Ok(())
}
