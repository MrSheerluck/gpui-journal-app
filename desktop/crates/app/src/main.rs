//! Dainiki desktop shell: window chrome, journal onboarding and the editor.
//!
//! App chrome and standard UI use gpui-kit components wherever one fits
//! (ADR-0006); the editor crate stays GPUI-primitives-only.

mod brand;
mod config;
mod journal;
mod media;
mod persistence;
mod video;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{Datelike, Local, NaiveDate};
use dainiki_editor::{
    Buffer, EditorEvent, EditorTheme, MarkdownEditor, MediaKind, MediaPlaybackState,
    commands::{BlockFormat, InlineFormat},
    destination_path, media_kind,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::TitleBar;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants, Toggle, ToggleGroup};
use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::scroll::{Scrollbar, ScrollbarAxis};
use gpui_kit::component::separator::Separator;
use gpui_kit::component::text::markdown;
use gpui_kit::component::toolbar::Toolbar;
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
#[cfg(not(test))]
use notify::Watcher as _;

actions!(
    dainiki,
    [
        Save,
        ToggleEditing,
        ToggleJumpToDate,
        PreviousDay,
        NextDay,
        FocusSearch,
        ToggleSettings
    ]
);

/// Autosave runs this long after the last keystroke.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

/// While something plays, the player's state is pushed this often.
const MEDIA_TICK: Duration = Duration::from_millis(100);

/// The hover group every day header shares: its members show only while the
/// header under the cursor asks them to.
const DAY_HEADER_GROUP: &str = "day-header";

/// How many scrolls a single caret reveal may spend. A wrong landing (a stale
/// layout, item heights not yet measured) corrects itself on the next frame;
/// the bound keeps a caret that cannot be brought into view from spinning.
const REVEAL_ATTEMPTS: u8 = 4;

/// The stream's viewport margins for caret keeping-visible: the chrome above
/// (title bar, toolbar, panels) and a little air below.
const STREAM_TOP_MARGIN: f32 = 120.;
const STREAM_BOTTOM_MARGIN: f32 = 48.;

/// The app shell. A chosen folder turns into a [`JournalSession`]; until then
/// the welcome screen asks for one.
struct DainikiApp {
    theme: EditorTheme,
    config: config::Config,
    screen: Screen,
    /// Watches the window appearance so `System` theme follows it.
    _appearance: Option<Subscription>,
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
    /// Tags over the non-active days; the active day's tags merge in per
    /// render, so the sidebar follows the open editor.
    tags: BTreeMap<String, BTreeSet<NaiveDate>>,
    /// What the stream is narrowed to (a tag).
    filter: Filter,
    /// The virtualized stream: one item per visible-range day, plus the edge.
    stream: ListState,
    /// Which day the stream last rendered as the live editor, so a day that
    /// swapped between editor and render (a different height) is remeasured.
    active_rendered: Cell<Option<NaiveDate>>,
    /// The open day's media revision as of the last layout: a decoded image
    /// changes the height of its line, and the list caches item heights.
    active_media: Cell<u64>,
    /// Read-only days' images, decoded by the app itself so it knows when a
    /// card settles — gpui's own loader would grow the card after the item's
    /// height had already been cached.
    media_images: RefCell<HashMap<PathBuf, MediaImageState>>,
    /// Bumped whenever a read-only image settles; the days the list has laid
    /// out are re-measured then.
    media_settled: Cell<u64>,
    /// The settle count as of the last re-measure.
    media_measured: Cell<u64>,
    /// The date-jump panel: input, open state, and the last parse error.
    jump_input: Entity<InputState>,
    jump_open: bool,
    jump_error: Option<String>,
    /// The search palette's state, shared across openings.
    search_palette: Entity<CommandState>,
    /// Set when the open day's caret should be scrolled into view on the next
    /// frame (typing, or a freshly opened day whose caret sits at the end).
    reveal_caret: Cell<bool>,
    /// The caret rect the last reveal saw, for the settle check.
    reveal_rect: Cell<Option<Bounds<Pixels>>>,
    /// Scrolls spent on the pending reveal; bounded so a caret that cannot be
    /// brought into view (a stale layout, a resize mid-flight) never spins.
    reveal_attempts: Cell<u8>,
    /// The days the stream's items currently stand for (newest first); the
    /// closing edge is one extra item after them.
    stream_dates: RefCell<Vec<NaiveDate>>,
    /// How many items the stream holds, as the app last set it up.
    stream_items: Cell<usize>,
    /// The session's "today", captured at open; a timer rolls it over past
    /// midnight.
    today: NaiveDate,
    /// Set by the rollover timer; processed in render (which has a window).
    pending_rollover: Cell<bool>,
    /// A day whose delete is awaiting confirmation.
    pending_delete: Option<NaiveDate>,
    /// A message shown in the status banner (save or trash failures).
    status: Option<String>,
    /// Watches the journal folder; external edits land through it.
    _watcher: Option<notify::RecommendedWatcher>,
    /// The open day changed on disk while clean: reload it in the next frame
    /// (which has a window to rebuild the editor with).
    pending_reload: Cell<Option<NaiveDate>>,
    /// The month the sidebar calendar shows, `(year, month)`.
    calendar_month: (i32, u32),
    /// Set by the jump input's Enter event; processed in render, which has a
    /// window to activate a day with.
    pending_submit: Cell<bool>,
    /// A date the next rendered frame should reveal in the stream.
    pending_reveal: Cell<Option<NaiveDate>>,
    /// The open day's media player.
    media: media::Player,
    /// Whether the playback tick loop is running.
    media_ticks: Cell<bool>,
    /// The open video's frame pump, when one is running.
    video: Option<video::Pump>,
    /// Which destination the frame pump belongs to.
    video_destination: Option<String>,
    /// The latest decoded frame, for read-only video cards: the destination,
    /// the image, and its position on the timeline.
    latest_frame: Option<(String, Arc<RenderImage>, u64)>,
    /// The current video's total length, as reported by the decoder.
    video_duration_ms: Option<u64>,
    _jump_events: Subscription,
}

/// What the stream is narrowed to: a tag.
#[derive(Default, Clone, Debug, PartialEq)]
struct Filter {
    tag: Option<String>,
}

impl Filter {
    fn is_active(&self) -> bool {
        self.tag.is_some()
    }
}

