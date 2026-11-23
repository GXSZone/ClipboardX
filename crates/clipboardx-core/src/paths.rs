use std::env;
use std::path::PathBuf;

use clipboardx_model::ClipboardXError;

/// Canonical filesystem layout for ClipboardX runtime data and configuration.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub config_path: PathBuf,
    pub history_db_path: PathBuf,
    pub vault_db_path: PathBuf,
    pub blobs_dir: PathBuf,
    pub plugins_dir: PathBuf,
    pub updates_dir: PathBuf,
    pub logs_dir: PathBuf,
}

impl AppPaths {
    /// Resolves OS-standard user directories without extra directory crates.
    pub fn resolve_default() -> Result<Self, ClipboardXError> {
        let base = resolve_os_data_root()?;
        Ok(Self::from_root(base))
    }

    pub fn from_root(root: PathBuf) -> Self {
        Self {
            config_path: root.join("config.toml"),
            history_db_path: root.join("history.db"),
            vault_db_path: root.join("vault.db"),
            blobs_dir: root.join("blobs"),
            plugins_dir: root.join("plugins"),
            updates_dir: root.join("updates"),
            logs_dir: root.join("logs"),
            data_dir: root,
        }
    }
}

fn resolve_os_data_root() -> Result<PathBuf, ClipboardXError> {
    #[cfg(target_os = "windows")]
    {
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            return Ok(PathBuf::from(local).join("ClipboardX"));
        }
        if let Some(profile) = env::var_os("USERPROFILE") {
            return Ok(PathBuf::from(profile)
                .join("AppData")
                .join("Local")
                .join("ClipboardX"));
        }
        Err(ClipboardXError::Config(
            "neither LOCALAPPDATA nor USERPROFILE is set".to_owned(),
        ))
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = env::var_os("HOME") {
            return Ok(PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("ClipboardX"));
        }
        Err(ClipboardXError::Config("HOME is not set".to_owned()))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(xdg) = env::var_os("XDG_DATA_HOME")
            && !xdg.is_empty()
        {
            return Ok(PathBuf::from(xdg).join("clipboardx"));
        }
        if let Some(home) = env::var_os("HOME") {
            return Ok(PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("clipboardx"));
        }
        Err(ClipboardXError::Config("HOME is not set".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_subpaths_under_root() {
        let paths = AppPaths::from_root(PathBuf::from("test_root"));
        assert_eq!(paths.history_db_path, PathBuf::from("test_root/history.db"));
        assert_eq!(paths.blobs_dir, PathBuf::from("test_root/blobs"));
    }
}
