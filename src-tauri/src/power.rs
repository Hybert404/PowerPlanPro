use serde::Serialize;
use std::process::Command;
use std::sync::Mutex;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerPlan {
    pub guid: String,
    pub name: String,
    pub is_active: bool,
}

pub struct PowerManager {
    lock: Mutex<()>,
}

impl PowerManager {
    pub fn new() -> Self {
        Self {
            lock: Mutex::new(()),
        }
    }

    pub fn list_plans(&self) -> Result<Vec<PowerPlan>, String> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "failed to lock power manager".to_string())?;

        let output = run_powercfg(["/L"])?;
        let mut plans = Vec::new();

        for line in output.lines() {
            if let Some(guid) = extract_guid(line) {
                let name = extract_name(line).unwrap_or_else(|| "Unknown".to_string());
                plans.push(PowerPlan {
                    guid,
                    name,
                    is_active: line.contains('*'),
                });
            }
        }

        if plans.is_empty() {
            return Err("no power plans found via powercfg".to_string());
        }

        Ok(plans)
    }

    pub fn get_active_plan_guid(&self) -> Result<String, String> {
        let plans = self.list_plans()?;
        let active = plans
            .into_iter()
            .find(|plan| plan.is_active)
            .ok_or_else(|| "failed to detect active power plan".to_string())?;
        Ok(active.guid)
    }

    pub fn set_active_plan(&self, guid: &str) -> Result<(), String> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "failed to lock power manager".to_string())?;

        run_powercfg(["/SETACTIVE", guid])?;
        Ok(())
    }
}

fn run_powercfg<const N: usize>(args: [&str; N]) -> Result<String, String> {
    let mut command = Command::new("powercfg");
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let output = command
        .args(args)
        .output()
        .map_err(|error| {
            let msg = format!("failed to execute powercfg: {error}");
            log::error!("{msg}");
            msg
        })?;

    if !output.status.success() {
        let stderr = decode_powercfg_text(&output.stderr);
        let stdout = decode_powercfg_text(&output.stdout);
        let msg = format!(
            "powercfg failed (code {:?}): {}{}",
            output.status.code(),
            stdout.trim(),
            if stderr.trim().is_empty() {
                "".to_string()
            } else {
                format!(" | {}", stderr.trim())
            }
        );
        log::error!("{msg}");
        return Err(msg);
    }

    Ok(decode_powercfg_text(&output.stdout))
}

#[cfg(windows)]
fn decode_powercfg_text(bytes: &[u8]) -> String {
    use windows::Win32::Globalization::{MultiByteToWideChar, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};

    const CP_OEMCP: u32 = 1;

    if bytes.is_empty() {
        return String::new();
    }

    unsafe {
        let wide_len = MultiByteToWideChar(CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, None);
        if wide_len <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }

        let mut wide = vec![0u16; wide_len as usize];
        let converted = MultiByteToWideChar(
            CP_OEMCP,
            MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0),
            bytes,
            Some(&mut wide),
        );
        if converted <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }

        String::from_utf16_lossy(&wide[..converted as usize])
    }
}

#[cfg(not(windows))]
fn decode_powercfg_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn extract_name(line: &str) -> Option<String> {
    let start = line.find('(')?;
    let end = line.rfind(')')?;
    if end <= start {
        return None;
    }

    let name = line[(start + 1)..end].trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn extract_guid(line: &str) -> Option<String> {
    for token in line.split_whitespace() {
        let candidate = token
            .trim_matches(|character: char| character == ':' || character == '*' || character == ',');
        if is_guid(candidate) {
            return Some(candidate.to_ascii_lowercase());
        }
    }
    None
}

fn is_guid(candidate: &str) -> bool {
    if candidate.len() != 36 {
        return false;
    }

    for (index, character) in candidate.chars().enumerate() {
        match index {
            8 | 13 | 18 | 23 => {
                if character != '-' {
                    return false;
                }
            }
            _ => {
                if !character.is_ascii_hexdigit() {
                    return false;
                }
            }
        }
    }

    true
}

/// Returns the GUID of the currently active power scheme by calling
/// `powercfg /getactivescheme`. No admin required. Called every ~3 s so the
/// subprocess overhead is negligible compared to the polling interval.
pub fn get_active_guid_fast() -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let out = std::process::Command::new("powercfg")
        .arg("/getactivescheme")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;

    // Output: "Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced)"
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Power Scheme GUID:") {
            let rest = rest.trim();
            if rest.len() >= 36 {
                return Some(rest[..36].to_lowercase());
            }
        }
    }
    None
}
