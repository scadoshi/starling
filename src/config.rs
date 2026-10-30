//! `~/.config/starling/`: the config file, and owner-only writes for everything stored there.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::app::NotificationMode;

const APP_DIR_NAME: &str = "starling";

#[derive(Debug, Serialize, Deserialize, Default)]
pub(crate) struct Config {
    /// The Azure app registration to sign in with. `CLIENT_ID` in the environment wins over it.
    pub(crate) client_id: Option<String>,
    pub(crate) notification_mode: Option<NotificationMode>,
}

/// `~/.config/starling` on Linux, `~/Library/Application Support/starling` on macOS, created owner-only if missing.
pub(crate) fn app_dir() -> Result<PathBuf> {
    let app_dir = dirs::config_dir()
        .context("Could not find config directory")?
        .join(APP_DIR_NAME);
    if !app_dir.exists() {
        fs::create_dir_all(&app_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&app_dir, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(app_dir)
}

/// Writes `contents` to `path`, readable and writable by the owner only on Unix.
///
/// Tightens the mode of a file that already exists before writing to it.
pub(crate) fn write_private(path: &Path, contents: &str) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        if path.exists() {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
    }
    options.open(path)?.write_all(contents.as_bytes())?;
    Ok(())
}

/// The config file, or `None` when it is missing or unreadable.
pub(crate) fn load_config() -> Option<Config> {
    let json = fs::read_to_string(app_dir().ok()?.join("config.json")).ok()?;
    serde_json::from_str(&json).ok()
}

/// Stores the client ID in `config.json`, keeping the other settings.
pub(crate) fn save_client_id(client_id: &str) -> Result<()> {
    let mut config = load_config().unwrap_or_default();
    config.client_id = Some(client_id.to_owned());
    save_config(&config)
}

pub(crate) fn save_config(config: &Config) -> Result<()> {
    write_private(
        &app_dir()?.join("config.json"),
        &serde_json::to_string_pretty(config)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_and_tolerates_missing_fields() {
        let config: Config = serde_json::from_str(r#"{"client_id":"abc"}"#).unwrap();
        assert_eq!(config.client_id.as_deref(), Some("abc"));
        assert!(config.notification_mode.is_none());

        let saved = serde_json::to_string(&Config {
            client_id: None,
            notification_mode: Some(NotificationMode::Both),
        })
        .unwrap();
        let loaded: Config = serde_json::from_str(&saved).unwrap();
        assert_eq!(loaded.notification_mode, Some(NotificationMode::Both));
    }

    #[cfg(unix)]
    #[test]
    fn write_private_sets_owner_only_mode() {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!("starling-private-{}", std::process::id()));
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        write_private(&path, "new").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let contents = fs::read_to_string(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(mode, 0o600);
        assert_eq!(contents, "new");
    }
}