/// A read-only day's image, as the app tracks it: gpui's loader grows the
/// card only after the item's height has been cached, so the app decodes for
/// itself and re-measures once a card settles.
#[derive(Clone)]
enum MediaImageState {
    Loading,
    Loaded(Arc<RenderImage>),
    Failed,
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
            _appearance: None,
        }
    }

    /// Starts following the system appearance (relevant while the theme
    /// preference is `System`).
    fn watch_appearance(&mut self, window: &Window, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        self._appearance = Some(window.observe_window_appearance(move |window, cx| {
            weak.update(cx, |app, cx| app.on_system_appearance(window, cx))
                .ok();
        }));
    }

    /// The window's appearance changed: re-resolve when the preference is
    /// `System`, otherwise the explicit choice wins.
    fn on_system_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.config.theme != config::ThemePreference::System {
            return;
        }
        let mode = resolve_mode(config::ThemePreference::System, window.appearance());
        if mode != brand::mode() {
            self.apply_mode(mode, window, cx);
        }
    }

    /// Persists and applies a theme preference.
    fn set_theme_preference(
        &mut self,
        preference: config::ThemePreference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.config.theme == preference {
            return;
        }
        self.config.theme = preference;
        // Tests exercise this path; never write the real user config from them.
        #[cfg(not(test))]
        if let Err(error) = config::save(&self.config) {
            tracing::warn!(error = %error, "could not save preferences");
        }
        self.apply_mode(resolve_mode(preference, window.appearance()), window, cx);
    }

    /// Repaints the whole app in `mode`: the brand palette, the component
    /// theme, the open editor, and every window.
    fn apply_mode(&mut self, mode: brand::Mode, window: &mut Window, cx: &mut Context<Self>) {
        brand::set_mode(mode);
        self.theme = brand::editor_theme_for(mode);
        let theme = self.theme.clone();
        if let Some(session) = self.session_mut()
            && let Some(active) = session.active.as_mut()
        {
            active
                .editor
                .update(cx, |editor, cx| editor.set_theme(theme, cx));
        }
        brand::apply_component_theme(cx);
        cx.refresh_windows();
        window.refresh();
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

    /// Marks the app dirty and asks every window for a frame.
    ///
    /// `cx.notify()` alone can be lost when background work lands between
    /// frames: gpui only wakes a window that holds a registered invalidator
    /// for the entity, and a notify that arrives during a draw is settled
    /// without a wake. Everything asynchronous (the journal scan, the file
    /// watcher, the playback tick, frame pushes, debounced saves) repaints
    /// through here.
    fn repaint(&self, cx: &mut Context<Self>) {
        cx.notify();
        cx.refresh_windows();
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

        let watcher = start_watcher(root, cx);
        start_rollover_timer(cx);
        let jump_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD, or “today”"));
        let jump_events = cx.subscribe(&jump_input, |app, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                if let Some(session) = app.session_mut() {
                    session.pending_submit.set(true);
                }
                cx.notify();
            }
        });
        let search_palette = cx.new(|cx| CommandState::new(window, cx));
        tracing::info!(root = %root.display(), path = %active.path.display(), "journal opened");
        self.screen = Screen::Journal(Box::new(JournalSession {
            root: root.to_path_buf(),
            active: Some(active),
            days: BTreeMap::new(),
            tags: BTreeMap::new(),
            filter: Filter::default(),
            stream: ListState::new(0, ListAlignment::Top, px(600.)),
            active_rendered: Cell::new(None),
            active_media: Cell::new(0),
            media_images: RefCell::new(HashMap::new()),
            media_settled: Cell::new(0),
            media_measured: Cell::new(0),
            jump_input,
            jump_open: false,
            jump_error: None,
            search_palette,
            reveal_caret: Cell::new(true),
            reveal_rect: Cell::new(None),
            reveal_attempts: Cell::new(0),
            stream_dates: RefCell::new(Vec::new()),
            stream_items: Cell::new(0),
            today,
            pending_rollover: Cell::new(false),
            pending_delete: None,
            status: None,
            _watcher: watcher,
            pending_reload: Cell::new(None),
            calendar_month: (today.year(), today.month()),
            pending_submit: Cell::new(false),
            pending_reveal: Cell::new(None),
            media: media::Player::platform(),
            media_ticks: Cell::new(false),
            video: None,
            video_destination: None,
            latest_frame: None,
            video_duration_ms: None,
            _jump_events: jump_events,
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
                    session.rebuild_tags();
                }
                app.repaint(cx);
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
            // The timeline is the only scroller: the open entry grows to its
            // content instead of scrolling inside its own box, so one wheel
            // gesture can never belong to two scrollers at once.
            editor.fit_content(cx);
            editor
        });
        editor.update(cx, |editor, cx| editor.place_caret_at_end(cx));
        let link_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("URL to link — select the text first")
        });
        let events = cx.subscribe(&editor, |app, _editor, event, cx| match event {
            EditorEvent::Edited => {
                if let Some(active) = app
                    .session_mut()
                    .and_then(|session| session.active.as_mut())
                {
                    active.dirty = true;
                }
                if let Some(session) = app.session_mut() {
                    // A fit-content editor cannot scroll itself; the timeline
                    // keeps the caret visible.
                    session.request_reveal_caret();
                }
                app.schedule_save(cx);
                cx.notify();
            }
            EditorEvent::MediaToggle {
                destination,
                resolved,
                kind,
            } => app.on_media_toggle(destination.clone(), resolved.clone(), *kind, cx),
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
                    // The caret starts at the entry's end: unless the day is
                    // short, the stream scrolls it into view next frame.
                    session.request_reveal_caret();
                }
                cx.notify();
            }
            Err(error) => {
                tracing::error!(date = %date, error = %error, "could not open the day");
            }
        }
    }

    /// Flushes the active day's save, stashes its text with the other days,
    /// and drops the editor — back to reading the stream. Media playback
    /// belongs to the open day: it stops here too.
    fn stop_editing(&mut self, cx: &mut Context<Self>) {
        self.save_active(cx);
        let Some(previous) = self.session_mut().and_then(|session| session.active.take()) else {
            return;
        };
        if let Some(session) = self.session_mut() {
            session.media.stop();
            session.media_ticks.set(false);
            session.video = None;
            session.video_destination = None;
        }
        let text = previous.editor.read(cx).content(cx);
        if let Some(session) = self.session_mut() {
            if text.trim().is_empty() {
                session.days.remove(&previous.date);
            } else {
                session.days.insert(previous.date, text);
            }
            session.rebuild_tags();
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
            let today = self.session().map(|session| session.today);
            if let Some(today) = today {
                self.activate_day(today, window, cx);
            }
        }
    }

    // -- media ------------------------------------------------------------

    /// The editor asked for playback: toggle the medium, push the state
    /// back, and keep a tick running while it plays.
    fn on_media_toggle(
        &mut self,
        destination: String,
        resolved: PathBuf,
        kind: MediaKind,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut() else {
            return;
        };
        tracing::info!(destination = %destination, kind = ?kind, "media toggled");
        let state = session.media.toggle(&destination, &resolved, kind);
        let playing = state.playing;
        if let Some(error) = state.error.clone() {
            let label = match session.media.kind() {
                Some(MediaKind::Video) => "video",
                _ => "audio",
            };
            session.status = Some(format!(
                "Could not play the {label} {} — {error}",
                resolved
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| resolved.display().to_string()),
            ));
        }
        let editor = session.active.as_ref().map(|active| active.editor.clone());
        if let Some(editor) = editor {
            editor.update(cx, |editor, cx| {
                editor.set_media_state(&destination, state, cx)
            });
        }
        if session.media.playing() {
            self.ensure_media_tick(cx);
        }
        if kind == MediaKind::Video {
            self.sync_video_pump(&destination, &resolved, playing, cx);
        }
        self.repaint(cx);
    }

    /// Keeps the frame pump in step with the player: a fresh pump for a new
    /// (or finished) video, resume/pause for the running one.
    fn sync_video_pump(
        &mut self,
        destination: &str,
        resolved: &Path,
        playing: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut() else {
            return;
        };
        let same_running = session.video.is_some()
            && session.video_destination.as_deref() == Some(destination)
            && !session.video.as_ref().is_some_and(|pump| pump.finished());
        if same_running {
            if let Some(pump) = &session.video {
                if playing {
                    pump.resume();
                } else {
                    pump.pause();
                }
            }
            return;
        }
        // Different video, or the previous pump ended: replace it.
        session.video = None;
        session.video_destination = None;
        if !playing {
            return;
        }
        match video::Pump::start(resolved) {
            Ok(pump) => {
                let frames = pump.frames();
                session.video = Some(pump);
                session.video_destination = Some(destination.to_string());
                let destination = destination.to_string();
                cx.spawn(async move |this, cx| {
                    while let Ok(frame) = frames.recv().await {
                        let keep = this
                            .update(cx, |app, cx| app.push_video_frame(&destination, frame, cx))
                            .is_ok();
                        if !keep {
                            break;
                        }
                    }
                })
                .detach();
            }
            Err(error) => {
                session.status = Some(format!("Could not play the video — {error}"));
            }
        }
    }

    /// One decoded frame (or a decode failure) from the open video.
    fn push_video_frame(
        &mut self,
        destination: &str,
        frame: Result<video::VideoFrame, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut() else {
            return;
        };
        match frame {
            Ok(frame) => {
                let pts_ms = frame.pts_ms;
                let duration_ms = frame.duration_ms;
                let Some(image) = frame.into_render_image() else {
                    return;
                };
                session.latest_frame = Some((destination.to_string(), image.clone(), pts_ms));
                if duration_ms.is_some() {
                    session.video_duration_ms = duration_ms;
                }
                let editor = session.active.as_ref().map(|active| active.editor.clone());
                if let Some(editor) = editor {
                    editor.update(cx, |editor, cx| {
                        editor.set_media_frame(destination, image, cx)
                    });
                }
            }
            Err(error) => {
                session.status = Some(format!("Could not play the video — {error}"));
                let editor = session.active.as_ref().map(|active| active.editor.clone());
                if let Some(editor) = editor {
                    let state = MediaPlaybackState {
                        error: Some(error),
                        ..MediaPlaybackState::default()
                    };
                    editor.update(cx, |editor, cx| {
                        editor.set_media_state(destination, state, cx)
                    });
                }
            }
        }
        self.repaint(cx);
    }

    /// Starts the playback tick unless it is already running.
    fn ensure_media_tick(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session_mut() else {
            return;
        };
        if session.media_ticks.replace(true) {
            return;
        }
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(MEDIA_TICK).await;
                let keep_going = this
                    .update(cx, |app, cx| app.media_tick(cx))
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    /// One playback tick: pushes the player's state to the open editor.
    /// Returns whether the loop should keep running (a pause ends it; the
    /// widget keeps its position without repainting).
    fn media_tick(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.session_mut() else {
            return false;
        };
        let Some((destination, mut state)) = session.media.state() else {
            session.media_ticks.set(false);
            return false;
        };
        let mut keep_going = session.media.playing();
        if session.media.silent() {
            // A silent video's clock is its frame pump: the position is the
            // newest frame's, the length is the decoder's, and the end is the
            // pump finishing.
            if let Some((frame_destination, _, pts_ms)) = &session.latest_frame
                && frame_destination == &destination
            {
                state.position_ms = *pts_ms;
            }
            state.duration_ms = session.video_duration_ms;
            let pump_gone = session.video.as_ref().is_none_or(|pump| pump.finished());
            if pump_gone {
                state.playing = false;
                keep_going = false;
                session.media.stop();
            }
        }
        let editor = session.active.as_ref().map(|active| active.editor.clone());
        if let Some(editor) = editor {
            editor.update(cx, |editor, cx| {
                editor.set_media_state(&destination, state, cx)
            });
        }
        if !keep_going {
            session.media_ticks.set(false);
        }
        self.repaint(cx);
        keep_going
    }

    /// Restarts the debounce window; the previous pending save is dropped.
    /// Starts decoding every image a day's text references, once each, so the
    /// app knows when a read-only card settles and can re-measure the day.
    fn ensure_day_images(&mut self, date: NaiveDate, text: &str, cx: &mut Context<Self>) {
        let Some(session) = self.session() else {
            return;
        };
        let root = session.root.clone();
        let destinations: Vec<String> = day_segments(text)
            .into_iter()
            .filter_map(|segment| match segment {
                DaySegment::Media {
                    kind: MediaKind::Image,
                    destination,
                } => Some(destination),
                _ => None,
            })
            .collect();
        for destination in destinations {
            let cleaned = destination_path(&destination);
            let resolved = journal::Journal::new(&root)
                .entry_path(date)
                .parent()
                .map(|directory| directory.join(&cleaned))
                .unwrap_or_else(|| PathBuf::from(&cleaned));
            {
                let session = self.session().expect("the session was just here");
                if session.media_images.borrow().contains_key(&resolved) {
                    continue;
                }
                session
                    .media_images
                    .borrow_mut()
                    .insert(resolved.clone(), MediaImageState::Loading);
            }
            cx.spawn(async move |app, cx| {
                let decoded = cx
                    .background_executor()
                    .spawn({
                        let resolved = resolved.clone();
                        async move { dainiki_editor::decode_image(&resolved) }
                    })
                    .await;
                let state = match decoded {
                    Ok(image) => MediaImageState::Loaded(image),
                    Err(error) => {
                        tracing::warn!(path = %resolved.display(), error = %error, "failed to load image");
                        MediaImageState::Failed
                    }
                };
                app.update(cx, |app, cx| {
                    if let Some(session) = app.session() {
                        session.media_images.borrow_mut().insert(resolved, state);
                        session.media_settled.set(session.media_settled.get() + 1);
                    }
                    app.repaint(cx);
                })
                .ok();
            })
            .detach();
        }
    }

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
        let status = {
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
                    None
                }
                persistence::SaveOutcome::ConflictCopy(conflict) => {
                    active.dirty = false;
                    active.last_seen = std::fs::read_to_string(&path).ok();
                    tracing::error!(
                        path = %conflict.display(),
                        "the journal changed on disk; preserved this version as a conflict copy"
                    );
                    let name = conflict
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    Some(format!(
                        "This day changed on disk — your version was kept as {name}"
                    ))
                }
                persistence::SaveOutcome::Failed(error) => {
                    tracing::error!(path = %path.display(), error = %error, "journal save failed");
                    Some(format!("Could not save {} — {error}", path.display()))
                }
            }
        };
        if let Some(message) = status
            && let Some(session) = self.session_mut()
        {
            session.status = Some(message);
        }
        self.repaint(cx);
    }

    fn on_save(&mut self, _: &Save, _window: &mut Window, cx: &mut Context<Self>) {
        self.save_active(cx);
    }

    // -- date navigation ----------------------------------------------------

    fn on_toggle_jump(
        &mut self,
        _: &ToggleJumpToDate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut() else {
            return;
        };
        session.jump_open = !session.jump_open;
        session.jump_error = None;
        if session.jump_open {
            let focus = session.jump_input.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    /// Reads the jump input, and either opens the day or shows a parse error.
    fn jump_from_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(raw) = self
            .session()
            .map(|session| session.jump_input.read(cx).value().to_string())
        else {
            return;
        };
        let today = Local::now().date_naive();
        match parse_jump_date(&raw, today) {
            Some(date) => self.jump_to(date, window, cx),
            None => {
                if let Some(session) = self.session_mut() {
                    session.jump_error = Some("Try 2026-10-05, or “today”.".to_string());
                }
                cx.notify();
            }
        }
    }

    /// Opens `date` for editing and reveals it in the stream. The future is
    /// clamped to today — the stream never shows days that have not happened.
    fn jump_to(&mut self, date: NaiveDate, window: &mut Window, cx: &mut Context<Self>) {
        let today = self.session().map_or(date, |session| session.today);
        let date = date.min(today);
        self.activate_day(date, window, cx);
        if let Some(session) = self.session_mut() {
            session.jump_open = false;
            session.jump_error = None;
            session.calendar_month = (date.year(), date.month());
            session.pending_reveal.set(Some(date));
            session
                .jump_input
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        cx.notify();
    }

    /// Moves the sidebar calendar by whole months.
    fn shift_calendar_month(&mut self, delta: i32, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut() {
            let (year, month) = session.calendar_month;
            let total = year * 12 + month as i32 - 1 + delta;
            session.calendar_month = (total.div_euclid(12), (total.rem_euclid(12) + 1) as u32);
        }
        cx.notify();
    }

    fn close_jump(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut() {
            session.jump_open = false;
            session.jump_error = None;
        }
        cx.notify();
    }

    // -- filters ------------------------------------------------------------

    /// Narrows the stream to `tag`, or clears the tag filter when it is
    /// already the selected one.
    fn toggle_tag_filter(&mut self, tag: &str, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut() {
            if session.filter.tag.as_deref() == Some(tag) {
                session.filter.tag = None;
            } else {
                session.filter.tag = Some(tag.to_string());
            }
        }
        cx.notify();
    }

    /// Back to the full stream.
    fn clear_filter(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut() {
            session.filter = Filter::default();
        }
        cx.notify();
    }

    // -- search -------------------------------------------------------------

    /// Opens the search palette: every day with an entry, newest first,
    /// filtered as you type. Enter or a click jumps to the day; Escape closes
    /// the palette (clearing the query first when one is typed).
    fn open_search_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session() else {
            return;
        };
        let palette = session.search_palette.clone();
        let this = cx.weak_entity();
        palette.update(cx, |state, cx| state.set_query("", window, cx));
        // The dialog shell takes focus when it opens; focus the query field on
        // the builder's first run only, never on a rebuild.
        let initial_focus = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if initial_focus.replace(false) {
                palette.update(cx, |state, cx| state.focus(window, cx));
            }
            // The builder may run again for a layer rebuild: clone what the
            // inner closures take, and re-read the app for fresh values.
            let palette = palette.clone();
            let this = this.clone();
            let items = this
                .upgrade()
                .map(|app| {
                    let app = app.read(cx);
                    app.session()
                        .map(|session| session.palette_items(cx))
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            let confirm_this = this.clone();
            dialog
                .close_button(false)
                .w(px(560.))
                .margin_top(px(96.))
                // Instant placement: a palette should feel like it appears in
                // place, not slides into it.
                .entrance(DialogEntrance::Fade)
                .content(move |content, _window, _cx| {
                    // A `Fn` content builder: clone what each callback takes.
                    let confirm_this = confirm_this.clone();
                    let items = items.clone();
                    content.child(
                        Command::new(&palette)
                            .items(items)
                            .placeholder("Search entries…")
                            .bordered(false)
                            .on_confirm(move |index, window, cx| {
                                confirm_this
                                    .update(cx, |app, cx| {
                                        app.jump_to_palette_row(index.row, window, cx)
                                    })
                                    .ok();
                            }),
                    )
                })
        });
    }

    /// Jumps to the day behind a palette row, then closes the palette. The
    /// row index is into the same day list the palette rendered.
    fn jump_to_palette_row(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(date) = self
            .session()
            .and_then(|session| session.palette_days(cx).into_iter().nth(row))
            .map(|day| day.date)
        else {
            return;
        };
        window.close_dialog(cx);
        self.jump_to(date, window, cx);
    }

    fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.open_search_palette(window, cx);
    }

    // -- settings -----------------------------------------------------------

    /// Opens the settings modal: appearance, empty days, and the journal
    /// folder. The builder re-reads the app so a toggled preference shows
    /// immediately; `Done`, Escape and the close button all dismiss it.
    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.session().map(|session| session.root.clone()) else {
            return;
        };
        let this = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _window, cx| {
            let this = this.clone();
            let theme_preference = this
                .upgrade()
                .map(|app| app.read(cx).config.theme)
                .unwrap_or(config::ThemePreference::System);

            let picker_this = this.clone();
            let reveal_root = root.clone();

            dialog
                .title("Settings")
                .w(px(440.))
                // A fade keeps the surface at its final bounds from the first
                // frame, so clicks land where the picker is drawn.
                .entrance(DialogEntrance::Fade)
                .content(move |content, _window, _cx| {
                    // A `Fn` content builder: clone what each callback takes.
                    let picker_this = picker_this.clone();
                    let reveal_root = reveal_root.clone();
                    content.child(
                        v_flex()
                            .gap_4()
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .child(div().text_size(px(12.)).child("Theme"))
                                    .child(
                                        div()
                                            .debug_selector(|| "theme-pref".to_string())
                                            .child(theme_picker(picker_this, theme_preference)),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(brand::outline())
                                            .child("Journal folder"),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_3()
                                            .items_center()
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .text_color(brand::on_surface_variant())
                                                    .child(reveal_root.display().to_string()),
                                            )
                                            .child(
                                                Button::new("reveal-journal")
                                                    .label("Reveal in Finder")
                                                    .compact()
                                                    .on_click(move |_, _window, cx| {
                                                        cx.open_url(&format!(
                                                            "file://{}",
                                                            reveal_root.display()
                                                        ));
                                                    }),
                                            ),
                                    ),
                            ),
                    )
                })
                .footer(
                    h_flex().justify_end().child(
                        Button::new("settings-done")
                            .label("Done")
                            .compact()
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                            }),
                    ),
                )
        });
    }

    fn on_toggle_settings(
        &mut self,
        _: &ToggleSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_settings(window, cx);
    }

    // -- deleting a day -----------------------------------------------------

    /// Asks for confirmation before moving a day's file to the OS trash.
    fn request_delete(&mut self, date: NaiveDate, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut() {
            session.pending_delete = Some(date);
        }
        cx.notify();
    }

    fn cancel_delete(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut() {
            session.pending_delete = None;
        }
        cx.notify();
    }

    /// Moves the day's file to the OS trash (recoverable), then drops it from
    /// the session. Deleting the open day first finishes editing it.
    fn confirm_delete(&mut self, date: NaiveDate, cx: &mut Context<Self>) {
        if self
            .session()
            .and_then(|session| session.active.as_ref())
            .is_some_and(|active| active.date == date)
        {
            self.stop_editing(cx);
        }
        let Some(session) = self.session_mut() else {
            return;
        };
        let path = journal::Journal::new(&session.root).entry_path(date);
        if path.exists()
            && let Err(error) = trash::delete(&path)
        {
            tracing::error!(path = %path.display(), error = %error, "could not move the entry to Trash");
            session.status = Some(format!(
                "Could not move {} to Trash — {error}",
                path.display()
            ));
            session.pending_delete = None;
            cx.notify();
            return;
        }
        session.apply_deleted_day(date);
        cx.notify();
    }

    fn dismiss_status(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut() {
            session.status = None;
        }
        cx.notify();
    }

    // -- midnight rollover --------------------------------------------------

    /// The session's "today" is captured at open; a timer asks for a rollover
    /// when the wall clock passes midnight. Whatever was open belongs to the
    /// old date and is flushed first; the new day then opens.
    fn roll_over_if_needed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = Local::now().date_naive();
        let Some(session) = self.session() else {
            return;
        };
        if now <= session.today {
            return;
        }
        tracing::info!(from = %session.today, to = %now, "rolling the journal over midnight");
        self.stop_editing(cx);
        if let Some(session) = self.session_mut() {
            session.today = now;
            session.pending_reveal.set(Some(now));
        }
        self.activate_day(now, window, cx);
        cx.notify();
    }

    // -- external changes ---------------------------------------------------

    /// Rebuilds the open day's editor from disk after an external change,
    /// without flushing: the editor was clean, so the disk copy is the newer
    /// one and the editor's copy is discarded.
    fn reload_active_day(&mut self, date: NaiveDate, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut()
            && session
                .active
                .as_ref()
                .is_some_and(|active| active.date == date)
        {
            session.active = None;
        }
        self.activate_day(date, window, cx);
    }

    /// Reconciles paths that changed on disk (from the watcher). Content is
    /// compared before anything changes, so our own saves are no-ops. A
    /// changed open day reloads only while it is clean; a dirty editor is
    /// protected by the save-time conflict copy instead.
    fn reconcile_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(session) = self.session_mut() else {
            return;
        };
        let journal = journal::Journal::new(&session.root);
        let changed: BTreeSet<NaiveDate> = paths
            .iter()
            .filter_map(|path| journal.date_for_path(path))
            .collect();
        if changed.is_empty() {
            return;
        }
        let changed_count = changed.len();

        let mut dirty = false;
        let mut reload = None;
        for date in changed {
            if let Some(active) = session.active.as_ref()
                && active.date == date
            {
                if !active.dirty {
                    let current = active.editor.read(cx).content(cx);
                    let disk = std::fs::read_to_string(journal.entry_path(date)).ok();
                    if disk.as_deref() != Some(current.as_str()) {
                        reload = Some(date);
                    }
                }
                continue;
            }
            let disk = std::fs::read_to_string(journal.entry_path(date))
                .ok()
                .filter(|text| !text.trim().is_empty());
            let stored = session.days.get(&date).cloned();
            if disk != stored {
                match disk {
                    Some(text) => {
                        session.days.insert(date, text);
                    }
                    None => {
                        session.days.remove(&date);
                    }
                }
                dirty = true;
            }
        }

        if dirty {
            session.rebuild_tags();
        }
        if let Some(date) = reload {
            session.pending_reload.set(Some(date));
        }
        if dirty || reload.is_some() {
            tracing::info!(changed = changed_count, "reconciled external changes");
            self.repaint(cx);
        }
    }

    fn on_previous_day(&mut self, _: &PreviousDay, window: &mut Window, cx: &mut Context<Self>) {
        let Some(date) = self
            .session()
            .and_then(|session| session.active.as_ref())
            .and_then(|active| active.date.pred_opt())
        else {
            return;
        };
        self.jump_to(date, window, cx);
    }

    fn on_next_day(&mut self, _: &NextDay, window: &mut Window, cx: &mut Context<Self>) {
        let Some(date) = self
            .session()
            .and_then(|session| session.active.as_ref())
            .and_then(|active| active.date.succ_opt())
        else {
            return;
        };
        self.jump_to(date, window, cx);
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

        // The hero glow, as two stacked washes approximating the landing's
        // three-stop gradient (gpui gradients take two stops, and there is no
        // blur): surface-container-high/40 over the full 340px, primary-fixed
        // /20 over the top half.
        let glow_layer = |height: f32, color: Hsla| {
            div()
                .absolute()
                .top(px(-128.))
                .left_0()
                .right_0()
                .h(px(height))
                .flex()
                .justify_center()
                .child(div().w(px(840.)).h_full().bg(linear_gradient(
                    180.,
                    linear_color_stop(color, 0.),
                    linear_color_stop(color.opacity(0.), 1.),
                )))
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(brand_title_bar(None, None))
            .child(
                div()
                    .flex_1()
                    .relative()
                    .overflow_hidden()
                    .bg(self.theme.background)
                    .child(glow_layer(340., brand::surface_container_high().opacity(0.4)))
                    .child(glow_layer(170., brand::primary_fixed().opacity(0.2)))
                    .child(
                        v_flex()
                            .size_full()
                            .items_center()
                            .justify_center()
                            .px_6()
                            .child(
                                // The landing hero's badge.
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .px(px(14.))
                                    .py(px(6.))
                                    .mb_8()
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
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child("DESIGNED BY SMBL"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(brand::outline_variant())
                                            .child("•"),
                                    )
                                    .child(div().text_size(px(11.)).child("Built in Rust & GPUI"))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(brand::outline_variant())
                                            .child("•"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(brand::primary())
                                            .child("Zero Cloud Lock-in"),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .items_center()
                                    .max_w(px(896.))
                                    .child(
                                        div()
                                            .font_family(brand::SERIF_FAMILY)
                                            .text_size(px(52.))
                                            .line_height(px(60.))
                                            .text_color(brand::on_surface())
                                            .text_center()
                                            .child("A quiet, continuous river"),
                                    )
                                    .child(
                                        div()
                                            .font_family(brand::SERIF_FAMILY)
                                            .text_size(px(52.))
                                            .line_height(px(60.))
                                            .text_color(brand::on_surface())
                                            .text_center()
                                            .child("for your thoughts."),
                                    ),
                            )
                            .child(
                                div()
                                    .mt_6()
                                    .max_w(px(672.))
                                    .text_size(px(19.))
                                    .line_height(px(31.))
                                    .text_center()
                                    .text_color(brand::on_surface_variant())
                                    .child(
                                        "Dainiki is a private, local-first daily journal crafted \
                                         for those who value reflection over productivity metrics. \
                                         No cards. No document switching. Just your life in an \
                                         unbroken, beautifully typeset stream.",
                                    ),
                            )
                            .child(
                                h_flex()
                                    .mt_10()
                                    .gap(px(10.))
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
                                    .mt_4()
                                    .max_w(px(512.))
                                    .text_size(px(11.))
                                    .text_center()
                                    .text_color(brand::on_surface_variant())
                                    .child(
                                        "Your journal is a folder of plain Markdown files you own — \
                                         offline, no account, no lock-in.",
                                    ),
                            )
                            .children(notice.as_ref().map(|notice| {
                                div()
                                    .mt_4()
                                    .max_w(px(512.))
                                    .text_center()
                                    .text_size(px(11.))
                                    .child(notice.clone())
                            })),
                    ),
            )
    }
}

impl JournalSession {
    /// Days with entries, newest first, as the search palette lists them.
    /// The open day's live text stands in for its stored copy.
    fn palette_days(&self, cx: &App) -> Vec<PaletteDay> {
        let active_date = self.active.as_ref().map(|active| active.date);
        let active_text = self
            .active
            .as_ref()
            .map(|active| active.editor.read(cx).content(cx));
        let mut dates: Vec<NaiveDate> = self.days.keys().copied().collect();
        if let Some(date) = active_date {
            dates.push(date);
        }
        dates.sort_unstable_by(|a, b| b.cmp(a));
        dates.dedup();
        dates
            .into_iter()
            .map(|date| {
                let text = match (Some(date) == active_date, active_text.as_deref()) {
                    (true, Some(text)) => text.to_string(),
                    _ => self.days.get(&date).cloned().unwrap_or_default(),
                };
                PaletteDay {
                    date,
                    label: palette_label(date, self.today, &text),
                    text,
                }
            })
            .collect()
    }

    /// The palette's command items, in the same order as `palette_days`.
    fn palette_items(&self, cx: &App) -> Vec<CommandItem> {
        self.palette_days(cx)
            .into_iter()
            .map(|day| {
                CommandItem::new()
                    .label(day.label)
                    .keywords([day.date.format("%Y-%m-%d").to_string(), day.text])
            })
            .collect()
    }

    /// A read-only day's body: markdown runs with media cards where whole-line
    /// media references sit. Days without media keep the single text view.
    fn day_body(
        &self,
        date: NaiveDate,
        text: &str,
        scale_factor: f32,
        this: &WeakEntity<DainikiApp>,
    ) -> AnyElement {
        let segments = day_segments(text);
        if !segments
            .iter()
            .any(|segment| matches!(segment, DaySegment::Media { .. }))
        {
            // No cards to place: one markdown run. It still goes through the
            // rewrite pass — inline references read as links, `==swipes==` as
            // marks — instead of the raw source.
            return markdown(rewritten_text(text)).into_any_element();
        }
        let playback = self.media.peek();
        v_flex()
            .children(segments.into_iter().map(|segment| match segment {
                DaySegment::Text(text) => markdown(text).into_any_element(),
                DaySegment::Media { kind, destination } => self.media_card(
                    date,
                    kind,
                    &destination,
                    playback.as_ref(),
                    scale_factor,
                    this,
                ),
            }))
            .into_any_element()
    }

    /// A media card in a read-only day: an image at its natural size, or a
    /// player row/card whose click toggles playback in the app's player.
    #[allow(clippy::too_many_arguments)]
    fn media_card(
        &self,
        date: NaiveDate,
        kind: MediaKind,
        destination: &str,
        playback: Option<&(String, MediaPlaybackState)>,
        scale_factor: f32,
        this: &WeakEntity<DainikiApp>,
    ) -> AnyElement {
        let cleaned = destination_path(destination);
        let resolved = journal::Journal::new(&self.root)
            .entry_path(date)
            .parent()
            .map(|directory| directory.join(&cleaned))
            .unwrap_or_else(|| PathBuf::from(&cleaned));
        let state = playback
            .filter(|(current, _)| current == destination)
            .map(|(_, state)| state.clone())
            .unwrap_or_default();
        let name = cleaned.rsplit('/').next().unwrap_or(&cleaned).to_string();

        if kind == MediaKind::Image {
            let state = self.media_images.borrow().get(&resolved).cloned();
            let card: AnyElement = match state {
                Some(MediaImageState::Loaded(image)) => {
                    let size = image.size(0);
                    let scale = scale_factor.max(1.);
                    let natural_width = (size.width.0 as f32 / scale).max(1.);
                    let natural_height = (size.height.0 as f32 / scale).max(1.);
                    let fit = (560. / natural_width).min(360. / natural_height).min(1.);
                    img(image)
                        .w(px(natural_width * fit))
                        .h(px(natural_height * fit))
                        .rounded_lg()
                        .into_any_element()
                }
                Some(MediaImageState::Failed) => div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(brand::surface_container())
                    .text_size(px(13.))
                    .text_color(brand::secondary())
                    .child(format!("▢ {name} — this image format cannot be displayed"))
                    .into_any_element(),
                _ => div()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(brand::surface_container())
                    .text_size(px(13.))
                    .text_color(brand::secondary())
                    .child(format!("▢ {name}"))
                    .into_any_element(),
            };
            return div()
                .debug_selector({
                    let destination = destination.to_string();
                    move || format!("media-card-{destination}")
                })
                .child(card)
                .into_any_element();
        }

        let playing = state.playing;
        let toggle = {
            let this = this.clone();
            let destination = destination.to_string();
            let resolved = resolved.clone();
            move |_: &ClickEvent, _window: &mut Window, cx: &mut App| {
                this.update(cx, |app, cx| {
                    app.on_media_toggle(destination.clone(), resolved.clone(), kind, cx)
                })
                .ok();
            }
        };
        let fraction = state
            .duration_ms
            .filter(|duration| *duration > 0)
            .map(|duration| (state.position_ms as f32 / duration as f32).clamp(0., 1.))
            .unwrap_or(0.);
        let progress = div()
            .h(px(3.))
            .w_full()
            .rounded_full()
            .bg(brand::outline_variant().opacity(0.5))
            .child(
                div()
                    .h_full()
                    .w(relative(fraction))
                    .rounded_full()
                    .bg(brand::primary()),
            );
        let icon = Icon::new(if playing {
            IconName::Pause
        } else {
            IconName::Play
        })
        .size(px(if kind == MediaKind::Video { 28. } else { 16. }))
        .text_color(brand::primary());
        let error = state.error.as_ref().map(|error| {
            div()
                .text_size(px(11.))
                .text_color(brand::secondary())
                .child(error.clone())
        });

        let card = div()
            .id(ElementId::Name(format!("media-card-{destination}").into()))
            .debug_selector({
                let destination = destination.to_string();
                move || format!("media-card-{destination}")
            })
            .w_full()
            .max_w(px(560.))
            .rounded_lg()
            .bg(brand::surface_container_low())
            .border_1()
            .border_color(brand::outline_variant().opacity(0.6))
            .cursor_pointer()
            .on_click(toggle);

        match kind {
            MediaKind::Video => {
                let framed = self
                    .latest_frame
                    .as_ref()
                    .filter(|(current, _, _)| current == destination);
                let area = if let Some((_, frame, _)) = framed {
                    div()
                        .w_full()
                        .h(px(150.))
                        .relative()
                        .child(img(frame.clone()).object_fit(ObjectFit::Cover).size_full())
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon),
                        )
                        .into_any_element()
                } else {
                    div()
                        .w_full()
                        .h(px(150.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon)
                        .into_any_element()
                };
                card.h(px(200.))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .justify_between()
                    .child(area)
                    .child(
                        v_flex()
                            .px_3()
                            .pb_3()
                            .gap_2()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(brand::on_surface_variant())
                                            .child(name.clone()),
                                    )
                                    .children(error),
                            )
                            .child(progress),
                    )
                    .into_any_element()
            }
            _ => card
                .flex()
                .flex_row()
                .gap_3()
                .items_center()
                .px_3()
                .py_3()
                .child(icon)
                .child(
                    v_flex()
                        .flex_1()
                        .gap_2()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(brand::on_surface_variant())
                                        .child(name.clone()),
                                )
                                .children(error),
                        )
                        .child(progress),
                )
                .into_any_element(),
        }
    }

    /// Brings the list's items in step with the rendered days, keeping the
    /// scroll anchor. Items are the days (newest first) followed by the
    /// closing edge. A splice (not a reset) means the opening scan landing —
    /// or a new day file appearing — does not yank the reader to the top.
    fn sync_stream_items(&self, dates: &[NaiveDate]) {
        let new_total = dates.len() + 1;
        let previous = self.stream_dates.borrow();
        let actual = self.stream.item_count();
        if actual != self.stream_items.get() {
            // Something else moved the list; rebuild it from its real size.
            self.stream.splice(0..actual, new_total);
            self.stream_dates.replace(dates.to_vec());
            self.stream_items.set(new_total);
            return;
        }
        if previous.as_slice() == dates {
            return;
        }
        // The edge is the item after the days, and the list starts out empty,
        // before there is one: it shows up as `None`, so it matches only
        // itself and never a day.
        let at = |days: &[NaiveDate], ix: usize| days.get(ix).copied();
        let old_total = self.stream_items.get();
        let mut prefix = 0;
        while prefix < old_total && prefix < new_total && at(&previous, prefix) == at(dates, prefix)
        {
            prefix += 1;
        }
        let mut suffix = 0;
        while suffix < old_total - prefix
            && suffix < new_total - prefix
            && at(&previous, old_total - 1 - suffix) == at(dates, new_total - 1 - suffix)
        {
            suffix += 1;
        }
        let replaced = prefix..old_total - suffix;
        let introduced = new_total - prefix - suffix;
        drop(previous);
        if replaced.start != replaced.end || introduced != 0 {
            self.stream.splice(replaced, introduced);
        }
        self.stream_dates.replace(dates.to_vec());
        self.stream_items.set(new_total);
    }

    /// Asks for the caret to be brought into view on the next frames: a fresh
    /// day, or a keystroke that may have pushed the caret off the screen.
    fn request_reveal_caret(&self) {
        self.reveal_caret.set(true);
        self.reveal_attempts.set(0);
    }

    /// Scrolls the stream so the open day's caret sits in the viewport.
    ///
    /// The editor grows to its content and cannot scroll itself, so the
    /// timeline has to. Two safeguards keep this honest:
    ///
    /// - the caret rect is only trusted when it has held still for a frame,
    ///   because the first layout of a freshly opened day can still be a
    ///   partial one (revealing against it either does nothing or scrolls to
    ///   the wrong place, and the stream's item heights may not be measured
    ///   yet);
    /// - the reveal keeps at it — up to [`REVEAL_ATTEMPTS`] scrolls — until
    ///   the caret is actually inside the viewport, so a wrong landing
    ///   corrects itself on the next frame.
    ///
    /// Before the first layout there is no rect at all; the app's render
    /// keeps a frame coming while the flag stands.
    fn reveal_caret(&self, window: &Window, cx: &App) {
        if !self.reveal_caret.get() {
            return;
        }
        let Some(active) = self.active.as_ref() else {
            self.reveal_caret.set(false);
            return;
        };
        let Some(rect) = active.editor.read(cx).caret_window_rect() else {
            return;
        };
        let settled = self.reveal_rect.get() == Some(rect);
        self.reveal_rect.set(Some(rect));
        if !settled {
            return;
        }
        let viewport_bottom = window.viewport_size().height - px(STREAM_BOTTOM_MARGIN);
        let delta = if rect.origin.y + rect.size.height > viewport_bottom {
            rect.origin.y + rect.size.height - viewport_bottom
        } else if rect.origin.y < px(STREAM_TOP_MARGIN) {
            rect.origin.y - px(STREAM_TOP_MARGIN)
        } else {
            // The caret is in the viewport: nothing left to do.
            self.reveal_caret.set(false);
            self.reveal_attempts.set(0);
            return;
        };
        if self.reveal_attempts.get() >= REVEAL_ATTEMPTS {
            self.reveal_caret.set(false);
            self.reveal_attempts.set(0);
            return;
        }
        self.reveal_attempts.set(self.reveal_attempts.get() + 1);
        // Move by offset inside the current top item rather than asking the
        // list to seek by pixels: a seek walks the *recorded* item heights,
        // and the day being revealed is often the one still being measured,
        // so a seek overshoots into later days. A stored offset lets the
        // layout measure its way down instead.
        let top = self.stream.logical_scroll_top();
        self.stream.scroll_to(ListOffset {
            item_ix: top.item_ix,
            offset_in_item: (top.offset_in_item + delta).max(Pixels::ZERO),
        });
    }

    fn view(
        &self,
        theme: &EditorTheme,
        show_empty_days: bool,
        window: &Window,
        this: WeakEntity<DainikiApp>,
        cx: &App,
    ) -> impl IntoElement {
        let today = self.today;
        let mut existing: BTreeSet<NaiveDate> = self.days.keys().copied().collect();
        if let Some(active) = &self.active {
            // The active day is edited, not rendered from the map, but it
            // still counts as a day that exists.
            existing.insert(active.date);
        }
        let mut dates = journal::stream_dates(&existing, today, show_empty_days);

        // The active day's tags come from the open editor, so the sidebar and
        // the filter follow it keystroke by keystroke.
        let active_date = self.active.as_ref().map(|active| active.date);
        let active_text = self
            .active
            .as_ref()
            .map(|active| active.editor.read(cx).content(cx));
        let active_tags: BTreeSet<String> = active_text
            .as_deref()
            .map(journal::tags_in)
            .unwrap_or_default();

        // Narrow the stream to the selected tag. The open day always stays
        // visible — you are editing it.
        if self.filter.is_active() {
            dates.retain(|date| match &self.filter.tag {
                None => true,
                Some(tag) => {
                    self.tags.get(tag).is_some_and(|days| days.contains(date))
                        || (Some(*date) == active_date && active_tags.contains(tag))
                }
            });
            if let Some(date) = active_date
                && !dates.contains(&date)
            {
                dates.push(date);
                dates.sort_unstable_by(|a, b| b.cmp(a));
            }
        }

        // The list renders one item per day plus the closing edge. Keep its
        // item count in step with the day set.
        // The list renders one item per day plus the closing edge. Bring the
        // item set in step with the day set while keeping the scroll anchor:
        // the opening scan lands a frame or two in, and a naive reset would
        // drop the reader (and the caret reveal just made) back to the top.
        self.sync_stream_items(&dates);

        // The open entry grows to its content height inside the list; the
        // timeline is the only scroller. Keep the caret visible while typing
        // by moving the stream itself. This runs after the item-count sync
        // above: a reset drops the scroll position, so revealing after it in
        // the same frame is what makes the reveal stick.
        self.reveal_caret(window, cx);

        // A day that swapped between editor and render changed height; the
        // list must remeasure it (preserving the scroll anchor).
        if self.active_rendered.get() != active_date {
            if let Some(date) = active_date.or(self.active_rendered.get())
                && let Some(ix) = dates.iter().position(|day| *day == date)
            {
                self.stream.remeasure_items(ix..ix + 1);
            }
            self.active_rendered.set(active_date);
        }

        // A settled image in a read-only day changes that item's height. The
        // list caches heights, so re-measure the days it laid out last frame
        // (the stream's viewport and its overdraw); without this the card
        // paints over the text below it.
        if self.media_settled.get() != self.media_measured.get() {
            for ix in 0..dates.len() {
                if self.stream.bounds_for_item(ix).is_some() {
                    self.stream.remeasure_items(ix..ix + 1);
                }
            }
            self.media_measured.set(self.media_settled.get());
        }

        // A settled image load changes the height of the open day's media
        // line; without a remeasure the list keeps the placeholder's height
        // and the card paints over the text below it.
        let media_revision = self
            .active
            .as_ref()
            .map(|active| active.editor.read(cx).media_revision())
            .unwrap_or_default();
        if media_revision != self.active_media.get() {
            if let Some(date) = active_date
                && let Some(ix) = dates.iter().position(|day| *day == date)
            {
                self.stream.remeasure_items(ix..ix + 1);
            }
            self.active_media.set(media_revision);
        }

        // A jump asked for this day to be revealed; do it on the frame that
        // knows where it sits in the stream. Pinning the item's top is
        // deterministic even when far items are still unmeasured.
        if let Some(date) = self.pending_reveal.take()
            && let Some(ix) = dates.iter().position(|day| *day == date)
        {
            self.stream.scroll_to(ListOffset {
                item_ix: ix,
                offset_in_item: px(0.),
            });
        }

        let oldest_label = dates
            .last()
            .map(|date| date.format("%B %-d, %Y").to_string());

        let active_editor = self.active.as_ref().map(|active| active.editor.clone());
        let day_count = dates.len();
        let item_this = this.clone();
        let stream_list = list(self.stream.clone(), move |ix, window, cx| {
            if ix == day_count {
                return stream_edge(oldest_label.clone());
            }
            let date = dates[ix];
            let is_active = Some(date) == active_date;

            // The header's right side: a confirm row while a delete is
            // pending, otherwise nothing until the header is hovered — the
            // per-entry hint and trash icons made every day look like a
            // toolbar.
            let pending_delete = item_this.upgrade().and_then(|app| {
                let app = app.read(cx);
                match &app.screen {
                    Screen::Journal(session) => session.pending_delete,
                    Screen::Welcome { .. } => None,
                }
            });
            let right: AnyElement = if pending_delete == Some(date) {
                let delete_this = item_this.clone();
                let cancel_this = item_this.clone();
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(brand::secondary())
                            .child("Move to Trash?"),
                    )
                    .child(
                        Button::new("confirm-delete")
                            .label("Move to Trash")
                            .danger()
                            .compact()
                            .on_click(move |_, _window, cx| {
                                delete_this
                                    .update(cx, |app, cx| app.confirm_delete(date, cx))
                                    .ok();
                            }),
                    )
                    .child(
                        Button::new("cancel-delete")
                            .label("Cancel")
                            .compact()
                            .on_click(move |_, _window, cx| {
                                cancel_this.update(cx, |app, cx| app.cancel_delete(cx)).ok();
                            }),
                    )
                    .into_any_element()
            } else {
                // Reading or editing: the delete control waits for the cursor
                // instead of sitting on every entry.
                delete_control(date, item_this.clone())
            };
            let body: AnyElement = if is_active {
                let editor = active_editor
                    .as_ref()
                    .expect("the active day always has an editor");
                div()
                    .text_size(px(15.2))
                    .line_height(px(24.7))
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
                    // Read-only cards decode through the app, so a settled
                    // image can re-measure this day's item (the list caches
                    // heights, and a card that grows would paint over the
                    // text below it).
                    item_this
                        .update(cx, |app, cx| app.ensure_day_images(date, &text, cx))
                        .ok();
                    div()
                        .font_family(brand::SERIF_FAMILY)
                        .text_size(px(15.2))
                        .line_height(px(24.7))
                        .children(item_this.upgrade().and_then(|app| {
                            let app = app.read(cx);
                            match &app.screen {
                                Screen::Journal(session) => Some(session.day_body(
                                    date,
                                    &text,
                                    window.scale_factor(),
                                    &item_this,
                                )),
                                Screen::Welcome { .. } => None,
                            }
                        }))
                        .into_any_element()
                } else {
                    div()
                        .font_family(brand::SERIF_FAMILY)
                        .text_size(px(12.))
                        .italic()
                        .text_color(brand::on_surface_variant().opacity(0.4))
                        .child("Add thoughts for this day…")
                        .into_any_element()
                };
                clickable_day(item_this.clone(), date, inner)
            };

            // The day as a row: the gutter number, the timeline rule, and the
            // content column filling the rest of the window.
            let content = h_flex()
                .w_full()
                .items_start()
                .child(day_gutter(date, today))
                .child(
                    div()
                        .w(px(1.))
                        .self_stretch()
                        .mx_4()
                        .bg(brand::outline_variant().opacity(0.2)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(day_header(date, today, right))
                        .child(body),
                );

            let divider = h_flex().my_4().justify_center().child(
                div()
                    .w(px(48.))
                    .h(px(1.))
                    .bg(brand::outline_variant().opacity(0.3)),
            );

            let day = v_flex().w_full().pb_6().child(content).child(divider);
            if is_active {
                day.into_any_element()
            } else {
                // The whole day is the click target, header included.
                clickable_day(item_this.clone(), date, day.into_any_element())
            }
        });

        let mut main = div().flex_1().min_h_0().flex().flex_col();

        // Failures that matter (save, trash) surface here instead of only in
        // the log.
        if let Some(status) = &self.status {
            let dismiss_this = this.clone();
            let message = status.clone();
            main = main.child(
                div()
                    .w_full()
                    .px_3()
                    .py_2()
                    .bg(brand::surface_container_high())
                    .border_b_1()
                    .border_color(brand::outline_variant().opacity(0.6))
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(brand::on_surface())
                                    .child(message),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("status-dismiss")
                                    .label("Dismiss")
                                    .compact()
                                    .on_click(move |_, _window, cx| {
                                        dismiss_this
                                            .update(cx, |app, cx| app.dismiss_status(cx))
                                            .ok();
                                    }),
                            ),
                    ),
            );
        }

        // The jump panel is available in reading mode too, so a day is always
        // one keystroke away.
        if self.jump_open {
            let jump_this = this.clone();
            let today_this = this.clone();
            let close_this = this.clone();
            main = main.child(
                div()
                    .w_full()
                    .px_3()
                    .py_2()
                    .bg(brand::surface_container_low())
                    .border_b_1()
                    .border_color(brand::outline_variant().opacity(0.6))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .max_w(px(760.))
                            .mx_auto()
                            .child(
                                div()
                                    .font_family(brand::SERIF_FAMILY)
                                    .text_color(brand::on_surface_variant())
                                    .child("Jump to"),
                            )
                            .child(div().w(px(200.)).child(Input::new(&self.jump_input)))
                            .child(Button::new("jump-go").label("Go").compact().on_click(
                                move |_, window, cx| {
                                    jump_this
                                        .update(cx, |app, cx| app.jump_from_input(window, cx))
                                        .ok();
                                },
                            ))
                            .child(Button::new("jump-today").label("Today").compact().on_click(
                                move |_, window, cx| {
                                    today_this
                                        .update(cx, |app, cx| {
                                            if let Some(today) =
                                                app.session().map(|session| session.today)
                                            {
                                                app.jump_to(today, window, cx);
                                            }
                                        })
                                        .ok();
                                },
                            ))
                            .child(div().flex_1())
                            .children(self.jump_error.as_ref().map(|error| {
                                div()
                                    .text_xs()
                                    .text_color(brand::on_surface_variant())
                                    .child(error.clone())
                            }))
                            .child(Button::new("jump-close").label("Close").compact().on_click(
                                move |_, _window, cx| {
                                    close_this.update(cx, |app, cx| app.close_jump(cx)).ok();
                                },
                            )),
                    ),
            );
        }

        // The toolbar belongs to editing: it appears only while a day is
        // open in the editor.
        if let Some(active) = &self.active {
            let today_this = this.clone();
            main = main.child(
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
                                    .content(
                                        div()
                                            .debug_selector(|| "inline-formats".to_string())
                                            .child(active.inline_toolbar(cx)),
                                    )
                                    .content(Separator::vertical())
                                    .content(
                                        div()
                                            .debug_selector(|| "block-formats".to_string())
                                            .child(active.block_toolbar(cx)),
                                    ),
                            )
                            .child(active.insert_menu(&self.root))
                            .child(
                                Button::new("today-button")
                                    .label("Today")
                                    .compact()
                                    .on_click(move |_, window, cx| {
                                        today_this
                                            .update(cx, |app, cx| {
                                                if let Some(today) =
                                                    app.session().map(|session| session.today)
                                                {
                                                    app.jump_to(today, window, cx);
                                                }
                                            })
                                            .ok();
                                    }),
                            )
                            .child(finish_toggle(this.clone()))
                            .child(active.help_menu()),
                    ),
            );
        }

        main = main.child(
            div()
                .flex_1()
                .min_h_0()
                .bg(theme.background)
                .relative()
                .child(
                    div()
                        .w_full()
                        .h_full()
                        .child(stream_list.size_full().px_8().pt_8()),
                )
                // The timeline's scrollbar is the page's only bar: the open
                // entry grows to its content instead of scrolling inside its
                // own box.
                .child(
                    Scrollbar::new(&self.stream)
                        .id("stream-scrollbar")
                        .axis(ScrollbarAxis::Vertical),
                ),
        );

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(brand_title_bar(
                self.active
                    .as_ref()
                    .is_some_and(|active| active.dirty)
                    .then(|| {
                        div()
                            .text_color(brand::secondary())
                            .child("•")
                            .into_any_element()
                    }),
                Some(self.search_pill(&this)),
            ))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .child(self.sidebar(&this, cx))
                    .child(main),
            )
    }

    /// The title bar's search pill: a styled trigger — the title bar is a
    /// window-drag region, so a raw input there would fight the drag.
    fn search_pill(&self, this: &WeakEntity<DainikiApp>) -> AnyElement {
        let search_this = this.clone();
        h_flex()
            .id("search-pill")
            .gap_2()
            .items_center()
            .justify_between()
            .w(px(176.))
            .pl(px(8.))
            .pr(px(10.))
            .py(px(4.))
            .rounded_full()
            .bg(brand::surface_container_low())
            .border_1()
            .border_color(brand::surface_container())
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                search_this
                    .update(cx, |app, cx| app.on_focus_search(&FocusSearch, window, cx))
                    .ok();
            })
            .child(
                Icon::new(IconName::Search)
                    .size(px(14.))
                    .text_color(brand::on_surface_variant().opacity(0.7)),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(px(11.))
                    .text_color(brand::on_surface_variant().opacity(0.8))
                    .child("Search entries…"),
            )
            .child(
                div()
                    .text_size(px(9.))
                    .font_family("Menlo")
                    .px(px(6.))
                    .py(px(2.))
                    .rounded_sm()
                    .bg(brand::surface_container())
                    .text_color(brand::on_surface_variant().opacity(0.8))
                    .child("⌘F"),
            )
            .into_any_element()
    }

    /// The left rail: Timeline, Search, Tags, and the bottom block.
    fn sidebar(&self, this: &WeakEntity<DainikiApp>, cx: &App) -> AnyElement {
        // Tag counts: the index over stored days, plus the open day's tags
        // when they are not already counted for it.
        let active_date = self.active.as_ref().map(|active| active.date);
        let active_tags = self
            .active
            .as_ref()
            .map(|active| journal::tags_in(&active.editor.read(cx).content(cx)))
            .unwrap_or_default();
        let mut counts: BTreeMap<String, usize> = self
            .tags
            .iter()
            .map(|(tag, days)| (tag.clone(), days.len()))
            .collect();
        if let Some(date) = active_date {
            for tag in &active_tags {
                let already = self.tags.get(tag).is_some_and(|days| days.contains(&date));
                if !already {
                    *counts.entry(tag.clone()).or_default() += 1;
                }
            }
        }

        let timeline_this = this.clone();
        let timeline_active = !self.filter.is_active();
        let search_this = this.clone();
        let settings_this = this.clone();

        v_flex()
            .w(px(220.))
            .h_full()
            .flex_shrink_0()
            .bg(brand::surface_container_low().opacity(0.7))
            .border_r_1()
            .border_color(brand::surface_container_high().opacity(0.5))
            .child(
                v_flex()
                    .id("sidebar")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .pt_2()
                    .gap_1()
                    .child(
                        // Timeline: the whole stream, no filters.
                        h_flex()
                            .justify_between()
                            .items_center()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .when(timeline_active, |el| {
                                el.bg(brand::surface_container_highest())
                                    .font_weight(FontWeight::MEDIUM)
                            })
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                timeline_this
                                    .update(cx, |app, cx| app.clear_filter(window, cx))
                                    .ok();
                            })
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(Icon::new(IconName::Rows2).size(px(16.)).text_color(
                                        if timeline_active {
                                            brand::primary()
                                        } else {
                                            brand::on_surface_variant()
                                        },
                                    ))
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(if timeline_active {
                                                brand::on_surface()
                                            } else {
                                                brand::on_surface_variant()
                                            })
                                            .child("Timeline"),
                                    ),
                            )
                            .when(timeline_active, |el| {
                                el.child(
                                    div()
                                        .w(px(6.))
                                        .h(px(6.))
                                        .rounded_full()
                                        .bg(brand::primary()),
                                )
                            }),
                    )
                    .child(
                        // Search: opens the search palette (cmd-F).
                        h_flex()
                            .justify_between()
                            .items_center()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                search_this
                                    .update(cx, |app, cx| {
                                        app.on_focus_search(&FocusSearch, window, cx)
                                    })
                                    .ok();
                            })
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        Icon::new(IconName::Search)
                                            .size(px(16.))
                                            .text_color(brand::on_surface_variant()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(brand::on_surface_variant())
                                            .child("Search"),
                                    ),
                            )
                            .child(kbd_chip("⌘F")),
                    )
                    .child(
                        v_flex()
                            .mt_2()
                            .gap_1()
                            .child(
                                div()
                                    .px_2()
                                    .text_size(px(10.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(brand::on_surface_variant().opacity(0.8))
                                    .child("TAGS"),
                            )
                            .children(
                                counts
                                    .iter()
                                    .map(|(tag, count)| self.tag_row(tag, *count, this)),
                            )
                            .when(counts.is_empty(), |el| {
                                el.child(
                                    div()
                                        .px_2()
                                        .text_size(px(11.))
                                        .text_color(brand::outline())
                                        .child("Tag entries with #tags."),
                                )
                            }),
                    ),
            )
            .child(
                // The bottom block: the calendar, settings, and the footnote.
                v_flex()
                    .gap_2()
                    .p_2()
                    .bg(brand::surface_container_low().opacity(0.4))
                    .border_t_1()
                    .border_color(brand::surface_container_high().opacity(0.4))
                    .child(self.calendar_card(active_date, this))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .px_1()
                            .cursor_pointer()
                            .debug_selector(|| "settings-row".to_string())
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                settings_this
                                    .update(cx, |app, cx| app.open_settings(window, cx))
                                    .ok();
                            })
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        Icon::new(IconName::Settings)
                                            .size(px(14.))
                                            .text_color(brand::on_surface_variant()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(brand::on_surface_variant())
                                            .child("Settings"),
                                    ),
                            )
                            .child(kbd_chip("⌘,")),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .px_1()
                            .items_center()
                            .child(
                                Icon::new(IconName::LockKeyhole)
                                    .size(px(12.))
                                    .text_color(brand::primary()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(brand::on_surface_variant().opacity(0.6))
                                    .child("Directly editable stream"),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// One tag row: `#name` with its day count; clicking filters the stream.
    fn tag_row(&self, tag: &str, count: usize, this: &WeakEntity<DainikiApp>) -> AnyElement {
        let selected = self.filter.tag.as_deref() == Some(tag);
        let tag_owned = tag.to_string();
        let this = this.clone();
        h_flex()
            .justify_between()
            .items_center()
            .px_2()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .when(selected, |el| el.bg(brand::surface_container_highest()))
            .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                this.update(cx, |app, cx| app.toggle_tag_filter(&tag_owned, cx))
                    .ok();
            })
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        div()
                            .font_family("Menlo")
                            .text_size(px(12.))
                            .text_color(brand::secondary().opacity(0.7))
                            .child("#"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(if selected {
                                brand::on_surface()
                            } else {
                                brand::on_surface_variant()
                            })
                            .child(tag.to_string()),
                    ),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(brand::on_surface_variant().opacity(0.6))
                    .child(count.to_string()),
            )
            .into_any_element()
    }

    /// The sidebar calendar: a month at a glance, entry days dotted, today in
    /// primary; clicking a day opens it.
    fn calendar_card(
        &self,
        active_date: Option<NaiveDate>,
        this: &WeakEntity<DainikiApp>,
    ) -> AnyElement {
        let (year, month) = self.calendar_month;
        let label = NaiveDate::from_ymd_opt(year, month, 1)
            .map(|first| first.format("%B %Y").to_string())
            .unwrap_or_default();
        let grid = month_grid(year, month);
        let today = self.today;
        let mut with_entries: BTreeSet<NaiveDate> = self.days.keys().copied().collect();
        if let Some(date) = active_date {
            with_entries.insert(date);
        }

        let chevron = |label: &'static str, delta: i32, this: WeakEntity<DainikiApp>| {
            div()
                .id(ElementId::Name(format!("cal-chevron-{delta}").into()))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                    this.update(cx, |app, cx| app.shift_calendar_month(delta, cx))
                        .ok();
                })
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(brand::on_surface_variant())
                        .child(label),
                )
                .into_any_element()
        };

        v_flex()
            .gap_1()
            .p_2()
            .rounded_lg()
            .bg(brand::surface_container_lowest().opacity(0.8))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .px_1()
                    .child(
                        div()
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(brand::on_surface())
                            .child(label),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(chevron("‹", -1, this.clone()))
                            .child(chevron("›", 1, this.clone())),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .children(["M", "T", "W", "T", "F", "S", "S"].map(|day| {
                        div()
                            .w(px(24.))
                            .text_center()
                            .text_size(px(9.))
                            .text_color(brand::outline())
                            .child(day)
                    })),
            )
            .child(v_flex().gap_1().children(grid.chunks(7).map(|row| {
                h_flex()
                    .gap_1()
                    .children(row.iter().map(|(date, in_month)| {
                        self.calendar_cell(*date, *in_month, today, &with_entries, this)
                    }))
            })))
            .into_any_element()
    }

    /// One calendar cell: the number, primary when it has an entry or is
    /// today, dotted when it has an entry. The future is not clickable.
    fn calendar_cell(
        &self,
        date: NaiveDate,
        in_month: bool,
        today: NaiveDate,
        with_entries: &BTreeSet<NaiveDate>,
        this: &WeakEntity<DainikiApp>,
    ) -> AnyElement {
        let has_entry = with_entries.contains(&date);
        let is_today = date == today;
        let clickable = date <= today;
        let cell_this = this.clone();
        v_flex()
            .id(ElementId::Name(format!("cal-{date}").into()))
            .w(px(24.))
            .h(px(26.))
            .items_center()
            .justify_center()
            .gap(px(1.))
            .rounded_sm()
            .when(clickable, |el| {
                el.cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        cell_this
                            .update(cx, |app, cx| app.jump_to(date, window, cx))
                            .ok();
                    })
            })
            .child(
                div()
                    .text_size(px(10.))
                    .when(has_entry || is_today, |el| {
                        el.font_weight(FontWeight::SEMIBOLD)
                            .text_color(brand::primary())
                    })
                    .when(!has_entry && !is_today, |el| {
                        el.text_color(brand::on_surface_variant().opacity(if in_month {
                            0.8
                        } else {
                            0.3
                        }))
                    })
                    .child(day_number(date)),
            )
            .when(has_entry, |el| {
                el.child(
                    div()
                        .w(px(4.))
                        .h(px(4.))
                        .rounded_full()
                        .bg(brand::primary()),
                )
            })
            .into_any_element()
    }

    /// Rebuilds the tag index over the non-active days. Called whenever
    /// `days` changes; the active day's tags are merged in per render.
    fn rebuild_tags(&mut self) {
        self.tags.clear();
        for (date, text) in &self.days {
            for tag in journal::tags_in(text) {
                self.tags.entry(tag).or_default().insert(*date);
            }
        }
    }

    /// Removes a day from the session after its file has left the disk.
    fn apply_deleted_day(&mut self, date: NaiveDate) {
        self.days.remove(&date);
        self.pending_delete = None;
        self.rebuild_tags();
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

/// One stream item's date header, following the landing's showcase: the
/// pulsing dot and "Today — Thursday, October 15" heading, with `right` on
/// the far side (the click hint and delete control, or a confirm row).
fn day_header(date: NaiveDate, today: NaiveDate, right: AnyElement) -> AnyElement {
    let is_today = date == today;
    h_flex()
        // Hovering the heading reveals the entry's delete control. Every
        // entry shares the name; each header is its own group, so only the
        // hovered one lights up.
        .group(DAY_HEADER_GROUP)
        .items_center()
        .justify_between()
        .mb_2()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .when(is_today, |el| {
                    el.child(
                        div()
                            .w(px(8.))
                            .h(px(8.))
                            .rounded_full()
                            .bg(brand::primary()),
                    )
                })
                .child(
                    div()
                        .font_family(brand::SERIF_FAMILY)
                        .text_size(px(24.))
                        .line_height(px(32.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(brand::on_surface())
                        .child(day_heading(date, today)),
                ),
        )
        .child(right)
        .into_any_element()
}

/// The trash control at a day header's far side. It stays invisible until the
/// header is hovered, so the reading view is dates and text, not chrome.
fn delete_control(date: NaiveDate, app: WeakEntity<DainikiApp>) -> AnyElement {
    div()
        .id(ElementId::Name(format!("delete-{date}").into()))
        .opacity(0.)
        .group_hover(DAY_HEADER_GROUP, |style| style.opacity(1.))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
            app.update(cx, |app, cx| app.request_delete(date, cx)).ok();
        })
        .child(
            Icon::new(IconName::Trash)
                .size(px(13.))
                .text_color(brand::on_surface_variant().opacity(0.4)),
        )
        .into_any_element()
}

