use crate::metrics::{MetricsState, UsageSnapshot};
use crate::power::{PowerManager, PowerPlan};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BALANCED_GUID: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";
const POWER_SAVER_GUID: &str = "a1841308-3541-4fab-bc81-f71556f20b4a";
const MAX_WINDOW_SECONDS: usize = 900;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleConfig {
    pub enabled: bool,
    pub condition_mode: ConditionMode,
    pub cpu_threshold: f64,
    pub gpu_threshold: f64,
    pub duration_seconds: u32,
    pub cooldown_seconds: u32,
    pub high_load_plan_guid: String,
    pub low_load_plan_guid: String,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConditionMode {
    And,
    Or,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub active_plan_guid: String,
    pub avg_cpu: f64,
    pub avg_gpu: f64,
    pub condition_met: bool,
    pub target_plan_guid: String,
    pub switches_count: u64,
    pub paused: bool,
    pub resume_at_ms: Option<u64>,
}

impl Default for EngineStatus {
    fn default() -> Self {
        Self {
            active_plan_guid: String::new(),
            avg_cpu: 0.0,
            avg_gpu: 0.0,
            condition_met: false,
            target_plan_guid: String::new(),
            switches_count: 0,
            paused: false,
            resume_at_ms: None,
        }
    }
}

pub struct AppCore {
    metrics: MetricsState,
    power: PowerManager,
    config: Mutex<RuleConfig>,
    runtime: Mutex<RuntimeState>,
    data_dir: Mutex<Option<PathBuf>>,
    on_plan_changed: Mutex<Option<Box<dyn Fn(&str) + Send + Sync + 'static>>>,
}

impl AppCore {
    pub fn new() -> Result<Self, String> {
        let metrics = MetricsState::new()?;
        let power = PowerManager::new();

        let plans = power.list_plans()?;
        let active_guid = plans
            .iter()
            .find(|plan| plan.is_active)
            .map(|plan| plan.guid.clone())
            .unwrap_or_else(|| plans[0].guid.clone());

        let high_guid = select_plan_guid(&plans, BALANCED_GUID, &active_guid);
        let low_guid = select_plan_guid(&plans, POWER_SAVER_GUID, &active_guid);

        Ok(Self {
            metrics,
            power,
            data_dir: Mutex::new(None),
            on_plan_changed: Mutex::new(None),
            config: Mutex::new(RuleConfig {
                enabled: false,
                condition_mode: ConditionMode::Or,
                cpu_threshold: 70.0,
                gpu_threshold: 70.0,
                duration_seconds: 30,
                cooldown_seconds: 20,
                high_load_plan_guid: high_guid,
                low_load_plan_guid: low_guid,
            }),
            runtime: Mutex::new(RuntimeState {
                window: VecDeque::with_capacity(MAX_WINDOW_SECONDS),
                active_plan_guid: active_guid,
                avg_cpu: 0.0,
                avg_gpu: 0.0,
                condition_met: false,
                target_plan_guid: String::new(),
                last_switch_ms: 0,
                switches_count: 0,
                tick_count: 0,
                paused: false,
                resume_at_ms: None,
            }),
        })
    }

    pub fn set_data_dir(&self, dir: PathBuf) {
        if let Ok(mut guard) = self.data_dir.lock() {
            *guard = Some(dir);
        }
    }

    /// Apply a config loaded from disk; skips plan-existence check since the
    /// plans may differ slightly between sessions on the same machine.
    pub fn apply_persisted_config(&self, config: RuleConfig) {
        if validate_rule_config(&config).is_err() {
            return;
        }
        if let Ok(mut cfg) = self.config.lock() {
            *cfg = config;
        }
    }

    pub fn set_plan_changed_callback<F>(&self, f: F)
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        if let Ok(mut guard) = self.on_plan_changed.lock() {
            *guard = Some(Box::new(f));
        }
    }

    pub fn start_background(self: &Arc<Self>) {
        let core = Arc::clone(self);
        thread::spawn(move || {
            log::info!("background rule engine thread started");
            loop {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    core.tick()
                }));
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        log::error!("rule engine tick error: {e}");
                    }
                    Err(panic) => {
                        let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                            s.to_string()
                        } else if let Some(s) = panic.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "Box<dyn Any>".to_string()
                        };
                        log::error!("rule engine tick panicked: {msg}");
                        // Recover: sleep and continue rather than dying.
                    }
                }
                thread::sleep(Duration::from_secs(1));
            }
        });
    }

    /// Checks every 3 ticks (~3 s) whether an external actor changed the active
    /// power plan. Uses a direct registry read — no subprocess, negligible cost.
    /// Returns the new GUID if an external change is detected, else None.
    fn check_external_plan_change(&self) -> Option<String> {
        {
            let mut runtime = self.runtime.lock().ok()?;
            runtime.tick_count = runtime.tick_count.wrapping_add(1);
            if runtime.tick_count % 3 != 0 || runtime.active_plan_guid.is_empty() {
                return None;
            }
        }
        // Release the runtime lock before spawning the subprocess.
        let actual = crate::power::get_active_guid_fast()?;
        let mut runtime = self.runtime.lock().ok()?;
        if actual.eq_ignore_ascii_case(&runtime.active_plan_guid) {
            return None;
        }
        // External change — sync state and impose a cooldown to avoid
        // immediately overriding the externally-set plan.
        runtime.active_plan_guid = actual.clone();
        runtime.last_switch_ms = now_ms();
        Some(actual)
        // runtime lock released here
    }

    pub fn get_usage_snapshot(&self) -> Result<UsageSnapshot, String> {
        self.metrics.sample()
    }

    pub fn list_power_plans(&self) -> Result<Vec<PowerPlan>, String> {
        self.power.list_plans()
    }

    pub fn set_power_plan(&self, plan_guid: String) -> Result<(), String> {
        // Global debounce: skip any switch within 2 seconds of the last one.
        // Mark the switch as in-progress BEFORE calling powercfg so subsequent
        // calls see the updated timestamp even while the subprocess is running.
        {
            let mut runtime = self.runtime.lock().map_err(|_| "failed to lock engine runtime".to_string())?;
            let elapsed_ms = now_ms().saturating_sub(runtime.last_switch_ms);
            if elapsed_ms < 2000 {
                log::debug!("set_power_plan: debounce active ({elapsed_ms}ms since last switch, ignoring)");
                return Ok(());
            }
            runtime.last_switch_ms = now_ms();
            runtime.active_plan_guid = plan_guid.clone();
            runtime.switches_count = runtime.switches_count.saturating_add(1);
        }

        log::debug!("setting active power plan to {plan_guid}");
        self.power.set_active_plan(&plan_guid)?;
        self.fire_plan_changed(&plan_guid);
        Ok(())
    }

    pub fn get_rule_config(&self) -> Result<RuleConfig, String> {
        self.config
            .lock()
            .map_err(|_| "failed to lock rule config".to_string())
            .map(|config| config.clone())
    }

    pub fn set_rule_config(&self, config: RuleConfig) -> Result<(), String> {
        validate_rule_config(&config)?;

        let available = self.power.list_plans()?;
        ensure_guid_exists(&available, &config.high_load_plan_guid)?;
        ensure_guid_exists(&available, &config.low_load_plan_guid)?;

        let mut cfg = self
            .config
            .lock()
            .map_err(|_| "failed to lock rule config".to_string())?;
        *cfg = config.clone();
        drop(cfg);
        self.try_save_config(&config);
        Ok(())
    }

    fn try_save_config(&self, config: &RuleConfig) {
        if let Ok(guard) = self.data_dir.lock() {
            if let Some(dir) = guard.as_ref() {
                if let Err(e) = crate::persist::save(dir, config) {
                    log::error!("failed to save config: {e}");
                }
            }
        }
    }

    pub fn get_engine_status(&self) -> Result<EngineStatus, String> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "failed to lock engine runtime".to_string())?;

        Ok(EngineStatus {
            active_plan_guid: runtime.active_plan_guid.clone(),
            avg_cpu: runtime.avg_cpu,
            avg_gpu: runtime.avg_gpu,
            condition_met: runtime.condition_met,
            target_plan_guid: runtime.target_plan_guid.clone(),
            switches_count: runtime.switches_count,
            paused: runtime.paused,
            resume_at_ms: runtime.resume_at_ms,
        })
    }

    pub fn pause_engine(&self, minutes: u32) -> Result<(), String> {
        if minutes == 0 || minutes > 30 {
            return Err("pause minutes must be in range 1..30".to_string());
        }
        let resume_at = now_ms() + (minutes as u64 * 60 * 1000);
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "failed to lock engine runtime".to_string())?;
        runtime.paused = true;
        runtime.resume_at_ms = Some(resume_at);
        Ok(())
    }

    pub fn unpause_engine(&self) -> Result<(), String> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "failed to lock engine runtime".to_string())?;
        runtime.paused = false;
        runtime.resume_at_ms = None;
        Ok(())
    }

    fn tick(&self) -> Result<(), String> {
        // Detect external plan changes (e.g. user changes via Control Panel).
        // The lock is released before the callback fires — no deadlock risk.
        if let Some(guid) = self.check_external_plan_change() {
            self.fire_plan_changed(&guid);
        }

        let snapshot = self.metrics.sample()?;
        let config = self
            .config
            .lock()
            .map_err(|_| "failed to lock rule config".to_string())?
            .clone();

        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "failed to lock engine runtime".to_string())?;

        // Auto-resume if the pause duration has elapsed.
        if runtime.paused {
            if let Some(resume_at) = runtime.resume_at_ms {
                if now_ms() >= resume_at {
                    log::info!("pause expired, resuming rule engine");
                    runtime.paused = false;
                    runtime.resume_at_ms = None;
                }
            }
        }

        runtime.window.push_back(snapshot);
        while runtime.window.len() > MAX_WINDOW_SECONDS {
            runtime.window.pop_front();
        }

        let sample_count = config.duration_seconds.max(1) as usize;
        let (avg_cpu, avg_gpu) = average_tail(&runtime.window, sample_count);
        runtime.avg_cpu = avg_cpu;
        runtime.avg_gpu = avg_gpu;

        let cpu_match = avg_cpu >= config.cpu_threshold;
        let gpu_match = avg_gpu >= config.gpu_threshold;
        let condition_met = match config.condition_mode {
            ConditionMode::And => cpu_match && gpu_match,
            ConditionMode::Or => cpu_match || gpu_match,
        };

        runtime.condition_met = condition_met;
        let target_plan_guid = if condition_met {
            config.high_load_plan_guid.clone()
        } else {
            config.low_load_plan_guid.clone()
        };
        runtime.target_plan_guid = target_plan_guid.clone();

        // Skip rule execution when paused or disabled.
        if runtime.paused || !config.enabled {
            return Ok(());
        }

        // Release the runtime lock before any subprocess calls.
        let active_guid = runtime.active_plan_guid.clone();
        drop(runtime);

        if active_guid.is_empty() {
            let discovered = self.power.get_active_plan_guid()?;
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| "failed to lock engine runtime".to_string())?;
            if runtime.active_plan_guid.is_empty() {
                runtime.active_plan_guid = discovered;
            }
        }

        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "failed to lock engine runtime".to_string())?;

        if runtime.active_plan_guid.eq_ignore_ascii_case(&target_plan_guid) {
            return Ok(());
        }

        if config.cooldown_seconds > 0 {
            let elapsed_ms = now_ms().saturating_sub(runtime.last_switch_ms);
            if elapsed_ms < (config.cooldown_seconds as u64 * 1000) {
                return Ok(());
            }
        }

        // Mark the switch as in-progress BEFORE calling powercfg so the
        // debounce guard works even while the subprocess is running.
        runtime.active_plan_guid = target_plan_guid.clone();
        runtime.last_switch_ms = now_ms();
        runtime.switches_count = runtime.switches_count.saturating_add(1);
        drop(runtime);
        log::info!("switching power plan: {target_plan_guid} (cpu_avg={avg_cpu:.1}, gpu_avg={avg_gpu:.1})");
        self.power.set_active_plan(&target_plan_guid)?;

        self.fire_plan_changed(&target_plan_guid);
        Ok(())
    }

    fn fire_plan_changed(&self, guid: &str) {
        if let Ok(guard) = self.on_plan_changed.lock() {
            if let Some(cb) = guard.as_ref() {
                cb(guid);
            }
        }
    }
}

