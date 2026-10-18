//! The journal folder: entry layout and today's entry.
//!
//! Entries live at `root/YYYY/MM/YYYY-MM-DD.md`, with media beside them in
//! `root/YYYY/MM/assets/`. The folder of Markdown files is the authority;
//! everything else in the app is derived from it.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{Datelike, NaiveDate};

/// Creates `<parent>/Dainiki` and returns it, adding a numeric suffix when
/// the name is taken. Used by "Create new journal".
pub fn create_journal_dir(parent: &Path) -> io::Result<PathBuf> {
    let first = parent.join("Dainiki");
    if !first.exists() {
        fs::create_dir_all(&first)?;
        return Ok(first);
    }
    let mut index = 2u32;
    loop {
        let candidate = parent.join(format!("Dainiki {index}"));
        if !candidate.exists() {
            fs::create_dir_all(&candidate)?;
            return Ok(candidate);
        }
        index += 1;
    }
}

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

    /// Every entry date on disk, newest first. Anything outside the
    /// `YYYY/MM/YYYY-MM-DD.md` layout is ignored; an unreadable root is
    /// simply an empty journal.
    pub fn entry_dates(&self) -> Vec<NaiveDate> {
        let mut dates = Vec::new();
        let Ok(years) = fs::read_dir(&self.root) else {
            return dates;
        };
        for year_entry in years.flatten() {
            let year_path = year_entry.path();
            let Some(year) = parse_component(&year_entry.file_name(), 4) else {
                continue;
            };
            if !year_path.is_dir() {
                continue;
            }
            let Ok(months) = fs::read_dir(&year_path) else {
                continue;
            };
            for month_entry in months.flatten() {
                let month_path = month_entry.path();
                let Some(month) = parse_component(&month_entry.file_name(), 2) else {
                    continue;
                };
                if !month_path.is_dir() {
                    continue;
                }
                let Ok(files) = fs::read_dir(&month_path) else {
                    continue;
                };
                for file_entry in files.flatten() {
                    if !file_entry.path().is_file() {
                        continue;
                    }
                    let file_name = file_entry.file_name();
                    let Some(stem) = file_name.to_str().and_then(|name| name.strip_suffix(".md"))
                    else {
                        continue;
                    };
                    let Ok(date) = NaiveDate::parse_from_str(stem, "%Y-%m-%d") else {
                        continue;
                    };
                    if date.year() != year || date.month() as i32 != month {
                        continue;
                    }
                    dates.push(date);
                }
            }
        }
        dates.sort_unstable_by(|a, b| b.cmp(a));
        dates.dedup();
        dates
    }
}

/// A folder component that is exactly `digits` ASCII digits, parsed.
fn parse_component(name: &OsStr, digits: usize) -> Option<i32> {
    let name = name.to_str()?;
    if name.len() != digits || !name.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    name.parse().ok()
}