/// The ghosted day number and its weekday caption in the day's left gutter,
/// as in the showcase.
fn day_gutter(date: NaiveDate, today: NaiveDate) -> AnyElement {
    let caption = if date == today {
        "TODAY".to_string()
    } else {
        date.format("%a").to_string().to_uppercase()
    };
    v_flex()
        .w(px(72.))
        .items_end()
        .pt_1()
        .opacity(0.4)
        .child(
            div()
                .font_family(brand::SERIF_FAMILY)
                .text_size(px(48.))
                .line_height(px(48.))
                .text_color(brand::outline_variant())
                .child(day_number(date)),
        )
        .child(
            div()
                .text_size(px(10.))
                .text_color(brand::on_surface_variant())
                .child(caption),
        )
        .into_any_element()
}

/// The stream's closing item, so even a fresh journal reads intentionally
/// bottom-out.
fn stream_edge(oldest: Option<String>) -> AnyElement {
    v_flex()
        .items_center()
        .gap_1()
        .pt_4()
        .pb_2()
        .opacity(0.6)
        .child(
            Icon::new(IconName::ArrowDown)
                .size(px(14.))
                .text_color(brand::on_surface_variant()),
        )
        .child(
            div()
                .text_size(px(9.))
                .text_color(brand::on_surface_variant())
                .child(match oldest {
                    Some(oldest) => format!("{oldest} & earlier"),
                    None => "The edge of your journal".to_string(),
                }),
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
    /// The block-format toggles, in order. The group callback maps the
    /// flipped index back to a format: the component replaces per-toggle
    /// callbacks inside a group, so the group owns the click.
    fn block_toolbar(&self, cx: &App) -> ToggleGroup {
        let formats = [
            BlockFormat::Heading(1),
            BlockFormat::Heading(2),
            BlockFormat::Heading(3),
            BlockFormat::Bullet,
            BlockFormat::Ordered,
            BlockFormat::Task,
            BlockFormat::Quote,
        ];
        let items: [(&'static str, IconName, &'static str); 7] = [
            ("h1", IconName::Heading1, "Heading 1 — ⌘1"),
            ("h2", IconName::Heading2, "Heading 2 — ⌘2"),
            ("h3", IconName::Heading3, "Heading 3 — ⌘3"),
            ("bullet", IconName::List, "Bullet list — ⌘⇧8"),
            ("ordered", IconName::ListOrdered, "Numbered list — ⌘⇧7"),
            ("task", IconName::ListTodo, "Task list — ⌘⇧9"),
            ("quote", IconName::TextQuote, "Quote"),
        ];
        let checked: Vec<bool> = formats
            .iter()
            .map(|format| self.editor.read(cx).block_format_active(*format, cx))
            .collect();
        let known = checked.clone();
        let editor = self.editor.clone();

        let mut group = ToggleGroup::new("block-formats").segmented();
        for (index, (id, icon, tooltip)) in items.into_iter().enumerate() {
            group = group.child(
                Toggle::new(id)
                    .icon(Icon::new(icon))
                    .tooltip(tooltip)
                    .checked(checked[index]),
            );
        }
        group.on_click(move |next: &Vec<bool>, _window, cx| {
            let Some(index) = next
                .iter()
                .zip(&known)
                .position(|(now, before)| now != before)
            else {
                return;
            };
            let format = formats[index];
            editor.update(cx, |editor, cx| editor.toggle_block_format(format, cx));
        })
    }

    /// The inline-format toggles, in order. Same group-callback contract as
    /// the block toolbar.
    fn inline_toolbar(&self, cx: &App) -> ToggleGroup {
        let formats = [
            InlineFormat::Bold,
            InlineFormat::Italic,
            InlineFormat::Strike,
            InlineFormat::Highlight,
            InlineFormat::Code,
        ];
        let items: [(&'static str, IconName, &'static str); 5] = [
            ("bold", IconName::Bold, "Bold — ⌘B"),
            ("italic", IconName::Italic, "Italic — ⌘I"),
            ("strike", IconName::Strikethrough, "Strikethrough — ⌘⇧X"),
            ("highlight", IconName::Highlighter, "Highlight — ⌘⇧H"),
            ("code", IconName::Code, "Inline code — ⌘`"),
        ];
        let checked: Vec<bool> = formats
            .iter()
            .map(|format| self.editor.read(cx).inline_format_active(*format))
            .collect();
        let known = checked.clone();
        let editor = self.editor.clone();

        let mut group = ToggleGroup::new("inline-formats").segmented();
        for (index, (id, icon, tooltip)) in items.into_iter().enumerate() {
            group = group.child(
                Toggle::new(id)
                    .icon(Icon::new(icon))
                    .tooltip(tooltip)
                    .checked(checked[index]),
            );
        }
        group.on_click(move |next: &Vec<bool>, _window, cx| {
            let Some(index) = next
                .iter()
                .zip(&known)
                .position(|(now, before)| now != before)
            else {
                return;
            };
            let format = formats[index];
            editor.update(cx, |editor, cx| editor.toggle_inline_format(format, cx));
        })
    }

    fn help_menu(&self) -> Popover {
        const SHORTCUTS: [(&str, &str); 18] = [
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
            ("Jump to a date", "⌘T"),
            ("Search entries", "⌘F"),
            ("Previous / next day", "⌘⇧[ / ⌘⇧]"),
            ("Reveal markup", "click into a construct"),
        ];
        Popover::new("shortcut-help")
            .trigger(
                Button::new("help-trigger")
                    .icon(Icon::new(IconName::CircleQuestionMark))
                    .tooltip("Shortcuts")
                    .compact(),
            )
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

    fn insert_menu(&self, journal_root: &Path) -> Popover {
        let editor = self.editor.clone();
        let link_input = self.link_input.clone();
        // Every attachment lands in one `assets/` folder at the journal root.
        let journal_root = journal_root.to_path_buf();
        let assets = journal_root.join("assets");
        let entry_path = self.path.clone();

        let link_editor = editor.clone();
        let link_state = link_input.clone();
        let fence_editor = editor.clone();
        let rule_editor = editor.clone();
        let image_editor = editor.clone();

        Popover::new("insert-menu")
            .trigger(
                Button::new("insert-trigger")
                    .icon(Icon::new(IconName::Plus))
                    .tooltip("Insert")
                    .compact(),
            )
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
                    .label("Media…")
                    .tooltip("Insert an image, audio or video file")
                    .compact()
                    .on_click({
                        let editor = image_editor.clone();
                        let assets = assets.clone();
                        let entry_path = entry_path.clone();
                        let journal_root = journal_root.clone();
                        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                            let assets = assets.clone();
                            let entry_path = entry_path.clone();
                            let journal_root = journal_root.clone();
                            let prompt = cx.prompt_for_paths(PathPromptOptions {
                                files: true,
                                directories: false,
                                multiple: false,
                                prompt: Some("Choose an image, audio or video file".into()),
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
                                    let copied = cx
                                        .background_executor()
                                        .spawn(async move { copy_into_assets(&source, &assets) })
                                        .await;
                                    let Ok((alt, name)) = copied else {
                                        return;
                                    };
                                    let reference =
                                        media_reference(&entry_path, &journal_root, &name);
                                    editor.update(cx, |editor, cx| {
                                        editor.insert_media(&alt, &reference, cx)
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

/// One row of the search palette: a day, its label, and the entry text the
/// palette matches against.
struct PaletteDay {
    date: NaiveDate,
    label: String,
    text: String,
}

/// The palette's one-line label for a day: relative day names, then the
/// entry's first meaningful line.
fn palette_label(date: NaiveDate, today: NaiveDate, text: &str) -> String {
    let date_text = if date == today {
        format!("Today — {}", date.format("%A, %B %-d"))
    } else if Some(date) == today.pred_opt() {
        format!("Yesterday — {}", date.format("%A, %B %-d"))
    } else if date.year() == today.year() {
        date.format("%A, %B %-d").to_string()
    } else {
        date.format("%A, %B %-d, %Y").to_string()
    };
    match palette_excerpt(text) {
        Some(excerpt) => format!("{date_text} — {excerpt}"),
        None => date_text,
    }
}

/// The first meaningful line of an entry, stripped of markdown furniture and
/// capped so a palette row stays one line.
fn palette_excerpt(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let line = line.trim_start_matches(['#', '*', '>', '-', ' ', '\t']);
    if line.is_empty() {
        return None;
    }
    let mut excerpt: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        excerpt.push('…');
    }
    Some(excerpt)
}

/// One piece of a read-only day's body: a markdown run or a media card.
enum DaySegment {
    Text(String),
    Media {
        kind: MediaKind,
        destination: String,
    },
}

/// Splits day text at whole-line `![alt](destination)` references. Inline
/// references stay inside the text runs and become links (so the reading view
/// shows a name rather than raw markup); fenced code is left untouched.
fn day_segments(text: &str) -> Vec<DaySegment> {
    let mut segments: Vec<DaySegment> = Vec::new();
    let mut pending = String::new();
    let mut in_fence = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            pending.push_str(line);
            continue;
        }
        if in_fence {
            pending.push_str(line);
            continue;
        }
        match whole_line_media(line.trim()) {
            Some((destination, kind)) => {
                if !pending.is_empty() {
                    segments.push(DaySegment::Text(std::mem::take(&mut pending)));
                }
                segments.push(DaySegment::Media { destination, kind });
            }
            None => pending.push_str(&highlight_marks(&inline_media_links(line))),
        }
    }
    if !pending.is_empty() {
        segments.push(DaySegment::Text(pending));
    }
    segments
}

/// Rewrites inline `![alt](destination)` references into `[label](destination)`
/// links: the reading view has no inline media cards, and a raw path reads as
/// a bug. The label is the alt text, or the file name when there is none.
/// The read-only copy of an entry that has no whole-line media: the same
/// rewrite pass `day_segments` applies (inline references become links,
/// `==swipes==` become marks), joined back into one markdown run.
fn rewritten_text(text: &str) -> String {
    let mut rewritten = String::with_capacity(text.len());
    for segment in day_segments(text) {
        if let DaySegment::Text(text) = segment {
            rewritten.push_str(&text);
        }
    }
    rewritten
}

/// `==text==` is the editor's highlighter. The reading view speaks inline
/// HTML marks, so a read-only day paints the same phrase with a swipe behind
/// it instead of showing the markers.
fn highlight_marks(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut rest = line;
    loop {
        let Some(start) = rest.find("==") else {
            output.push_str(rest);
            return output;
        };
        let after = &rest[start + 2..];
        let Some(end) = after.find("==") else {
            output.push_str(rest);
            return output;
        };
        if end == 0 {
            // `====`: an empty swipe, not a highlight.
            output.push_str(&rest[..start + 4]);
            rest = &after[2..];
            continue;
        }
        output.push_str(&rest[..start]);
        output.push_str("<mark>");
        output.push_str(&after[..end]);
        output.push_str("</mark>");
        rest = &after[end + 2..];
    }
}

fn inline_media_links(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find("![") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(alt_end) = after.find("](") else {
            output.push_str(&rest[start..]);
            return output;
        };
        let Some(destination_end) = after[alt_end + 2..].find(')') else {
            output.push_str(&rest[start..]);
            return output;
        };
        let alt = after[..alt_end].trim();
        let destination = &after[alt_end + 2..alt_end + 2 + destination_end];
        if destination.is_empty() {
            output.push_str(&rest[start..start + 2 + alt_end + 2 + destination_end + 1]);
        } else {
            let label = if alt.is_empty() {
                destination.rsplit('/').next().unwrap_or(destination)
            } else {
                alt
            };
            output.push_str(&format!("[{label}]({destination})"));
        }
        rest = &after[alt_end + 2 + destination_end + 1..];
    }
    output.push_str(rest);
    output
}

/// The destination and kind of a whole-line `![alt](destination)`, when the
/// line is exactly that and the kind renders as a card.
fn whole_line_media(line: &str) -> Option<(String, MediaKind)> {
    let rest = line.strip_prefix("![")?;
    let (alt, rest) = rest.split_once("](")?;
    let destination = rest.strip_suffix(')')?;
    if destination.is_empty() || alt.contains('[') || destination.contains('(') {
        return None;
    }
    let kind = media_kind(destination);
    if kind == MediaKind::File {
        return None;
    }
    Some((destination.to_string(), kind))
}

/// The cells of a month's calendar grid, Monday first, padded with the
/// neighbouring months' dates: `(date, in_this_month)`.
fn month_grid(year: i32, month: u32) -> Vec<(NaiveDate, bool)> {
    let Some(first) = NaiveDate::from_ymd_opt(year, month, 1) else {
        return Vec::new();
    };
    let offset = first.weekday().num_days_from_monday() as u64;
    let days = days_in_month(year, month) as u64;
    let rows = (offset + days).div_ceil(7);
    let start = first - chrono::Days::new(offset);
    (0..rows * 7)
        .map(|index| {
            let date = start + chrono::Days::new(index);
            (date, date.month() == month)
        })
        .collect()
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("valid month");
    let next = NaiveDate::from_ymd_opt(next_year, next_month, 1).expect("valid month");
    (next - first).num_days() as u32
}

/// The small monospace shortcut chip used across the chrome (`⌘F`, `⌘,`).
fn kbd_chip(label: &'static str) -> AnyElement {
    div()
        .text_size(px(9.))
        .font_family("Menlo")
        .px(px(6.))
        .py(px(2.))
        .rounded_sm()
        .bg(brand::surface_container())
        .text_color(brand::on_surface_variant().opacity(0.8))
        .child(label)
        .into_any_element()
}

/// Watches the journal folder: edits made elsewhere (another editor, a sync
/// client) land in the reconcile channel. Our own atomic writes are
/// idempotent there — the reconcile compares content before changing
/// anything. Returns the watcher so it lives as long as the session.
#[cfg(not(test))]
fn start_watcher(root: &Path, cx: &mut Context<DainikiApp>) -> Option<notify::RecommendedWatcher> {
    let (tx, rx) = async_channel::unbounded::<PathBuf>();
    let watcher = match notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        if matches!(
            event.kind,
            notify::EventKind::Create(_)
                | notify::EventKind::Modify(_)
                | notify::EventKind::Remove(_)
        ) {
            for path in event.paths {
                let _ = tx.try_send(path);
            }
        }
    }) {
        Ok(mut watcher) => {
            if let Err(error) = watcher.watch(root, notify::RecursiveMode::Recursive) {
                tracing::warn!(root = %root.display(), error = %error, "could not watch the journal folder");
                None
            } else {
                Some(watcher)
            }
        }
        Err(error) => {
            tracing::warn!(error = %error, "could not start the journal watcher");
            None
        }
    };
    cx.spawn(async move |this, cx| {
        while let Ok(first) = rx.recv().await {
            let mut paths = vec![first];
            // Coalesce the burst a save produces.
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            while let Ok(more) = rx.try_recv() {
                paths.push(more);
            }
            this.update(cx, |app, cx| app.reconcile_paths(paths, cx))
                .ok();
        }
    })
    .detach();
    watcher
}

/// Tests do not start the OS watcher: it runs on its own thread, which the
/// deterministic test scheduler rejects as non-deterministic activity. The
/// reconcile logic is exercised directly instead.
#[cfg(test)]
fn start_watcher(
    _root: &Path,
    _cx: &mut Context<DainikiApp>,
) -> Option<notify::RecommendedWatcher> {
    None
}

/// Wakes the app every half minute so a session that crosses midnight rolls
/// over to the new day instead of writing to yesterday's file. The check
/// itself runs in render, where a window exists to open the new day with.
#[cfg(not(test))]
fn start_rollover_timer(cx: &mut Context<DainikiApp>) {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_secs(30))
                .await;
            let alive = this
                .update(cx, |app, cx| {
                    if let Some(session) = app.session()
                        && Local::now().date_naive() > session.today
                    {
                        session.pending_rollover.set(true);
                        app.repaint(cx);
                    }
                })
                .is_ok();
            if !alive {
                break;
            }
        }
    })
    .detach();
}

