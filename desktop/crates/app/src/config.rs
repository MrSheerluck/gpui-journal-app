//! Nonsecret app configuration in the OS config directory (charter §2).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

/// The app's configuration, persisted as typed TOML.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Config {
    /// The journal folder chosen during onboarding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journal_root: Option<PathBuf>,
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

    use super::{Config, load_from};

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
}