#[derive(Debug)]
struct RuntimeState {
    window: VecDeque<UsageSnapshot>,
    active_plan_guid: String,
    avg_cpu: f64,
    avg_gpu: f64,
    condition_met: bool,
    target_plan_guid: String,
    last_switch_ms: u64,
    switches_count: u64,
    tick_count: u64,
    paused: bool,
    resume_at_ms: Option<u64>,
}

fn validate_rule_config(config: &RuleConfig) -> Result<(), String> {
    if !(0.0..=100.0).contains(&config.cpu_threshold) {
        return Err("cpuThreshold must be in range 0..100".to_string());
    }
    if !(0.0..=100.0).contains(&config.gpu_threshold) {
        return Err("gpuThreshold must be in range 0..100".to_string());
    }
    if config.duration_seconds == 0 || config.duration_seconds as usize > MAX_WINDOW_SECONDS {
        return Err(format!(
            "durationSeconds must be in range 1..{}",
            MAX_WINDOW_SECONDS
        ));
    }
    if config.cooldown_seconds > 3600 {
        return Err("cooldownSeconds must be <= 3600".to_string());
    }
    if config.high_load_plan_guid.eq_ignore_ascii_case(&config.low_load_plan_guid) {
        return Err("highLoadPlanGuid and lowLoadPlanGuid must be different".to_string());
    }

    Ok(())
}

