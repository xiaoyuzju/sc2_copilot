use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppSettings {
    pub lead_time_seconds: u64,
    pub overlay_position: [f32; 2],
    pub hotkey: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            lead_time_seconds: 30,
            overlay_position: [24.0, 120.0],
            hotkey: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn for_current_user() -> Result<Self, SettingsError> {
        let app_data = std::env::var_os("APPDATA").ok_or(SettingsError::MissingAppData)?;
        Ok(Self::new(
            PathBuf::from(app_data)
                .join("SC2 Copilot")
                .join("settings.json"),
        ))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<AppSettings, SettingsError> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(AppSettings::default())
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn save(&self, settings: &AppSettings) -> Result<(), SettingsError> {
        let parent = self.path.parent().ok_or(SettingsError::MissingParent)?;
        fs::create_dir_all(parent)?;
        let temporary_path = self.path.with_extension("json.tmp");
        fs::write(&temporary_path, serde_json::to_vec_pretty(settings)?)?;
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        fs::rename(temporary_path, &self.path)?;
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("APPDATA is unavailable")]
    MissingAppData,
    #[error("settings path has no parent directory")]
    MissingParent,
    #[error("settings I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid settings JSON: {0}")]
    Json(#[from] serde_json::Error),
}
