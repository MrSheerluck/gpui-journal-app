use std::fs;

use dainiki_editor::{Buffer, EditorElement, EditorTheme};
use gpui::{
    App, Bounds, Context, Entity, Render, TitlebarOptions, Window, WindowBounds, WindowOptions,
    div, prelude::*, px, size,
};
use gpui_platform::application;
use tracing_subscriber::EnvFilter;

const SAMPLE: &str = "\
# Dainiki workbench

The editor renders the raw Markdown source as plain text for now; live preview
arrives one construct family at a time (D2 first). A long paragraph like this
one exists in the sample to show that soft wrapping follows the column width —
resize the window and the line breaks should move accordingly.

- Unicode shaping works too: héllo — 日本語 — emoji 🎉
- This second item exists mainly to make the list rhythm visible.

> A quote line, rendered as plain text for now.

Run me against a real file:

    cargo run -p dainiki-editor --example workbench -- ~/journal/2026-10-17.md
";

struct Workbench {
    theme: EditorTheme,
    buffer: Entity<Buffer>,
}

impl Render for Workbench {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(self.theme.background)
            .text_color(self.theme.text)
            .flex()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(720.0))
                    .px_6()
                    .py_6()
                    .child(EditorElement::new(self.buffer.clone())),
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
            move |_window, cx| {
                cx.new(|_cx| Workbench {
                    theme: EditorTheme::default(),
                    buffer: buffer.clone(),
                })
            },
        )
        .expect("failed to open workbench window");
        cx.activate(true);
    });
}
