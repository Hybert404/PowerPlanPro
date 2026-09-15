use serde::Serialize;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use windows::core::PCWSTR;
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhGetFormattedCounterValue, PdhOpenQueryW, PDH_FMT_COUNTERVALUE, PDH_FMT_COUNTERVALUE_ITEM_W,
    PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub cpu_usage: f64,
    pub gpu_usage: f64,
    pub timestamp_ms: u64,
}

pub struct MetricsState {
    monitor: Mutex<UsageMonitor>,
}

impl MetricsState {
    pub fn new() -> Result<Self, String> {
        log::info!("initializing PDH metrics monitor");
        let monitor = UsageMonitor::new()?;
        log::info!("PDH metrics initialized (gpu_counter: {})", monitor.gpu_counter.is_some());
        Ok(Self {
            monitor: Mutex::new(monitor),
        })
    }

    pub fn sample(&self) -> Result<UsageSnapshot, String> {
        let mut monitor = self
            .monitor
            .lock()
            .map_err(|_| "failed to lock metrics monitor".to_string())?;
        monitor.sample()
    }
}

struct UsageMonitor {
    query: PDH_HQUERY,
    cpu_counter: PDH_HCOUNTER,
    gpu_counter: Option<PDH_HCOUNTER>,
}

unsafe impl Send for UsageMonitor {}

impl UsageMonitor {
    fn new() -> Result<Self, String> {
        let mut query = PDH_HQUERY::default();
        let open_status = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) };
        ensure_ok(open_status, "open PDH query")?;

        let cpu_path = to_wide("\\Processor Information(_Total)\\% Processor Utility");
        let mut cpu_counter = PDH_HCOUNTER::default();
        let cpu_status = unsafe {
            PdhAddEnglishCounterW(
                query,
                PCWSTR(cpu_path.as_ptr()),
                0,
                &mut cpu_counter,
            )
        };
        ensure_ok(cpu_status, "add CPU counter")?;

        // _Total is not an aggregate instance for GPU Engine — use wildcard
        // and read the result array with PdhGetFormattedCounterArrayW.
        let gpu_path = to_wide("\\GPU Engine(*)\\Utilization Percentage");
        let mut gpu_counter = PDH_HCOUNTER::default();
        let gpu_status = unsafe {
            PdhAddEnglishCounterW(
                query,
                PCWSTR(gpu_path.as_ptr()),
                0,
                &mut gpu_counter,
            )
        };

        let first_collect_status = unsafe { PdhCollectQueryData(query) };
        ensure_ok(first_collect_status, "prime PDH query")?;

        Ok(Self {
            query,
            cpu_counter,
            gpu_counter: if gpu_status == 0 {
                Some(gpu_counter)
            } else {
                None
            },
        })
    }

    fn sample(&mut self) -> Result<UsageSnapshot, String> {
        let collect_start = std::time::Instant::now();
        let collect_status = unsafe { PdhCollectQueryData(self.query) };
        let collect_elapsed = collect_start.elapsed();
        if collect_elapsed.as_millis() > 500 {
            log::warn!("PdhCollectQueryData took {collect_elapsed:?} (possible bottleneck)");
        }
        ensure_ok(collect_status, "collect PDH data")?;

        let cpu_usage = read_counter_value(self.cpu_counter).unwrap_or(0.0).clamp(0.0, 100.0);
        let gpu_usage = if let Some(counter) = self.gpu_counter {
            read_gpu_max_engine(counter)
        } else {
            0.0
        };

        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "failed to read system clock".to_string())?
            .as_millis() as u64;

        Ok(UsageSnapshot {
            cpu_usage,
            gpu_usage,
            timestamp_ms,
        })
    }
}

impl Drop for UsageMonitor {
    fn drop(&mut self) {
        let _ = unsafe { PdhCloseQuery(self.query) };
    }
}

/// Returns a GPU utilization figure that matches what Task Manager reports.
///
/// The `GPU Engine(*)` wildcard produces one PDH item per (process, engine-type)
/// pair. Task Manager shows the **busiest engine type**: for each engine type
/// (3D, Copy, VideoDecode, …) it sums contributions from all processes, then
/// takes the maximum across engine types. Summing everything together (as a
/// naive implementation does) inflates the number significantly.
fn read_gpu_max_engine(counter: PDH_HCOUNTER) -> f64 {
    use std::collections::HashMap;

    let mut buf_size: u32 = 0;
    let mut item_count: u32 = 0;
    // Probe: fills buf_size and item_count; return value is PDH_MORE_DATA, not an error.
    unsafe {
        PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut buf_size,
            &mut item_count,
            None,
        )
    };
    if item_count == 0 || buf_size == 0 {
        return 0.0;
    }
    let mut buf: Vec<u8> = vec![0u8; buf_size as usize];
    let status = unsafe {
        PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut buf_size,
            &mut item_count,
            Some(buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W),
        )
    };
    if status != 0 {
        return 0.0;
    }
    let stride = std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();

    // Sum per-process values within each engine type.
    let mut engine_sums: HashMap<String, f64> = HashMap::new();
    for i in 0..item_count as usize {
        let offset = i * stride;
        if offset + stride > buf.len() {
            break;
        }
        // SAFETY: offset and stride verified above; buf contains PDH_FMT_COUNTERVALUE_ITEM_W.
        let item =
            unsafe { &*(buf.as_ptr().add(offset) as *const PDH_FMT_COUNTERVALUE_ITEM_W) };
        // CStatus 0 = PDH_CSTATUS_VALID_DATA; 1 = PDH_CSTATUS_NEW_DATA — both valid.
        if item.FmtValue.CStatus > 1 {
            continue;
        }
        let value = unsafe { item.FmtValue.Anonymous.doubleValue };
        // Instance names look like: "pid_1234_luid_0x00000000_0x0BDXGI_engtype_3D"
        let engtype = unsafe {
            let ptr = item.szName.0;
            if ptr.is_null() {
                "Unknown".to_string()
            } else {
                let len = (0usize..)
                    .take_while(|&j| *ptr.add(j) != 0)
                    .count();
                let name = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
                if let Some(pos) = name.find("engtype_") {
                    name[pos + 8..].to_string()
                } else {
                    "Unknown".to_string()
                }
            }
        };
        *engine_sums.entry(engtype).or_insert(0.0) += value;
    }

    // The displayed figure is the busiest engine type, capped at 100 %.
    engine_sums
        .values()
        .cloned()
        .fold(0.0f64, f64::max)
        .clamp(0.0, 100.0)
}

fn read_counter_value(counter: PDH_HCOUNTER) -> Result<f64, String> {
    let mut counter_type = 0u32;
    let mut value = PDH_FMT_COUNTERVALUE::default();
    let status = unsafe {
        PdhGetFormattedCounterValue(counter, PDH_FMT_DOUBLE, Some(&mut counter_type), &mut value)
    };
    ensure_ok(status, "read counter value")?;

    let number = unsafe { value.Anonymous.doubleValue };
    Ok(number)
}

fn ensure_ok(status: u32, action: &str) -> Result<(), String> {
    if status == 0 {
        Ok(())
    } else {
        Err(format!("failed to {action} (PDH status: 0x{status:08X})"))
    }
}

fn to_wide(input: &str) -> Vec<u16> {
    input.encode_utf16().chain(std::iter::once(0)).collect()
}
