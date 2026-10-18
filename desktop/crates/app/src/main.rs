//! Dainiki desktop shell: window chrome, journal onboarding and the editor.

mod config;
mod journal;
mod persistence;

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Local;
use dainiki_editor::{
    Buffer, EditorEvent, EditorTheme, MarkdownEditor,
    commands::{BlockFormat, InlineFormat},
};
use gpui_kit::component::Disableable;
use gpui_kit::component::TitleBar;
use gpui_kit::component::button::{Button, ButtonVariants, Toggle, ToggleGroup};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::text::markdown;
use gpui_kit::component::toolbar::Toolbar;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

actions!(dainiki, [Save, TogglePreview]);

/// Autosave runs this long after the last keystroke.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

/// The app shell. A chosen folder turns into a [`JournalSession`]; until then
/// the welcome screen asks for one.
struct DainikiApp {
    theme: EditorTheme,
    config: config::Config,
    screen: Screen,
}

enum Screen {
    /// First run: no journal folder yet — create one or open an existing one.
    Welcome {
        notice: Option<String>,
    },
    Journal(Box<JournalSession>),
}

/// An open journal: today's entry in the editor, autosave and chrome state.
struct JournalSession {
    path: PathBuf,
    editor: Entity<MarkdownEditor>,
    dirty: bool,
    last_seen: Option<String>,
    save_task: Option<Task<()>>,
    link_input: Entity<InputState>,
    preview: bool,
    _editor_events: Subscription,
    _editor_observer: Subscription,
}

impl DainikiApp {
    fn new(theme: EditorTheme, config: config::Config) -> Self {
        Self {
            theme,
            config,
            screen: Screen::Welcome { notice: None },
        }
    }

    fn session_mut(&mut self) -> Option<&mut JournalSession> {
        match &mut self.screen {
            Screen::Journal(session) => Some(session),
            Screen::Welcome { .. } => None,
        }
    }

    fn show_notice(&mut self, message: String, cx: &mut Context<Self>) {
        self.screen = Screen::Welcome {
            notice: Some(message),
        };
        cx.notify();
    }

    /// Opens `root` as the journal; a failure lands back on the welcome
    /// screen with the reason shown.
    fn enter_journal(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        match self.open_session(&root, window, cx) {
            Ok(()) => {}
            Err(error) => {
                tracing::error!(root = %root.display(), error = %error, "could not open journal");
                self.show_notice(format!("Could not open {} — {error}", root.display()), cx);
            }
        }
    }

    /// Makes sure today's entry exists, loads it, builds the editor, and
    /// swaps the screen to the journal.
    fn open_session(
        &mut self,
        root: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> io::Result<()> {
        let journal = journal::Journal::new(root);
        let today = Local::now().date_naive();
        let path = journal.ensure_entry(today)?;
        let loaded = persistence::load(&path)?;
        let content = loaded.clone().unwrap_or_default();
        let base_dir = path.parent().map(Path::to_path_buf);

        let theme = self.theme.clone();
        let buffer = cx.new(|_| Buffer::from_text(&content));
        let editor = cx.new(|cx| {
            let mut editor = MarkdownEditor::new(buffer, theme.clone(), cx);
            editor.set_base_dir(base_dir, cx);
            editor
        });
        let link_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("URL to link — select the text first")
        });
        let events = cx.subscribe(&editor, |app, _editor, event, cx| {
            if matches!(event, EditorEvent::Edited) {
                if let Some(session) = app.session_mut() {
                    session.dirty = true;
                }
                app.schedule_save(cx);
                cx.notify();
            }
        });
        let observer = cx.observe(&editor, |_app, _editor, cx| cx.notify());

