//! Dainiki's live-preview Markdown editor, built directly on GPUI primitives.
//!
//! # Embedding
//!
//! ```text
//! let buffer = cx.new(|_| Buffer::from_text("# Hello\n"));
//! let theme = EditorTheme::default();
//! let editor = cx.new(|cx| MarkdownEditor::new(buffer, theme.clone(), cx));
//! div().size_full().bg(theme.background).child(editor.clone())
//! ```
//!
//! `dainiki-desktop` is the reference host (toolbar, persistence, reading view).
//!
//! # Public API (frozen at F5)
//!
//! - [`MarkdownEditor`] — the view entity. Hosts assign it
//!   [`MarkdownEditor::set_base_dir`] so relative image paths resolve, focus it
//!   via [`MarkdownEditor::focus_handle`], read text with
//!   [`MarkdownEditor::content`], subscribe to [`EditorEvent`], and drive the
//!   formatting commands ([`MarkdownEditor::toggle_inline_format`],
//!   [`MarkdownEditor::toggle_block_format`], [`MarkdownEditor::insert_link`],
//!   [`MarkdownEditor::insert_image`], [`MarkdownEditor::insert_code_fence`],
//!   [`MarkdownEditor::insert_rule`]) with their active-state queries
//!   ([`MarkdownEditor::inline_format_active`],
//!   [`MarkdownEditor::block_format_active`]), and place the caret at the end
//!   of a freshly swapped buffer ([`MarkdownEditor::place_caret_at_end`]).
//! - [`Buffer`] — the rope-backed text store.
//! - [`EditorTheme`] — colors, heading scales and the optional body font family.
//! - [`commands`] — pure `(text, selection) -> Edit` transforms, independently
//!   testable and usable without any GPUI context.
//! - [`EditorEvent`] — `Edited`, emitted once per buffer mutation.
//!
//! Everything else (`display`, `markdown`, `input`, `buffer` internals) is
//! implementation detail and may change between versions.
//!
//! # Boundary
//!
//! This crate depends only on the raw GPUI snapshot (`gpui-pre`) and never on
//! `gpui-kit`, `gpui-base` or `gpui-component`; CI asserts it with
//! `cargo tree -p dainiki-editor` (see `crates/editor/README.md`).

pub mod buffer;
pub mod commands;
pub mod display;
pub mod input;
pub mod markdown;
pub mod theme;

mod editor;

pub use buffer::Buffer;
pub use display::MediaKind;
pub use editor::{EditorEvent, MarkdownEditor, MediaPlaybackState};
pub use input::EditorElement;
pub use theme::EditorTheme;
