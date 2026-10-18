//! Dainiki's visual identity, mirrored from the landing page tokens
//! (`landing/src/styles/app.css`) and extended with a warm dark mode.
//! Keep the light palette in step with the landing.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use dainiki_editor::EditorTheme;
use gpui_kit::component::theme::Theme;
use gpui_kit::*;

// -- modes -------------------------------------------------------------------

/// The app's colour mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
}

static MODE: AtomicU8 = AtomicU8::new(0);

/// Sets the active colour mode; the next frame paints in it.
pub fn set_mode(mode: Mode) {
    MODE.store(
        match mode {
            Mode::Light => 0,
            Mode::Dark => 1,
        },
        Ordering::Relaxed,
    );
}

/// The active colour mode.
pub fn mode() -> Mode {
    if MODE.load(Ordering::Relaxed) == 1 {
        Mode::Dark
    } else {
        Mode::Light
    }
}

// -- palette -----------------------------------------------------------------

/// Every brand token, resolved for one mode.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// Warm paper.
    pub surface: Hsla,
    pub surface_container_low: Hsla,
    pub surface_container_lowest: Hsla,
    pub surface_container: Hsla,
    pub surface_container_high: Hsla,
    pub surface_container_highest: Hsla,
    pub on_surface: Hsla,
    pub on_surface_variant: Hsla,
    /// Moss green.
    pub primary: Hsla,
    pub primary_container: Hsla,
    pub primary_foreground: Hsla,
    pub primary_active: Hsla,
    /// Light mint — chips and today's badge.
    pub primary_fixed: Hsla,
    pub on_primary_fixed: Hsla,
    pub outline: Hsla,
    pub outline_variant: Hsla,
    /// Burnt orange — tag hashes and journal accents.
    pub secondary: Hsla,
}

impl Palette {
    fn light() -> Self {
        Self {
            surface: rgb(0xFEF8F6).into(),
            surface_container_low: rgb(0xF8F2F0).into(),
            surface_container_lowest: rgb(0xFFFFFF).into(),
            surface_container: rgb(0xF2EDEB).into(),
            surface_container_high: rgb(0xEDE7E5).into(),
            surface_container_highest: rgb(0xE7E1DF).into(),
            on_surface: rgb(0x1D1B1A).into(),
            on_surface_variant: rgb(0x424843).into(),
            primary: rgb(0x32533F).into(),
            primary_container: rgb(0x4A6B56).into(),
            primary_foreground: rgb(0xFFFFFF).into(),
            primary_active: rgb(0x2D4D3A).into(),
            primary_fixed: rgb(0xC6EBD1).into(),
            on_primary_fixed: rgb(0x002111).into(),
            outline: rgb(0x727973).into(),
            outline_variant: rgb(0xC2C8C1).into(),
            secondary: rgb(0x944924).into(),
        }
    }

    fn dark() -> Self {
        Self {
            surface: rgb(0x1C1917).into(),
            surface_container_low: rgb(0x24211E).into(),
            surface_container_lowest: rgb(0x141110).into(),
            surface_container: rgb(0x2A2724).into(),
            surface_container_high: rgb(0x322E2A).into(),
            surface_container_highest: rgb(0x3A3531).into(),
            on_surface: rgb(0xEDE9E4).into(),
            on_surface_variant: rgb(0xB3ADA6).into(),
            primary: rgb(0x9BC9A8).into(),
            primary_container: rgb(0x8ABC97).into(),
            primary_foreground: rgb(0x10231A).into(),
            primary_active: rgb(0x86B894).into(),
            primary_fixed: rgb(0xC6EBD1).into(),
            on_primary_fixed: rgb(0x002111).into(),
            outline: rgb(0x8E8880).into(),
            outline_variant: rgb(0x454038).into(),
            secondary: rgb(0xE09A6E).into(),
        }
    }
}

fn palette(mode: Mode) -> &'static Palette {
    static LIGHT: OnceLock<Palette> = OnceLock::new();
    static DARK: OnceLock<Palette> = OnceLock::new();
    match mode {
        Mode::Light => LIGHT.get_or_init(Palette::light),
        Mode::Dark => DARK.get_or_init(Palette::dark),
    }
}

fn current() -> &'static Palette {
    palette(mode())
}

pub fn surface_container_low() -> Hsla {
    current().surface_container_low
}

pub fn surface_container_lowest() -> Hsla {
    current().surface_container_lowest
}

pub fn surface_container() -> Hsla {
    current().surface_container
}

pub fn surface_container_high() -> Hsla {
    current().surface_container_high
}

pub fn surface_container_highest() -> Hsla {
    current().surface_container_highest
}

pub fn on_surface() -> Hsla {
    current().on_surface
}

pub fn on_surface_variant() -> Hsla {
    current().on_surface_variant
}

/// Moss green.
pub fn primary() -> Hsla {
    current().primary
}

/// Light mint — chips and today's badge.
pub fn primary_fixed() -> Hsla {
    current().primary_fixed
}

pub fn outline() -> Hsla {
    current().outline
}

pub fn outline_variant() -> Hsla {
    current().outline_variant
}