/// Tests drive `roll_over_if_needed` directly; a looping timer task would
/// keep the deterministic scheduler from ever parking.
#[cfg(test)]
fn start_rollover_timer(_cx: &mut Context<DainikiApp>) {}

/// The title bar every screen shares: the logo, the wordmark, and whatever
/// sits on either side.
fn brand_title_bar(left_extra: Option<AnyElement>, right: Option<AnyElement>) -> TitleBar {
    let mut bar = TitleBar::new().child(
        h_flex()
            .gap_2()
            .items_center()
            .children(brand::logo().map(|logo| img(logo).w(px(24.)).h(px(24.))))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Dainiki"),
            )
            .children(left_extra),
    );
    if let Some(right) = right {
        bar = bar.child(right);
    }
    bar
}

/// The ghosted day number beside a day's heading, as in the landing's
/// showcase: `19`.
fn day_number(date: NaiveDate) -> String {
    date.format("%-d").to_string()
}

/// A day's heading, as in the landing's showcase: `Today — Monday, October 19`
/// — with the year only when it is not the current year.
fn day_heading(date: NaiveDate, today: NaiveDate) -> String {
    let base = date.format("%A, %B %-d").to_string();
    let dated = if date.year() == today.year() {
        base
    } else {
        format!("{base}, {}", date.year())
    };
    if date == today {
        format!("Today — {dated}")
    } else {
        dated
    }
}

