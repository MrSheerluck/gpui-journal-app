use gpui::{Hsla, rgb, rgba};

#[derive(Clone, Debug)]
pub struct EditorTheme {
    pub background: Hsla,
    pub text: Hsla,
    pub scrollbar: Hsla,
}

impl Default for EditorTheme {
    fn default() -> Self {
        Self {
            background: rgb(0xFAFAF8).into(),
            text: rgb(0x1F2328).into(),
            scrollbar: rgba(0x00000033).into(),
        }
    }
}
