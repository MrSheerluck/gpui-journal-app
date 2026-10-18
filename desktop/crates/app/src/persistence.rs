//! Durable journal files: reads and atomic writes.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Loads a UTF-8 journal file. `Ok(None)` means the file does not exist yet,
/// which is normal for a day whose entry has not been started.
pub fn load(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Writes `text` to a temporary file in the same directory, flushes it, and
/// renames it into place, so a crash can never leave a half-written journal
/// file behind.
pub fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{file_name}.dainiki.tmp"));
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{load, write_atomic};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn scratch_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dainiki-persistence-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn load_reports_missing_files_as_none() {
        let dir = scratch_dir();
        assert_eq!(load(&dir.join("absent.md")).unwrap(), None);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_then_load_roundtrips_and_leaves_no_tmp() {
        let dir = scratch_dir();
        let path = dir.join("2026-10-18.md");
        write_atomic(&path, "# hello\n日本語 🎉\n").expect("write");
        assert_eq!(
            load(&path).unwrap().as_deref(),
            Some("# hello\n日本語 🎉\n")
        );
        assert!(!dir.join(".2026-10-18.md.dainiki.tmp").exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_replaces_existing_content() {
        let dir = scratch_dir();
        let path = dir.join("entry.md");
        write_atomic(&path, "first").expect("write");
        write_atomic(&path, "second").expect("overwrite");
        assert_eq!(load(&path).unwrap().as_deref(), Some("second"));
        fs::remove_dir_all(&dir).ok();
    }
}
