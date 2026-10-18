//! Undo history: inverse edits with run coalescing. Pure — no GPUI.

use std::ops::Range;

use crate::{buffer::Buffer, input::Selection};

/// One applied edit, stored as enough data to invert it: the affected range
/// as it was *before* the edit, the text that was removed, the text that was
/// inserted, and the selection on both sides.
#[derive(Clone, Debug)]
pub struct Edit {
    pub range: Range<usize>,
    pub removed: String,
    pub inserted: String,
    pub selection_before: Selection,
    pub selection_after: Selection,
    pub(crate) kind: MergeKind,
}

/// How an edit may merge with the previous one when pushed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeKind {
    /// A one-character insertion typed at the caret.
    TypingInsert,
    /// A one-grapheme backward deletion.
    TypingDelete,
    /// An IME composition update or its commit.
    Composition,
    /// Anything else: never merges.
    None,
}

impl Edit {
    /// The range this edit occupies in the buffer after it was applied.
    fn applied_range(&self) -> Range<usize> {
        self.range.start..self.range.start + self.inserted.len()
    }
}

/// What an undo/redo application did to the buffer.
#[derive(Clone, Debug)]
pub struct AppliedEdit {
    pub selection: Selection,
    /// The byte range as it was before this application.
    pub range: Range<usize>,
    /// How many bytes were written.
    pub new_len: usize,
}

#[derive(Default)]
pub struct History {
    undo_stack: Vec<Edit>,
    redo_stack: Vec<Edit>,
}

impl History {
    /// Records an applied edit, merging it with the previous one when the
    /// merge rules say the user would expect a single undo step.
    pub fn record(&mut self, edit: Edit, merge: MergeKind) {
        self.redo_stack.clear();
        if let Some(last) = self.undo_stack.last_mut()
            && merge != MergeKind::None
            && last.kind == merge
            && Self::adjacent(last, &edit, merge)
        {
            Self::merge(last, edit);
            return;
        }
        self.undo_stack.push(edit);
    }

    /// Ends the current typing run so the next edit cannot merge into it.
    pub fn break_run(&mut self) {
        if let Some(last) = self.undo_stack.last_mut() {
            last.kind = MergeKind::None;
        }
    }

    /// Undoes the newest edit; returns what was applied.
    pub fn undo(&mut self, buffer: &mut Buffer) -> Option<AppliedEdit> {
        let edit = self.undo_stack.pop()?;
        let applied = edit.applied_range();
        buffer.replace(applied.clone(), &edit.removed);
        let summary = AppliedEdit {
            selection: edit.selection_before,
            range: applied,
            new_len: edit.removed.len(),
        };
        self.redo_stack.push(edit);
        Some(summary)
    }

    /// Redoes the newest undone edit; returns what was applied.
    pub fn redo(&mut self, buffer: &mut Buffer) -> Option<AppliedEdit> {
        let edit = self.redo_stack.pop()?;
        buffer.replace(edit.range.clone(), &edit.inserted);
        let summary = AppliedEdit {
            selection: edit.selection_after,
            range: edit.range.clone(),
            new_len: edit.inserted.len(),
        };
        self.undo_stack.push(edit);
        Some(summary)
    }

    fn adjacent(last: &Edit, edit: &Edit, merge: MergeKind) -> bool {
        match merge {
            MergeKind::TypingInsert => last.range.start + last.inserted.len() == edit.range.start,
            MergeKind::TypingDelete => edit.range.end == last.range.start,
            MergeKind::Composition => last.applied_range() == edit.range,
            MergeKind::None => false,
        }
    }

