use serde::Serialize;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

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

const POWERCFG_TIMEOUT: Duration = Duration::from_secs(5);

/// Global guard ensuring at most one `powercfg` subprocess runs at a time.
/// Prevents system-wide power lock contention from stacking during rapid
/// plan switches.
static POWERCFG_RUNNING: AtomicBool = AtomicBool::new(false);

fn run_powercfg<const N: usize>(args: [&str; N]) -> Result<String, String> {
    if POWERCFG_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        let msg = "powercfg already running — skipping".to_string();
        log::warn!("{msg}");
        return Err(msg);
    }

    let result = run_powercfg_inner(args);

    POWERCFG_RUNNING.store(false, Ordering::SeqCst);
    result
}

fn run_powercfg_inner<const N: usize>(args: [&str; N]) -> Result<String, String> {
    let mut command = Command::new("powercfg");
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| {
            let msg = format!("failed to execute powercfg: {error}");
            log::error!("{msg}");
            msg
        })?;

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child.wait_with_output().map_err(|error| {
                    let msg = format!("failed to read powercfg output: {error}");
                    log::error!("{msg}");
                    msg
                })?;
                if !status.success() {
                    let stderr = decode_powercfg_text(&output.stderr);
                    let stdout = decode_powercfg_text(&output.stdout);
                    let msg = format!(
                        "powercfg failed (code {:?}): {}{}",
                        status.code(),
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
                return Ok(decode_powercfg_text(&output.stdout));
            }
            Ok(None) => {
                if start.elapsed() > POWERCFG_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    let msg = format!("powercfg timed out after {POWERCFG_TIMEOUT:?}");
                    log::error!("{msg}");
                    return Err(msg);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                let msg = format!("failed to wait for powercfg: {error}");
                log::error!("{msg}");
                return Err(msg);
            }
        }
    }
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
    // Guard: skip if another powercfg call is already running.
    if POWERCFG_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        log::debug!("get_active_guid_fast: skipped (powercfg busy)");
        return None;
    }

    let result = get_active_guid_fast_inner();

    POWERCFG_RUNNING.store(false, Ordering::SeqCst);
    result
}

fn get_active_guid_fast_inner() -> Option<String> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let mut child = std::process::Command::new("powercfg")
        .arg("/getactivescheme")
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                let output = child.wait_with_output().ok()?;
                let text = String::from_utf8_lossy(&output.stdout);
                for line in text.lines() {
                    if let Some(rest) = line.strip_prefix("Power Scheme GUID:") {
                        let rest = rest.trim();
                        if rest.len() >= 36 {
                            return Some(rest[..36].to_lowercase());
                        }
                    }
                }
                return None;
            }
            Ok(None) => {
                if start.elapsed() > POWERCFG_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    log::warn!("get_active_guid_fast: timed out");
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
}
