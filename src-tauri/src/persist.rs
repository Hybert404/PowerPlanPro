use crate::engine::RuleConfig;
use std::path::Path;

const CONFIG_FILE: &str = "rule_config.json";

pub fn load(dir: &Path) -> Option<RuleConfig> {
    let data = std::fs::read_to_string(dir.join(CONFIG_FILE)).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn save(dir: &Path, config: &RuleConfig) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("failed to create config directory: {e}"))?;
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("failed to serialize config: {e}"))?;
    std::fs::write(dir.join(CONFIG_FILE), json)
        .map_err(|e| format!("failed to write config file: {e}"))?;
    Ok(())
}