fn ensure_guid_exists(plans: &[PowerPlan], guid: &str) -> Result<(), String> {
    let exists = plans.iter().any(|plan| plan.guid.eq_ignore_ascii_case(guid));
    if exists {
        Ok(())
    } else {
        Err(format!("power plan not found: {guid}"))
    }
}

fn select_plan_guid(plans: &[PowerPlan], preferred_guid: &str, fallback_guid: &str) -> String {
    if let Some(plan) = plans
        .iter()
        .find(|plan| plan.guid.eq_ignore_ascii_case(preferred_guid))
    {
        return plan.guid.clone();
    }

    if let Some(plan) = plans
        .iter()
        .find(|plan| plan.guid.eq_ignore_ascii_case(fallback_guid))
    {
        return plan.guid.clone();
    }

    plans[0].guid.clone()
}

fn average_tail(window: &VecDeque<UsageSnapshot>, take_count: usize) -> (f64, f64) {
    let mut cpu_sum = 0.0;
    let mut gpu_sum = 0.0;
    let mut count = 0usize;

    for item in window.iter().rev().take(take_count) {
        cpu_sum += item.cpu_usage;
        gpu_sum += item.gpu_usage;
        count += 1;
    }

    if count == 0 {
        return (0.0, 0.0);
    }

    (cpu_sum / count as f64, gpu_sum / count as f64)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}
