use std::fs;
use std::path::{Path, PathBuf};

use clipboardx_model::{AppConfig, ClipboardXError};

/// Loads and persists `AppConfig` atomically as TOML.
#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_or_default(&self) -> Result<AppConfig, ClipboardXError> {
        if !self.path.exists() {
            let defaults = AppConfig::default();
            self.save(&defaults)?;
            return Ok(defaults);
        }
        let raw = fs::read_to_string(&self.path)
            .map_err(|err| ClipboardXError::Config(format!("read config failed: {err}")))?;
        match toml::from_str(&raw) {
            Ok(cfg) => Ok(cfg),
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "config.toml schema mismatch; resetting to current defaults"
                );
                let defaults = AppConfig::default();
                let _ = self.save(&defaults);
                Ok(defaults)
            }
        }
    }

    pub fn save(&self, config: &AppConfig) -> Result<(), ClipboardXError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                ClipboardXError::Config(format!("create config dir failed: {err}"))
            })?;
        }
        let encoded = toml::to_string_pretty(config)
            .map_err(|err| ClipboardXError::Config(format!("serialize config failed: {err}")))?;
        let tmp = self.path.with_extension("toml.tmp");
        fs::write(&tmp, encoded)
            .map_err(|err| ClipboardXError::Config(format!("write temp config failed: {err}")))?;
        fs::rename(&tmp, &self.path)
            .map_err(|err| ClipboardXError::Config(format!("replace config.toml failed: {err}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip() {
        let dir = std::env::temp_dir().join(format!("clipboardx_cfg_{}", std::process::id()));
        let store = ConfigStore::new(dir.join("config.toml"));
        let mut cfg = store.load_or_default().unwrap();
        cfg.retention.max_items = 5000;
        store.save(&cfg).unwrap();

        let reloaded = store.load_or_default().unwrap();
        assert_eq!(reloaded.retention.max_items, 5000);
        let _ = fs::remove_dir_all(dir);
    }
}
