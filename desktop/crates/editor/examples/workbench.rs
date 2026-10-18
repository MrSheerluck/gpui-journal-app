use std::fs;

use dainiki_editor::{Buffer, EditorTheme, MarkdownEditor};
use gpui::{
    App, Bounds, Context, Entity, Render, TitlebarOptions, Window, WindowBounds, WindowOptions,
    div, prelude::*, px, size,
};
use gpui_platform::application;
use tracing_subscriber::EnvFilter;

const SAMPLE: &str = "\
# Dainiki workbench

The editor renders the raw Markdown source as plain text for now; live preview
arrives one construct family at a time (D2 first). Long paragraphs like this one
show that soft wrapping follows the column width - resize the window and the
line breaks move accordingly.

Keep scrolling: the offset is owned by the editor view, not by a GPUI scroll
container, and the element skips lines outside the viewport when painting. The
thin indicator on the right tracks your position through the document.

Type into it: click to focus, then type. Arrows move the caret (option steps by
word, command by line, command-up/down jumps to the document bounds), shift
extends the selection, and Enter inserts a newline. IME composition renders an
underline until you commit it.

- Unicode shaping works too: héllo - 日本語 - emoji 🎉
- This second item exists mainly to make the list rhythm visible.

> A quote line, rendered as plain text for now.

Scroll to the end to confirm the indicator reaches the bottom and the last line
stays fully visible above it:

    cargo run -p dainiki-editor --example workbench -- ~/journal/2026-10-17.md

One more paragraph so the sample reliably overflows a 640 pt window. Editing is
still caret-and-keyboard only: deletion, undo and mouse selection arrive in
later steps.
";

struct Workbench {
    theme: EditorTheme,
    editor: Entity<MarkdownEditor>,
}

impl Render for Workbench {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(self.theme.background)
            .flex()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(720.0))
                    .h_full()
                    .px_6()
                    .py_6()
                    .child(self.editor.clone()),
            )
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn main() {
    init_tracing();

    let text = match std::env::args().nth(1) {
        Some(path) => match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("failed to read {path}: {error}");
                std::process::exit(1);
            }
        },
        None => SAMPLE.to_string(),
    };

    application().run(move |cx: &mut App| {
        let buffer = cx.new(|_| Buffer::from_text(&text));
        let bounds = Bounds::centered(None, size(px(900.0), px(640.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Dainiki — Workbench".into()),
                    ..Default::default()
                }),
                window_min_size: Some(size(px(480.0), px(320.0))),
                ..Default::default()
            },
            move |window, cx| {
                let theme = EditorTheme::default();
                let editor = cx.new(|cx| MarkdownEditor::new(buffer.clone(), theme.clone(), cx));
                let focus_handle = editor.read(cx).focus_handle().clone();
                window.focus(&focus_handle, cx);
                cx.new(|_cx| Workbench { theme, editor })
            },
        )
        .expect("failed to open workbench window");
        cx.activate(true);
    });
}
