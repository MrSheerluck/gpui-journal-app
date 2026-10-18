//! Pure text movement over the buffer: grapheme clusters, words and line
//! boundaries. All offsets are utf-8 byte offsets; newline bytes are never
//! returned — a move across a line boundary lands on the neighbouring line's
//! first or last content byte.
//!
//! Word movement follows macOS text-editing convention: a step right lands at
//! the end of the current or next word, a step left at the start of the
//! previous word.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::buffer::Buffer;

/// One grapheme cluster to the left, crossing line boundaries; stays at the
/// document start.
pub fn left(buffer: &Buffer, offset: usize) -> usize {
    let (ix, local) = buffer.offset_to_line(offset);
    if local > 0 {
        buffer.line_start_byte(ix) + grapheme_left(&buffer.line_text(ix), local)
    } else if ix > 0 {
        buffer.line_end_byte(ix - 1)
    } else {
        0
    }
}

/// One grapheme cluster to the right, crossing line boundaries; stays at the
/// document end.
pub fn right(buffer: &Buffer, offset: usize) -> usize {
    let (ix, local) = buffer.offset_to_line(offset);
    let text = buffer.line_text(ix);
    if local < text.len() {
        buffer.line_start_byte(ix) + grapheme_right(&text, local)
    } else if ix + 1 < buffer.len_lines() {
        buffer.line_start_byte(ix + 1)
    } else {
        buffer.len_bytes()
    }
}

/// Start of the previous word, skipping non-word graphemes first (line breaks
/// count as non-word).
pub fn word_left(buffer: &Buffer, offset: usize) -> usize {
    let (mut ix, mut local) = buffer.offset_to_line(offset);
    // Phase 1: skip non-word graphemes backwards, crossing empty lines.
    loop {
        if local == 0 {
            if ix == 0 {
                return 0;
            }
            ix -= 1;
            local = buffer.line_text(ix).len();
            continue;
        }
        let text = buffer.line_text(ix);
        let (start, grapheme) = prev_grapheme(&text, local);
        if is_word_grapheme(grapheme) {
            break;
        }
        local = start;
    }
    // Phase 2: skip the word itself back to where it starts.
    loop {
        if local == 0 {
            return buffer.line_start_byte(ix);
        }
        let text = buffer.line_text(ix);
        let (start, grapheme) = prev_grapheme(&text, local);
        if !is_word_grapheme(grapheme) {
            return buffer.line_start_byte(ix) + local;
        }
        local = start;
    }
}

/// End of the current word if the caret sits inside one, otherwise end of the
/// next word; stays at the document end.
pub fn word_right(buffer: &Buffer, offset: usize) -> usize {
    let (ix, local) = buffer.offset_to_line(offset);
    let (word_ix, word_local) = scan_forward(buffer, ix, local, true);
    if (word_ix, word_local) != (ix, local) {
        return buffer.line_start_byte(word_ix) + word_local;
    }
    let (gap_ix, gap_local) = scan_forward(buffer, word_ix, word_local, false);
    let (end_ix, end_local) = scan_forward(buffer, gap_ix, gap_local, true);
    buffer.line_start_byte(end_ix) + end_local
}

/// First content byte of the line containing `offset`.
pub fn line_start(buffer: &Buffer, offset: usize) -> usize {
    let (ix, _) = buffer.offset_to_line(offset);
    buffer.line_start_byte(ix)
}

/// Last content byte of the line containing `offset`.
pub fn line_end(buffer: &Buffer, offset: usize) -> usize {
    let (ix, _) = buffer.offset_to_line(offset);
    buffer.line_end_byte(ix)
}

/// The maximal run of same-class graphemes around `offset` within its line:
/// a word's extent, or the extent of separators (whitespace, punctuation)
/// when the offset sits on one. Used for double-click selection.
pub fn run_at(buffer: &Buffer, offset: usize) -> Range<usize> {
    let (ix, local) = buffer.offset_to_line(offset);
    let line_start = buffer.line_start_byte(ix);
    let text = buffer.line_text(ix);
    let len = text.len();
    if len == 0 {
        return line_start..line_start;
    }

    let probe = if local >= len {
        prev_grapheme(&text, len).1
    } else {
        next_grapheme(&text, local).1
    };
    let word = is_word_grapheme(probe);
    let local = local.min(len);

    let mut start = local;
    while start > 0 {
        let (previous, grapheme) = prev_grapheme(&text, start);
        if is_word_grapheme(grapheme) != word {
            break;
        }
        start = previous;
    }
    let mut end = local;
    while end < len {
        let (next, grapheme) = next_grapheme(&text, end);
        if is_word_grapheme(grapheme) != word {
            break;
        }
        end = next;
    }
    line_start + start..line_start + end
}

/// Start of the grapheme cluster ending at `local` within `text`.
pub fn grapheme_left(text: &str, local: usize) -> usize {
    prev_grapheme(text, local).0
}

/// Byte offset one grapheme cluster right of `local` within `text`.
pub fn grapheme_right(text: &str, local: usize) -> usize {
    match text[local..].graphemes(true).next() {
        Some(grapheme) => local + grapheme.len(),
        None => text.len(),
    }
}

/// Advances over one class of graphemes starting at `(ix, local)`; returns the
/// first position holding the opposite class. Word-class scanning stops at a
/// line end (words do not span line breaks); non-word scanning crosses lines.
fn scan_forward(
    buffer: &Buffer,
    mut ix: usize,
    mut local: usize,
    want_word: bool,
) -> (usize, usize) {
    loop {
        let text = buffer.line_text(ix);
        while local < text.len() {
            let (next, grapheme) = next_grapheme(&text, local);
            if is_word_grapheme(grapheme) != want_word {
                return (ix, local);
            }
            local = next;
        }
        if want_word || ix + 1 == buffer.len_lines() {
            return (ix, local);
        }
        ix += 1;
        local = 0;
    }
}

