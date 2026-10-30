pub const APP_DIR_NAME: &str = "starling";

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::app::NotificationMode;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    pub client_id: Option<String>,
    pub notification_mode: Option<NotificationMode>,
}

pub fn get_app_dir() -> Result<PathBuf> {
    let config_dir = dirs::config_dir().context("Could not find config directory")?;
    let app_dir = config_dir.join(APP_DIR_NAME);
    fs::create_dir_all(&app_dir)?;
    Ok(app_dir)
}

/// Writes `contents` to `path`, readable and writable by the owner only on Unix.
///
/// Tightens the mode of a file that already exists, so a token saved by an older build is fixed on the next write.
pub fn write_private(path: &Path, contents: &str) -> Result<()> {
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

pub fn load_config() -> Option<Config> {
    let app_dir = get_app_dir().ok()?;
    let config_path = app_dir.join("config.json");

    if !config_path.exists() {
        return None;
    }

    let json = fs::read_to_string(config_path).ok()?;
    serde_json::from_str(&json).ok()
}

pub fn save_config(config: &Config) -> Result<()> {
    let app_dir = get_app_dir()?;
    let config_path = app_dir.join("config.json");
    let json = serde_json::to_string_pretty(config)?;
    write_private(&config_path, &json)?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn write_private_sets_owner_only_mode() {
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
