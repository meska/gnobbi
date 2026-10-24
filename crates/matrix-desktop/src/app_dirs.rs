//! Per-platform config/data/cache directories for the app.
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Linux,
    MacOs,
    Windows,
}

impl Platform {
    pub const CURRENT: Platform = if cfg!(target_os = "macos") {
        Platform::MacOs
    } else if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Linux
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppDirs {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
}

/// Absolute, non-empty environment value; anything else counts as unset (XDG rule).
fn absolute(env: &impl Fn(&str) -> Option<String>, key: &str) -> Option<PathBuf> {
    let value = env(key).filter(|v| !v.is_empty())?;
    Path::new(&value).is_absolute().then(|| PathBuf::from(value))
}

/// Directories for `platform`, or `None` when the base variables are missing or unusable.
pub fn app_dirs(platform: Platform, env: impl Fn(&str) -> Option<String>) -> Option<AppDirs> {
    match platform {
        Platform::Linux => {
            let home = absolute(&env, "HOME");
            let pick = |var: &str, fallback: &str| {
                absolute(&env, var)
                    .or_else(|| home.as_ref().map(|h| h.join(fallback)))
                    .map(|base| base.join("gnobbi"))
            };
            Some(AppDirs {
                config: pick("XDG_CONFIG_HOME", ".config")?,
                data: pick("XDG_DATA_HOME", ".local/share")?,
                cache: pick("XDG_CACHE_HOME", ".cache")?,
            })
        }
        Platform::MacOs => {
            let library = absolute(&env, "HOME")?.join("Library");
            let support = library.join("Application Support").join("Gnobbi");
            Some(AppDirs {
                config: support.clone(),
                data: support,
                cache: library.join("Caches").join("Gnobbi"),
            })
        }
        Platform::Windows => {
            let local = absolute_windows(&env, "LOCALAPPDATA")?.join("Gnobbi");
            Some(AppDirs {
                config: absolute_windows(&env, "APPDATA")?.join("Gnobbi"),
                cache: local.join("cache"),
                data: local,
            })
        }
    }
}

// shortcut: a Windows path is only checked for being non-empty, because `is_absolute` is host-specific; fine while the variables come from Windows itself.
fn absolute_windows(env: &impl Fn(&str) -> Option<String>, key: &str) -> Option<PathBuf> {
    env(key).filter(|v| !v.is_empty()).map(PathBuf::from)
}

impl AppDirs {
    /// Create all three directories, owner-only where the platform supports it.
    pub fn ensure(&self) -> std::io::Result<()> {
        for dir in [&self.config, &self.data, &self.cache] {
            #[cfg(unix)]
            crate::create_private_dir(dir)?;
            #[cfg(not(unix))]
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    /// Regenerable thumbnail cache, created on demand.
    pub fn thumbnails(&self) -> std::io::Result<PathBuf> {
        let dir = self.cache.join("thumbnails");
        #[cfg(unix)]
        crate::create_private_dir(&dir)?;
        #[cfg(not(unix))]
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}