    fn merge(last: &mut Edit, edit: Edit) {
        match edit.kind {
            MergeKind::TypingInsert => {
                last.inserted.push_str(&edit.inserted);
                last.selection_after = edit.selection_after;
            }
            MergeKind::TypingDelete => {
                last.range.start = edit.range.start;
                last.removed = format!("{}{}", edit.removed, last.removed);
                last.selection_after = edit.selection_after;
            }
            MergeKind::Composition => {
                last.inserted = edit.inserted;
                last.selection_after = edit.selection_after;
                last.kind = MergeKind::Composition;
            }
            MergeKind::None => unreachable!("merge is only called for mergeable kinds"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Edit, History, MergeKind};
    use crate::{buffer::Buffer, input::Selection};

    fn edit(range: std::ops::Range<usize>, removed: &str, inserted: &str, kind: MergeKind) -> Edit {
        Edit {
            selection_before: Selection::caret(range.start),
            selection_after: Selection::caret(range.start + inserted.len()),
            range,
            removed: removed.to_string(),
            inserted: inserted.to_string(),
            kind,
        }
    }

    fn apply(
        buffer: &mut Buffer,
        history: &mut History,
        range: std::ops::Range<usize>,
        text: &str,
        kind: MergeKind,
    ) {
        let removed = buffer.replace(range.clone(), text);
        history.record(edit(range, &removed, text, kind), kind);
    }

    #[test]
    fn undo_and_redo_roundtrip() {
        let mut buffer = Buffer::from_text("hello");
        let mut history = History::default();
        apply(&mut buffer, &mut history, 5..5, " world", MergeKind::None);
        assert_eq!(buffer.slice(0..buffer.len_bytes()), "hello world");

        let selection = history.undo(&mut buffer);
        assert!(selection.is_some());
        assert_eq!(buffer.slice(0..buffer.len_bytes()), "hello");

        let selection = history.redo(&mut buffer);
        assert!(selection.is_some());
        assert_eq!(buffer.slice(0..buffer.len_bytes()), "hello world");
    }

    #[test]
    fn undo_restores_removed_text() {
        let mut buffer = Buffer::from_text("hello world");
        let mut history = History::default();
        apply(&mut buffer, &mut history, 5..11, "", MergeKind::None);
        assert_eq!(buffer.slice(0..buffer.len_bytes()), "hello");

        history.undo(&mut buffer).expect("edit to undo");
        assert_eq!(buffer.slice(0..buffer.len_bytes()), "hello world");
    }

    #[test]
    fn typing_runs_coalesce_into_one_undo() {
        let mut buffer = Buffer::from_text("");
        let mut history = History::default();
        for (ix, ch) in "hey".chars().enumerate() {
            let at = ix;
            apply(
                &mut buffer,
                &mut history,
                at..at,
                &ch.to_string(),
                MergeKind::TypingInsert,
            );
        }
        assert_eq!(buffer.slice(0..buffer.len_bytes()), "hey");

        history.undo(&mut buffer).expect("run to undo");
        assert_eq!(buffer.slice(0..buffer.len_bytes()), "");
        assert!(history.undo(&mut buffer).is_none());
    }

    #[test]
    fn backspaces_coalesce_into_one_undo() {
        let mut buffer = Buffer::from_text("hey");
        let mut history = History::default();
        for at in (0..3).rev() {
            apply(
                &mut buffer,
                &mut history,
                at..at + 1,
                "",
                MergeKind::TypingDelete,
            );
        }
        assert_eq!(buffer.slice(0..buffer.len_bytes()), "");

        history.undo(&mut buffer).expect("run to undo");
        assert_eq!(buffer.slice(0..buffer.len_bytes()), "hey");
    }

    #[test]
    fn breaking_the_run_separates_typing() {
        let mut buffer = Buffer::from_text("");
        let mut history = History::default();
        apply(
            &mut buffer,
            &mut history,
            0..0,
            "a",
            MergeKind::TypingInsert,
        );
        history.break_run();
        apply(
            &mut buffer,
            &mut history,
            1..1,
            "b",
            MergeKind::TypingInsert,
        );
        assert_eq!(buffer.slice(0..buffer.len_bytes()), "ab");

        history.undo(&mut buffer).expect("second run");
        assert_eq!(buffer.slice(0..buffer.len_bytes()), "a");
        history.undo(&mut buffer).expect("first run");
        assert_eq!(buffer.slice(0..buffer.len_bytes()), "");
    }

    #[test]
    fn new_edit_clears_the_redo_stack() {
        let mut buffer = Buffer::from_text("");
        let mut history = History::default();
        apply(&mut buffer, &mut history, 0..0, "a", MergeKind::None);
        history.undo(&mut buffer).expect("undo");
        apply(&mut buffer, &mut history, 0..0, "b", MergeKind::None);
        assert!(history.redo(&mut buffer).is_none());
        assert_eq!(buffer.slice(0..buffer.len_bytes()), "b");
    }

    #[test]
    fn composition_updates_merge_until_committed() {
        let mut buffer = Buffer::from_text("x");
        let mut history = History::default();
        apply(&mut buffer, &mut history, 1..1, "n", MergeKind::Composition);
        apply(
            &mut buffer,
            &mut history,
            1..2,
            "ni",
            MergeKind::Composition,
        );
        apply(
            &mut buffer,
            &mut history,
            1..3,
            "日本",
            MergeKind::Composition,
        );
        assert_eq!(buffer.slice(0..buffer.len_bytes()), "x日本");

        history.undo(&mut buffer).expect("composition run");
        assert_eq!(buffer.slice(0..buffer.len_bytes()), "x");
    }

    #[test]
    fn selections_are_restored_on_both_sides() {
        let mut buffer = Buffer::from_text("one two");
        let mut history = History::default();
        buffer.replace(0..3, "1");
        history.record(
            Edit {
                range: 0..3,
                removed: "one".to_string(),
                inserted: "1".to_string(),
                selection_before: Selection::new(0, 3),
                selection_after: Selection::caret(1),
                kind: MergeKind::None,
            },
            MergeKind::None,
        );
        assert_eq!(buffer.slice(0..buffer.len_bytes()), "1 two");

        let before = history.undo(&mut buffer).expect("edit to undo");
        assert_eq!(before.selection, Selection::new(0, 3));
        let after = history.redo(&mut buffer).expect("edit to redo");
        assert_eq!(after.selection, Selection::caret(1));
    }
}
