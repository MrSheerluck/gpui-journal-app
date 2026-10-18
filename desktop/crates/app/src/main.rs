//! Dainiki desktop shell: window chrome, journal persistence and the editor.

mod persistence;

use std::path::{Path, PathBuf};
use std::time::Duration;

use dainiki_editor::{
    Buffer, EditorEvent, EditorTheme, MarkdownEditor,
    commands::{BlockFormat, InlineFormat},
};
use gpui_kit::component::Disableable;
use gpui_kit::component::TitleBar;
use gpui_kit::component::button::{Button, Toggle, ToggleGroup};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::text::markdown;
use gpui_kit::component::toolbar::Toolbar;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

actions!(dainiki, [Save, TogglePreview]);

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
    last_seen: Option<String>,
    save_task: Option<Task<()>>,
    link_input: Entity<InputState>,
    preview: bool,
    _editor_events: Subscription,
    _editor_observer: Subscription,
}

impl DainikiApp {
    fn new(
        editor: Entity<MarkdownEditor>,
        theme: EditorTheme,
        path: Option<PathBuf>,
        last_seen: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let link_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("URL to link — select the text first")
        });
        let events = cx.subscribe(&editor, |app, _editor, event, cx| {
            if matches!(event, EditorEvent::Edited) {
                app.dirty = true;
                app.schedule_save(cx);
                cx.notify();
            }
        });
        let observer = cx.observe(&editor, |_app, _editor, cx| cx.notify());
        Self {
            editor,
            theme,
            path,
            dirty: false,
            last_seen,
            save_task: None,
            link_input,
            preview: false,
            _editor_events: events,
            _editor_observer: observer,
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
        match persistence::save(&path, &text, self.last_seen.as_deref()) {
            persistence::SaveOutcome::Saved => {
                self.dirty = false;
                let bytes = text.len();
                self.last_seen = Some(text);
                tracing::info!(path = %path.display(), bytes, "journal saved");
            }
            persistence::SaveOutcome::ConflictCopy(conflict) => {
                self.dirty = false;
                self.last_seen = std::fs::read_to_string(&path).ok();
                tracing::error!(
                    path = %conflict.display(),
                    "the journal changed on disk; preserved this version as a conflict copy"
                );
            }
            persistence::SaveOutcome::Failed(error) => {
                tracing::error!(path = %path.display(), error = %error, "journal save failed");
            }
        }
        cx.notify();
    }

    fn on_save(&mut self, _: &Save, _window: &mut Window, cx: &mut Context<Self>) {
        self.save_now(cx);
    }

    fn on_toggle_preview(
        &mut self,
        _: &TogglePreview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preview = !self.preview;
        if !self.preview {
            let focus = self.editor.read(cx).focus_handle().clone();
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn preview_toggle(&self) -> Toggle {
        Toggle::new("preview")
            .label("Read")
            .tooltip("Reading view — ⌘E")
            .checked(self.preview)
            .on_click(move |_checked, window, cx| {
                window.dispatch_action(Box::new(TogglePreview), cx);
            })
    }

    fn window_title(&self) -> String {
        let dirty = if self.dirty { " •" } else { "" };
        match &self.path {
            Some(path) => format!("Dainiki — {}{dirty}", path.display()),
            None => format!("Dainiki{dirty}"),
        }
    }

    fn inline_toggle(
        &self,
        id: &'static str,
        label: &'static str,
        format: InlineFormat,
        tooltip: &'static str,
        cx: &Context<Self>,
    ) -> Toggle {
        let active = self.editor.read(cx).inline_format_active(format);
        let editor = self.editor.clone();
        Toggle::new(id)
            .label(label)
            .tooltip(tooltip)
            .checked(active)
            .on_click(move |_checked, _window, cx| {
                editor.update(cx, |editor, cx| editor.toggle_inline_format(format, cx));
            })
    }

    fn block_toggle(
        &self,
        id: &'static str,
        label: &'static str,
        format: BlockFormat,
        tooltip: &'static str,
        cx: &Context<Self>,
    ) -> Toggle {
        let active = self.editor.read(cx).block_format_active(format, cx);
        let editor = self.editor.clone();
        Toggle::new(id)
            .label(label)
            .tooltip(tooltip)
            .checked(active)
            .on_click(move |_checked, _window, cx| {
                editor.update(cx, |editor, cx| editor.toggle_block_format(format, cx));
            })
    }

    fn block_toolbar(&self, cx: &mut Context<Self>) -> ToggleGroup {
        ToggleGroup::new("block-formats").segmented().children([
            self.block_toggle("h1", "H1", BlockFormat::Heading(1), "Heading 1 — ⌘1", cx),
            self.block_toggle("h2", "H2", BlockFormat::Heading(2), "Heading 2 — ⌘2", cx),
            self.block_toggle("h3", "H3", BlockFormat::Heading(3), "Heading 3 — ⌘3", cx),
            self.block_toggle("bullet", "•", BlockFormat::Bullet, "Bullet list — ⌘⇧8", cx),
            self.block_toggle(
                "ordered",
                "1.",
                BlockFormat::Ordered,
                "Numbered list — ⌘⇧7",
                cx,
            ),
            self.block_toggle("task", "☐", BlockFormat::Task, "Task list — ⌘⇧9", cx),
            self.block_toggle("quote", ">", BlockFormat::Quote, "Quote", cx),
        ])
    }

    fn inline_toolbar(&self, cx: &mut Context<Self>) -> ToggleGroup {
        ToggleGroup::new("inline-formats").segmented().children([
            self.inline_toggle("bold", "B", InlineFormat::Bold, "Bold — ⌘B", cx),
            self.inline_toggle("italic", "I", InlineFormat::Italic, "Italic — ⌘I", cx),
            self.inline_toggle(
                "strike",
                "S",
                InlineFormat::Strike,
                "Strikethrough — ⌘⇧X",
                cx,
            ),
            self.inline_toggle(
                "highlight",
                "H",
                InlineFormat::Highlight,
                "Highlight — ⌘⇧H",
                cx,
            ),
            self.inline_toggle("code", "`", InlineFormat::Code, "Inline code — ⌘`", cx),
        ])
    }

    fn help_menu(&self) -> Popover {
        const SHORTCUTS: [(&str, &str); 15] = [
            ("Command menu", "type /"),
            ("Bold / Italic", "⌘B / ⌘I"),
            ("Strikethrough / Highlight", "⌘⇧X / ⌘⇧H"),
            ("Inline code", "⌘`"),
            ("Link", "⌘K"),
            ("Headings 1–3", "⌘1 … ⌘3"),
            ("Numbered / Bullet / Task list", "⌘⇧7 / ⌘⇧8 / ⌘⇧9"),
            ("Undo / Redo", "⌘Z / ⌘⇧Z"),
            ("Select all", "⌘A"),
            ("Copy / Cut / Paste", "⌘C / ⌘X / ⌘V"),
            ("Word step", "⌥← / ⌥→"),
            ("Line / Document jumps", "⌘← / ⌘↑"),
            ("Save", "⌘S"),
            ("New line / Tab", "⏎ / ⇥"),
            ("Reveal markup", "click into a construct"),
        ];
        Popover::new("shortcut-help")
            .trigger(Button::new("help-trigger").label("?").compact())
            .content(move |_state, _window, _cx| {
                v_flex()
                    .w(px(360.))
                    .gap_1()
                    .text_sm()
                    .children(SHORTCUTS.map(|(action, keys)| {
                        h_flex().justify_between().gap_4().child(action).child(keys)
                    }))
            })
    }

    fn insert_menu(&self) -> Popover {
        let editor = self.editor.clone();
        let link_input = self.link_input.clone();
        let document_dir = self
            .path
            .as_ref()
            .and_then(|path| path.parent().map(Path::to_path_buf));

        let link_editor = editor.clone();
        let link_state = link_input.clone();
        let fence_editor = editor.clone();
        let rule_editor = editor.clone();
        let image_editor = editor.clone();
        let image_dir = document_dir.clone();

        Popover::new("insert-menu")
            .trigger(Button::new("insert-trigger").label("Insert").compact())
            .content(move |_state, _window, _cx| {
                let link = Button::new("insert-link")
                    .label("Link")
                    .compact()
                    .on_click({
                        let editor = link_editor.clone();
                        let input = link_state.clone();
                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                            let url = input.read(cx).value().to_string();
                            let url = url.trim().to_string();
                            if url.is_empty() {
                                return;
                            }
                            editor.update(cx, |editor, cx| editor.insert_link(&url, cx));
                            input.update(cx, |input, cx| input.set_value("", window, cx));
                        }
                    });
                let image = Button::new("insert-image")
                    .label("Image…")
                    .compact()
                    .disabled(image_dir.is_none())
                    .on_click({
                        let editor = image_editor.clone();
                        let directory = image_dir.clone();
                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                            let Some(directory) = directory.clone() else {
                                return;
                            };
                            let prompt = cx.prompt_for_paths(PathPromptOptions {
                                files: true,
                                directories: false,
                                multiple: false,
                                prompt: Some("Choose an image".into()),
                            });
                            let editor = editor.clone();
                            window
                                .spawn(cx, async move |cx| {
                                    let Ok(Ok(Some(paths))) = prompt.await else {
                                        return;
                                    };
                                    let Some(source) = paths.into_iter().next() else {
                                        return;
                                    };
                                    let assets = directory.join("assets");
                                    let copied = cx
                                        .background_executor()
                                        .spawn(async move { copy_image(&source, &assets) })
                                        .await;
                                    let Ok((alt, relative)) = copied else {
                                        return;
                                    };
                                    editor.update(cx, |editor, cx| {
                                        editor.insert_image(&alt, &relative, cx)
                                    });
                                })
                                .detach();
                        }
                    });
                let fence = Button::new("insert-fence")
                    .label("Code fence")
                    .compact()
                    .on_click({
                        let editor = fence_editor.clone();
                        move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                            editor.update(cx, |editor, cx| editor.insert_code_fence(cx));
                        }
                    });
                let rule = Button::new("insert-rule")
                    .label("Horizontal rule")
                    .compact()
                    .on_click({
                        let editor = rule_editor.clone();
                        move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                            editor.update(cx, |editor, cx| editor.insert_rule(cx));
                        }
                    });

                v_flex()
                    .w(px(300.))
                    .gap_2()
                    .child(Input::new(&link_state))
                    .child(h_flex().gap_2().child(link).child(image))
                    .child(h_flex().gap_2().child(fence).child(rule))
            })
    }
}

