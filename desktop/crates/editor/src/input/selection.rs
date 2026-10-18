//! Editor selection: an anchor and a head byte offset. Movement always moves
//! the head; the anchor marks where the current selection started.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Selection {
    anchor: usize,
    head: usize,
}

impl Selection {
    /// A caret (empty selection) at `offset`.
    pub fn caret(offset: usize) -> Self {
        Self {
            anchor: offset,
            head: offset,
        }
    }

    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    pub fn anchor(&self) -> usize {
        self.anchor
    }

    pub fn head(&self) -> usize {
        self.head
    }

    /// Lower byte offset of the selection.
    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    /// Higher byte offset of the selection.
    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    /// True when the selection has no range.
    pub fn is_caret(&self) -> bool {
        self.anchor == self.head
    }

    /// True when the selection extends backwards from the anchor.
    pub fn reversed(&self) -> bool {
        self.head < self.anchor
    }

    /// Moves the head, keeping the anchor fixed (shift-extend semantics).
    pub fn move_head(&mut self, head: usize) {
        self.head = head;
    }

    /// Replaces the selection with a caret at `offset`.
    pub fn collapse(&mut self, offset: usize) {
        *self = Self::caret(offset);
    }
}

#[cfg(test)]
mod tests {
    use super::Selection;

    #[test]
    fn caret_has_no_range() {
        let selection = Selection::caret(7);
        assert!(selection.is_caret());
        assert_eq!(selection.start(), 7);
        assert_eq!(selection.end(), 7);
        assert!(!selection.reversed());
    }

    #[test]
    fn start_and_end_normalize_direction() {
        let forward = Selection::new(2, 9);
        assert_eq!((forward.start(), forward.end()), (2, 9));
        assert!(!forward.reversed());

        let backward = Selection::new(9, 2);
        assert_eq!((backward.start(), backward.end()), (2, 9));
        assert!(backward.reversed());
        assert_eq!(backward.anchor(), 9);
        assert_eq!(backward.head(), 2);
    }

    #[test]
    fn move_head_keeps_the_anchor() {
        let mut selection = Selection::new(2, 9);
        selection.move_head(4);
        assert_eq!((selection.anchor(), selection.head()), (2, 4));
    }

    #[test]
    fn collapse_resets_both_ends() {
        let mut selection = Selection::new(2, 9);
        selection.collapse(5);
        assert_eq!(selection, Selection::caret(5));
    }
}