/// The dates the timeline shows, newest first.
///
/// `existing` holds the dates with real content (today may be absent — the
/// live editor owns it). With `show_empty_days` every date from the oldest
/// entry through today is included; otherwise only dates with content, plus
/// today. Future dates (clock changes) are ignored.
pub fn stream_dates(
    existing: &BTreeSet<NaiveDate>,
    today: NaiveDate,
    show_empty_days: bool,
) -> Vec<NaiveDate> {
    let mut dates: Vec<NaiveDate> = existing
        .iter()
        .copied()
        .filter(|date| *date <= today)
        .collect();
    dates.sort_unstable_by(|a, b| b.cmp(a));

    if !show_empty_days {
        if !dates.contains(&today) {
            dates.insert(0, today);
        }
        return dates;
    }

    let oldest = dates.last().copied().unwrap_or(today);
    let mut stream = Vec::new();
    let mut date = today;
    loop {
        stream.push(date);
        if date <= oldest {
            break;
        }
        match date.pred_opt() {
            Some(previous) => date = previous,
            None => break,
        }
    }
    stream
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::NaiveDate;

    use super::{Journal, create_journal_dir, stream_dates};

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

    #[test]
    fn create_journal_dir_creates_the_folder_in_a_free_location() {
        let dir = scratch_dir();
        let root = create_journal_dir(&dir).expect("create");
        assert_eq!(root, dir.join("Dainiki"));
        assert!(root.is_dir());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn create_journal_dir_suffixes_when_the_name_is_taken() {
        let dir = scratch_dir();
        let first = create_journal_dir(&dir).expect("first");
        let second = create_journal_dir(&dir).expect("second");
        let third = create_journal_dir(&dir).expect("third");
        assert_eq!(first, dir.join("Dainiki"));
        assert_eq!(second, dir.join("Dainiki 2"));
        assert_eq!(third, dir.join("Dainiki 3"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn entry_dates_discovers_entries_newest_first() {
        let dir = scratch_dir();
        let journal = Journal::new(&dir);
        for date in [
            date(2026, 10, 18),
            date(2026, 10, 1),
            date(2026, 1, 3),
            date(2025, 12, 31),
        ] {
            let path = journal.entry_path(date);
            fs::create_dir_all(path.parent().unwrap()).expect("dirs");
            fs::write(&path, "# entry\n").expect("seed");
        }
        assert_eq!(
            journal.entry_dates(),
            vec![
                date(2026, 10, 18),
                date(2026, 10, 1),
                date(2026, 1, 3),
                date(2025, 12, 31),
            ]
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn entry_dates_ignores_anything_outside_the_layout() {
        let dir = scratch_dir();
        let journal = Journal::new(&dir);
        let valid = journal.entry_path(date(2026, 10, 18));
        fs::create_dir_all(valid.parent().unwrap()).expect("dirs");
        fs::write(&valid, "# entry\n").expect("seed");

        // Wrong file name, wrong month folder, and a date that does not match
        // its folders.
        fs::write(dir.join("2026/10/notes.md"), "x").expect("junk");
        fs::write(dir.join("2026/10/2026-10-18.md.bak"), "x").expect("junk");
        fs::create_dir_all(dir.join("2026/13")).expect("junk");
        fs::write(dir.join("2026/13/2026-13-01.md"), "x").expect("junk");
        fs::write(dir.join("2026/10/2026-11-02.md"), "x").expect("junk");
        fs::create_dir_all(dir.join("archive/2026/10")).expect("junk");
        fs::create_dir_all(dir.join("2026/10/2026-10-05.md")).expect("dir named like an entry");

        assert_eq!(journal.entry_dates(), vec![date(2026, 10, 18)]);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn entry_dates_of_a_missing_root_is_empty() {
        let dir = scratch_dir();
        let journal = Journal::new(dir.join("nope"));
        assert_eq!(journal.entry_dates(), Vec::<NaiveDate>::new());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stream_of_an_empty_journal_is_today() {
        let today = date(2026, 10, 19);
        let existing = BTreeSet::new();
        assert_eq!(stream_dates(&existing, today, true), vec![today]);
        assert_eq!(stream_dates(&existing, today, false), vec![today]);
    }

    #[test]
    fn stream_without_empty_days_shows_only_content_days() {
        let today = date(2026, 10, 19);
        let existing = BTreeSet::from([date(2026, 10, 5), date(2026, 10, 19)]);
        assert_eq!(
            stream_dates(&existing, today, false),
            vec![date(2026, 10, 19), date(2026, 10, 5)]
        );
    }

    #[test]
    fn stream_with_empty_days_covers_the_whole_range() {
        let today = date(2026, 10, 19);
        let existing = BTreeSet::from([date(2026, 10, 14)]);
        let stream = stream_dates(&existing, today, true);
        assert_eq!(stream.len(), 6, "six days from the 14th through the 19th");
        assert_eq!(stream.first(), Some(&today));
        assert_eq!(stream.last(), Some(&date(2026, 10, 14)));

        // Today never appears twice when it already has content.
        let with_today = BTreeSet::from([date(2026, 10, 14), today]);
        assert_eq!(stream_dates(&with_today, today, true), stream);
    }

    #[test]
    fn stream_ignores_future_dates() {
        let today = date(2026, 10, 19);
        let existing = BTreeSet::from([date(2026, 10, 21)]);
        assert_eq!(stream_dates(&existing, today, true), vec![today]);
        assert_eq!(stream_dates(&existing, today, false), vec![today]);
    }
}
