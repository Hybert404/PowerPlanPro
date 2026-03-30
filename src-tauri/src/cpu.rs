use sysinfo::System;
use std::sync::Mutex;
use lazy_static::lazy_static;

lazy_static! {
    static ref SYS: Mutex<System> = Mutex::new(System::new_all());
}

/// Get current CPU usage as a percentage (0-100)
pub fn get_cpu_usage() -> f32 {
    let mut sys = SYS.lock().unwrap();
    sys.refresh_cpu_usage();

    let global_load = sys.global_cpu_usage();
    global_load.min(100.0)
}

/// Get per-core CPU usage
pub fn get_cpu_usage_per_core() -> Vec<f32> {
    let mut sys = SYS.lock().unwrap();
    sys.refresh_cpu_usage();

    sys.cpus()
        .iter()
        .map(|cpu| cpu.cpu_usage().min(100.0))
        .collect()
}