fn prev_grapheme(text: &str, local: usize) -> (usize, &str) {
    match text[..local].grapheme_indices(true).next_back() {
        Some((start, grapheme)) => (start, grapheme),
        None => (0, ""),
    }
}

fn next_grapheme(text: &str, local: usize) -> (usize, &str) {
    match text[local..].grapheme_indices(true).next() {
        Some((start, grapheme)) => (local + start + grapheme.len(), grapheme),
        None => (text.len(), ""),
    }
}

fn is_word_grapheme(grapheme: &str) -> bool {
    grapheme
        .chars()
        .next()
        .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
}

#[cfg(test)]
mod tests {
    use super::{left, line_end, line_start, right, run_at, word_left, word_right};
    use crate::buffer::Buffer;

    #[test]
    fn grapheme_steps_treat_clusters_as_one_unit() {
        let buffer = Buffer::from_text("e\u{0301}x");
        assert_eq!(right(&buffer, 0), 3);
        assert_eq!(left(&buffer, 3), 0);

        let buffer = Buffer::from_text("a🎉b");
        assert_eq!(right(&buffer, 1), 5);
        assert_eq!(left(&buffer, 5), 1);
    }

    #[test]
    fn grapheme_steps_cross_lines() {
        let buffer = Buffer::from_text("ab\ncd");
        assert_eq!(right(&buffer, 1), 2);
        assert_eq!(right(&buffer, 2), 3);
        assert_eq!(left(&buffer, 3), 2);
        assert_eq!(left(&buffer, 0), 0);
        assert_eq!(right(&buffer, 5), 5);
    }

    #[test]
    fn grapheme_steps_skip_crlf() {
        let buffer = Buffer::from_text("ab\r\ncd");
        assert_eq!(right(&buffer, 2), 4);
        assert_eq!(left(&buffer, 4), 2);
    }

    #[test]
    fn word_steps_follow_macos_convention() {
        let buffer = Buffer::from_text("foo bar baz");
        assert_eq!(word_right(&buffer, 0), 3);
        assert_eq!(word_right(&buffer, 3), 7);
        assert_eq!(word_right(&buffer, 7), 11);
        assert_eq!(word_right(&buffer, 11), 11);
        assert_eq!(word_left(&buffer, 11), 8);
        assert_eq!(word_left(&buffer, 8), 4);
        assert_eq!(word_left(&buffer, 4), 0);
        assert_eq!(word_left(&buffer, 0), 0);
    }

    #[test]
    fn word_steps_cross_lines() {
        let buffer = Buffer::from_text("foo\nbar");
        assert_eq!(word_right(&buffer, 0), 3);
        assert_eq!(word_right(&buffer, 3), 7);
        assert_eq!(word_left(&buffer, 7), 4);
        assert_eq!(word_left(&buffer, 4), 0);
    }

    #[test]
    fn word_steps_skip_punctuation() {
        let buffer = Buffer::from_text("hello, world");
        assert_eq!(word_right(&buffer, 0), 5);
        assert_eq!(word_right(&buffer, 5), 12);
        assert_eq!(word_left(&buffer, 12), 7);
        assert_eq!(word_left(&buffer, 7), 0);
    }

    #[test]
    fn underscores_and_digits_are_word_chars() {
        let buffer = Buffer::from_text("snake_case1 x");
        assert_eq!(word_right(&buffer, 0), 11);
        assert_eq!(word_left(&buffer, 11), 0);
        assert_eq!(word_left(&buffer, 13), 12);
    }

    #[test]
    fn word_steps_handle_cjk() {
        let buffer = Buffer::from_text("日本語 の");
        assert_eq!(word_right(&buffer, 0), 9);
        assert_eq!(word_left(&buffer, 9), 0);
        assert_eq!(word_right(&buffer, 9), 13);
    }

    #[test]
    fn runs_select_words_and_separators() {
        let buffer = Buffer::from_text("hello, world");
        assert_eq!(run_at(&buffer, 2), 0..5);
        assert_eq!(run_at(&buffer, 5), 5..7);
        assert_eq!(run_at(&buffer, 8), 7..12);
        assert_eq!(run_at(&buffer, 12), 7..12);
    }

    #[test]
    fn runs_handle_cjk_and_empty_lines() {
        let buffer = Buffer::from_text("日本語 test\n\nx");
        assert_eq!(run_at(&buffer, 0), 0..9);
        assert_eq!(run_at(&buffer, 10), 10..14);
        assert_eq!(run_at(&buffer, 15), 15..15);
    }

    #[test]
    fn word_steps_cross_empty_lines() {
        let buffer = Buffer::from_text("a\n\nb");
        assert_eq!(word_right(&buffer, 0), 1);
        assert_eq!(word_right(&buffer, 1), 4);
        assert_eq!(word_left(&buffer, 4), 3);
        assert_eq!(word_left(&buffer, 3), 0);
    }

    #[test]
    fn line_boundaries_resolve_from_any_offset() {
        let buffer = Buffer::from_text("ab\ncd");
        assert_eq!(line_start(&buffer, 0), 0);
        assert_eq!(line_start(&buffer, 3), 3);
        assert_eq!(line_end(&buffer, 2), 2);
        assert_eq!(line_end(&buffer, 4), 5);

        let buffer = Buffer::from_text("ab\n");
        assert_eq!(right(&buffer, 3), 3);
        assert_eq!(line_end(&buffer, 3), 3);
    }
}
