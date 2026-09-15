use crate::engine::RuleConfig;
use std::path::Path;

const CONFIG_FILE: &str = "rule_config.json";

pub fn load(dir: &Path) -> Option<RuleConfig> {
    let path = dir.join(CONFIG_FILE);
    let data = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str(&data) {
        Ok(config) => {
            log::info!("loaded config from {}", path.display());
            Some(config)
        }
        Err(e) => {
            log::warn!("failed to parse config from {}: {e}", path.display());
            None
        }
    }
}

pub fn save(dir: &Path, config: &RuleConfig) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("failed to create config directory: {e}"))?;
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("failed to serialize config: {e}"))?;
    let path = dir.join(CONFIG_FILE);
    std::fs::write(&path, &json)
        .map_err(|e| format!("failed to write config file: {e}"))?;
    log::debug!("saved config to {}", path.display());
    Ok(())
}
