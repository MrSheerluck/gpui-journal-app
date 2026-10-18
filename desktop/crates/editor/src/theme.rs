use gpui::{Hsla, SharedString, rgb, rgba};

#[derive(Clone, Debug)]
pub struct EditorTheme {
    pub background: Hsla,
    pub text: Hsla,
    pub scrollbar: Hsla,
    pub caret: Hsla,
    pub selection: Hsla,
    /// Background chip behind inline code.
    pub code_background: Hsla,
    /// Background behind `==highlight==`.
    pub highlight_background: Hsla,
    /// Family used for inline code and fenced code.
    pub mono_font_family: SharedString,
    /// Font size multipliers for H1–H3; deeper headings use the last entry.
    pub heading_scales: [f32; 3],
}

impl Default for EditorTheme {
    fn default() -> Self {
        Self {
            background: rgb(0xFAFAF8).into(),
            text: rgb(0x1F2328).into(),
            scrollbar: rgba(0x00000033).into(),
            caret: rgb(0x2F6FEB).into(),
            selection: rgba(0x2F6FEB33).into(),
            code_background: rgba(0x0000000F).into(),
            highlight_background: rgba(0xFFD54A80).into(),
            mono_font_family: SharedString::new_static("Menlo"),
            heading_scales: [1.6, 1.35, 1.15],
        }
    }
}

impl EditorTheme {
    /// The font size multiplier for a heading level (1-based).
    pub fn heading_scale(&self, level: u8) -> f32 {
        let index = level.saturating_sub(1).min(2) as usize;
        self.heading_scales[index]
    }
}
