//! Dainiki desktop shell: window chrome, journal onboarding and the editor.
//!
//! App chrome and standard UI use gpui-kit components wherever one fits
//! (ADR-0006); the editor crate stays GPUI-primitives-only.

mod brand;
mod config;
mod journal;
mod persistence;

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{Local, NaiveDate};
use dainiki_editor::{
    Buffer, EditorEvent, EditorTheme, MarkdownEditor,
    commands::{BlockFormat, InlineFormat},
};
use gpui_kit::component::Disableable;
use gpui_kit::component::TitleBar;
use gpui_kit::component::button::{Button, ButtonVariants, Toggle, ToggleGroup};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::scroll::{Scrollbar, ScrollbarAxis};
use gpui_kit::component::text::markdown;
use gpui_kit::component::toolbar::Toolbar;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

actions!(dainiki, [Save, ToggleEditing]);

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

/// An open journal: the stream of days, with at most one day open for editing.
struct JournalSession {
    root: PathBuf,
    /// The day the editor is editing; `None` = reading the stream.
    active: Option<ActiveDay>,
    /// Every day with content except the active one, rendered read-only.
    days: BTreeMap<NaiveDate, String>,
    /// The virtualized stream: one item per visible-range day, plus the edge.
    stream: ListState,
    /// Which day the stream last rendered as the live editor, so a day that
    /// swapped between editor and render (a different height) is remeasured.
    active_rendered: Cell<Option<NaiveDate>>,
}

/// The day currently open in the editor, with its autosave state.
struct ActiveDay {
    date: NaiveDate,
    path: PathBuf,
    editor: Entity<MarkdownEditor>,
    link_input: Entity<InputState>,
    dirty: bool,
    last_seen: Option<String>,
    save_task: Option<Task<()>>,
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

