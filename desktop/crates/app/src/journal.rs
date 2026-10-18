//! The journal folder: entry layout and today's entry.
//!
//! Entries live at `root/YYYY/MM/YYYY-MM-DD.md`, with media beside them in
//! `root/YYYY/MM/assets/`. The folder of Markdown files is the authority;
//! everything else in the app is derived from it.

use std::fs;
use std::io;
use std::path::PathBuf;

use chrono::{Datelike, NaiveDate};

/// A journal rooted at a user-chosen folder.
#[derive(Debug, Clone)]
pub struct Journal {
    root: PathBuf,
}

impl Journal {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The entry path for `date`: `root/YYYY/MM/YYYY-MM-DD.md`.
    pub fn entry_path(&self, date: NaiveDate) -> PathBuf {
        self.root
            .join(date.year().to_string())
            .join(format!("{:02}", date.month()))
            .join(format!("{date}.md"))
    }

    /// Makes sure the entry's folders and file exist, then returns the entry
    /// path. An existing entry is never touched, so this is safe to call on
    /// every launch.
    pub fn ensure_entry(&self, date: NaiveDate) -> io::Result<PathBuf> {
        let path = self.entry_path(date);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(path),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::NaiveDate;

    use super::Journal;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn scratch_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dainiki-journal-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid test date")
    }

    #[test]
    fn entry_path_matches_the_layout() {
        let journal = Journal::new("/journals/dainiki");
        assert_eq!(
            journal.entry_path(date(2026, 10, 18)),
            PathBuf::from("/journals/dainiki/2026/10/2026-10-18.md")
        );
        assert_eq!(
            journal.entry_path(date(2027, 1, 2)),
            PathBuf::from("/journals/dainiki/2027/01/2027-01-02.md")
        );
    }

    #[test]
    fn ensure_entry_creates_folders_and_an_empty_file() {
        let dir = scratch_dir();
        let journal = Journal::new(&dir);
        let path = journal.ensure_entry(date(2026, 10, 18)).expect("ensure");
        assert!(path.is_file());
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        assert!(dir.join("2026/10").is_dir());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_entry_never_touches_an_existing_entry() {
        let dir = scratch_dir();
        let journal = Journal::new(&dir);
        let path = journal.ensure_entry(date(2026, 10, 18)).expect("ensure");
        fs::write(&path, "# already written\n").expect("seed");
        let again = journal.ensure_entry(date(2026, 10, 18)).expect("ensure");
        assert_eq!(again, path);
        assert_eq!(fs::read_to_string(&path).unwrap(), "# already written\n");
        fs::remove_dir_all(&dir).ok();
    }
}