/// Parses what the date-jump panel accepts: `YYYY-MM-DD` (slashes too) and
/// the words `today`, `yesterday`, `tomorrow`.
fn parse_jump_date(input: &str, today: NaiveDate) -> Option<NaiveDate> {
    let text = input.trim().to_lowercase();
    match text.as_str() {
        "today" => return Some(today),
        "yesterday" => return today.pred_opt(),
        "tomorrow" => return today.succ_opt(),
        _ => {}
    }
    NaiveDate::parse_from_str(&text.replace('/', "-"), "%Y-%m-%d").ok()
}

/// Copies a media file into the journal's `assets/` folder under a fresh,
/// sanitized name, returning `(alt text, file name)`. Images, audio and video
/// all ride the same `![](…)` reference.
fn copy_into_assets(source: &Path, assets: &Path) -> Result<(String, String), String> {
    std::fs::create_dir_all(assets).map_err(|error| error.to_string())?;
    let original = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "media".to_string());
    let alt = original
        .rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .unwrap_or_else(|| original.clone())
        .replace(['[', ']'], "");
    let sanitized = sanitized_media_name(&original);
    let (stem, extension) = match sanitized.rsplit_once('.') {
        Some((stem, extension)) => (stem.to_string(), format!(".{extension}")),
        None => (sanitized.clone(), String::new()),
    };
    let mut counter = 0usize;
    let name = loop {
        let candidate = if counter == 0 {
            sanitized.clone()
        } else {
            format!("{stem}-{counter}{extension}")
        };
        if !assets.join(&candidate).exists() {
            break candidate;
        }
        counter += 1;
    };
    std::fs::copy(source, assets.join(&name)).map_err(|error| error.to_string())?;
    Ok((alt, name))
}