    fn session(&self) -> Option<&JournalSession> {
        match &self.screen {
            Screen::Journal(session) => Some(session),
            Screen::Welcome { .. } => None,
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

    /// Opens the journal with today's entry in the editor and the past days
    /// loaded for the stream, and swaps the screen to it. Only today's file
    /// is read on the UI thread; the rest of the journal loads in the
    /// background and fills the stream in.
    fn open_session(
        &mut self,
        root: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> io::Result<()> {
        let today = Local::now().date_naive();
        let active = self.open_day(root, today, window, cx)?;
        tracing::info!(root = %root.display(), path = %active.path.display(), "journal opened");
        self.screen = Screen::Journal(Box::new(JournalSession {
            root: root.to_path_buf(),
            active: Some(active),
            days: BTreeMap::new(),
            stream: ListState::new(0, ListAlignment::Top, px(600.)),
            active_rendered: Cell::new(None),
        }));
        cx.notify();

        // The full scan runs off the UI thread; when it lands, merge in the
        // days we do not already have (an in-session day is newer than disk).
        let scan_root = root.to_path_buf();
        let scan = cx
            .background_executor()
            .spawn(async move { read_days(&scan_root, today) });
        cx.spawn(async move |this, cx| {
            let days = scan.await;
            this.update(cx, |app, cx| {
                if let Some(session) = app.session_mut() {
                    for (date, text) in days {
                        session.days.entry(date).or_insert(text);
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        Ok(())
    }

    /// Builds the editing state for one day: ensures the entry exists, loads
    /// it, builds a fresh editor (undo never crosses days) with the caret at
    /// the end, and focuses it.
    fn open_day(
        &self,
        root: &Path,
        date: NaiveDate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> io::Result<ActiveDay> {
        let journal = journal::Journal::new(root);
        let path = journal.ensure_entry(date)?;
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
        editor.update(cx, |editor, cx| editor.place_caret_at_end(cx));
        let link_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("URL to link — select the text first")
        });
        let events = cx.subscribe(&editor, |app, _editor, event, cx| {
            if matches!(event, EditorEvent::Edited) {
                if let Some(active) = app
                    .session_mut()
                    .and_then(|session| session.active.as_mut())
                {
                    active.dirty = true;
                }
                app.schedule_save(cx);
                cx.notify();
            }
        });
        let observer = cx.observe(&editor, |_app, _editor, cx| cx.notify());

        window.focus(&editor.read(cx).focus_handle().clone(), cx);
        Ok(ActiveDay {
            date,
            path,
            editor,
            link_input,
            dirty: false,
            last_seen: loaded,
            save_task: None,
            _editor_events: events,
            _editor_observer: observer,
        })
    }

    // -- editing ----------------------------------------------------------

    /// Opens `date` for editing, flushing and stashing the day being left.
    fn activate_day(&mut self, date: NaiveDate, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .session()
            .and_then(|session| session.active.as_ref())
            .is_some_and(|active| active.date == date)
        {
            return;
        }
        self.stop_editing(cx);
        let Some(root) = self.session().map(|session| session.root.clone()) else {
            return;
        };
        match self.open_day(&root, date, window, cx) {
            Ok(active) => {
                if let Some(session) = self.session_mut() {
                    session.days.remove(&date);
                    session.active = Some(active);
                }
                cx.notify();
            }
            Err(error) => {
                tracing::error!(date = %date, error = %error, "could not open the day");
            }
        }
    }

    /// Flushes the active day's save, stashes its text with the other days,
    /// and drops the editor — back to reading the stream.
    fn stop_editing(&mut self, cx: &mut Context<Self>) {
        self.save_active(cx);
        let Some(previous) = self.session_mut().and_then(|session| session.active.take()) else {
            return;
        };
        let text = previous.editor.read(cx).content(cx);
        if let Some(session) = self.session_mut() {
            if text.trim().is_empty() {
                session.days.remove(&previous.date);
            } else {
                session.days.insert(previous.date, text);
            }
        }
        cx.notify();
    }

    fn on_toggle_editing(
        &mut self,
        _: &ToggleEditing,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editing = self
            .session()
            .is_some_and(|session| session.active.is_some());
        if editing {
            self.stop_editing(cx);
        } else {
            self.activate_day(Local::now().date_naive(), window, cx);
        }
    }

    /// Restarts the debounce window; the previous pending save is dropped.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self
            .session_mut()
            .and_then(|session| session.active.as_mut())
        else {
            return;
        };
        active.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DEBOUNCE).await;
            this.update(cx, |app, cx| app.save_active(cx)).ok();
        }));
    }

    fn save_active(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self
            .session_mut()
            .and_then(|session| session.active.as_mut())
        else {
            return;
        };
        let path = active.path.clone();
        let text = active.editor.read(cx).content(cx);
        match persistence::save(&path, &text, active.last_seen.as_deref()) {
            persistence::SaveOutcome::Saved => {
                active.dirty = false;
                let bytes = text.len();
                active.last_seen = Some(text);
                tracing::info!(path = %path.display(), bytes, "journal saved");
            }
            persistence::SaveOutcome::ConflictCopy(conflict) => {
                active.dirty = false;
                active.last_seen = std::fs::read_to_string(&path).ok();
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
        self.save_active(cx);
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
                    .gap_6()
                    .px_8()
                    .bg(self.theme.background)
                    .child(
                        // The landing-page badge.
                        h_flex()
                            .gap_2()
                            .items_center()
                            .px_3()
                            .py_1()
                            .rounded_full()
                            .bg(brand::surface_container_high())
                            .text_color(brand::on_surface_variant())
                            .child(
                                div()
                                    .w(px(6.))
                                    .h(px(6.))
                                    .rounded_full()
                                    .bg(brand::primary()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .child("BUILT IN RUST & GPUI · ZERO CLOUD LOCK-IN"),
                            ),
                    )
                    .child(
                        v_flex()
                            .items_center()
                            .gap_4()
                            .max_w(px(620.))
                            .child(
                                div()
                                    .font_family(brand::SERIF_FAMILY)
                                    .text_size(px(36.))
                                    .line_height(rems(1.25))
                                    .text_color(brand::on_surface())
                                    .text_center()
                                    .child("A quiet, continuous river for your thoughts."),
                            )
                            .child(
                                div()
                                    .text_center()
                                    .text_color(brand::on_surface_variant())
                                    .child(
                                        "Dainiki is a private, local-first daily journal. \
                                         No cards, no document switching — just your life in an \
                                         unbroken, beautifully typeset stream.",
                                    ),
                            ),
                    )
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
                    .child(
                        div()
                            .text_xs()
                            .max_w(px(460.))
                            .text_center()
                            .text_color(brand::outline())
                            .child(
                                "Your journal is a folder of plain Markdown files you own — \
                                 offline, no account, no lock-in.",
                            ),
                    )
                    .children(notice.as_ref().map(|notice| {
                        div()
                            .max_w(px(460.))
                            .text_center()
                            .text_sm()
                            .child(notice.clone())
                    })),
            )
    }
}

impl JournalSession {
    fn view(
        &self,
        theme: &EditorTheme,
        show_empty_days: bool,
        window: &Window,
        this: WeakEntity<DainikiApp>,
        cx: &App,
    ) -> impl IntoElement {
        let today = Local::now().date_naive();
        let mut existing: BTreeSet<NaiveDate> = self.days.keys().copied().collect();
        if let Some(active) = &self.active {
            // The active day is edited, not rendered from the map, but it
            // still counts as a day that exists.
            existing.insert(active.date);
        }
        let dates = journal::stream_dates(&existing, today, show_empty_days);

        // The list renders one item per day plus the closing edge. Keep its
        // item count in step with the day set.
        if self.stream.item_count() != dates.len() + 1 {
            self.stream.reset(dates.len() + 1);
        }

        // The live day gets a capped editing surface inside the stream, so
        // history is always visible below it.
        let editor_height = px((window.viewport_size().height.as_f32() * 0.55).clamp(320., 440.));

        // A day that swapped between editor and render changed height; the
        // list must remeasure it (preserving the scroll anchor).
        let active_date = self.active.as_ref().map(|active| active.date);
        if self.active_rendered.get() != active_date {
            if let Some(date) = active_date.or(self.active_rendered.get())
                && let Some(ix) = dates.iter().position(|day| *day == date)
            {
                self.stream.remeasure_items(ix..ix + 1);
            }
            self.active_rendered.set(active_date);
        }

        let active_editor = self.active.as_ref().map(|active| active.editor.clone());
        let day_count = dates.len();
        let item_this = this.clone();
        let stream_list = list(self.stream.clone(), move |ix, _window, cx| {
            if ix == day_count {
                return stream_edge();
            }
            let date = dates[ix];
            let header = day_header(date, today);
            let body: AnyElement = if Some(date) == active_date {
                let editor = active_editor
                    .as_ref()
                    .expect("the active day always has an editor");
                div()
                    .h(editor_height)
                    .child(editor.clone())
                    .into_any_element()
            } else {
                let text = item_this.upgrade().and_then(|app| {
                    let app = app.read(cx);
                    match &app.screen {
                        Screen::Journal(session) => session.days.get(&date).cloned(),
                        Screen::Welcome { .. } => None,
                    }
                });
                let inner = if let Some(text) = text {
                    div().child(markdown(text)).into_any_element()
                } else {
                    div()
                        .font_family(brand::SERIF_FAMILY)
                        .italic()
                        .text_color(brand::outline())
                        .child("Nothing written this day — click to write it.")
                        .into_any_element()
                };
                clickable_day(item_this.clone(), date, inner)
            };
            let day = v_flex().w_full().gap_4().pb_8().child(header).child(body);
            if Some(date) == active_date {
                day.into_any_element()
            } else {
                // The whole day is the click target, header included.
                clickable_day(item_this.clone(), date, day.into_any_element())
            }
        });

        let mut column = div()
            .size_full()
            .flex()
            .flex_col()
            .child(TitleBar::new().child(self.window_title()));

        // The toolbar belongs to editing: it appears only while a day is
        // open in the editor.
        if let Some(active) = &self.active {
            column = column.child(
                div()
                    .w_full()
                    .px_3()
                    .py_1()
                    .bg(brand::surface_container_low())
                    .border_b_1()
                    .border_color(brand::outline_variant().opacity(0.6))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Toolbar::new("editor-toolbar")
                                    .child(active.inline_toolbar(cx))
                                    .child(active.block_toolbar(cx)),
                            )
                            .child(active.insert_menu())
                            .child(finish_toggle(this.clone()))
                            .child(active.help_menu()),
                    ),
            );
        }

        column.child(
            div()
                .flex_1()
                .min_h_0()
                .bg(theme.background)
                .relative()
                .child(
                    div()
                        .w_full()
                        .max_w(px(760.))
                        .mx_auto()
                        .h_full()
                        .child(stream_list.size_full().px_10().pt_6()),
                )
                .child(
                    Scrollbar::new(&self.stream)
                        .id("stream-scrollbar")
                        .axis(ScrollbarAxis::Vertical),
                ),
        )
    }

    fn window_title(&self) -> String {
        match &self.active {
            Some(active) => {
                let dirty = if active.dirty { " •" } else { "" };
                format!("Dainiki — {}{dirty}", active.path.display())
            }
            None => {
                let name = self
                    .root
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| self.root.display().to_string());
                format!("Dainiki — {name}")
            }
        }
    }
}

/// Reads every past day with content, off the UI thread. Days with an empty
/// file are no entry and stay out.
fn read_days(root: &Path, today: NaiveDate) -> BTreeMap<NaiveDate, String> {
    let journal = journal::Journal::new(root);
    let mut days = BTreeMap::new();
    for date in journal.entry_dates() {
        if date >= today {
            continue;
        }
        let day_path = journal.entry_path(date);
        match std::fs::read_to_string(&day_path) {
            Ok(text) if !text.trim().is_empty() => {
                days.insert(date, text);
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(path = %day_path.display(), error = %error, "could not read an entry");
            }
        }
    }
    days
}

/// One stream item's date header: the TODAY chip, the serif date, a hairline.
fn day_header(date: NaiveDate, today: NaiveDate) -> AnyElement {
    let is_today = date == today;
    let mut header = h_flex().items_center().gap_3();
    if is_today {
        header = header.child(
            div()
                .px_2()
                .py(px(2.))
                .rounded_full()
                .bg(brand::primary_fixed())
                .text_xs()
                .text_color(brand::on_primary_fixed())
                .child("TODAY"),
        );
    }
    header
        .child(
            div()
                .font_family(brand::SERIF_FAMILY)
                .text_xl()
                .text_color(if is_today {
                    brand::on_surface()
                } else {
                    brand::on_surface_variant()
                })
                .child(day_label(date)),
        )
        .child(
            div()
                .flex_1()
                .h(px(1.))
                .bg(brand::outline_variant().opacity(0.55)),
        )
        .into_any_element()
}

/// The stream's closing item, so even a fresh journal reads intentionally
/// bottom-out.
fn stream_edge() -> AnyElement {
    v_flex()
        .items_center()
        .gap_2()
        .pt_4()
        .pb_8()
        .child(div().text_xs().text_color(brand::outline()).child("· · ·"))
        .child(
            div()
                .font_family(brand::SERIF_FAMILY)
                .italic()
                .text_color(brand::outline())
                .child(
                    "This is the edge of your journal — everything you write adds to the stream.",
                ),
        )
        .into_any_element()
}

/// Wraps a rendered day so a click opens it for editing.
/// Wraps a whole rendered day so a click anywhere opens it for editing.
fn clickable_day(this: WeakEntity<DainikiApp>, date: NaiveDate, body: AnyElement) -> AnyElement {
    div()
        .id(ElementId::Name(format!("day-{date}").into()))
        .w_full()
        .cursor_text()
        .debug_selector(|| "day-body".to_string())
        .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
            this.update(cx, |app, cx| app.activate_day(date, window, cx))
                .ok();
        })
        .child(body)
        .into_any_element()
}