/// Copies an image into the journal's `assets/` folder under a fresh name,
/// returning `(alt text, relative markdown path)`.
fn copy_image(source: &Path, assets: &Path) -> Result<(String, String), String> {
    std::fs::create_dir_all(assets).map_err(|error| error.to_string())?;
    let stem = source
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".to_string());
    let extension = source
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut counter = 0usize;
    let name = loop {
        let name = if counter == 0 {
            source
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "image".to_string())
        } else if extension.is_empty() {
            format!("{stem}-{counter}")
        } else {
            format!("{stem}-{counter}.{extension}")
        };
        if !assets.join(&name).exists() {
            break name;
        }
        counter += 1;
    };
    std::fs::copy(source, assets.join(&name)).map_err(|error| error.to_string())?;
    Ok((stem, format!("assets/{name}")))
}

impl Render for DainikiApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = if self.preview {
            let text = self.editor.read(cx).content(cx);
            div()
                .id("reading-view")
                .w_full()
                .max_w(px(720.))
                .overflow_y_scroll()
                .child(markdown(text))
                .into_any_element()
        } else {
            self.editor.clone().into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_toggle_preview))
            .child(TitleBar::new().child(self.window_title()))
            .child(
                div().w_full().px_3().py_1().child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            Toolbar::new("editor-toolbar")
                                .child(self.inline_toolbar(cx))
                                .child(self.block_toolbar(cx)),
                        )
                        .child(self.insert_menu())
                        .child(self.preview_toggle())
                        .child(self.help_menu()),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .bg(self.theme.background)
                    .px_10()
                    .py_6()
                    .child(content),
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
        cx.bind_keys([
            KeyBinding::new("cmd-s", Save, None),
            KeyBinding::new("cmd-e", TogglePreview, None),
        ]);
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

    let loaded = match &path {
        Some(path) => match persistence::load(path) {
            Ok(loaded) => loaded,
            Err(error) => {
                tracing::error!(path = %path.display(), error = %error, "failed to read journal");
                None
            }
        },
        None => None,
    };
    let last_seen = loaded.clone();
    let content = loaded.unwrap_or_else(|| {
        if path.is_some() {
            String::new()
        } else {
            WELCOME.to_string()
        }
    });
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
        cx.new(|cx| DainikiApp::new(editor, theme, path, last_seen, window, cx))
    })?;

    Ok(())
}