/// Burnt orange — tag hashes and journal accents.
pub fn secondary() -> Hsla {
    current().secondary
}

/// The UI typeface for app chrome (bundled; see `load_fonts`).
pub const UI_FAMILY: &str = "Inter";

/// The serif for journal type — the landing's Newsreader, bundled so the app
/// and the site are set in the same face.
pub const SERIF_FAMILY: &str = "Newsreader";

// -- themes ------------------------------------------------------------------

/// The app mark, decoded once from the bundled PNG.
pub fn logo() -> Option<std::sync::Arc<RenderImage>> {
    use std::sync::{Arc, OnceLock};

    static LOGO: OnceLock<Option<Arc<RenderImage>>> = OnceLock::new();
    LOGO.get_or_init(|| {
        let bytes = include_bytes!("../assets/logo.png");
        let decoded = image::load_from_memory(bytes).ok()?;
        let frame = image::Frame::new(decoded.to_rgba8());
        Some(Arc::new(RenderImage::new([frame])))
    })
    .clone()
}

/// Registers the bundled brand fonts with the text system. Call once at
/// startup, before any window renders.
pub fn load_fonts(cx: &mut App) {
    use std::borrow::Cow;

    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Medium.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-SemiBold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Newsreader-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Newsreader-Medium.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Newsreader-Italic.ttf").as_slice()),
    ];
    if let Err(error) = cx.text_system().add_fonts(fonts) {
        tracing::warn!(error = %error, "could not load the bundled fonts; falling back to system fonts");
    }
}

/// The editor theme for one mode, in the brand palette.
pub fn editor_theme_for(mode: Mode) -> EditorTheme {
    let palette = palette(mode);
    match mode {
        Mode::Light => EditorTheme {
            background: palette.surface,
            text: palette.on_surface,
            scrollbar: rgba(0x72797340).into(),
            caret: palette.primary,
            selection: rgba(0x32533F2E).into(),
            code_background: rgba(0x554A3514).into(),
            code_block_background: rgba(0x554A3510).into(),
            highlight_background: rgba(0xFE9E7255).into(),
            link: palette.primary_container,
            bullet: rgba(0x424843B3).into(),
            quote_bar: rgba(0x42484380).into(),
            checkbox_background: rgba(0xFFFFFF00).into(),
            checkbox_border: rgba(0x72797399).into(),
            checkbox_hover_background: rgba(0x32533F14).into(),
            checkbox_checked_background: palette.primary,
            checkbox_check: palette.surface,
            rule: rgba(0xC2C8C166).into(),
            media_background: rgba(0x554A350A).into(),
            media_border: rgba(0x72797333).into(),
            media_control: rgba(0x1D1B1AE6).into(),
            media_track: rgba(0x7279732E).into(),
            media_progress: palette.primary_container,
            tooltip_background: rgba(0x32302FF2).into(),
            tooltip_text: rgb(0xF5EFED).into(),
            menu_background: rgba(0xFFFFFFF7).into(),
            mono_font_family: "Menlo".into(),
            heading_scales: [1.6, 1.35, 1.15],
            body_font_family: Some(SERIF_FAMILY.into()),
        },
        Mode::Dark => EditorTheme {
            background: palette.surface,
            text: palette.on_surface,
            scrollbar: rgba(0xEDE9E42E).into(),
            caret: palette.primary,
            selection: rgba(0x9BC9A83D).into(),
            code_background: rgba(0xFFFFFF12).into(),
            code_block_background: rgba(0xFFFFFF0D).into(),
            highlight_background: rgba(0xE09A6E4D).into(),
            link: palette.primary,
            bullet: rgba(0xB3ADA6CC).into(),
            quote_bar: rgba(0xB3ADA6A6).into(),
            checkbox_background: rgba(0xFFFFFF00).into(),
            checkbox_border: rgba(0x8E8880CC).into(),
            checkbox_hover_background: rgba(0x9BC9A81F).into(),
            checkbox_checked_background: palette.primary,
            checkbox_check: rgb(0x10231A).into(),
            rule: rgba(0xEDE9E440).into(),
            media_background: rgba(0xFFFFFF0A).into(),
            media_border: rgba(0xFFFFFF24).into(),
            media_control: rgba(0xEDE9E4E6).into(),
            media_track: rgba(0xFFFFFF24).into(),
            media_progress: palette.primary,
            tooltip_background: rgba(0xEDE9E4F2).into(),
            tooltip_text: rgb(0x1C1917).into(),
            menu_background: rgba(0x2A2724F7).into(),
            mono_font_family: "Menlo".into(),
            heading_scales: [1.6, 1.35, 1.15],
            body_font_family: Some(SERIF_FAMILY.into()),
        },
    }
}

/// The editor theme in Dainiki's palette, for the active mode. Test support:
/// the app resolves a mode once per window instead.
#[cfg(test)]
pub fn editor_theme() -> EditorTheme {
    editor_theme_for(mode())
}

