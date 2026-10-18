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
}
