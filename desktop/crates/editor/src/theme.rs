use gpui::{Hsla, rgb};

#[derive(Clone, Debug)]
pub struct EditorTheme {
    pub background: Hsla,
    pub text: Hsla,
}

impl Default for EditorTheme {
    fn default() -> Self {
        Self {
            background: rgb(0xFAFAF8).into(),
            text: rgb(0x1F2328).into(),
        }
    }
}
