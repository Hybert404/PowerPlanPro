use log::{Level, LevelFilter, Log, Metadata, Record};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::SystemTime;

static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

struct FileLogger;

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // Filter out noisy internal warnings from tao/wry event loop.
        let target = metadata.target();
        if target.starts_with("tao::") || target.starts_with("wry::") {
            return false;
        }
        true
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let Ok(guard) = LOG_FILE.lock() else {
            return;
        };
        let Some(ref file) = *guard else {
            return;
        };
        let mut file = match file.try_clone() {
            Ok(f) => f,
            Err(_) => return,
        };
        drop(guard);

        let level = match record.level() {
            Level::Error => "ERROR",
            Level::Warn => " WARN",
            Level::Info => " INFO",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        };

        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| {
                let secs = d.as_secs();
                let millis = d.subsec_millis();
                let days = secs / 86400;
                let time_of_day = secs % 86400;
                let hours = time_of_day / 3600;
                let minutes = (time_of_day % 3600) / 60;
                let seconds = time_of_day % 60;
                // Simple days-to-date (good enough for log timestamps)
                let (year, month, day) = days_to_date(days);
                format!("{year:04}-{month:02}-{day:02} {hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
            })
            .unwrap_or_else(|_| "0000-00-00 00:00:00.000".to_string());

        let _ = writeln!(file, "[{timestamp}] [{level}] {}", record.args());
    }

    fn flush(&self) {
        if let Ok(guard) = LOG_FILE.lock() {
            if let Some(ref file) = *guard {
                let _ = file.try_clone().and_then(|mut f| f.flush());
            }
        }
    }
}

/// Initialize logging to a file in the given directory.
/// Also installs a panic hook that logs backtraces.
pub fn init(data_dir: &Path) {
    let log_path = data_dir.join("powerplanpro.log");

    // Truncate old log on startup so it doesn't grow forever.
    match OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
    {
        Ok(file) => {
            if let Ok(mut guard) = LOG_FILE.lock() {
                *guard = Some(file);
            }
        }
        Err(e) => {
            eprintln!("[logging] failed to open log file: {e}");
            return;
        }
    }

    // Install the logger at Info level. leak the Box to get &'static.
    let logger: &'static FileLogger = Box::leak(Box::new(FileLogger));
    if log::set_logger(logger).is_err() {
        eprintln!("[logging] failed to set logger");
        return;
    }
    log::set_max_level(LevelFilter::Info);

    log::info!("logging initialized to {}", log_path.display());

    // Install panic hook that logs to the same file.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>");

        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Box<dyn Any>".to_string()
        };

        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());

        log::error!(
            "PANIC in thread '{thread_name}': {payload}\n  at {location}"
        );

        if std::env::var("RUST_BACKTRACE").is_ok() {
            log::error!("backtrace:\n{}", std::backtrace::Backtrace::force_capture());
        }

        // Also call the default hook so debug builds still print to stderr.
        default_hook(info);
    }));
}

/// Convert days since UNIX epoch to (year, month, day). Good enough for log timestamps.
fn days_to_date(days: u64) -> (u32, u32, u32) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as u32, m as u32, d as u32)
}
