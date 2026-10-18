use std::fs;

use dainiki_editor::{
    Buffer, EditorTheme, MarkdownEditor,
    commands::{BlockFormat, InlineFormat},
};
use gpui::{
    App, Bounds, Context, Entity, Hsla, MouseButton, Render, Subscription, TitlebarOptions, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, size,
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

Inline styles: **bold**, *italic*, ~~struck through~~, `inline code` and
==highlighted== all render collapsed; put the caret inside a span to reveal its
markers. Links collapse to colored text: hover
[the Dainiki repository](https://github.com/MrSheerluck/smbl-dainiki) for the
URL, cmd-click to open it, click to reveal the source. Headings collapse to
real sizes once the caret leaves their line.

Images reference files next to the journal; a missing one shows a labeled
placeholder until you point it at a real file:

![missing](assets/does-not-exist.png)

- Unicode shaping works too: héllo - 日本語 - emoji 🎉
- This second item exists mainly to make the list rhythm visible.
- [ ] Click this checkbox: it toggles without moving the caret, and one undo
      restores it.
- [x] This one starts checked.

> A quote line, rendered as plain text for now.

Scroll to the end to confirm the indicator reaches the bottom and the last line
stays fully visible above it:

```rust
fn main() {
    build_a_journal();
}
```

One more paragraph so the sample reliably overflows a 640 pt window. Type / on
any blank spot to format without writing Markdown: headings, lists, quotes,
code, links — all keyboard-driven.
";

struct Workbench {
    theme: EditorTheme,
    editor: Entity<MarkdownEditor>,
    _editor_events: Subscription,
}

impl Workbench {
    /// The same operations as the app toolbar, so the harness always shows
    /// every action the editor supports. Active states follow the caret.
    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme.clone();
        let mut buttons: Vec<gpui::AnyElement> = Vec::new();

        for (label, format) in [
            ("B", InlineFormat::Bold),
            ("I", InlineFormat::Italic),
            ("S", InlineFormat::Strike),
            ("H", InlineFormat::Highlight),
            ("`", InlineFormat::Code),
        ] {
            let active = self.editor.read(cx).inline_format_active(format);
            let editor = self.editor.clone();
            buttons.push(
                toolbar_button(label, active, &theme, move |_window, cx| {
                    editor.update(cx, |editor, cx| editor.toggle_inline_format(format, cx));
                })
                .into_any_element(),
            );
        }
        buttons.push(toolbar_separator(&theme).into_any_element());

        for (label, format) in [
            ("H1", BlockFormat::Heading(1)),
            ("H2", BlockFormat::Heading(2)),
            ("H3", BlockFormat::Heading(3)),
            ("•", BlockFormat::Bullet),
            ("1.", BlockFormat::Ordered),
            ("☐", BlockFormat::Task),
            (">", BlockFormat::Quote),
        ] {
            let active = self.editor.read(cx).block_format_active(format, cx);
            let editor = self.editor.clone();
            buttons.push(
                toolbar_button(label, active, &theme, move |_window, cx| {
                    editor.update(cx, |editor, cx| editor.toggle_block_format(format, cx));
                })
                .into_any_element(),
            );
        }
        buttons.push(toolbar_separator(&theme).into_any_element());

        let editor = self.editor.clone();
        buttons.push(
            toolbar_button("Link", false, &theme, move |_window, cx| {
                editor.update(cx, |editor, cx| editor.insert_link("", cx));
            })
            .into_any_element(),
        );
        let editor = self.editor.clone();
        buttons.push(
            toolbar_button("Code block", false, &theme, move |_window, cx| {
                editor.update(cx, |editor, cx| editor.insert_code_fence(cx));
            })
            .into_any_element(),
        );
        let editor = self.editor.clone();
        buttons.push(
            toolbar_button("Divider", false, &theme, move |_window, cx| {
                editor.update(cx, |editor, cx| editor.insert_rule(cx));
            })
            .into_any_element(),
        );

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.))
            .px(px(10.))
            .py(px(6.))
            .border_b(px(1.))
            .border_color(Hsla {
                a: 0.08,
                ..theme.text
            })
            .children(buttons)
    }
}

fn toolbar_button(
    label: &'static str,
    active: bool,
    theme: &EditorTheme,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let background = if active {
        theme.selection
    } else {
        theme.background
    };
    let hover_background = Hsla {
        a: 0.08,
        ..theme.text
    };
    div()
        .px(px(7.))
        .py(px(3.))
        .rounded(px(6.))
        .text_sm()
        .text_color(theme.text)
        .bg(background)
        .hover(move |style| style.bg(hover_background))
        .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
            on_click(window, cx);
        })
        .child(label)
}

fn toolbar_separator(theme: &EditorTheme) -> impl IntoElement {
    div().w(px(1.)).h(px(16.)).mx(px(4.)).bg(Hsla {
        a: 0.15,
        ..theme.text
    })
}

impl Render for Workbench {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(self.theme.background)
            .flex()
            .flex_col()
            .child(self.toolbar(cx))
            .child(
                div().flex_1().w_full().flex().justify_center().child(
                    div()
                        .w_full()
                        .max_w(px(720.0))
                        .h_full()
                        .px_6()
                        .py_6()
                        .child(self.editor.clone()),
                ),
            )
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn main() {
    init_tracing();

    let mut base_dir = None;
    let text = match std::env::args().nth(1) {
        Some(path) => {
            base_dir = std::path::Path::new(&path)
                .parent()
                .map(std::path::Path::to_path_buf);
            match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(error) => {
                    eprintln!("failed to read {path}: {error}");
                    std::process::exit(1);
                }
            }
        }
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
                let editor = cx.new(|cx| {
                    let mut editor = MarkdownEditor::new(buffer.clone(), theme.clone(), cx);
                    editor.set_base_dir(base_dir.clone(), cx);
                    editor
                });
                let focus_handle = editor.read(cx).focus_handle().clone();
                window.focus(&focus_handle, cx);
                cx.new(|cx| {
                    let _editor_events = cx.observe(&editor, |_this, _editor, cx| cx.notify());
                    Workbench {
                        theme,
                        editor,
                        _editor_events,
                    }
                })
            },
        )
        .expect("failed to open workbench window");
        cx.activate(true);
    });
}