/// Leaves editing and returns to the stream.
fn finish_toggle(this: WeakEntity<DainikiApp>) -> Toggle {
    Toggle::new("finish-editing")
        .label("Done")
        .tooltip("Finish editing — ⌘E")
        .checked(false)
        .on_click(move |_checked, _window, cx| {
            this.update(cx, |app, cx| app.stop_editing(cx)).ok();
        })
}

impl ActiveDay {
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
            ("Finish editing", "⌘E"),
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

/// "Monday, 19 October 2026".
fn day_label(date: NaiveDate) -> String {
    date.format("%A, %-d %B %Y").to_string()
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        let screen = match &self.screen {
            Screen::Welcome { notice } => self.welcome_view(notice, cx).into_any_element(),
            Screen::Journal(session) => session
                .view(&self.theme, self.config.show_empty_days, window, this, cx)
                .into_any_element(),
        };
        div()
            .size_full()
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_toggle_editing))
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
        brand::apply_component_theme(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-s", Save, None),
            KeyBinding::new("cmd-e", ToggleEditing, None),
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
        let theme = brand::editor_theme();
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

    use chrono::{Local, NaiveDate};
    use gpui::{ScrollDelta, ScrollWheelEvent, TestAppContext, point, px};

    // NOTE: no `use super::*` here. The crate root glob-imports `gpui_kit::*`,
    // which re-exports gpui's `test` proc macro; globbing it in would shadow
    // the builtin `#[test]` inside the generated harness and recurse forever.
    use super::{DainikiApp, Screen, brand, config, day_label, journal, resolve_cli_root};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// The component library needs its theme global before any view renders;
    /// production also applies the brand palette.
    fn init(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(brand::apply_component_theme);
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

    /// Writes a past day's entry into the journal.
    fn seed_day(journal: &journal::Journal, date: NaiveDate, text: &str) {
        let path = journal.entry_path(date);
        fs::create_dir_all(path.parent().unwrap()).expect("dirs");
        fs::write(&path, text).expect("seed");
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

    #[test]
    fn day_labels_name_the_day() {
        let earlier = NaiveDate::from_ymd_opt(2026, 10, 5).expect("date");
        assert_eq!(day_label(earlier), "Monday, 5 October 2026");
    }

    #[gpui::test]
    async fn the_app_starts_on_the_welcome_screen(cx: &mut TestAppContext) {
        init(cx);
        let (app, cx) = cx.add_window_view(|_window, _cx| {
            DainikiApp::new(brand::editor_theme(), config::Config::default())
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

        // A past day with content joins the stream; an empty past day is not
        // an entry.
        let yesterday = today.pred_opt().expect("yesterday");
        let blank_day = yesterday.pred_opt().expect("earlier");
        seed_day(&journal, yesterday, "# yesterday's words\n");
        seed_day(&journal, blank_day, "   \n");

        let (app, cx) = cx.add_window_view({
            let dir = dir.clone();
            move |window, cx| {
                let mut app = DainikiApp::new(brand::editor_theme(), config::Config::default());
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
            let active = session.active.as_ref().expect("today is open for editing");
            assert_eq!(active.date, today);
            assert_eq!(active.path, path);
            assert_eq!(
                active.editor.read(cx).content(cx),
                "# written earlier today\n"
            );
            assert_eq!(session.days.len(), 1, "only the day with content streams");
            assert_eq!(
                session.days.get(&yesterday).map(String::as_str),
                Some("# yesterday's words\n")
            );
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn activating_a_day_swaps_the_live_editor(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        let yesterday = today.pred_opt().expect("yesterday");
        seed_day(&journal, yesterday, "# yesterday's words\n");
        let today_path = journal.entry_path(today);

        let (app, cx) = cx.add_window_view({
            let dir = dir.clone();
            move |window, cx| {
                let mut app = DainikiApp::new(brand::editor_theme(), config::Config::default());
                app.enter_journal(dir, window, cx);
                app
            }
        });

        // Open yesterday for editing.
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.activate_day(yesterday, window, cx));
            let _ = window.draw(cx);
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            let active = session.active.as_ref().expect("editing yesterday");
            assert_eq!(active.date, yesterday);
            assert_eq!(active.editor.read(cx).content(cx), "# yesterday's words\n");
            assert!(
                !session.days.contains_key(&yesterday),
                "the active day is not also rendered"
            );
            assert!(today_path.is_file(), "leaving today flushed it to disk");
        });

        // Finish editing: everything renders read-only again.
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| app.stop_editing(cx));
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            assert!(session.active.is_none(), "editing finished");
            assert_eq!(
                session.days.get(&yesterday).map(String::as_str),
                Some("# yesterday's words\n")
            );
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn a_two_year_journal_opens_virtualized(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        let mut date = today;
        for _ in 0..730 {
            date = date.pred_opt().expect("previous day");
            seed_day(&journal, date, "# day\n\nA line written that day.\n");
        }

        let started = std::time::Instant::now();
        let (app, cx) = cx.add_window_view({
            let dir = dir.clone();
            move |window, cx| {
                let mut app = DainikiApp::new(brand::editor_theme(), config::Config::default());
                app.enter_journal(dir, window, cx);
                app
            }
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        println!("open + first frame: {:?}", started.elapsed());
        cx.run_until_parked();
        println!("scan settled: {:?}", started.elapsed());
        let frame = std::time::Instant::now();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        println!("warm frame: {:?}", frame.elapsed());

        // The scan fills the stream: 730 past days + today + the closing edge.
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            assert_eq!(session.stream.item_count(), 732);
            assert!(session.stream.bounds_for_item(0).is_some());
            assert!(
                session.stream.bounds_for_item(700).is_none(),
                "days far below the fold are not mounted"
            );
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn clicking_a_rendered_day_opens_it(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        let yesterday = today.pred_opt().expect("yesterday");
        seed_day(&journal, yesterday, "# yesterday's words\n");

        let (app, cx) = cx.add_window_view({
            let dir = dir.clone();
            move |window, cx| {
                let mut app = DainikiApp::new(brand::editor_theme(), config::Config::default());
                app.enter_journal(dir, window, cx);
                app
            }
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        // Click into yesterday's day body (the interactive wrapper); the
        // item's geometric center can fall in the gap between its header
        // and its body.
        let target = cx
            .debug_bounds("day-body")
            .expect("yesterday's body is painted")
            .center();
        cx.simulate_click(target, gpui::Modifiers::default());
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            let active = session.active.as_ref().expect("yesterday opened");
            assert_eq!(active.date, yesterday);
            assert_eq!(active.editor.read(cx).content(cx), "# yesterday's words\n");
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn the_stream_scrolls_through_history(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        let mut date = today;
        for _ in 0..12 {
            let day = date.pred_opt().expect("previous day");
            seed_day(&journal, day, "# day\n\nSome words written for this day.\n");
            date = day;
        }

        let (app, cx) = cx.add_window_view({
            let dir = dir.clone();
            move |window, cx| {
                let mut app = DainikiApp::new(brand::editor_theme(), config::Config::default());
                app.enter_journal(dir, window, cx);
                app
            }
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        // A wheel over today's (short) editor must chain through to the
        // stream instead of being swallowed. The stream column is centered
        // and capped at 760 px, so aim inside it.
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(960.), px(300.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-240.))),
            ..Default::default()
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            let top = session.stream.logical_scroll_top();
            assert!(
                top.item_ix > 0 || top.offset_in_item != px(0.),
                "the stream did not scroll: {top:?}"
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
                let mut app = DainikiApp::new(brand::editor_theme(), config::Config::default());
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
