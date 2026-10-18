//! Dainiki's visual identity, mirrored from the landing page tokens
//! (`landing/src/styles/app.css`). Keep the two in step.

use dainiki_editor::EditorTheme;
use gpui_kit::component::theme::Theme;
use gpui_kit::*;

// -- palette -----------------------------------------------------------------

/// Warm paper.
pub fn surface() -> Hsla {
    rgb(0xFEF8F6).into()
}

pub fn surface_container_low() -> Hsla {
    rgb(0xF8F2F0).into()
}

pub fn surface_container() -> Hsla {
    rgb(0xF2EDEB).into()
}

pub fn surface_container_high() -> Hsla {
    rgb(0xEDE7E5).into()
}

pub fn on_surface() -> Hsla {
    rgb(0x1D1B1A).into()
}

pub fn on_surface_variant() -> Hsla {
    rgb(0x424843).into()
}

/// Moss green.
pub fn primary() -> Hsla {
    rgb(0x32533F).into()
}

pub fn primary_container() -> Hsla {
    rgb(0x4A6B56).into()
}

/// Light mint — chips and today's badge.
pub fn primary_fixed() -> Hsla {
    rgb(0xC6EBD1).into()
}

pub fn on_primary_fixed() -> Hsla {
    rgb(0x002111).into()
}

pub fn outline() -> Hsla {
    rgb(0x727973).into()
}

pub fn outline_variant() -> Hsla {
    rgb(0xC2C8C1).into()
}

/// Serif display family for journal type. Georgia ships on every target OS;
/// bundling Newsreader + Inter (the landing fonts) is a tracked follow-up.
pub const SERIF_FAMILY: &str = "Georgia";

// -- themes ------------------------------------------------------------------

/// The editor theme in Dainiki's palette.
pub fn editor_theme() -> EditorTheme {
    EditorTheme {
        background: surface(),
        text: on_surface(),
        scrollbar: rgba(0x72797340).into(),
        caret: primary(),
        selection: rgba(0x32533F2E).into(),
        code_background: rgba(0x554A3514).into(),
        code_block_background: rgba(0x554A3510).into(),
        highlight_background: rgba(0xFE9E7255).into(),
        link: primary_container(),
        tooltip_background: rgba(0x32302FF2).into(),
        tooltip_text: rgb(0xF5EFED).into(),
        menu_background: rgba(0xFFFFFFF7).into(),
        mono_font_family: "Menlo".into(),
        heading_scales: [1.6, 1.35, 1.15],
        body_font_family: Some(SERIF_FAMILY.into()),
    }
}

/// Recolors the gpui-kit components (buttons, popovers, separators, labels,
/// scrollbars) to the same palette, so chrome and paper read as one product.
pub fn apply_component_theme(cx: &mut App) {
    let theme = Theme::global_mut(cx);
    let colors = &mut theme.colors;
    colors.background = surface();
    colors.foreground = on_surface();
    colors.border = outline_variant();
    colors.muted = surface_container();
    colors.muted_foreground = outline();
    colors.selection = rgba(0x32533F2E).into();
    colors.caret = primary();
    colors.primary = primary();
    colors.primary_foreground = rgb(0xFFFFFF).into();
    colors.primary_hover = primary_container();
    colors.primary_active = rgb(0x2D4D3A).into();
    colors.accent = primary_fixed();
    colors.accent_foreground = on_primary_fixed();
    colors.popover = surface();
    colors.popover_foreground = on_surface();
    colors.button = surface_container();
    colors.button_foreground = on_surface();
    colors.button_hover = surface_container_high();
    colors.button_active = surface_container_high();
    theme.tokens.background = surface().into();
}
