use gpui::{Hsla, SharedString, rgb, rgba};

#[derive(Clone, Debug)]
pub struct EditorTheme {
    pub background: Hsla,
    pub text: Hsla,
    pub scrollbar: Hsla,
    pub caret: Hsla,
    pub selection: Hsla,
    /// Family for body text; `None` inherits the host's UI font.
    pub body_font_family: Option<SharedString>,
    /// Background chip behind inline code.
    pub code_background: Hsla,
    /// Background across fenced code block content lines.
    pub code_block_background: Hsla,
    /// Background behind `==highlight==`.
    pub highlight_background: Hsla,
    /// Link text color.
    pub link: Hsla,
    /// The dot of a collapsed bullet marker.
    pub bullet: Hsla,
    /// The rounded bar of a collapsed blockquote marker.
    pub quote_bar: Hsla,
    /// Fill of an unchecked task box.
    pub checkbox_background: Hsla,
    /// Border of an unchecked task box.
    pub checkbox_border: Hsla,
    /// Fill of a task box under the pointer.
    pub checkbox_hover_background: Hsla,
    /// Fill of a checked task box.
    pub checkbox_checked_background: Hsla,
    /// The check mark on a checked task box.
    pub checkbox_check: Hsla,
    /// The `---` rule line.
    pub rule: Hsla,
    /// Surface of media cards and rows.
    pub media_background: Hsla,
    /// The border around media cards and rows.
    pub media_border: Hsla,
    /// Media control glyphs (play/pause, labels).
    pub media_control: Hsla,
    /// The unfilled part of a media progress track.
    pub media_track: Hsla,
    /// The filled part of a media progress track.
    pub media_progress: Hsla,
    /// Hover tooltip behind link URLs.
    pub tooltip_background: Hsla,
    pub tooltip_text: Hsla,
    /// Slash-menu panel.
    pub menu_background: Hsla,
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
            body_font_family: None,
            code_background: rgba(0x0000000F).into(),
            code_block_background: rgba(0x00000012).into(),
            highlight_background: rgba(0xFFD54A80).into(),
            link: rgb(0x2F6FEB).into(),
            bullet: rgba(0x1F2328B3).into(),
            quote_bar: rgba(0x1F2328A6).into(),
            checkbox_background: rgba(0x00000000).into(),
            checkbox_border: rgba(0x1F232880).into(),
            checkbox_hover_background: rgba(0x0000000D).into(),
            checkbox_checked_background: rgb(0x2F6FEB).into(),
            checkbox_check: rgb(0xFFFFFF).into(),
            rule: rgba(0x00000040).into(),
            media_background: rgba(0x00000008).into(),
            media_border: rgba(0x00000014).into(),
            media_control: rgba(0x1F2328E6).into(),
            media_track: rgba(0x0000001A).into(),
            media_progress: rgb(0x2F6FEB).into(),
            tooltip_background: rgba(0x1F2328F2).into(),
            tooltip_text: rgb(0xFAFAF8).into(),
            menu_background: rgba(0xFFFFFFF7).into(),
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