        window.focus(&editor.read(cx).focus_handle().clone(), cx);
        tracing::info!(root = %root.display(), path = %path.display(), "journal opened");
        self.screen = Screen::Journal(Box::new(JournalSession {
            path,
            editor,
            dirty: false,
            last_seen: loaded,
            save_task: None,
            link_input,
            preview: false,
            _editor_events: events,
            _editor_observer: observer,
        }));
        cx.notify();
        Ok(())
    }

    // -- welcome actions ----------------------------------------------------

    fn create_journal_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose where to create your journal".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            let Some(parent) = paths.into_iter().next() else {
                return;
            };
            this.update_in(cx, |app, window, cx| {
                app.create_journal_in(parent, window, cx)
            })
            .ok();
        })
        .detach();
    }

    fn create_journal_in(&mut self, parent: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        match journal::create_journal_dir(&parent) {
            Ok(root) => {
                self.remember_journal(&root);
                self.enter_journal(root, window, cx);
            }
            Err(error) => {
                tracing::error!(parent = %parent.display(), error = %error, "could not create the journal folder");
                self.show_notice(
                    format!("Could not create a journal in {}", parent.display()),
                    cx,
                );
            }
        }
    }

    fn open_journal_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose your journal folder".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            let Some(root) = paths.into_iter().next() else {
                return;
            };
            this.update_in(cx, |app, window, cx| {
                app.remember_journal(&root);
                app.enter_journal(root, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Best-effort persistence of the chosen journal; a failed config write
    /// only means the choice is asked again next launch.
    fn remember_journal(&mut self, root: &Path) {
        self.config.journal_root = Some(root.to_path_buf());
        if let Err(error) = config::save(&self.config) {
            tracing::warn!(root = %root.display(), error = %error, "could not save the journal choice");
        }
    }

    /// Persists a preferences change (the in-memory value stays authoritative
    /// for this session even when the write fails).
    fn set_show_empty_days(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.config.show_empty_days == show {
            return;
        }
        self.config.show_empty_days = show;
        if let Err(error) = config::save(&self.config) {
            tracing::warn!(error = %error, "could not save preferences");
        }
        cx.notify();
    }

    /// Journal preferences, available from the toolbar.
    fn settings_menu(&self, cx: &Context<Self>) -> Popover {
        let this = cx.weak_entity();
        let show_empty_days = self.config.show_empty_days;
        Popover::new("settings")
            .trigger(Button::new("settings-trigger").label("Settings").compact())
            .content(move |_state, _window, _cx| {
                let this = this.clone();
                v_flex()
                    .w(px(320.))
                    .gap_2()
                    .text_sm()
                    .child(
                        Toggle::new("show-empty-days")
                            .label("Show days without entries")
                            .checked(show_empty_days)
                            .on_click(move |checked, _window, cx| {
                                this.update(cx, |app, cx| app.set_show_empty_days(*checked, cx))
                                    .ok();
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .child("Empty days appear in the timeline, ready to fill in later."),
                    )
            })
    }

    // -- journal actions ----------------------------------------------------

    /// Restarts the debounce window; the previous pending save is dropped.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session_mut() else {
            return;
        };
        session.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DEBOUNCE).await;
            this.update(cx, |app, cx| app.save_now(cx)).ok();
        }));
    }

    fn save_now(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session_mut() else {
            return;
        };
        let path = session.path.clone();
        let text = session.editor.read(cx).content(cx);
        match persistence::save(&path, &text, session.last_seen.as_deref()) {
            persistence::SaveOutcome::Saved => {
                session.dirty = false;
                let bytes = text.len();
                session.last_seen = Some(text);
                tracing::info!(path = %path.display(), bytes, "journal saved");
            }
            persistence::SaveOutcome::ConflictCopy(conflict) => {
                session.dirty = false;
                session.last_seen = std::fs::read_to_string(&path).ok();
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
        let Some(session) = self.session_mut() else {
            return;
        };
        session.preview = !session.preview;
        if !session.preview {
            let focus = session.editor.read(cx).focus_handle().clone();
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn welcome_view(&self, notice: &Option<String>, cx: &Context<Self>) -> impl IntoElement {
        let create = cx.weak_entity();
        let open = cx.weak_entity();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(TitleBar::new().child("Dainiki"))
            .child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .bg(self.theme.background)
                    .child(div().text_2xl().child("Welcome to Dainiki"))
                    .child(div().max_w(px(440.)).text_sm().child(
                        "Your journal is a folder of plain Markdown files you own — \
                                 offline, no account, no lock-in. Create one, or open a folder \
                                 you already have.",
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .mt_2()
                            .child(
                                Button::new("create-journal")
                                    .label("Create new journal")
                                    .primary()
                                    .on_click(move |_, window, cx| {
                                        create
                                            .update(cx, |app, cx| {
                                                app.create_journal_prompt(window, cx)
                                            })
                                            .ok();
                                    }),
                            )
                            .child(
                                Button::new("open-journal")
                                    .label("Open existing folder")
                                    .on_click(move |_, window, cx| {
                                        open.update(cx, |app, cx| {
                                            app.open_journal_prompt(window, cx)
                                        })
                                        .ok();
                                    }),
                            ),
                    )
                    .children(notice.as_ref().map(|notice| {
                        div().mt_2().max_w(px(440.)).text_sm().child(notice.clone())
                    })),
            )
    }
}

impl JournalSession {
    fn view(&self, theme: &EditorTheme, settings: Popover, cx: &App) -> impl IntoElement {
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
                        .child(settings)
                        .child(self.preview_toggle())
                        .child(self.help_menu()),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .bg(theme.background)
                    .px_10()
                    .py_6()
                    .child(content),
            )
    }

    fn window_title(&self) -> String {
        let dirty = if self.dirty { " •" } else { "" };
        format!("Dainiki — {}{dirty}", self.path.display())
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

    fn inline_toggle(
        &self,
        id: &'static str,
        label: &'static str,
        format: InlineFormat,
        tooltip: &'static str,
        cx: &App,
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
        cx: &App,
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

    fn block_toolbar(&self, cx: &App) -> ToggleGroup {
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

    fn inline_toolbar(&self, cx: &App) -> ToggleGroup {
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
        let document_dir = self.path.parent().map(Path::to_path_buf);

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
        let screen = match &self.screen {
            Screen::Welcome { notice } => self.welcome_view(notice, cx).into_any_element(),
            Screen::Journal(session) => {
                let settings = self.settings_menu(cx);
                session.view(&self.theme, settings, cx).into_any_element()
            }
        };
        div()
            .size_full()
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_toggle_preview))
            .child(screen)
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();

    // A folder argument overrides the configured journal for this session;
    // it is never persisted.
    let config = config::load();
    let cli_root = resolve_cli_root(std::env::args_os().nth(1).map(PathBuf::from));
    let root = cli_root.or_else(|| config.journal_root.clone());
    if let Some(root) = &root {
        tracing::info!(root = %root.display(), "opening journal");
    }

    gpui_kit::application().run(move |cx: &mut App| {
        gpui_kit::init(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-s", Save, None),
            KeyBinding::new("cmd-e", TogglePreview, None),
        ]);
        open_app_window(cx, config, root.clone()).expect("failed to open the Dainiki window");
        cx.activate(true);
    });
}

/// Resolves a folder argument for this session: an existing folder is used
/// as-is; a path that does not exist yet is created, so a fresh journal can
/// be spun up from the command line; anything else is ignored.
fn resolve_cli_root(arg: Option<PathBuf>) -> Option<PathBuf> {
    let path = arg?;
    if path.is_dir() {
        return Some(path);
    }
    if !path.exists() {
        return match std::fs::create_dir_all(&path) {
            Ok(()) => {
                tracing::info!(path = %path.display(), "created the journal folder from the argument");
                Some(path)
            }
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "could not create the journal folder");
                None
            }
        };
    }
    tracing::warn!(path = %path.display(), "argument ignored: pass a journal folder");
    None
}

fn open_app_window(cx: &mut App, config: config::Config, root: Option<PathBuf>) -> Result<()> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(160.0), px(120.0)),
            size: size(px(880.0), px(700.0)),
        })),
        window_min_size: Some(size(px(480.0), px(360.0))),
        ..TitleBar::window_options()
    };

    gpui_kit::open_window(options, cx, move |window, cx| {
        let theme = EditorTheme::default();
        cx.new(|cx| {
            let mut app = DainikiApp::new(theme, config);
            if let Some(root) = root {
                if root.is_dir() {
                    app.enter_journal(root, window, cx);
                } else {
                    app.show_notice(format!("Journal folder not found — {}", root.display()), cx);
                }
            }
            app
        })
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::Local;
    use gpui::TestAppContext;

    // NOTE: no `use super::*` here. The crate root glob-imports `gpui_kit::*`,
    // which re-exports gpui's `test` proc macro; globbing it in would shadow
    // the builtin `#[test]` inside the generated harness and recurse forever.
    use super::{DainikiApp, EditorTheme, Screen, config, journal, resolve_cli_root};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// The component library needs its theme global before any view renders.
    fn init(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
    }

    fn scratch_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dainiki-app-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn no_argument_resolves_to_nothing() {
        assert_eq!(resolve_cli_root(None), None);
    }

    #[test]
    fn an_existing_folder_argument_is_used_as_is() {
        let dir = scratch_dir();
        assert_eq!(resolve_cli_root(Some(dir.clone())), Some(dir.clone()));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_folder_argument_is_created() {
        let dir = scratch_dir();
        let fresh = dir.join("fresh-journal");
        assert_eq!(resolve_cli_root(Some(fresh.clone())), Some(fresh.clone()));
        assert!(fresh.is_dir());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_argument_is_ignored() {
        let dir = scratch_dir();
        let file = dir.join("entry.md");
        fs::write(&file, "x").expect("seed");
        assert_eq!(resolve_cli_root(Some(file)), None);
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn the_app_starts_on_the_welcome_screen(cx: &mut TestAppContext) {
        init(cx);
        let (app, cx) = cx.add_window_view(|_window, _cx| {
            DainikiApp::new(EditorTheme::default(), config::Config::default())
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            assert!(matches!(
                app.read(cx).screen,
                Screen::Welcome { notice: None }
            ));
        });
    }

    #[gpui::test]
    async fn entering_a_journal_creates_and_loads_todays_entry(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        let path = journal.ensure_entry(today).expect("ensure");
        fs::write(&path, "# written earlier today\n").expect("seed");

        let (app, cx) = cx.add_window_view({
            let dir = dir.clone();
            move |window, cx| {
                let mut app = DainikiApp::new(EditorTheme::default(), config::Config::default());
                app.enter_journal(dir, window, cx);
                app
            }
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            assert_eq!(session.path, path);
            assert_eq!(
                session.editor.read(cx).content(cx),
                "# written earlier today\n"
            );
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn a_failed_open_lands_on_welcome_with_a_notice(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let not_a_folder = dir.join("entry.md");
        fs::write(&not_a_folder, "not a journal").expect("seed");

        let (app, cx) = cx.add_window_view({
            let root = not_a_folder.clone();
            move |window, cx| {
                let mut app = DainikiApp::new(EditorTheme::default(), config::Config::default());
                app.enter_journal(root, window, cx);
                app
            }
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            assert!(matches!(
                app.read(cx).screen,
                Screen::Welcome { notice: Some(_) }
            ));
        });
        fs::remove_dir_all(&dir).ok();
    }
}
