use ropey::{LineType, Rope};

const LINE_TYPE: LineType = LineType::LF;

pub struct Buffer {
    rope: Rope,
}

impl Buffer {
    pub fn from_text(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
        }
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines(LINE_TYPE)
    }

    pub fn line_text(&self, ix: usize) -> String {
        let mut text = self.rope.line(ix, LINE_TYPE).to_string();

        if text.ends_with('\n') {
            text.pop();
            if text.ends_with('\r') {
                text.pop();
            }
        }
        text
    }

    /// Total length of the buffer in utf-8 bytes.
    pub fn len_bytes(&self) -> usize {
        self.rope.len()
    }

    /// Byte offset where the given line begins.
    pub fn line_start_byte(&self, ix: usize) -> usize {
        self.rope
            .line_to_byte_idx(ix.min(self.len_lines().saturating_sub(1)), LINE_TYPE)
    }

    /// Byte offset where the line's text ends, excluding its newline (`\n` or
    /// `\r\n`).
    pub fn line_end_byte(&self, ix: usize) -> usize {
        self.line_start_byte(ix) + self.line_text(ix).len()
    }

    /// Splits a byte offset into `(line index, byte offset within the line's
    /// text)`. Offsets that fall inside a newline sequence resolve to the end
    /// of the preceding line, so movement never lands on a newline byte.
    pub fn offset_to_line(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.len_bytes());
        let last = self.len_lines() - 1;
        let ix = if offset == self.len_bytes() {
            last
        } else {
            self.rope.byte_to_line_idx(offset, LINE_TYPE).min(last)
        };
        let local = (offset - self.line_start_byte(ix)).min(self.line_text(ix).len());
        (ix, local)
    }
}

#[cfg(test)]
mod tests {
    use super::Buffer;

    #[test]
    fn counts_lines_like_an_editor() {
        assert_eq!(Buffer::from_text("").len_lines(), 1);
        assert_eq!(Buffer::from_text("a").len_lines(), 1);
        assert_eq!(Buffer::from_text("a\nb\nc").len_lines(), 3);
        assert_eq!(Buffer::from_text("a\nb\n").len_lines(), 3);
    }

    #[test]
    fn line_text_is_newline_free() {
        let buffer = Buffer::from_text("first\nsecond\n");
        assert_eq!(buffer.line_text(0), "first");
        assert_eq!(buffer.line_text(1), "second");
        assert_eq!(buffer.line_text(2), "");
    }

    #[test]
    fn keeps_empty_lines_inside() {
        let buffer = Buffer::from_text("a\n\nb");
        assert_eq!(buffer.len_lines(), 3);
        assert_eq!(buffer.line_text(1), "");
    }

    #[test]
    fn crlf_lines_strip_both_bytes() {
        let buffer = Buffer::from_text("a\r\nb");
        assert_eq!(buffer.len_lines(), 2);
        assert_eq!(buffer.line_text(0), "a");
        assert_eq!(buffer.line_text(1), "b");
    }

    #[test]
    fn unicode_lines_roundtrip() {
        let buffer = Buffer::from_text("héllo 🎉\n第三行\n");
        assert_eq!(buffer.line_text(0), "héllo 🎉");
        assert_eq!(buffer.line_text(1), "第三行");
        assert_eq!(buffer.line_text(2), "");
    }

    #[test]
    fn long_lines_are_not_wrapped_in_the_buffer() {
        let long = "x".repeat(4096);
        let buffer = Buffer::from_text(&long);
        assert_eq!(buffer.len_lines(), 1);
        assert_eq!(buffer.line_text(0).len(), 4096);
    }

    #[test]
    fn line_boundaries_are_byte_offsets() {
        let buffer = Buffer::from_text("a\nb\nc");
        assert_eq!(buffer.len_bytes(), 5);
        assert_eq!(buffer.line_start_byte(0), 0);
        assert_eq!(buffer.line_start_byte(1), 2);
        assert_eq!(buffer.line_start_byte(2), 4);
        assert_eq!(buffer.line_end_byte(0), 1);
        assert_eq!(buffer.line_end_byte(1), 3);
        assert_eq!(buffer.line_end_byte(2), 5);
        assert_eq!(buffer.offset_to_line(1), (0, 1));
        assert_eq!(buffer.offset_to_line(2), (1, 0));
        assert_eq!(buffer.offset_to_line(5), (2, 1));
    }

    #[test]
    fn crlf_offsets_never_point_inside_the_newline() {
        let buffer = Buffer::from_text("a\r\nb");
        assert_eq!(buffer.line_start_byte(1), 3);
        assert_eq!(buffer.line_end_byte(0), 1);
        assert_eq!(buffer.offset_to_line(1), (0, 1));
        assert_eq!(buffer.offset_to_line(2), (0, 1));
        assert_eq!(buffer.offset_to_line(3), (1, 0));
    }

    #[test]
    fn trailing_newline_has_an_empty_last_line() {
        let buffer = Buffer::from_text("a\n");
        assert_eq!(buffer.len_bytes(), 2);
        assert_eq!(buffer.line_end_byte(0), 1);
        assert_eq!(buffer.offset_to_line(2), (1, 0));
    }

    #[test]
    fn final_offset_without_trailing_newline_clamps_to_last_line() {
        let buffer = Buffer::from_text("abc");
        assert_eq!(buffer.offset_to_line(3), (0, 3));
        assert_eq!(buffer.offset_to_line(99), (0, 3));
    }

    #[test]
    fn unicode_offsets_are_bytes() {
        let buffer = Buffer::from_text("héllo\n🎉");
        assert_eq!(buffer.line_end_byte(0), 6);
        assert_eq!(buffer.line_start_byte(1), 7);
        assert_eq!(buffer.offset_to_line(7), (1, 0));
        assert_eq!(buffer.line_end_byte(1), 11);
    }
}