/// A file name that survives Markdown destinations and every filesystem we
/// target: letters, digits, `-`, `_` and `.` are kept (Unicode letters
/// included), anything else collapses to a single dash.
fn sanitized_media_name(file_name: &str) -> String {
    let (stem, extension) = match file_name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => (stem, extension),
        _ => (file_name, ""),
    };
    let mut cleaned = String::with_capacity(stem.len());
    let mut dash = false;
    for ch in stem.chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '.' {
            cleaned.push(ch);
            dash = false;
        } else if !dash {
            cleaned.push('-');
            dash = true;
        }
    }
    let cleaned = cleaned.trim_matches('-');
    let cleaned = if cleaned.is_empty() { "media" } else { cleaned };
    let extension: String = extension
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    if extension.is_empty() {
        cleaned.to_string()
    } else {
        format!("{cleaned}.{extension}")
    }
}

/// The reference written into an entry for a file in the journal's root
/// `assets/` folder: the link walks up from the entry's own folder (two
/// levels for the `YYYY/MM/` layout), so it stays correct if the layout
/// deepens.
fn media_reference(entry_path: &Path, journal_root: &Path, file_name: &str) -> String {
    let depth = entry_path
        .parent()
        .and_then(|directory| directory.strip_prefix(journal_root).ok())
        .map(|relative| relative.components().count())
        .unwrap_or(2);
    format!("{}assets/{file_name}", "../".repeat(depth))
}

impl Render for DainikiApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A caret reveal is pending: keep a frame coming until it settles, or
        // it would wait forever on an idle window.
        if self
            .session()
            .is_some_and(|session| session.reveal_caret.get())
        {
            cx.notify();
        }
        // The jump input's Enter event lands here, where a window exists to
        // open the day with.
        if self
            .session()
            .is_some_and(|session| session.pending_submit.get())
        {
            if let Some(session) = self.session_mut() {
                session.pending_submit.set(false);
            }
            self.jump_from_input(window, cx);
        }
        // An external change to the open day reloads it here, where a window
        // exists to rebuild the editor.
        if let Some(date) = self
            .session()
            .and_then(|session| session.pending_reload.take())
        {
            self.reload_active_day(date, window, cx);
        }
        // The rollover timer asks for a new day past midnight.
        if self
            .session()
            .is_some_and(|session| session.pending_rollover.take())
        {
            self.roll_over_if_needed(window, cx);
        }
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
            .on_action(cx.listener(Self::on_toggle_jump))
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_toggle_settings))
            .on_action(cx.listener(Self::on_previous_day))
            .on_action(cx.listener(Self::on_next_day))
            .child(screen)
    }
}

/// The app's key bindings; also registered by tests so they exercise the
/// real keymap.
fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-s", Save, None),
        KeyBinding::new("cmd-e", ToggleEditing, None),
        KeyBinding::new("cmd-t", ToggleJumpToDate, None),
        KeyBinding::new("cmd-f", FocusSearch, None),
        KeyBinding::new("cmd-,", ToggleSettings, None),
        KeyBinding::new("cmd-shift-[", PreviousDay, None),
        KeyBinding::new("cmd-shift-]", NextDay, None),
    ]);
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

    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            brand::load_fonts(cx);
            brand::apply_component_theme(cx);
            bind_keys(cx);
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

/// Resolves a theme preference against the current window appearance.
fn resolve_mode(preference: config::ThemePreference, appearance: WindowAppearance) -> brand::Mode {
    match preference {
        config::ThemePreference::Light => brand::Mode::Light,
        config::ThemePreference::Dark => brand::Mode::Dark,
        config::ThemePreference::System => match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => brand::Mode::Dark,
            _ => brand::Mode::Light,
        },
    }
}

/// The System/Light/Dark picker. The group callback owns the change (the
/// component replaces per-toggle callbacks inside a group) and decodes the
/// *flipped* index: a segmented group toggles one item without clearing the
/// others, so `next` can hold two truths at once.
fn theme_picker(this: WeakEntity<DainikiApp>, current: config::ThemePreference) -> ToggleGroup {
    use config::ThemePreference as Pref;
    let known = [
        current == Pref::System,
        current == Pref::Light,
        current == Pref::Dark,
    ];
    ToggleGroup::new("theme-pref")
        .segmented()
        .child(
            Toggle::new("theme-system")
                .label("System")
                .checked(known[0]),
        )
        .child(Toggle::new("theme-light").label("Light").checked(known[1]))
        .child(Toggle::new("theme-dark").label("Dark").checked(known[2]))
        .on_click(move |next: &Vec<bool>, window, cx| {
            let Some(index) = next
                .iter()
                .zip(&known)
                .position(|(now, before)| now != before)
            else {
                return;
            };
            let preference = [Pref::System, Pref::Light, Pref::Dark][index];
            this.update(cx, |app, cx| {
                app.set_theme_preference(preference, window, cx)
            })
            .ok();
        })
}