/// Recolors the gpui-kit components (buttons, popovers, separators, labels,
/// scrollbars) to the same palette, so chrome and paper read as one product.
/// `Theme::update` reconciles the legacy tokens from the colors, which is
/// what the components actually read.
pub fn apply_component_theme(cx: &mut App) {
    let palette = current();
    Theme::update(cx, |theme| {
        theme.font_family = UI_FAMILY.into();
        theme.font_size = px(13.);
        // The landing's radius scale: DEFAULT 2px, lg 4px, xl 8px, full 12px.
        theme.radius = px(2.);
        theme.radius_lg = px(8.);
        let colors = &mut theme.colors;
        colors.background = palette.surface;
        colors.foreground = palette.on_surface;
        colors.border = palette.outline_variant;
        colors.muted = palette.surface_container;
        colors.muted_foreground = palette.outline;
        colors.selection = match mode() {
            Mode::Light => rgba(0x32533F2E).into(),
            Mode::Dark => rgba(0x9BC9A83D).into(),
        };
        colors.caret = palette.primary;
        colors.primary = palette.primary;
        colors.primary_foreground = palette.primary_foreground;
        colors.primary_hover = palette.primary_container;
        colors.primary_active = palette.primary_active;
        colors.accent = palette.primary_fixed;
        colors.accent_foreground = palette.on_primary_fixed;
        colors.popover = palette.surface;
        colors.popover_foreground = palette.on_surface;
        colors.button = palette.surface_container;
        colors.button_foreground = palette.on_surface;
        colors.button_hover = palette.surface_container_high;
        colors.button_active = palette.surface_container_high;
        colors.button_primary = palette.primary;
        colors.button_primary_foreground = palette.primary_foreground;
        colors.button_primary_hover = palette.primary_container;
        colors.button_primary_active = palette.primary_active;
        colors.button_secondary = palette.surface_container;
        colors.button_secondary_foreground = palette.on_surface;
        colors.button_secondary_hover = palette.surface_container_high;
        colors.button_secondary_active = palette.surface_container_high;
        colors.secondary = palette.surface_container;
        colors.secondary_foreground = palette.on_surface;
        colors.secondary_hover = palette.surface_container_high;
        colors.secondary_active = palette.surface_container_high;
        // Failure reds, warm like the rest of the palette.
        let (danger, danger_hover, danger_active, danger_foreground) = match mode() {
            Mode::Light => (
                rgb(0xB3372C).into(),
                rgb(0xC24036).into(),
                rgb(0x9E2B21).into(),
                rgb(0xFFFFFF).into(),
            ),
            Mode::Dark => (
                rgb(0xE4A5A0).into(),
                rgb(0xECB5B0).into(),
                rgb(0xD69A96).into(),
                rgb(0x4A1210).into(),
            ),
        };
        colors.danger = danger;
        colors.danger_hover = danger_hover;
        colors.danger_active = danger_active;
        colors.danger_foreground = danger_foreground;
        colors.button_danger = danger;
        colors.button_danger_hover = danger_hover;
        colors.button_danger_active = danger_active;
        colors.button_danger_foreground = danger_foreground;
        colors.input = palette.surface_container_lowest;
        colors.ring = palette.primary;
        colors.link = palette.primary;
        colors.scrollbar = rgba(0x00000000).into();
        colors.scrollbar_thumb = match mode() {
            Mode::Light => rgba(0x727973E6).into(),
            Mode::Dark => rgba(0x8E8880E6).into(),
        };
        colors.scrollbar_thumb_hover = palette.outline;
        // Window chrome: the title bar, status bar and border follow the
        // palette so a dark window does not sit under a light bar.
        colors.title_bar = palette.surface_container_low;
        colors.title_bar_border = palette.outline_variant;
        colors.status_bar = palette.surface_container_low;
        colors.status_bar_border = palette.outline_variant;
        colors.window_border = palette.outline_variant;
    });
}

#[cfg(test)]
mod tests {
    // No `use super::*`: the crate root glob-imports `gpui_kit::*`, which
    // re-exports gpui's `test` proc macro; globbing it here would shadow the
    // builtin `#[test]` and recurse.
    use super::{Mode, editor_theme_for, mode, palette, set_mode};

    #[test]
    fn dark_and_light_palettes_differ() {
        let light = palette(Mode::Light);
        let dark = palette(Mode::Dark);
        assert_ne!(light.surface, dark.surface);
        assert_ne!(light.on_surface, dark.on_surface);
        assert_ne!(light.primary, dark.primary);
        assert_ne!(light.outline_variant, dark.outline_variant);
    }

    #[test]
    fn mode_roundtrips() {
        let previous = mode();
        set_mode(Mode::Dark);
        assert_eq!(mode(), Mode::Dark);
        set_mode(Mode::Light);
        assert_eq!(mode(), Mode::Light);
        // Leave the global as it was: tests share this process.
        set_mode(previous);
    }

    #[test]
    fn editor_themes_carry_their_mode_backgrounds() {
        let light = editor_theme_for(Mode::Light);
        let dark = editor_theme_for(Mode::Dark);
        assert_eq!(light.background, palette(Mode::Light).surface);
        assert_eq!(dark.background, palette(Mode::Dark).surface);
        assert_ne!(light.media_control, dark.media_control);
        assert_ne!(light.selection, dark.selection);
    }
}
