//! Dainiki desktop shell: window chrome, journal persistence and the editor.

mod persistence;

use std::path::PathBuf;
use std::time::Duration;

use dainiki_editor::{Buffer, EditorEvent, EditorTheme, MarkdownEditor};
use gpui_kit::component::TitleBar;
use gpui_kit::*;

actions!(dainiki, [Save]);

/// Autosave runs this long after the last keystroke.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

const WELCOME: &str = "\
# Dainiki

Your journal lives here: local-first, offline, and file-first.

Open a file to write into it — `cargo run -p dainiki-desktop -- ~/journal/2026-10-18.md`
opens (or creates) that day's entry, with autosave and `cmd-S`.
";

struct DainikiApp {
    editor: Entity<MarkdownEditor>,
    theme: EditorTheme,
    path: Option<PathBuf>,
    dirty: bool,
    save_task: Option<Task<()>>,
    _editor_events: Subscription,
}

impl DainikiApp {
    fn new(
        editor: Entity<MarkdownEditor>,
        theme: EditorTheme,
        path: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Self {
        let events = cx.subscribe(&editor, |app, _editor, event, cx| {
            if matches!(event, EditorEvent::Edited) {
                app.dirty = true;
                app.schedule_save(cx);
                cx.notify();
            }
        });
        Self {
            editor,
            theme,
            path,
            dirty: false,
            save_task: None,
            _editor_events: events,
        }
    }

    /// Restarts the debounce window; the previous pending save is dropped.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        if self.path.is_none() {
            return;
        }
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DEBOUNCE).await;
            this.update(cx, |app, cx| app.save_now(cx)).ok();
        }));
    }

    fn save_now(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let text = self.editor.read(cx).content(cx);
        match persistence::write_atomic(&path, &text) {
            Ok(()) => {
                self.dirty = false;
                tracing::info!(path = %path.display(), bytes = text.len(), "journal saved");
            }
            Err(error) => {
                tracing::error!(path = %path.display(), error = %error, "journal save failed");
            }
        }
        cx.notify();
    }

    fn on_save(&mut self, _: &Save, _window: &mut Window, cx: &mut Context<Self>) {
        self.save_now(cx);
    }

    fn window_title(&self) -> String {
        let dirty = if self.dirty { " •" } else { "" };
        match &self.path {
            Some(path) => format!("Dainiki — {}{dirty}", path.display()),
            None => format!("Dainiki{dirty}"),
        }
    }
}

impl Render for DainikiApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(Self::on_save))
            .child(TitleBar::new().child(self.window_title()))
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

    let path = std::env::args_os().nth(1).map(PathBuf::from);

    gpui_kit::application().run(move |cx: &mut App| {
        gpui_kit::init(cx);
        cx.bind_keys([KeyBinding::new("cmd-s", Save, None)]);
        open_app_window(cx, path).expect("failed to open the Dainiki window");
        cx.activate(true);
    });
}

fn open_app_window(cx: &mut App, path: Option<PathBuf>) -> Result<()> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(160.0), px(120.0)),
            size: size(px(880.0), px(700.0)),
        })),
        window_min_size: Some(size(px(480.0), px(360.0))),
        ..TitleBar::window_options()
    };

    let content = match &path {
        Some(path) => match persistence::load(path) {
            Ok(loaded) => loaded.unwrap_or_default(),
            Err(error) => {
                tracing::error!(path = %path.display(), error = %error, "failed to read journal");
                String::new()
            }
        },
        None => WELCOME.to_string(),
    };
    let base_dir = path
        .as_ref()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf));

    gpui_kit::open_window(options, cx, move |window, cx| {
        let theme = EditorTheme::default();
        let buffer = cx.new(|_| Buffer::from_text(&content));
        let editor = cx.new(|cx| {
            let mut editor = MarkdownEditor::new(buffer, theme.clone(), cx);
            editor.set_base_dir(base_dir.clone(), cx);
            editor
        });
        let focus_handle = editor.read(cx).focus_handle().clone();
        window.focus(&focus_handle, cx);
        cx.new(|cx| DainikiApp::new(editor, theme, path, cx))
    })?;

    Ok(())
}