fn open_app_window(cx: &mut App, config: config::Config, root: Option<PathBuf>) -> Result<()> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(140.0), px(100.0)),
            size: size(px(1180.0), px(780.0)),
        })),
        window_min_size: Some(size(px(720.0), px(480.0))),
        ..TitleBar::window_options()
    };

    gpui_kit::open_window(options, cx, move |window, cx| {
        // The window exists: resolve the theme against its appearance before
        // anything paints.
        let preference = config.theme;
        let mode = resolve_mode(preference, window.appearance());
        brand::set_mode(mode);
        brand::apply_component_theme(cx);
        let theme = brand::editor_theme_for(mode);
        cx.new(|cx| {
            let mut app = DainikiApp::new(theme, config);
            app.watch_appearance(window, cx);
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
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::{Datelike, Local, NaiveDate};
    use gpui::{
        AppContext as _, Bounds, Entity, ListOffset, ScrollDelta, ScrollWheelEvent, TestAppContext,
        VisualTestContext, WindowBounds, WindowOptions, point, px,
    };

    // NOTE: no `use super::*` here. The crate root glob-imports `gpui_kit::*`,
    // which re-exports gpui's `test` proc macro; globbing it in would shadow
    // the builtin `#[test]` inside the generated harness and recurse forever.
    use gpui_kit::component::WindowExt as _;

    use super::{
        DainikiApp, Screen, brand, config, day_heading, day_number, journal, month_grid,
        parse_jump_date, resolve_cli_root, resolve_mode,
    };

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// The component library needs its theme global before any view renders;
    /// production also loads the brand fonts, palette and key bindings.
    fn init(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(brand::load_fonts);
        cx.update(brand::apply_component_theme);
        cx.update(super::bind_keys);
    }

    /// Opens a window the way production does: through `gpui_kit::open_window`
    /// so the app sits inside a `gpui_base::Root` and the component plugin
    /// layers (dialogs, notifications) exist. Returns the app entity.
    fn add_rooted_app(
        cx: &mut TestAppContext,
        dir: &Path,
    ) -> (Entity<DainikiApp>, VisualTestContext) {
        let dir = dir.to_path_buf();
        let (window, app) = cx
            .update(move |cx| {
                let options = WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::maximized(None, cx))),
                    ..Default::default()
                };
                gpui_kit::open_window(options, cx, move |window, cx| {
                    cx.new(|cx| {
                        let mut app =
                            DainikiApp::new(brand::editor_theme(), config::Config::default());
                        app.enter_journal(dir, window, cx);
                        app
                    })
                })
            })
            .expect("the app window opens");
        let cx = VisualTestContext::from_window(window, cx);
        (app, cx)
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

    #[gpui::test]
    async fn toggling_a_missing_medium_surfaces_an_error(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        seed_day(&journal, today, "![](assets/missing.mp3)\n");

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

        let missing = dir.join("2026/10/assets/missing.mp3");
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                app.on_media_toggle(
                    "assets/missing.mp3".into(),
                    missing.clone(),
                    super::MediaKind::Audio,
                    cx,
                );
            });
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let session = app.session().expect("an open session");
            assert!(
                !session.media.playing(),
                "a missing file must not report playing"
            );
            let status = session.status.clone().unwrap_or_default();
            assert!(
                status.contains("Could not play the audio"),
                "the failure should surface in the banner, got {status:?}"
            );
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn toggling_a_missing_video_surfaces_an_error(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        seed_day(&journal, today, "![](assets/missing.mp4)\n");

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

        let missing = dir.join("2026/10/assets/missing.mp4");
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                app.on_media_toggle(
                    "assets/missing.mp4".into(),
                    missing.clone(),
                    super::MediaKind::Video,
                    cx,
                );
            });
        });
        // The decode failure lands from the worker thread: poll for it.
        let mut status = String::new();
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            cx.run_until_parked();
            status = cx.update(|_window, cx| {
                app.read(cx)
                    .session()
                    .and_then(|session| session.status.clone())
                    .unwrap_or_default()
            });
            if status.contains("Could not play") {
                break;
            }
        }
        assert!(
            status.contains("Could not play the video"),
            "the failure should surface in the banner, got {status:?}"
        );

        cx.update(|_window, cx| {
            // The pump is stopped or failed; the session must not claim a live
            // pump for this destination afterwards.
            app.update(cx, |app, cx| app.stop_editing(cx));
            let app = app.read(cx);
            let session = app.session().expect("an open session");
            assert!(session.video.is_none(), "closing the day drops the pump");
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn a_toolbar_toggle_formats_the_open_line(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        seed_day(&journal, today, "hello");

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

        let target = cx
            .debug_bounds("block-formats")
            .expect("the block toolbar renders while editing")
            .center();
        cx.simulate_click(target, gpui::Modifiers::default());
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let active = app
                .session()
                .and_then(|session| session.active.as_ref())
                .expect("still editing");
            let content = active.editor.read(cx).content(cx);
            assert_ne!(content, "hello", "a toolbar click must reach the editor");
            assert!(
                content.starts_with("# ")
                    || content.starts_with("- ")
                    || content.starts_with("1. ")
                    || content.starts_with("> "),
                "the clicked toggle should apply a block format, got {content:?}"
            );
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn copying_a_spaced_name_sanitizes_it_into_one_assets_folder() {
        let dir = scratch_dir();
        let source = dir.join("Screen Recording 2026-10-15 at 09.54.02-1.mov");
        fs::write(&source, b"x").expect("seed");
        let assets = dir.join("assets");
        let (alt, name) = super::copy_into_assets(&source, &assets).expect("copy");
        assert_eq!(
            name, "Screen-Recording-2026-10-15-at-09.54.02-1.mov",
            "spaces and odd characters collapse to dashes"
        );
        assert_eq!(alt, "Screen Recording 2026-10-15 at 09.54.02-1");
        assert!(assets.join(&name).exists(), "the copy lands in assets/");
        assert_eq!(
            super::media_reference(
                Path::new("/journal/2026/10/2026-10-19.md"),
                Path::new("/journal"),
                &name
            ),
            "../../assets/Screen-Recording-2026-10-15-at-09.54.02-1.mov",
            "entries link back up to the journal's assets folder"
        );
        assert_eq!(
            super::media_reference(
                Path::new("/journal/2026-10-19.md"),
                Path::new("/journal"),
                &name
            ),
            "assets/Screen-Recording-2026-10-15-at-09.54.02-1.mov",
            "a shallower layout links one level up"
        );

        // A second copy of the same name takes a numbered suffix.
        let (_, again) = super::copy_into_assets(&source, &assets).expect("copy again");
        assert_eq!(again, "Screen-Recording-2026-10-15-at-09.54.02-1-1.mov");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn media_names_sanitize_to_markdown_safe_filenames() {
        use super::sanitized_media_name;
        assert_eq!(
            sanitized_media_name("Screenshot 2026-10-18 at 08.11.33-1.png"),
            "Screenshot-2026-10-18-at-08.11.33-1.png"
        );
        assert_eq!(sanitized_media_name("a  b\tc.png"), "a-b-c.png");
        assert_eq!(
            sanitized_media_name("clip(1)[draft].mov"),
            "clip-1-draft.mov"
        );
        assert_eq!(sanitized_media_name("shot #2?.png"), "shot-2.png");
        assert_eq!(
            sanitized_media_name("日本語 ノート.png"),
            "日本語-ノート.png"
        );
        assert_eq!(sanitized_media_name("   .png"), "media.png");
        assert_eq!(sanitized_media_name("no-extension"), "no-extension");
        assert_eq!(sanitized_media_name("weird.EXT!"), "weird.EXT");
    }

    #[test]
    fn no_argument_resolves_to_nothing() {
        assert_eq!(resolve_cli_root(None), None);
    }

    #[test]
    fn theme_preference_resolves_against_the_system() {
        use super::config::ThemePreference as Pref;
        use gpui::WindowAppearance;
        assert_eq!(
            resolve_mode(Pref::System, WindowAppearance::Dark),
            brand::Mode::Dark
        );
        assert_eq!(
            resolve_mode(Pref::System, WindowAppearance::VibrantDark),
            brand::Mode::Dark
        );
        assert_eq!(
            resolve_mode(Pref::System, WindowAppearance::Light),
            brand::Mode::Light
        );
        assert_eq!(
            resolve_mode(Pref::Dark, WindowAppearance::Light),
            brand::Mode::Dark,
            "an explicit choice wins over the system"
        );
        assert_eq!(
            resolve_mode(Pref::Light, WindowAppearance::Dark),
            brand::Mode::Light
        );
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
    fn month_grids_start_on_monday_and_pad_neighbours() {
        let grid = month_grid(2026, 10);
        assert_eq!(grid.len(), 35, "five rows for October 2026");
        assert_eq!(
            grid[0],
            (NaiveDate::from_ymd_opt(2026, 9, 28).unwrap(), false)
        );
        assert_eq!(
            grid[3],
            (NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(), true)
        );
        assert_eq!(
            grid[34],
            (NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(), false)
        );
        assert_eq!(grid.iter().filter(|(_, inside)| *inside).count(), 31);

        let february = month_grid(2024, 2);
        assert_eq!(february.iter().filter(|(_, inside)| *inside).count(), 29);
    }

    #[test]
    fn day_headings_follow_the_showcase() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 19).expect("date");
        let earlier = NaiveDate::from_ymd_opt(2026, 10, 5).expect("date");
        let last_year = NaiveDate::from_ymd_opt(2025, 12, 31).expect("date");
        assert_eq!(day_number(earlier), "5");
        assert_eq!(day_heading(earlier, today), "Monday, October 5");
        assert_eq!(day_heading(today, today), "Today — Monday, October 19");
        assert_eq!(
            day_heading(last_year, today),
            "Wednesday, December 31, 2025"
        );
    }

    #[test]
    fn jump_input_parses_dates_and_words() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 19).expect("date");
        assert_eq!(
            parse_jump_date("2026-10-05", today),
            Some(NaiveDate::from_ymd_opt(2026, 10, 5).unwrap())
        );
        assert_eq!(
            parse_jump_date("  2026/10/05 ", today),
            Some(NaiveDate::from_ymd_opt(2026, 10, 5).unwrap())
        );
        assert_eq!(parse_jump_date("today", today), Some(today));
        assert_eq!(
            parse_jump_date("Yesterday", today),
            Some(NaiveDate::from_ymd_opt(2026, 10, 18).unwrap())
        );
        assert_eq!(
            parse_jump_date("tomorrow", today),
            Some(NaiveDate::from_ymd_opt(2026, 10, 20).unwrap())
        );
        assert_eq!(parse_jump_date("", today), None);
        assert_eq!(parse_jump_date("someday", today), None);
        assert_eq!(parse_jump_date("2026-13-01", today), None);
    }

    #[gpui::test]
    async fn jumping_to_an_old_day_reveals_it_in_the_stream(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        let mut oldest = today;
        for _ in 0..30 {
            oldest = oldest.pred_opt().expect("previous day");
            seed_day(&journal, oldest, "# day\n\nA line written that day.\n");
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

        // Jump to the oldest day: it opens and the stream scrolls to it.
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.jump_to(oldest, window, cx));
            let _ = window.draw(cx);
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            let active = session.active.as_ref().expect("the day opened");
            assert_eq!(active.date, oldest);
            assert_eq!(
                active.editor.read(cx).content(cx),
                "# day\n\nA line written that day.\n"
            );
            let top = session.stream.logical_scroll_top();
            assert!(
                top.item_ix > 0,
                "the stream should have scrolled to the jumped day: {top:?}"
            );
            assert_eq!(
                session.calendar_month,
                (oldest.year(), oldest.month()),
                "the sidebar calendar follows a jump"
            );
        });

        // Month stepping wraps correctly across years.
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| app.shift_calendar_month(-1, cx));
        });
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| app.shift_calendar_month(1, cx));
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            assert_eq!(session.calendar_month, (oldest.year(), oldest.month()));
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn previous_day_shortcut_moves_the_editor(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        let yesterday = today.pred_opt().expect("yesterday");
        let before_yesterday = yesterday.pred_opt().expect("the day before");
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

        // cmd-T opens the jump panel and focuses its input; typing a day and
        // pressing Enter opens it.
        cx.simulate_keystrokes("cmd-t");
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            assert!(session.jump_open, "cmd-T did not open the jump panel");
        });
        cx.simulate_input("yesterday");
        cx.simulate_keystrokes("enter");
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            assert!(!session.jump_open, "the jump panel closes after a jump");
            let active = session.active.as_ref().expect("a day is open");
            assert_eq!(active.date, yesterday);
            assert_eq!(active.editor.read(cx).content(cx), "# yesterday's words\n");
        });

        // The editor holds focus; the day-step shortcut must still reach the
        // app (the editor consumes every arrow-key variant, so day steps use
        // bracket keys).
        cx.simulate_keystrokes("cmd-shift-[");
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            let active = session.active.as_ref().expect("a day is open");
            assert_eq!(active.date, before_yesterday);
        });
        fs::remove_dir_all(&dir).ok();
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

        // Reading mode first: with the full-height editor open, yesterday
        // sits below the fold. Finishing editing renders today as a day too.
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| app.stop_editing(cx));
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
    async fn a_tag_filters_the_stream(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        let tagged = today.pred_opt().expect("yesterday");
        let other = tagged.pred_opt().expect("the day before");
        seed_day(
            &journal,
            tagged,
            "# travel\n\nA #travel day through the mountains.\n",
        );
        seed_day(&journal, other, "# work\n\nPlain words without tags.\n");

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

        // The scan lands, the index sees the tag, and the full stream shows
        // today + two days + the edge.
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            assert_eq!(
                session.tags.get("travel").map(|days| days.len()),
                Some(1),
                "the tag index sees yesterday"
            );
            assert_eq!(session.stream.item_count(), 4);
        });

        // Filtering by the tag keeps the tagged day and the open day only.
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| app.toggle_tag_filter("travel", cx));
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            assert_eq!(session.filter.tag.as_deref(), Some("travel"));
            assert_eq!(
                session.stream.item_count(),
                3,
                "today + the tagged day + the edge"
            );
        });

        // Clicking the tag again clears the filter.
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| app.toggle_tag_filter("travel", cx));
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            assert!(!session.filter.is_active());
            assert_eq!(session.stream.item_count(), 4);
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn reading_days_render_media_cards(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        let yesterday = today.pred_opt().expect("yesterday");
        fs::create_dir_all(yesterday.format("%Y/%m").to_string()).ok();
        // One file in the month's own assets folder, one in the journal-root
        // folder the insert flow now targets.
        fs::create_dir_all(dir.join("assets")).expect("root assets");
        fs::write(dir.join("assets/root.mp3"), b"x").expect("seed root asset");
        seed_day(
            &journal,
            yesterday,
            "# Media day\n\nAn audio take:\n\n![](assets/take.mp3)\n\nA root take:\n\n![](../../assets/root.mp3)\n\nA clip:\n\n![](assets/clip.mp4)\n",
        );

        let (app, cx) = cx.add_window_view({
            let dir = dir.clone();
            move |window, cx| {
                let mut app = DainikiApp::new(brand::editor_theme(), config::Config::default());
                app.enter_journal(dir, window, cx);
                app
            }
        });
        // Reading mode: the past day renders as a day, not as today's editor.
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| app.stop_editing(cx));
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        // The whole-line media references became cards in the reading view.
        assert!(
            cx.debug_bounds("media-card-assets/take.mp3").is_some(),
            "the audio reference renders as a card"
        );
        assert!(
            cx.debug_bounds("media-card-../../assets/root.mp3")
                .is_some(),
            "a journal-root asset renders as a card"
        );
        assert!(
            cx.debug_bounds("media-card-assets/clip.mp4").is_some(),
            "the video reference renders as a card"
        );
        // And the markdown around them still renders: the day exists and its
        // text body is not the media lines.
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let session = app.session().expect("session");
            let day = session.days.get(&yesterday).expect("the stored day");
            assert_eq!(
                super::day_segments(day)
                    .iter()
                    .filter(|segment| matches!(segment, super::DaySegment::Media { .. }))
                    .count(),
                3,
                "three media segments"
            );
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn highlights_become_marks_when_reading() {
        use super::{day_segments, highlight_marks, rewritten_text};

        assert_eq!(
            highlight_marks("a ==swipe== b"),
            "a <mark>swipe</mark> b",
            "a highlight becomes a mark"
        );
        assert_eq!(
            highlight_marks("==one== and ==two=="),
            "<mark>one</mark> and <mark>two</mark>",
            "each pair becomes its own mark"
        );
        assert_eq!(
            highlight_marks("unclosed ==here"),
            "unclosed ==here",
            "a lone marker is left alone"
        );
        assert_eq!(highlight_marks("===="), "====", "an empty swipe is not one");
        assert!(
            matches!(
                day_segments("say ==this== twice\n").first(),
                Some(super::DaySegment::Text(text)) if text == "say <mark>this</mark> twice\n"
            ),
            "read-only days carry the swipe into the markdown view"
        );
        // A day without cards takes the joined rewrite, not the raw source.
        assert_eq!(
            rewritten_text("say ==this== twice\n"),
            "say <mark>this</mark> twice\n"
        );
        assert_eq!(
            rewritten_text("inline ![a](p.png) here\n"),
            "inline [a](p.png) here\n",
            "inline references read as links even without a card in the day"
        );
    }

    #[test]
    fn day_segments_split_whole_line_media_only() {
        use super::{DaySegment, day_segments};
        use dainiki_editor::MediaKind;

        // The exact shape a real entry takes: spacey macOS attachment names,
        // bare (the fallback grammar handles it) and bracketed.
        let real = "Ok Testing this journal write now #thoughts\n\n\
                    ![Screenshot 2026-10-18 at 08.11.33](assets/Screenshot 2026-10-18 at 08.11.33-1.png)\n\n\
                    ![Screen Recording 2026-10-15 at 09.54.02](assets/Screen Recording 2026-10-15 at 09.54.02-1.mov)\n";
        let segments = day_segments(real);
        assert_eq!(segments.len(), 4, "text, image, text, video");
        assert!(
            matches!(&segments[1], DaySegment::Media { kind, destination }
                if *kind == MediaKind::Image
                    && destination == "assets/Screenshot 2026-10-18 at 08.11.33-1.png")
        );
        assert!(
            matches!(&segments[3], DaySegment::Media { kind, destination }
                if *kind == MediaKind::Video
                    && destination == "assets/Screen Recording 2026-10-15 at 09.54.02-1.mov")
        );

        let text = "intro\n\n![](assets/a.mp3)\n\nmiddle\n\n![](assets/b.mp4)\n\nend\n";
        let segments = day_segments(text);
        assert_eq!(segments.len(), 5);
        assert!(matches!(&segments[0], DaySegment::Text(text) if text == "intro\n\n"));
        assert!(
            matches!(&segments[1], DaySegment::Media { kind, destination }
                if *kind == MediaKind::Audio && destination == "assets/a.mp3")
        );
        assert!(matches!(&segments[2], DaySegment::Text(text) if text == "\nmiddle\n\n"));
        assert!(
            matches!(&segments[3], DaySegment::Media { kind, destination }
                if *kind == MediaKind::Video && destination == "assets/b.mp4")
        );
        assert!(matches!(&segments[4], DaySegment::Text(text) if text == "\nend\n"));

        // Inline references become links, with the alt text or file name as
        // the label; unknown files stay text.
        let inline = "text ![a](p.png) more\n";
        assert!(matches!(
            day_segments(inline).as_slice(),
            [DaySegment::Text(text)] if text == "text [a](p.png) more\n"
        ));
        let no_alt = "text ![](assets/p.png) more\n";
        assert!(matches!(
            day_segments(no_alt).as_slice(),
            [DaySegment::Text(text)] if text == "text [p.png](assets/p.png) more\n"
        ));
        let file = "![](notes.txt)\n";
        assert!(matches!(
            day_segments(file).as_slice(),
            [DaySegment::Text(_)]
        ));
        // Fenced code keeps its source untouched.
        let fence = "```\n![a](p.png)\n```\n";
        assert!(matches!(
            day_segments(fence).as_slice(),
            [DaySegment::Text(text)] if text == fence
        ));
        // Images are media too.
        let image = "![](assets/photo.png)\n";
        assert!(matches!(
            day_segments(image).as_slice(),
            [DaySegment::Media {
                kind: MediaKind::Image,
                ..
            }]
        ));
    }

    #[gpui::test]
    async fn search_opens_a_palette_and_jumps_to_a_matched_day(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        let bamboo = today - chrono::Days::new(2);
        seed_day(&journal, bamboo, "# dawn\n\nThe bamboo path at dawn.\n");

        let (app, mut cx) = add_rooted_app(cx, &dir);
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        // cmd-F opens the palette as a modal.
        cx.simulate_keystrokes("cmd-f");
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|window, cx| {
            assert!(
                window.has_active_dialog(cx),
                "cmd-F should open the search palette"
            );
        });

        // It lists every day with an entry, newest first: today, then bamboo.
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let session = app.session().expect("an open session");
            let days = session.palette_days(cx);
            assert_eq!(days.len(), 2, "today and the bamboo day");
            assert_eq!(days[0].date, today);
            assert!(days[0].label.starts_with("Today —"));
            assert!(
                days[1].label.contains("dawn"),
                "the row shows the entry's first line: {:?}",
                days[1].label
            );
            assert_eq!(
                session.search_palette.read(cx).matched_count(),
                2,
                "the palette lists both days"
            );
        });

        // Typing narrows the list to the day whose text matches.
        cx.simulate_input("bamboo");
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let session = app.session().expect("an open session");
            assert_eq!(
                session.search_palette.read(cx).matched_count(),
                1,
                "only the bamboo day matches"
            );
        });

        // Enter confirms the highlighted day: the palette closes and the day
        // opens in the editor.
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|window, cx| {
            assert!(
                !window.has_active_dialog(cx),
                "confirming closes the palette"
            );
            let app = app.read(cx);
            let active = app
                .session()
                .and_then(|session| session.active.as_ref())
                .expect("a day is open for editing");
            assert_eq!(active.date, bamboo, "the matched day opened");
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn settings_open_as_a_modal_from_the_row_and_the_shortcut(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        seed_day(&journal, today, "# hello\n");

        let (_app, mut cx) = add_rooted_app(cx, &dir);
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        // cmd-, opens the settings modal; its content draws a frame.
        cx.simulate_keystrokes("cmd-,");
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|window, cx| {
            assert!(
                window.has_active_dialog(cx),
                "cmd-, should open the settings modal"
            );
        });

        // Done dismisses it.
        cx.update(|window, cx| window.close_dialog(cx));
        cx.update(|window, cx| {
            assert!(!window.has_active_dialog(cx));
        });

        // The sidebar row opens the same modal.
        let target = cx
            .debug_bounds("settings-row")
            .expect("the settings row renders")
            .center();
        cx.simulate_click(target, gpui::Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|window, cx| {
            assert!(
                window.has_active_dialog(cx),
                "the settings row opens the modal"
            );
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn the_settings_picker_switches_the_theme(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        seed_day(&journal, today, "# hello\n");

        let (app, mut cx) = add_rooted_app(cx, &dir);
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.open_settings(window, cx));
        });
        cx.run_until_parked();
        // Let the dialog's entrance animation settle so the geometry below
        // is its resting position.
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(600));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        // The Dark segment sits at the right end of the picker; System at the
        // left. Clicks on the ends stay clear of the segment boundaries, and
        // the bounds are re-read per click: switching themes re-lays the modal
        // out.
        let click_end = |cx: &mut VisualTestContext, from_right: bool| {
            let picker = cx
                .debug_bounds("theme-pref")
                .expect("the theme picker renders in the modal");
            let x = if from_right {
                picker.origin.x + picker.size.width - px(12.)
            } else {
                picker.origin.x + px(12.)
            };
            cx.simulate_click(
                point(x, picker.origin.y + picker.size.height / 2.),
                gpui::Modifiers::default(),
            );
        };

        let previous = cx.update(|_window, cx| app.read(cx).config.theme);

        click_end(&mut cx, true);
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            let app = app.read(cx);
            assert_eq!(
                app.config.theme,
                config::ThemePreference::Dark,
                "clicking Dark must choose the dark theme"
            );
            assert_eq!(
                brand::mode(),
                brand::Mode::Dark,
                "the palette must follow the preference"
            );
        });

        // Clicking the other end switches again, and the modal stays open.
        click_end(&mut cx, false);
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            assert!(
                window.has_active_dialog(cx),
                "the modal survives the theme switch"
            );
            let app = app.read(cx);
            assert_eq!(
                app.config.theme,
                config::ThemePreference::System,
                "clicking System must choose the system theme"
            );
        });

        // Leave the process-global palette as the tests found it.
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.set_theme_preference(previous, window, cx));
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn palette_labels_name_the_day_and_its_first_line() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 19).expect("a date");
        assert_eq!(
            super::palette_label(today, today, "# Hello\n\nmore\n"),
            "Today — Monday, October 19 — Hello"
        );
        assert_eq!(
            super::palette_label(today - chrono::Days::new(1), today, "> quoted line\n"),
            "Yesterday — Sunday, October 18 — quoted line"
        );
        assert_eq!(
            super::palette_label(today - chrono::Days::new(2), today, ""),
            "Saturday, October 17"
        );
        let last_year = NaiveDate::from_ymd_opt(2025, 12, 31).expect("a date");
        assert_eq!(
            super::palette_label(last_year, today, ""),
            "Wednesday, December 31, 2025"
        );
        assert_eq!(super::palette_excerpt("\n\n#   \n"), None, "no words");
        let long = "x".repeat(80);
        let excerpt = super::palette_excerpt(&long).expect("an excerpt");
        assert_eq!(excerpt.chars().count(), 61, "60 chars plus the ellipsis");
        assert!(excerpt.ends_with('…'));
    }

    #[gpui::test]
    async fn a_failed_save_surfaces_in_the_status_banner(cx: &mut TestAppContext) {
        use std::os::unix::fs::PermissionsExt;

        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        let entry = journal.entry_path(today);

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
        cx.simulate_input("words that cannot be saved");
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        // Make the entry's folder read-only, then force the save.
        let month = entry.parent().expect("month folder");
        fs::set_permissions(month, fs::Permissions::from_mode(0o555)).expect("chmod");
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| app.save_active(cx));
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            let status = session.status.as_deref().expect("a save failure is shown");
            assert!(status.contains("Could not save"), "{status:?}");
            assert!(
                session.active.as_ref().is_some_and(|a| a.dirty),
                "still dirty"
            );
        });

        // Restore permissions so cleanup works.
        fs::set_permissions(month, fs::Permissions::from_mode(0o755)).expect("chmod back");
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn midnight_rollover_opens_the_new_day(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        let yesterday = today.pred_opt().expect("yesterday");
        let yesterday_path = journal.entry_path(yesterday);

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

        // Pretend the session started yesterday: the open day is yesterday,
        // and someone typed into it.
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                if let Some(session) = app.session_mut() {
                    session.today = yesterday;
                }
                cx.notify();
            });
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.jump_to(yesterday, window, cx));
            let _ = window.draw(cx);
        });
        cx.simulate_input("old words typed before midnight");
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.roll_over_if_needed(window, cx));
            let _ = window.draw(cx);
        });

        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            assert_eq!(session.today, today, "the session moved to the new day");
            let active = session.active.as_ref().expect("the new day is open");
            assert_eq!(active.date, today);
            let flushed = fs::read_to_string(&yesterday_path).expect("yesterday flushed");
            assert!(
                flushed.contains("old words typed before midnight"),
                "the old day was flushed before the rollover: {flushed:?}"
            );
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn external_changes_reconcile_into_the_stream(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        let yesterday = today.pred_opt().expect("yesterday");
        let before_yesterday = yesterday.pred_opt().expect("the day before");
        seed_day(&journal, yesterday, "# travel\n\n#travel day\n");
        seed_day(&journal, before_yesterday, "# old words\n");

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

        // Another editor rewrites yesterday.
        let yesterday_path = journal.entry_path(yesterday);
        fs::write(&yesterday_path, "# changed outside\n\nNow with #nature\n").expect("write");
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                app.reconcile_paths(vec![yesterday_path.clone()], cx)
            });
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            assert_eq!(
                session.days.get(&yesterday).map(String::as_str),
                Some("# changed outside\n\nNow with #nature\n")
            );
            assert!(
                session.tags.contains_key("nature"),
                "tags follow the change"
            );
        });

        // The day before is deleted outside.
        let removed = journal.entry_path(before_yesterday);
        fs::remove_file(&removed).expect("remove");
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| app.reconcile_paths(vec![removed.clone()], cx));
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            assert!(!session.days.contains_key(&before_yesterday));
        });

        // Today (open and clean) changes on disk: the editor reloads from
        // the disk copy rather than flushing its stale one over it.
        let today_path = journal.entry_path(today);
        fs::write(&today_path, "# written elsewhere\n").expect("write");
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                app.reconcile_paths(vec![today_path.clone()], cx)
            });
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            let active = session.active.as_ref().expect("today stays open");
            assert_eq!(active.date, today);
            assert_eq!(active.editor.read(cx).content(cx), "# written elsewhere\n");
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn deleting_a_day_asks_then_removes_it(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        let yesterday = today.pred_opt().expect("yesterday");
        seed_day(&journal, yesterday, "# travel\n\n#travel day\n");

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

        // The first click only asks.
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| app.request_delete(yesterday, cx));
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            assert_eq!(session.pending_delete, Some(yesterday));
        });

        // Cancel clears it.
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| app.cancel_delete(cx));
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            assert_eq!(session.pending_delete, None);
            assert!(session.days.contains_key(&yesterday));
        });

        // The state transition after the file leaves the disk (the trash move
        // itself is one syscall, kept out of tests so they never write to the
        // user's Trash).
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                if let Some(session) = app.session_mut() {
                    session.apply_deleted_day(yesterday);
                }
                cx.notify();
            });
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let Screen::Journal(session) = &app.screen else {
                panic!("expected an open journal session");
            };
            assert!(!session.days.contains_key(&yesterday));
            assert!(
                !session.tags.contains_key("travel"),
                "the tag index forgets the deleted day"
            );
            assert_eq!(session.stream.item_count(), 2, "today and the edge");
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn the_wheel_scrolls_the_page_through_the_open_entry(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        let yesterday = today.pred_opt().expect("yesterday");
        seed_day(&journal, yesterday, "# yesterday\n\nwords\n");
        // A tall entry: in the old nested-scroll model this was a second
        // scroller beside the timeline.
        let long: String = (0..200).map(|ix| format!("line {ix}\n")).collect();
        seed_day(&journal, today, &long);

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
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        // The open entry grows to its content: it has no scroller of its own.
        let editor_offset = cx.update(|_window, cx| {
            app.read(cx)
                .session()
                .and_then(|session| session.active.as_ref())
                .map(|active| active.editor.read(cx).scroll_offset())
                .unwrap_or_default()
        });
        assert_eq!(editor_offset, px(0.), "the entry does not scroll itself");

        // The wheel, wherever it sits over the entry, scrolls the timeline.
        // (Wheel up: opening the day already revealed the caret at its end.)
        let before = cx.update(|_window, cx| {
            let app = app.read(cx);
            let session = app.session().expect("an open session");
            let top = session.stream.logical_scroll_top();
            (top.item_ix, top.offset_in_item.as_f32())
        });
        for _ in 0..3 {
            cx.simulate_event(ScrollWheelEvent {
                position: point(px(700.), px(300.)),
                delta: ScrollDelta::Pixels(point(px(0.), px(180.))),
                ..Default::default()
            });
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
        }
        let after = cx.update(|_window, cx| {
            let app = app.read(cx);
            let session = app.session().expect("an open session");
            let top = session.stream.logical_scroll_top();
            (top.item_ix, top.offset_in_item.as_f32())
        });
        assert!(
            after.1 < before.1 || after.0 < before.0,
            "the wheel scrolls the timeline, before {before:?} after {after:?}"
        );
        let editor_offset = cx.update(|_window, cx| {
            app.read(cx)
                .session()
                .and_then(|session| session.active.as_ref())
                .map(|active| active.editor.read(cx).scroll_offset())
                .unwrap_or_default()
        });
        assert_eq!(editor_offset, px(0.), "the entry still does not scroll");
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn a_long_entry_opens_with_its_caret_visible(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        let yesterday = today.pred_opt().expect("yesterday");
        // Yesterday makes the stream's item count settle a frame late (the
        // opening scan discovers it), resetting the list's scroll position
        // after the first reveal — the reveal must survive that.
        seed_day(&journal, yesterday, "# yesterday\n\nwords\n");
        let long: String = (0..200).map(|ix| format!("line {ix}\n")).collect();
        seed_day(&journal, today, &long);

        let (app, cx) = cx.add_window_view({
            let dir = dir.clone();
            move |window, cx| {
                let mut app = DainikiApp::new(brand::editor_theme(), config::Config::default());
                app.enter_journal(dir, window, cx);
                app
            }
        });
        // The caret sits at the end of the entry; the timeline scrolls so it
        // is visible instead of the editor scrolling itself.
        for _ in 0..4 {
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
        }
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let session = app.session().expect("an open session");
            let top = session.stream.logical_scroll_top();
            assert_eq!(top.item_ix, 0, "still inside today's item");
            assert!(
                top.offset_in_item > px(0.),
                "the stream scrolled down into the entry to show the caret"
            );
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn discovered_days_keep_the_scroll_position(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        // Enough days for the stream to scroll.
        for days_back in 1..=8 {
            let date = today
                .checked_sub_signed(chrono::Duration::days(days_back))
                .expect("a past day");
            seed_day(
                &journal,
                date,
                &format!("# day {days_back}\n\nwords {days_back}\n"),
            );
        }

        let (app, cx) = cx.add_window_view({
            let dir = dir.clone();
            move |window, cx| {
                let mut app = DainikiApp::new(brand::editor_theme(), config::Config::default());
                app.enter_journal(dir, window, cx);
                app
            }
        });
        for _ in 0..3 {
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
        }

        // Read a little further down the timeline.
        for _ in 0..2 {
            cx.simulate_event(ScrollWheelEvent {
                position: point(px(700.), px(400.)),
                delta: ScrollDelta::Pixels(point(px(0.), px(-150.))),
                ..Default::default()
            });
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
        }
        let before = cx.update(|_window, cx| {
            let app = app.read(cx);
            let session = app.session().expect("an open session");
            let top = session.stream.logical_scroll_top();
            (
                (top.item_ix, top.offset_in_item),
                session.stream.item_count(),
            )
        });
        assert!(
            before.0.0 > 0 || before.0.1 > px(0.),
            "the wheel moved the stream, got {:?}",
            before.0
        );

        // A day beyond the ones already seen appears (an editor from another
        // tool wrote it): the stream grows below, the reader stays put.
        let older = today
            .checked_sub_signed(chrono::Duration::days(30))
            .expect("an older day");
        let older_path = journal.entry_path(older);
        seed_day(&journal, older, "# old, discovered late\n\nwords\n");
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| app.reconcile_paths(vec![older_path], cx));
        });
        for _ in 0..2 {
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
        }
        cx.update(|_window, cx| {
            let app = app.read(cx);
            let session = app.session().expect("an open session");
            assert!(
                session.stream.item_count() > before.1,
                "the discovered day (and the days between) joined the stream: {} -> {}",
                before.1,
                session.stream.item_count()
            );
            let top = session.stream.logical_scroll_top();
            assert_eq!(
                (top.item_ix, top.offset_in_item),
                before.0,
                "the reader keeps their place when days appear"
            );
        });
        fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn typing_pulls_the_view_back_to_the_caret(cx: &mut TestAppContext) {
        init(cx);
        let dir = scratch_dir();
        let journal = journal::Journal::new(&dir);
        let today = Local::now().date_naive();
        let long: String = (0..200).map(|ix| format!("line {ix}\n")).collect();
        seed_day(&journal, today, &long);

        let (app, cx) = cx.add_window_view({
            let dir = dir.clone();
            move |window, cx| {
                let mut app = DainikiApp::new(brand::editor_theme(), config::Config::default());
                app.enter_journal(dir, window, cx);
                app
            }
        });
        for _ in 0..3 {
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
        }

        // Wander off to the top of the entry, far from the caret.
        cx.update(|_window, cx| {
            app.update(cx, |app, _cx| {
                let session = app.session_mut().expect("an open session");
                session.stream.scroll_to(ListOffset {
                    item_ix: 0,
                    offset_in_item: px(0.),
                });
            });
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        // A keystroke asks for the caret again, wherever the reader wandered.
        cx.simulate_input("x");
        for _ in 0..3 {
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
        }
        cx.update(|window, cx| {
            let app = app.read(cx);
            let session = app.session().expect("an open session");
            let active = session.active.as_ref().expect("today is open");
            let caret = active
                .editor
                .read(cx)
                .caret_window_rect()
                .expect("the editor has a layout");
            let viewport = window.viewport_size().height;
            assert!(
                caret.origin.y + caret.size.height <= viewport,
                "the caret came back into the viewport at {caret:?} of {viewport:?}"
            );
            assert!(
                session.stream.logical_scroll_top().offset_in_item > px(0.),
                "the stream scrolled down to it"
            );
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
