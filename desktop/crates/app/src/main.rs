//! Dainiki desktop shell: window chrome, toolbar and the editor surface.

use dainiki_editor::{Buffer, EditorTheme, MarkdownEditor};
use gpui_kit::component::TitleBar;
use gpui_kit::*;

const WELCOME: &str = "\
# Dainiki

Your journal lives here: local-first, offline, and file-first. This build wires
the editor into the real application shell — opening and saving real journal
files arrives in the next step.

Start typing to see the caret, selection, undo, clipboard and IME plumbing work
the same as they do in the workbench.
";

struct DainikiApp {
    editor: Entity<MarkdownEditor>,
    theme: EditorTheme,
}

impl Render for DainikiApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(TitleBar::new().child("Dainiki"))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .bg(self.theme.background)
                    .px_10()
                    .py_6()
                    .child(self.editor.clone()),
            )
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();

    gpui_kit::application().run(|cx: &mut App| {
        gpui_kit::init(cx);
        open_app_window(cx).expect("failed to open the Dainiki window");
        cx.activate(true);
    });
}

fn open_app_window(cx: &mut App) -> Result<()> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(160.0), px(120.0)),
            size: size(px(880.0), px(700.0)),
        })),
        window_min_size: Some(size(px(480.0), px(360.0))),
        ..TitleBar::window_options()
    };

    gpui_kit::open_window(options, cx, |window, cx| {
        let theme = EditorTheme::default();
        let buffer = cx.new(|_| Buffer::from_text(WELCOME));
        let editor = cx.new(|cx| MarkdownEditor::new(buffer, theme.clone(), cx));
        let focus_handle = editor.read(cx).focus_handle().clone();
        window.focus(&focus_handle, cx);
        cx.new(|_| DainikiApp { editor, theme })
    })?;

    Ok(())
}
