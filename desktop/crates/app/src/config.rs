//! Nonsecret app configuration in the OS config directory (charter §2).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

/// Which palette the app paints in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    /// Follow the system appearance.
    #[default]
    System,
    Light,
    Dark,
}

/// The app's configuration, persisted as typed TOML.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Config {
    /// The journal folder chosen during onboarding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journal_root: Option<PathBuf>,
    /// Whether days without an entry appear in the journal timeline.
    #[serde(default = "default_show_empty_days")]
    pub show_empty_days: bool,
    /// The colour mode; defaults to following the system.
    #[serde(default)]
    pub theme: ThemePreference,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            journal_root: None,
            show_empty_days: default_show_empty_days(),
            theme: ThemePreference::default(),
        }
    }
}

fn default_show_empty_days() -> bool {
    true
}

/// Path of `config.toml` under the OS config directory, if there is one.
pub fn path() -> Option<PathBuf> {
    ProjectDirs::from("dev", "smbl", "Dainiki").map(|dirs| dirs.config_dir().join("config.toml"))
}

/// Loads the config; a missing or unreadable file means defaults.
pub fn load() -> Config {
    let Some(path) = path() else {
        return Config::default();
    };
    match load_from(&path) {
        Ok(config) => config,
        Err(error) => {
            tracing::warn!(path = %path.display(), error = %error, "config unreadable; using defaults");
            Config::default()
        }
    }
}

/// Writes the config atomically, creating the config directory as needed.
pub fn save(config: &Config) -> io::Result<()> {
    let path = path().ok_or_else(|| io::Error::other("no config directory on this platform"))?;
    save_to(&path, config)
}

fn save_to(path: &Path, config: &Config) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(config)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    crate::persistence::write_atomic(path, &text)
}

fn load_from(path: &Path) -> io::Result<Config> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(error) => return Err(error),
    };
    toml::from_str(&text).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{Config, load_from, save_to};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn scratch_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dainiki-config-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn missing_file_is_defaults() {
        let dir = scratch_dir();
        let config = load_from(&dir.join("config.toml")).expect("load");
        assert_eq!(config, Config::default());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn journal_root_roundtrips() {
        let dir = scratch_dir();
        let path = dir.join("config.toml");
        fs::write(&path, "journal_root = \"/home/user/Journal\"\n").expect("seed");
        let config = load_from(&path).expect("load");
        assert_eq!(
            config.journal_root.as_deref(),
            Some(PathBuf::from("/home/user/Journal").as_path())
        );
        assert!(config.show_empty_days, "empty days default to visible");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn show_empty_days_roundtrips() {
        let dir = scratch_dir();
        let path = dir.join("config.toml");
        fs::write(&path, "show_empty_days = false\n").expect("seed");
        let config = load_from(&path).expect("load");
        assert!(!config.show_empty_days);
        assert_eq!(
            config.theme,
            super::ThemePreference::System,
            "default theme"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn theme_roundtrips_and_defaults_to_system() {
        let dir = scratch_dir();
        let path = dir.join("config.toml");
        fs::write(&path, "theme = \"dark\"\n").expect("seed");
        let config = load_from(&path).expect("load");
        assert_eq!(config.theme, super::ThemePreference::Dark);

        let config = Config {
            journal_root: None,
            show_empty_days: true,
            theme: super::ThemePreference::Light,
        };
        save_to(&path, &config).expect("save");
        assert_eq!(load_from(&path).expect("load"), config);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_file_is_defaults() {
        let dir = scratch_dir();
        let path = dir.join("config.toml");
        fs::write(&path, "").expect("seed");
        let config = load_from(&path).expect("load");
        assert_eq!(config, Config::default());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_file_is_an_error() {
        let dir = scratch_dir();
        let path = dir.join("config.toml");
        fs::write(&path, "journal_root = [not valid\n").expect("seed");
        let error = load_from(&path).expect_err("corrupt config must not load");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_to_then_load_from_roundtrips() {
        let dir = scratch_dir();
        let path = dir.join("nested/config.toml");
        let config = Config {
            journal_root: Some(PathBuf::from("/journals/dainiki")),
            show_empty_days: false,
            theme: super::ThemePreference::Dark,
        };
        save_to(&path, &config).expect("save");
        assert_eq!(load_from(&path).expect("load"), config);
        fs::remove_dir_all(&dir).ok();
    }
}
