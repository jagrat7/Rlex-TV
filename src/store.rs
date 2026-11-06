//! Where Rlex keeps its files, and the small JSON documents it saves there.

use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::addons::Addon;

/// User preferences: library folders, installed sources, the download folder
/// and the OMDb key.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub library_folders: Vec<PathBuf>,
    /// Stremio addons the user added. Rlex ships with none.
    pub addons: Vec<Addon>,
    pub download_dir: Option<PathBuf>,
    /// The user's own OMDb key, for Rotten Tomatoes and Metacritic scores.
    #[serde(default)]
    pub omdb_api_key: Option<String>,
}

impl Settings {
    pub fn download_dir(&self) -> PathBuf {
        self.download_dir.clone().unwrap_or_else(|| {
            dirs::download_dir()
                .or_else(dirs::home_dir)
                .unwrap_or_else(|| PathBuf::from("."))
                .join("Rlex")
        })
    }
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rlex")
}

/// Per-session sockets live here (mpv IPC).
pub fn runtime_dir() -> PathBuf {
    dirs::runtime_dir().unwrap_or_else(std::env::temp_dir)
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// Reads `name` from the data folder, or returns the default when it doesn't
/// exist yet. An unreadable file is moved aside rather than overwritten later.
pub fn load<T: DeserializeOwned + Default>(name: &str) -> T {
    let path = data_dir().join(name);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|err| {
            let backup = path.with_extension(format!("broken-{}", now()));
            log::warn!(
                "{} is unreadable ({err}); keeping it as {}",
                path.display(),
                backup.display()
            );
            let _ = std::fs::rename(&path, &backup);
            T::default()
        }),
        Err(err) if err.kind() == ErrorKind::NotFound => T::default(),
        Err(err) => {
            log::warn!("can't read {}: {err}", path.display());
            T::default()
        }
    }
}

static READ_ONLY: AtomicBool = AtomicBool::new(false);

/// Stops [`save`] from writing anything, for headless renders: a preview reads
/// the real library and history but must never change them.
pub fn set_read_only() {
    READ_ONLY.store(true, Ordering::Relaxed);
}

/// Writes `value` to `name` in the data folder, replacing it atomically.
pub fn save<T: Serialize>(name: &str, value: &T) -> anyhow::Result<()> {
    if READ_ONLY.load(Ordering::Relaxed) {
        return Ok(());
    }
    let dir = data_dir();
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("{name}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(&tmp, dir.join(name))?;
    Ok(())
}
