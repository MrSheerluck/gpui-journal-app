use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point, Render, RenderImage,
    ScrollWheelEvent, UTF16Selection, Window, div, point, prelude::*, px, size,
};

use crate::{
    buffer::Buffer,
    commands::{self, BlockFormat, Edit as CommandEdit, InlineFormat, SlashItem},
    display::{DisplayPlan, MediaKind, WidgetKind, destination_path, media_kind},
    input::{
        EditorElement, EditorLayout, EditorScrollState, Selection,
        history::{Edit, History, MergeKind},
        movement,
    },
    markdown::{Construct, ConstructKind, MarkdownIndex},
    theme::EditorTheme,
};
const CARET_BLINK_INTERVAL: Duration = Duration::from_millis(530);
const AUTOSCROLL_INTERVAL: Duration = Duration::from_millis(16);

/// Events the host listens to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorEvent {
    /// The buffer text changed: typing, deletion, paste, or undo/redo.
    Edited,
    /// A media widget was activated: the host should start playback and push
    /// state back through [`MarkdownEditor::set_media_state`].
    MediaToggle {
        /// The destination as written in the document (`assets/take.mp3`).
        destination: String,
        /// The same destination resolved against the entry's folder.
        resolved: PathBuf,
        kind: MediaKind,
    },
}

impl EventEmitter<EditorEvent> for MarkdownEditor {}

/// Playback state the host pushes back for one media destination.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaPlaybackState {
    pub playing: bool,
    /// Elapsed playback time.
    pub position_ms: u64,
    /// Total length, when the host knows it.
    pub duration_ms: Option<u64>,
    /// A playback failure to show on the widget.
    pub error: Option<String>,
}

pub struct MarkdownEditor {
    buffer: Entity<Buffer>,
    theme: EditorTheme,
    scroll: Rc<EditorScrollState>,
    layout: EditorLayout,
    selection: Selection,
    focus_handle: FocusHandle,
    marked_range: Option<Range<usize>>,
    history: History,
    index: MarkdownIndex,
    plan: Rc<DisplayPlan>,
    revision: u64,
    display_revision: u64,
    display_selection: Selection,
    caret_visible: bool,
    blink_epoch: u64,
    focused: bool,
    dragging: bool,
    drag_position: Point<Pixels>,
    hover_link: Option<(String, Point<Pixels>)>,
    /// The source range of the task marker under the pointer, for hover feedback.
    hover_task: Option<Range<usize>>,
    autoscroll_active: bool,
    base_dir: Option<PathBuf>,
    images: HashMap<String, ImageState>,
    /// Bumped whenever an image load settles; see [`Self::media_revision`].
    media_revision: u64,
    /// Host-pushed playback state per media destination.
    media_states: HashMap<String, MediaPlaybackState>,
    /// Host-pushed video frames per media destination (the latest one wins).
    media_frames: HashMap<String, Arc<RenderImage>>,
    pending_edit: Option<PendingEdit>,
    slash: Option<SlashMenu>,
    /// A slash position the user dismissed with Escape; no menu re-opens
    /// there until the `/` itself is gone.
    slash_dismissed: Option<usize>,
    /// Set when the caret or text changed; consumed by one paint to scroll
    /// the caret into view. Scrolling never triggers it, so the wheel is
    /// never fought by the reveal.
    reveal_pending: bool,
    /// Grow to the content height instead of filling the host's box; the
    /// host's scroll viewport then owns scrolling (one scroller for a
    /// timeline of entries).
    fit_content: bool,
}

/// Decoded state of one referenced image, keyed by its destination string.
#[derive(Clone)]
pub(crate) enum ImageState {
    Loading,
    Loaded(Arc<RenderImage>),
    Failed,
}

/// The edit a render pass should hand the markdown index. Multiple edits
/// between frames collapse into a full rebuild for safety.
#[derive(Clone)]
enum PendingEdit {
    One { range: Range<usize>, new_len: usize },
    Full,
}

/// The slash menu's live state, derived from the buffer on every projection.
#[derive(Clone, Debug)]
pub(crate) struct SlashMenu {
    /// Byte offset of the `/` that opened the menu.
    pub query_start: usize,
    /// What has been typed after the slash so far.
    pub query: String,
    pub items: Vec<SlashItem>,
    pub selected: usize,
}

pub(crate) const SLASH_ROW_HEIGHT: f32 = 26.;
pub(crate) const SLASH_PANEL_WIDTH: f32 = 240.;

impl MarkdownEditor {
    pub fn new(buffer: Entity<Buffer>, theme: EditorTheme, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            buffer,
            theme,
            scroll: Rc::new(EditorScrollState::default()),
            layout: EditorLayout::default(),
            selection: Selection::default(),
            focus_handle: cx.focus_handle(),
            marked_range: None,
            history: History::default(),
            index: MarkdownIndex::new(),
            plan: Rc::new(DisplayPlan::default()),
            revision: 0,
            display_revision: u64::MAX,
            display_selection: Selection::default(),
            caret_visible: true,
            blink_epoch: 0,
            focused: false,
            dragging: false,
            drag_position: Point::default(),
            hover_link: None,
            hover_task: None,
            autoscroll_active: false,
            base_dir: None,
            images: HashMap::new(),
            media_revision: 0,
            media_states: HashMap::new(),
            media_frames: HashMap::new(),
            pending_edit: None,
            slash: None,
            slash_dismissed: None,
            reveal_pending: true,
            fit_content: false,
        };
        this.spawn_blink(cx);
        this
    }

    /// Toggles caret visibility while the editor is focused. Movement calls
    /// [`Self::restart_blink`], which delays the next toggle by bumping the
    /// epoch; the loop only toggles when the epoch it last saw still matches.
    fn spawn_blink(&mut self, cx: &mut Context<Self>) {
        let mut seen_epoch = self.blink_epoch;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CARET_BLINK_INTERVAL).await;
                let Ok(epoch) = this.update(cx, |this, cx| {
                    if this.focused && this.blink_epoch == seen_epoch {
                        this.caret_visible = !this.caret_visible;
                        cx.notify();
                    }
                    this.blink_epoch
                }) else {
                    break;
                };
                seen_epoch = epoch;
            }
        })
        .detach();
    }

    fn restart_blink(&mut self) {
        self.caret_visible = true;
        self.blink_epoch = self.blink_epoch.wrapping_add(1);
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    /// Swaps the editor's theme. Colors repaint on the next frame; tokens
    /// that affect metrics (heading scales, body font) apply to lines shaped
    /// after the next full re-layout.
    pub fn set_theme(&mut self, theme: EditorTheme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    pub(crate) fn theme(&self) -> &EditorTheme {
        &self.theme
    }

    pub(crate) fn selection(&self) -> Selection {
        self.selection
    }

    pub(crate) fn caret_visible(&self) -> bool {
        self.caret_visible
    }

    pub(crate) fn marked_range(&self) -> Option<Range<usize>> {
        self.marked_range.clone()
    }

    /// The editor's text content, for hosts that persist the buffer.
    pub fn content(&self, cx: &App) -> String {
        let buffer = self.buffer.read(cx);
        buffer.slice(0..buffer.len_bytes())
    }

    /// Collapses the caret to the end of the entry. Hosts call this when they
    /// swap the buffer to a day the user just opened, so typing continues
    /// where the entry left off. The next render reveals the caret.
    pub fn place_caret_at_end(&mut self, cx: &mut Context<Self>) {
        let end = self.buffer.read(cx).len_bytes();
        self.selection = Selection::caret(end);
        self.history.break_run();
        self.restart_blink();
        cx.notify();
    }

    /// The current display projection (cheap `Rc` clone).
    pub(crate) fn plan(&self) -> Rc<DisplayPlan> {
        self.plan.clone()
    }

    /// Records a buffer edit for the next render's incremental re-parse.
    fn record_edit(&mut self, range: Range<usize>, new_len: usize) {
        self.pending_edit = match self.pending_edit.take() {
            None => Some(PendingEdit::One { range, new_len }),
            Some(_) => Some(PendingEdit::Full),
        };
        self.revision += 1;
    }

    /// Re-projects the display when the text or selection changed since the
    /// last call. Called at the top of every render; a no-op otherwise.
    fn sync_display(&mut self, cx: &mut Context<Self>) {
        if self.revision == self.display_revision && self.selection == self.display_selection {
            return;
        }
        let text = self.content(cx);
        match self.pending_edit.take() {
            Some(PendingEdit::One { range, new_len }) => {
                self.index.update_edited(&text, range, new_len)
            }
            Some(PendingEdit::Full) | None => self.index.update(&text),
        }
        self.plan = Rc::new(DisplayPlan::compute(&text, &self.index, self.selection));
        self.display_revision = self.revision;
        self.display_selection = self.selection;
        self.reveal_pending = true;
        self.sync_slash(&text);
        self.ensure_images(&text, cx);
    }

    /// Re-derives the slash menu from the buffer: a `/` typed at a word
    /// boundary on the caret's line, followed only by a short query.
    fn sync_slash(&mut self, text: &str) {
        let previous = self.slash.take();
        self.slash = self.detect_slash(text).map(|mut menu| {
            if let Some(previous) = &previous
                && previous.query_start == menu.query_start
                && previous.query == menu.query
                && !menu.items.is_empty()
            {
                menu.selected = previous.selected.min(menu.items.len() - 1);
            }
            menu
        });
    }

    fn detect_slash(&self, text: &str) -> Option<SlashMenu> {
        if !self.selection.is_caret() {
            return None;
        }
        let caret = self.selection.head().min(text.len());
        let line_start = text[..caret].rfind('\n').map(|ix| ix + 1).unwrap_or(0);
        let typed = &text[line_start..caret];

        let mut query_start = None;
        for (ix, ch) in typed.char_indices() {
            let at_boundary = typed[..ix]
                .chars()
                .last()
                .map(|before| before.is_whitespace())
                .unwrap_or(true);
            if ch == '/' && at_boundary {
                query_start = Some(line_start + ix);
            }
        }
        let query_start = query_start?;
        if self.slash_dismissed == Some(query_start) {
            return None;
        }
        // No menu inside code spans or fences.
        if self
            .index
            .constructs_in(query_start..query_start + 1)
            .any(|c| matches!(c.kind, ConstructKind::CodeSpan | ConstructKind::Fence))
        {
            return None;
        }

        let query = &text[query_start + 1..caret];
        if query.chars().any(char::is_whitespace) || query.len() > 24 {
            return None;
        }
        let items = commands::slash_items(query);
        if items.is_empty() {
            return None;
        }
        Some(SlashMenu {
            query_start,
            query: query.to_string(),
            items,
            selected: 0,
        })
    }

    #[cfg(test)]
    pub(crate) fn slash_menu(&self) -> Option<SlashMenu> {
        self.slash.clone()
    }

    fn apply_slash_item(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(menu) = self.slash.take() else {
            return;
        };
        let Some(item) = menu.items.get(index).copied() else {
            return;
        };
        let text = self.content(cx);
        let caret = self.selection.head();
        let command = commands::apply_slash(&text, caret, menu.query_start, item);
        self.slash_dismissed = None;
        self.apply_command(command, cx);
    }

    /// The dropdown panel's screen rectangle for the current slash menu,
    /// anchored below the slash and clamped to the editor's bounds.
    pub(crate) fn slash_panel(&self, cx: &App) -> Option<(SlashMenu, gpui::Bounds<Pixels>)> {
        let menu = self.slash.clone()?;
        let bounds = self.layout.bounds()?;
        let line_height = self.layout.body_line_height()?;
        let (line_ix, local) = {
            let buffer = self.buffer.read(cx);
            buffer.offset_to_line(menu.query_start)
        };
        let display = self
            .plan
            .lines
            .get(line_ix)
            .map(|line| line.display_for_source(menu.query_start))
            .unwrap_or(local);
        let (x, segment_y) = self.layout.with_line(line_ix, |line, height| {
            line.position_for_index(display, height)
                .map(|position| (position.x, position.y))
        })??;
        let top = self.layout.line_top(line_ix)?;

        let width = px(SLASH_PANEL_WIDTH);
        let rows = menu.items.len();
        let height = px(SLASH_ROW_HEIGHT * rows as f32 + 8.);
        let gap = px(4.);
        let mut origin_x = bounds.origin.x + x;
        let below = bounds.origin.y + top + segment_y + line_height - self.scroll.offset() + gap;
        let origin_y = if below + height <= bounds.origin.y + bounds.size.height {
            below
        } else {
            (below - height - line_height - px(8.)).max(bounds.origin.y)
        };
        origin_x = origin_x
            .min(bounds.origin.x + bounds.size.width - width - gap)
            .max(bounds.origin.x + gap);
        Some((
            menu,
            gpui::Bounds {
                origin: point(origin_x, origin_y),
                size: gpui::Size { width, height },
            },
        ))
    }

    /// Whether this frame should scroll the caret into view. Consumed once.
    pub(crate) fn reveal_pending(&self) -> bool {
        self.reveal_pending
    }

    pub(crate) fn clear_reveal(&mut self) {
        self.reveal_pending = false;
    }

    /// The directory relative image destinations resolve against.
    pub fn set_base_dir(&mut self, dir: Option<PathBuf>, cx: &mut Context<Self>) {
        self.base_dir = dir;
        self.images.clear();
        self.media_states.clear();
        self.media_frames.clear();
        self.pending_edit = Some(PendingEdit::Full);
        self.revision += 1;
        cx.notify();
    }

    /// Pushes playback state for one media destination so the widget can show
    /// it. Hosts call this on every meaningful change and while playing.
    pub fn set_media_state(
        &mut self,
        destination: &str,
        state: MediaPlaybackState,
        cx: &mut Context<Self>,
    ) {
        let changed = self.media_states.get(destination) != Some(&state);
        if changed {
            self.media_states.insert(destination.to_string(), state);
            cx.notify();
        }
    }

    /// Pushes the latest decoded video frame for a destination.
    pub fn set_media_frame(
        &mut self,
        destination: &str,
        frame: Arc<RenderImage>,
        cx: &mut Context<Self>,
    ) {
        self.media_frames.insert(destination.to_string(), frame);
        cx.notify();
    }

    /// Snapshot of playback states for painting.
    pub(crate) fn media_states(&self) -> HashMap<String, MediaPlaybackState> {
        self.media_states.clone()
    }

    /// Snapshot of the latest video frames for painting.
    pub(crate) fn media_frames(&self) -> HashMap<String, Arc<RenderImage>> {
        self.media_frames.clone()
    }

    /// The resolved filesystem path for a media destination.
    fn resolved_asset_path(&self, destination: &str) -> PathBuf {
        let path = PathBuf::from(destination_path(destination));
        if path.is_absolute() {
            return path;
        }
        match &self.base_dir {
            Some(dir) => dir.join(path),
            None => path,
        }
    }

    /// Starts background loads for every referenced image not yet tracked.
    /// Audio, video and other files are not decoded here; the host plays
    /// them, and video poster frames arrive through the host as well.
    fn ensure_images(&mut self, text: &str, cx: &mut Context<Self>) {
        let destinations: Vec<String> = self
            .index
            .constructs()
            .iter()
            .filter_map(|construct| match construct.kind {
                ConstructKind::Image => construct
                    .inner
                    .get(1)
                    .map(|range| text[range.clone()].to_string()),
                _ => None,
            })
            .filter(|destination| media_kind(destination) == MediaKind::Image)
            .collect();
        for destination in destinations {
            if self.images.contains_key(&destination) {
                continue;
            }
            self.images.insert(destination.clone(), ImageState::Loading);
            let path = self.resolved_asset_path(&destination);
            cx.spawn(async move |this, cx| {
                let decoded = cx
                    .background_executor()
                    .spawn({
                        let path = path.clone();
                        async move { load_render_image(&path) }
                    })
                    .await;
                let state = match decoded {
                    Ok(image) => ImageState::Loaded(image),
                    Err(error) => {
                        tracing::warn!(path = %path.display(), error = %error, "failed to load image");
                        ImageState::Failed
                    }
                };
                this.update(cx, |editor, cx| {
                    editor.images.insert(destination, state);
                    editor.revision += 1;
                    editor.media_revision += 1;
                    cx.notify();
                    // A plain notify can be dropped when it lands between
                    // frames; a decoded image must reach the screen without
                    // waiting for the next input event.
                    cx.refresh_windows();
                })
                .ok();
            })
            .detach();
        }
    }

    /// Snapshot of image load states for layout and painting.
    pub(crate) fn image_metrics(&self) -> HashMap<String, ImageState> {
        self.images.clone()
    }

    /// Bumped whenever an image load settles. A decoded image changes the
    /// height its line occupies, so hosts that cache item heights — a list of
    /// days — must re-measure when this moves.
    pub fn media_revision(&self) -> u64 {
        self.media_revision
    }

    /// Scrolls by `delta`; returns whether the offset actually changed, so
    /// hosts can implement scroll chaining at the editor's bounds.
    pub fn scroll_by(&mut self, delta: Pixels, cx: &mut Context<Self>) -> bool {
        let scrolled = self.scroll.scroll_by(delta);
        if scrolled {
            cx.notify();
        }
        scrolled
    }

    /// The editor's scroll offset. Hosts use it to persist or restore a
    /// position; tests use it to observe wheel chaining.
    pub fn scroll_offset(&self) -> Pixels {
        self.scroll.offset()
    }

    /// Grows the editor to its content instead of filling the host's box.
    ///
    /// Use it when the host owns the scroll viewport — a timeline of entries
    /// is one scroller — and reveal the caret with
    /// [`Self::caret_window_rect`] instead of relying on the editor's own
    /// scrollbar.
    pub fn fit_content(&mut self, cx: &mut Context<Self>) {
        if self.fit_content {
            return;
        }
        self.fit_content = true;
        cx.notify();
    }

    pub(crate) fn is_fit_content(&self) -> bool {
        self.fit_content
    }

    /// The caret's rectangle in window coordinates, from the last layout, or
    /// `None` before the first frame. Hosts that own the viewport scroll this
    /// rect into view while typing.
    pub fn caret_window_rect(&self) -> Option<Bounds<Pixels>> {
        let plan_index = self.plan.line_for_source(self.selection.head())?;
        let projected = self.plan.lines.get(plan_index)?;
        let bounds = self.layout.bounds()?;
        let top = self.layout.line_top(plan_index)?;
        let (x, y, height) = self.layout.with_line(plan_index, |line, height| {
            let display = projected.display_for_source(self.selection.head());
            line.position_for_index(display, height)
                .map(|position| (position.x, position.y, height))
        })??;
        let offset = self.scroll.offset().max(Pixels::ZERO);
        Some(Bounds {
            origin: point(bounds.origin.x + x, bounds.origin.y + top + y - offset),
            size: gpui::size(px(2.), height),
        })
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(window.line_height()).y;
        if self.scroll_by(delta, cx) {
            // The editor consumed this wheel event; don't also scroll the
            // timeline it sits in. At a scroll bound `scroll_by` is false and
            // the event bubbles, so the timeline takes over.
            cx.stop_propagation();
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A click inside the slash panel picks its row.
        if let Some((menu, rect)) = self.slash_panel(cx)
            && rect.contains(&event.position)
        {
            let row = ((event.position.y - rect.origin.y - px(4.)) / px(SLASH_ROW_HEIGHT)) as usize;
            if row < menu.items.len() {
                self.apply_slash_item(row, cx);
                return;
            }
        }
        tracing::debug!(
            focused = self.focus_handle.is_focused(window),
            "editor mouse down"
        );
        window.focus(&self.focus_handle, cx);
        self.restart_blink();
        self.history.break_run();
        self.dragging = true;
        self.drag_position = event.position;
        self.hover_link = None;
        let Some(hit) = self.hit_test(event.position, cx) else {
            return;
        };
        if event.click_count == 1
            && let Some((marker, WidgetKind::Task { checked })) = hit.widget
        {
            self.toggle_task(marker, checked, cx);
            return;
        }
        // A plain click on an audio or video widget asks the host to play it.
        if event.click_count == 1
            && !event.modifiers.platform
            && let Some((_, WidgetKind::Media { kind, path })) = &hit.widget
            && matches!(kind, MediaKind::Audio | MediaKind::Video)
        {
            let resolved = self.resolved_asset_path(path);
            cx.emit(EditorEvent::MediaToggle {
                destination: path.clone(),
                resolved,
                kind: *kind,
            });
            return;
        }
        if event.modifiers.platform && !event.modifiers.shift {
            if let Some((_, WidgetKind::Media { path, .. })) = &hit.widget {
                let path = self.resolved_asset_path(path);
                cx.open_url(&format!("file://{}", path.display()));
                return;
            }
            if let Some(url) = self.link_at(hit.offset, cx) {
                cx.open_url(&url);
                return;
            }
        }
        match event.click_count {
            2 => {
                let run = {
                    let buffer = self.buffer.read(cx);
                    movement::run_at(buffer, hit.offset)
                };
                self.selection = Selection::new(run.start, run.end);
            }
            3 => {
                let (start, end) = {
                    let buffer = self.buffer.read(cx);
                    (
                        movement::line_start(buffer, hit.offset),
                        movement::line_end(buffer, hit.offset),
                    )
                };
                self.selection = Selection::new(start, end);
            }
            _ => {
                if event.modifiers.shift {
                    self.selection.move_head(hit.offset);
                } else {
                    self.selection.collapse(hit.offset);
                }
            }
        }
        cx.notify();
    }

    /// Toggles a task checkbox in place: one undo step, caret untouched.
    fn toggle_task(&mut self, marker: Range<usize>, checked: bool, cx: &mut Context<Self>) {
        let text = if checked { "[ ]" } else { "[x]" };
        let selection_before = self.selection;
        let removed = self
            .buffer
            .update(cx, |buffer, _| buffer.replace(marker.clone(), text));
        self.record_edit(marker.clone(), text.len());
        self.history.record(
            Edit {
                range: marker,
                removed,
                inserted: text.to_string(),
                selection_before,
                selection_after: selection_before,
                kind: MergeKind::None,
            },
            MergeKind::None,
        );
        self.restart_blink();
        cx.emit(EditorEvent::Edited);
        cx.notify();
    }

    /// Entry point for window-level mouse moves: extends a drag while the
    /// button is held, otherwise tracks link hover for the tooltip.
    pub(crate) fn on_mouse_move(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.dragging {
            self.on_drag_move(position, cx);
        } else {
            self.update_hover(position, cx);
        }
    }

    fn update_hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let hit = self.hit_test(position, cx);
        let url = hit.as_ref().and_then(|hit| self.link_at(hit.offset, cx));
        let hover_task = hit.as_ref().and_then(|hit| match &hit.widget {
            Some((range, WidgetKind::Task { .. })) => Some(range.clone()),
            _ => None,
        });
        let task_changed = self.hover_task != hover_task;
        if task_changed {
            self.hover_task = hover_task;
        }
        let previous = self.hover_link.clone();
        let next = url.map(|url| (url, position));
        let link_changed = match (&previous, &next) {
            (None, None) => false,
            (Some((before, at)), Some((after, to))) => {
                before != after || (at.x - to.x).abs() > px(2.) || (at.y - to.y).abs() > px(2.)
            }
            _ => true,
        };
        if link_changed {
            self.hover_link = next;
        }
        if task_changed || link_changed {
            cx.notify();
        }
    }

    /// The URL of the link construct covering `offset`, if any.
    fn link_at(&self, offset: usize, cx: &App) -> Option<String> {
        let construct = self
            .index
            .constructs_in(offset..offset + 1)
            .find(|construct| construct.kind == ConstructKind::Link)?;
        let destination = construct.inner.get(1)?;
        let buffer = self.buffer.read(cx);
        Some(buffer.slice(destination.clone()))
    }

    pub(crate) fn hover_link(&self) -> Option<(String, Point<Pixels>)> {
        self.hover_link.clone()
    }

    /// The task marker under the pointer, for painting its hover state.
    pub(crate) fn hover_task(&self) -> Option<Range<usize>> {
        self.hover_task.clone()
    }

    pub(crate) fn on_drag_move(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.dragging {
            return;
        }
        self.drag_position = position;
        if let Some(offset) = self.offset_for_point(position, cx) {
            self.selection.move_head(offset);
        }
        if self.over_edge() {
            self.autoscroll_tick(cx);
            self.ensure_autoscroll(cx);
        }
        self.restart_blink();
        cx.notify();
    }

    pub(crate) fn end_drag(&mut self, _cx: &mut Context<Self>) {
        self.dragging = false;
    }

    fn over_edge(&self) -> bool {
        let Some(bounds) = self.layout.bounds() else {
            return false;
        };
        let margin = self.layout.body_line_height().unwrap_or(px(20.));
        autoscroll_step(
            self.drag_position.y,
            bounds.origin.y,
            bounds.origin.y + bounds.size.height,
            margin,
        )
        .is_some()
    }

    /// Scrolls one step toward the pointer while it is past the viewport edge
    /// during a drag; returns whether more scrolling is expected.
    fn autoscroll_tick(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.dragging {
            return false;
        }
        let Some(bounds) = self.layout.bounds() else {
            return false;
        };
        let margin = self.layout.body_line_height().unwrap_or(px(20.));
        let Some(step) = autoscroll_step(
            self.drag_position.y,
            bounds.origin.y,
            bounds.origin.y + bounds.size.height,
            margin,
        ) else {
            return false;
        };
        if !self.scroll.scroll_by(step) {
            return false;
        }
        if let Some(offset) = self.offset_for_point(self.drag_position, cx) {
            self.selection.move_head(offset);
        }
        self.restart_blink();
        cx.notify();
        true
    }

    /// Keeps one autoscroll task alive while the pointer rests past an edge:
    /// the task ticks every frame-ish interval and ends itself when there is
    /// no more to scroll or the drag is over.
    fn ensure_autoscroll(&mut self, cx: &mut Context<Self>) {
        if self.autoscroll_active {
            return;
        }
        self.autoscroll_active = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(AUTOSCROLL_INTERVAL).await;
                let running = this
                    .update(cx, |editor, cx| {
                        let running = editor.autoscroll_tick(cx);
                        if !running {
                            editor.autoscroll_active = false;
                        }
                        running
                    })
                    .unwrap_or(false);
                if !running {
                    break;
                }
            }
        })
        .detach();
    }

    /// The byte offset a window position hits, plus the widget under the
    /// pointer when the click landed on one.
    fn hit_test(&self, position: Point<Pixels>, cx: &mut Context<Self>) -> Option<PointHit> {
        let bounds = self.layout.bounds()?;
        let content_y = position.y - bounds.origin.y + self.scroll.offset();
        if content_y < Pixels::ZERO {
            return Some(PointHit {
                offset: 0,
                widget: None,
            });
        }
        let Some((line_ix, within_line)) = self.layout.line_at_y(content_y) else {
            return Some(PointHit {
                offset: self.buffer.read(cx).len_bytes(),
                widget: None,
            });
        };
        let projected = self.plan.lines.get(line_ix)?;
        let x = position.x - bounds.origin.x;
        let display = self.layout.with_line(line_ix, |line, height| {
            snap_index(line.closest_index_for_position(point(x, within_line), height))
        })?;
        let widget = projected.widget_at_display(display).or_else(|| {
            display
                .checked_sub(1)
                .and_then(|d| projected.widget_at_display(d))
        });
        Some(PointHit {
            // `source_for_display` returns a document-level byte offset.
            offset: projected.source_for_display(display),
            widget,
        })
    }

    /// Hit-tests a window position to a byte offset: the shaped line containing
    /// the y coordinate, then the grapheme boundary closest to x, mapped from
    /// display space back to source.
    fn offset_for_point(&self, position: Point<Pixels>, cx: &mut Context<Self>) -> Option<usize> {
        self.hit_test(position, cx).map(|hit| hit.offset)
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        tracing::debug!(key = %keystroke.key, "editor key down");
        if let Some(menu) = self.slash.as_ref() {
            let (selected, query_start, count) =
                (menu.selected, menu.query_start, menu.items.len());
            match keystroke.key.as_str() {
                "up" => {
                    if let Some(menu) = self.slash.as_mut() {
                        menu.selected = selected.saturating_sub(1);
                    }
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                "down" => {
                    if let Some(menu) = self.slash.as_mut() {
                        menu.selected = (selected + 1).min(count.saturating_sub(1));
                    }
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                "enter" => {
                    self.apply_slash_item(selected, cx);
                    cx.stop_propagation();
                    return;
                }
                "escape" => {
                    self.slash_dismissed = Some(query_start);
                    self.slash = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                _ => {}
            }
        }
        // Return and Tab carry control characters: macOS routes them through
        // key-command selectors that land here, not through text input, so we
        // insert them ourselves.
        if keystroke.key == "enter" {
            self.insert_text_at_selection("\n", cx);
            cx.stop_propagation();
            return;
        }
        if keystroke.key == "tab" {
            self.insert_text_at_selection("\t", cx);
            cx.stop_propagation();
            return;
        }
        if keystroke.modifiers.platform {
            match keystroke.key.as_str() {
                "c" => {
                    self.copy_selection(cx);
                    cx.stop_propagation();
                    return;
                }
                "x" if !keystroke.modifiers.shift => {
                    self.cut_selection(cx);
                    cx.stop_propagation();
                    return;
                }
                "v" => {
                    self.paste_clipboard(cx);
                    cx.stop_propagation();
                    return;
                }
                "a" => {
                    self.select_all(cx);
                    cx.stop_propagation();
                    return;
                }
                "z" => {
                    self.undo_redo(keystroke.modifiers.shift, cx);
                    cx.stop_propagation();
                    return;
                }
                "b" if !keystroke.modifiers.shift => {
                    self.toggle_inline_format(InlineFormat::Bold, cx);
                    cx.stop_propagation();
                    return;
                }
                "i" if !keystroke.modifiers.shift => {
                    self.toggle_inline_format(InlineFormat::Italic, cx);
                    cx.stop_propagation();
                    return;
                }
                "x" if keystroke.modifiers.shift => {
                    self.toggle_inline_format(InlineFormat::Strike, cx);
                    cx.stop_propagation();
                    return;
                }
                "h" if keystroke.modifiers.shift => {
                    self.toggle_inline_format(InlineFormat::Highlight, cx);
                    cx.stop_propagation();
                    return;
                }
                "`" => {
                    self.toggle_inline_format(InlineFormat::Code, cx);
                    cx.stop_propagation();
                    return;
                }
                "k" if !keystroke.modifiers.shift => {
                    self.insert_link("", cx);
                    cx.stop_propagation();
                    return;
                }
                "1" if !keystroke.modifiers.shift => {
                    self.toggle_block_format(BlockFormat::Heading(1), cx);
                    cx.stop_propagation();
                    return;
                }
                "2" if !keystroke.modifiers.shift => {
                    self.toggle_block_format(BlockFormat::Heading(2), cx);
                    cx.stop_propagation();
                    return;
                }
                "3" if !keystroke.modifiers.shift => {
                    self.toggle_block_format(BlockFormat::Heading(3), cx);
                    cx.stop_propagation();
                    return;
                }
                "7" if keystroke.modifiers.shift => {
                    self.toggle_block_format(BlockFormat::Ordered, cx);
                    cx.stop_propagation();
                    return;
                }
                "8" if keystroke.modifiers.shift => {
                    self.toggle_block_format(BlockFormat::Bullet, cx);
                    cx.stop_propagation();
                    return;
                }
                "9" if keystroke.modifiers.shift => {
                    self.toggle_block_format(BlockFormat::Task, cx);
                    cx.stop_propagation();
                    return;
                }
                _ => {}
            }
        }
        if matches!(keystroke.key.as_str(), "backspace" | "delete") {
            let backward = keystroke.key == "backspace";
            let alt = keystroke.modifiers.alt;
            let platform = keystroke.modifiers.platform;
            self.delete_input(backward, alt, platform, cx);
            cx.stop_propagation();
            return;
        }
        let extend = keystroke.modifiers.shift;
        let backward = matches!(keystroke.key.as_str(), "left" | "up" | "home");
        let target = {
            let buffer = self.buffer.read(cx);
            self.movement_target(
                buffer,
                &keystroke.key,
                keystroke.modifiers.alt,
                keystroke.modifiers.platform,
            )
        };
        let Some(target) = target else {
            return;
        };
        self.history.break_run();
        self.restart_blink();
        if extend {
            self.selection.move_head(target);
        } else if !self.selection.is_caret() {
            let collapse = if backward {
                self.selection.start()
            } else {
                self.selection.end()
            };
            self.selection.collapse(collapse);
        } else {
            self.selection.collapse(target);
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Applies one command edit as a single undo step and restores the
    /// selection the command computed.
    fn apply_command(&mut self, command: CommandEdit, cx: &mut Context<Self>) {
        let selection_before = self.selection;
        let removed = self.buffer.update(cx, |buffer, _| {
            buffer.replace(command.range.clone(), &command.new_text)
        });
        self.selection = command.new_selection;
        self.history.record(
            Edit {
                range: command.range,
                removed,
                inserted: command.new_text,
                selection_before,
                selection_after: self.selection,
                kind: MergeKind::None,
            },
            MergeKind::None,
        );
        self.restart_blink();
        cx.emit(EditorEvent::Edited);
        cx.notify();
    }

    /// Toggles an inline format around the selection: wraps, unwraps, or
    /// inserts a delimiter pair. Always one undo step.
    ///
    /// This is the toolbar/shortcut entry point.
    pub fn toggle_inline_format(&mut self, format: InlineFormat, cx: &mut Context<Self>) {
        let text = self.content(cx);
        if let Some(construct) = self.containing_construct(format)
            && construct.markers.len() >= 2
        {
            let open = construct.markers[0].clone();
            let close = construct.markers[construct.markers.len() - 1].clone();
            let command = commands::unwrap_delimiters(&text, self.selection, open, close);
            self.apply_command(command, cx);
            return;
        }
        if let Some(command) = commands::toggle_inline(&text, self.selection, format) {
            self.apply_command(command, cx);
        }
    }

    /// Whether any line the selection touches carries the block format:
    /// the block toolbar's active state.
    pub fn block_format_active(&self, format: BlockFormat, cx: &App) -> bool {
        let text = self.content(cx);
        commands::selection_block_formats(&text, self.selection)
            .into_iter()
            .any(|block| block == Some(format))
    }

    /// Toggles a block format across the selected lines. One undo step.
    pub fn toggle_block_format(&mut self, format: BlockFormat, cx: &mut Context<Self>) {
        let text = self.content(cx);
        if let Some(command) = commands::toggle_block(&text, self.selection, format) {
            self.apply_command(command, cx);
        }
    }

    /// Inserts a Markdown link around the selection (toolbar popover).
    pub fn insert_link(&mut self, url: &str, cx: &mut Context<Self>) {
        let text = self.content(cx);
        let command = commands::insert_link(&text, self.selection, url);
        self.apply_command(command, cx);
    }

    /// Inserts an image reference at the caret (toolbar popover).
    pub fn insert_image(&mut self, alt: &str, path: &str, cx: &mut Context<Self>) {
        let command = commands::insert_image(self.selection, alt, path);
        self.apply_command(command, cx);
    }

    /// Inserts a media reference on its own paragraph, replacing any
    /// selection (the toolbar's Media… flow). Attachments render as cards only
    /// as whole lines, so this adds the surrounding breaks.
    pub fn insert_media(&mut self, alt: &str, path: &str, cx: &mut Context<Self>) {
        let text = self.content(cx);
        let command = commands::insert_media(&text, self.selection, alt, path);
        self.apply_command(command, cx);
    }

    /// Wraps the selection in a fenced code block, or inserts an empty one.
    pub fn insert_code_fence(&mut self, cx: &mut Context<Self>) {
        let text = self.content(cx);
        if let Some(command) = commands::insert_code_fence(&text, self.selection) {
            self.apply_command(command, cx);
        }
    }

    /// Inserts a horizontal rule on its own line.
    pub fn insert_rule(&mut self, cx: &mut Context<Self>) {
        let text = self.content(cx);
        if let Some(command) = commands::insert_rule(&text, self.selection) {
            self.apply_command(command, cx);
        }
    }

    /// Whether the selection is inside a construct of the format, or wraps
    /// one: the toolbar's active state.
    pub fn inline_format_active(&self, format: InlineFormat) -> bool {
        let kind = inline_construct_kind(format);
        self.index.constructs().iter().any(|construct| {
            construct.kind == kind
                && (construct.range.start < self.selection.start()
                    && self.selection.end() < construct.range.end
                    || construct.range.start >= self.selection.start()
                        && construct.range.end <= self.selection.end())
        })
    }

    /// The parsed construct of the matching kind that strictly contains the
    /// selection, if any.
    fn containing_construct(&self, format: InlineFormat) -> Option<Construct> {
        let kind = inline_construct_kind(format);
        self.index
            .constructs()
            .iter()
            .find(|construct| {
                construct.kind == kind
                    && construct.range.start < self.selection.start()
                    && self.selection.end() < construct.range.end
            })
            .cloned()
    }

    /// Inserts literal text at the selection as one edit. Used for keys the
    /// platform routes through key-command selectors (Return, Tab).
    fn insert_text_at_selection(&mut self, text: &str, cx: &mut Context<Self>) {
        let range = self.selection.start()..self.selection.end();
        self.apply_input_edit(range, text, None, false, MergeKind::None, cx);
    }

    /// Backspace/delete: removes the selection, or one grapheme/word/line
    /// before (backspace) or after (delete) the caret.
    fn delete_input(&mut self, backward: bool, word: bool, line: bool, cx: &mut Context<Self>) {
        let range = {
            let buffer = self.buffer.read(cx);
            deletion_range(buffer, self.selection, backward, word, line)
        };
        if let Some(range) = range {
            let merge = if backward && !word && !line && self.selection.is_caret() {
                MergeKind::TypingDelete
            } else {
                MergeKind::None
            };
            self.restart_blink();
            self.apply_input_edit(range, "", None, false, merge, cx);
        }
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        if self.selection.is_caret() {
            return;
        }
        let text = {
            let buffer = self.buffer.read(cx);
            buffer.slice(self.selection.start()..self.selection.end())
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn cut_selection(&mut self, cx: &mut Context<Self>) {
        if self.selection.is_caret() {
            return;
        }
        self.copy_selection(cx);
        self.restart_blink();
        self.apply_input_edit(
            self.selection.start()..self.selection.end(),
            "",
            None,
            false,
            MergeKind::None,
            cx,
        );
    }

    fn paste_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        let range = self.resolve_input_range(None, cx);
        self.apply_input_edit(range, &text, None, false, MergeKind::None, cx);
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        let end = self.buffer.read(cx).len_bytes();
        self.selection = Selection::new(0, end);
        self.history.break_run();
        self.restart_blink();
        cx.notify();
    }

    fn undo_redo(&mut self, redo: bool, cx: &mut Context<Self>) {
        let applied = self.buffer.update(cx, |buffer, _| {
            if redo {
                self.history.redo(buffer)
            } else {
                self.history.undo(buffer)
            }
        });
        if let Some(applied) = applied {
            self.selection = applied.selection;
            self.record_edit(applied.range, applied.new_len);
            self.restart_blink();
            cx.emit(EditorEvent::Edited);
            cx.notify();
        }
    }

    /// The byte offset a navigation key should move the caret to: grapheme and
    /// word steps, line and document bounds, and vertical movement by shaped
    /// line geometry.
    fn movement_target(
        &self,
        buffer: &Buffer,
        key: &str,
        alt: bool,
        platform: bool,
    ) -> Option<usize> {
        let head = self.selection.head();
        let target = match key {
            "left" => {
                if platform {
                    movement::line_start(buffer, head)
                } else if alt {
                    movement::word_left(buffer, head)
                } else {
                    movement::left(buffer, head)
                }
            }
            "right" => {
                if platform {
                    movement::line_end(buffer, head)
                } else if alt {
                    movement::word_right(buffer, head)
                } else {
                    movement::right(buffer, head)
                }
            }
            "up" => {
                if platform {
                    0
                } else {
                    self.vertical_offset(buffer, -1)?
                }
            }
            "down" => {
                if platform {
                    buffer.len_bytes()
                } else {
                    self.vertical_offset(buffer, 1)?
                }
            }
            "home" => movement::line_start(buffer, head),
            "end" => movement::line_end(buffer, head),
            _ => return None,
        };
        Some(target)
    }

    fn vertical_offset(&self, buffer: &Buffer, direction: i32) -> Option<usize> {
        let (line_ix, local) = buffer.offset_to_line(self.selection.head());
        let projected = self.plan.lines.get(line_ix)?;
        let display = projected.display_for_source(projected.source.start + local);
        let (x, y, line_height) = self.layout.with_line(line_ix, |line, line_height| {
            line.position_for_index(display, line_height)
                .map(|position| (position.x, position.y, line_height))
        })??;

        let segments = |ix: usize| {
            self.layout
                .with_line(ix, |line, _| line.wrap_boundaries().len() + 1)
                .unwrap_or(1)
        };
        let target_y = y + line_height * direction as f32;
        if target_y >= Pixels::ZERO && target_y < line_height * segments(line_ix) as f32 {
            let index = self.layout.with_line(line_ix, |line, line_height| {
                snap_index(line.closest_index_for_position(point(x, target_y), line_height))
            })?;
            // `source_for_display` returns a document-level byte offset.
            return Some(projected.source_for_display(index));
        }

        if direction < 0 {
            if line_ix == 0 {
                return Some(0);
            }
            let ix = line_ix - 1;
            let target = self.plan.lines.get(ix)?;
            let last_y = line_height * (segments(ix) - 1) as f32;
            let index = self.layout.with_line(ix, |line, line_height| {
                snap_index(line.closest_index_for_position(point(x, last_y), line_height))
            })?;
            Some(target.source_for_display(index))
        } else {
            let ix = line_ix + 1;
            if ix >= buffer.len_lines() {
                return Some(buffer.len_bytes());
            }
            let target = self.plan.lines.get(ix)?;
            let index = self.layout.with_line(ix, |line, line_height| {
                snap_index(line.closest_index_for_position(point(x, Pixels::ZERO), line_height))
            })?;
            Some(target.source_for_display(index))
        }
    }
}

fn snap_index(index: Result<usize, usize>) -> usize {
    match index {
        Ok(index) | Err(index) => index,
    }
}

/// One autoscroll step toward a pointer past the top or bottom edge of the
/// viewport; `None` while the pointer is inside the margins.
fn autoscroll_step(
    pointer_y: Pixels,
    top: Pixels,
    bottom: Pixels,
    margin: Pixels,
) -> Option<Pixels> {
    if pointer_y < top + margin {
        Some(-margin)
    } else if pointer_y > bottom - margin {
        Some(margin)
    } else {
        None
    }
}

/// The construct kind that backs an inline format.
fn inline_construct_kind(format: InlineFormat) -> ConstructKind {
    match format {
        InlineFormat::Bold => ConstructKind::Strong,
        InlineFormat::Italic => ConstructKind::Emphasis,
        InlineFormat::Strike => ConstructKind::Strikethrough,
        InlineFormat::Highlight => ConstructKind::Highlight,
        InlineFormat::Code => ConstructKind::CodeSpan,
    }
}

/// A hit-test result: the source offset for the caret and, when the click
/// landed on a widget, the widget's source range and kind.
struct PointHit {
    offset: usize,
    widget: Option<(Range<usize>, WidgetKind)>,
}

/// Reads and decodes an image file into a renderer-ready image.
fn load_render_image(path: &std::path::Path) -> Result<Arc<RenderImage>, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let decoded = image::load_from_memory(&bytes).map_err(|error| error.to_string())?;
    // `RenderImage` holds BGRA; decoding to RGBA swaps red and blue on screen
    // (a red cover reads blue) while the reading view, which goes through
    // gpui's own loader, stays right. `image` has no BGRA buffer type, so the
    // swap is by hand.
    let mut rgba = decoded.to_rgba8();
    for pixel in rgba.as_mut().as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let frame = image::Frame::new(rgba);
    Ok(Arc::new(RenderImage::new([frame])))
}

/// Converts a UTF-16 offset inside `text` to a byte offset. An offset inside a
/// surrogate pair rounds down to the character's start.
fn utf16_to_byte_in(text: &str, utf16_offset: usize) -> usize {
    let mut utf16 = 0;
    for (byte, ch) in text.char_indices() {
        let next = utf16 + ch.len_utf16();
        if next > utf16_offset {
            return byte;
        }
        utf16 = next;
    }
    text.len()
}

/// The byte range backspace/delete should remove: the selection when one
/// exists, otherwise one grapheme, word or line on the requested side of the
/// caret. `None` when there is nothing to delete.
fn deletion_range(
    buffer: &Buffer,
    selection: Selection,
    backward: bool,
    word: bool,
    line: bool,
) -> Option<Range<usize>> {
    if !selection.is_caret() {
        return Some(selection.start()..selection.end());
    }
    let head = selection.head();
    let target = match (backward, line, word) {
        (true, true, _) => movement::line_start(buffer, head),
        (true, _, true) => movement::word_left(buffer, head),
        (true, _, _) => movement::left(buffer, head),
        (false, true, _) => movement::line_end(buffer, head),
        (false, _, true) => movement::word_right(buffer, head),
        (false, _, _) => movement::right(buffer, head),
    };
    let (start, end) = if backward {
        (target, head)
    } else {
        (head, target)
    };
    (start != end).then_some(start..end)
}

impl EntityInputHandler for MarkdownEditor {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let buffer = self.buffer.read(cx);
        let range = buffer.utf16_to_byte(range.start)..buffer.utf16_to_byte(range.end);
        Some(buffer.slice(range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let buffer = self.buffer.read(cx);
        let range = buffer.byte_to_utf16(self.selection.start())
            ..buffer.byte_to_utf16(self.selection.end());
        Some(UTF16Selection {
            range,
            reversed: self.selection.reversed(),
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        let buffer = self.buffer.read(cx);
        let marked = self.marked_range.as_ref()?;
        Some(buffer.byte_to_utf16(marked.start)..buffer.byte_to_utf16(marked.end))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.resolve_input_range(range, cx);
        let merge = if self.marked_range.is_some() {
            MergeKind::Composition
        } else if text.chars().count() == 1 && !text.contains('\n') {
            MergeKind::TypingInsert
        } else {
            MergeKind::None
        };
        self.apply_input_edit(range, text, None, false, merge, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.resolve_input_range(range, cx);
        let selection = new_selected_range.map(|range| {
            utf16_to_byte_in(new_text, range.start)..utf16_to_byte_in(new_text, range.end)
        });
        self.apply_input_edit(range, new_text, selection, true, MergeKind::Composition, cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (line_ix, local) = {
            let buffer = self.buffer.read(cx);
            buffer.offset_to_line(buffer.utf16_to_byte(range_utf16.start))
        };
        let projected = self.plan.lines.get(line_ix)?;
        let display = projected.display_for_source(projected.source.start + local);
        let position = self.layout.with_line(line_ix, |line, line_height| {
            line.position_for_index(display, line_height)
        })??;
        let top = self.layout.line_top(line_ix)?;
        let line_height = self.layout.with_line(line_ix, |_, height| height)?;
        let origin = point(
            element_bounds.origin.x + position.x,
            element_bounds.origin.y + top + position.y - self.scroll.offset(),
        );
        Some(Bounds {
            origin,
            size: size(px(2.), line_height),
        })
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let offset = self.offset_for_point(position, cx)?;
        Some(self.buffer.read(cx).byte_to_utf16(offset))
    }

    fn text_length_utf16(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Option<usize> {
        Some(self.buffer.read(cx).len_utf16())
    }
}

impl MarkdownEditor {
    /// Converts a platform-supplied UTF-16 range to bytes. `None` means the
    /// platform did not track a range: the marked range wins while composing,
    /// otherwise the current selection.
    fn resolve_input_range(
        &self,
        range: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) -> Range<usize> {
        let buffer = self.buffer.read(cx);
        match range {
            Some(range) => buffer.utf16_to_byte(range.start)..buffer.utf16_to_byte(range.end),
            None => match &self.marked_range {
                Some(marked) => marked.clone(),
                None => self.selection.start()..self.selection.end(),
            },
        }
    }

    /// Applies one input edit: replaces `range` with `text`, moves the caret to
    /// `caret_in_text` (a byte range inside `text`; `None` puts it at the end),
    /// tracks marked text while an IME composes, and records undo history.
    fn apply_input_edit(
        &mut self,
        range: Range<usize>,
        text: &str,
        caret_in_text: Option<Range<usize>>,
        mark_composition: bool,
        merge: MergeKind,
        cx: &mut Context<Self>,
    ) {
        let selection_before = self.selection;
        let caret = range.start + caret_in_text.map_or(text.len(), |range| range.end);
        tracing::debug!(chars = text.chars().count(), "editor input edit");
        let removed = self
            .buffer
            .update(cx, |buffer, _| buffer.replace(range.clone(), text));
        self.selection.collapse(caret);
        self.marked_range = mark_composition.then(|| range.start..range.start + text.len());
        self.record_edit(range.clone(), text.len());
        self.history.record(
            Edit {
                range,
                removed,
                inserted: text.to_string(),
                selection_before,
                selection_after: self.selection,
                kind: merge,
            },
            merge,
        );
        self.restart_blink();
        cx.emit(EditorEvent::Edited);
        cx.notify();
    }
}

impl Render for MarkdownEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_display(cx);
        let focused = self.focus_handle.is_focused(window);
        if focused != self.focused {
            tracing::info!(focused, "editor focus changed");
            self.focused = focused;
            self.restart_blink();
        }
        div()
            .size_full()
            .overflow_hidden()
            .text_color(self.theme.text)
            .track_focus(&self.focus_handle)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(EditorElement::new(
                cx.entity(),
                self.scroll.clone(),
                self.layout.clone(),
            ))
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::commands::SlashItem;
    use gpui::{TestAppContext, VisualTestContext};

    fn setup<'a>(
        cx: &'a mut TestAppContext,
        text: &str,
    ) -> (Entity<MarkdownEditor>, &'a mut VisualTestContext) {
        let text = text.to_string();
        cx.add_window_view(move |window, cx| {
            let buffer = cx.new(|_| Buffer::from_text(&text));
            let theme = EditorTheme::default();
            let editor = MarkdownEditor::new(buffer, theme, cx);
            window.focus(editor.focus_handle(), cx);
            editor
        })
    }

    #[gpui::test]
    async fn wrapped_lines_advance_by_every_row(cx: &mut TestAppContext) {
        // A line wide enough to wrap in the test window, then a follower.
        let long = "wrapping words ".repeat(40);
        let (editor, cx) = setup(cx, &format!("{long}\ntail\n"));
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_, app| {
            let editor = editor.read(app);
            let layout = &editor.layout;
            let rows = layout
                .with_line(0, |line, _| line.wrap_boundaries().len() + 1)
                .unwrap_or(1);
            assert!(rows > 1, "the first line wrapped ({rows} rows)");
            let base = layout.body_line_height().expect("a measured layout");
            let first = layout.line_top(0).expect("line 0");
            let second = layout.line_top(1).expect("line 1");
            assert_eq!(
                second - first,
                base * rows as f32,
                "the follower clears every wrapped row: layout and paint both \
                 advance by a line's full height, not its base height"
            );
        });
    }

    #[gpui::test]
    async fn typing_inserts_characters(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "");
        cx.simulate_input("hi");
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).content(app),
                "hi",
                "typing produced no text"
            );
        });
    }

    #[gpui::test]
    async fn arrow_keys_move_the_caret(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "hello\nworld\n");
        // Render one frame so the display plan and measured layout exist.
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.simulate_keystrokes("right right");
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).selection().head(),
                2,
                "right arrows moved nothing"
            );
        });
        cx.simulate_keystrokes("left");
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).selection().head(),
                1,
                "left arrow moved nothing"
            );
        });
        cx.simulate_keystrokes("down");
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).selection().head(),
                7,
                "down arrow moved nothing"
            );
        });
        cx.simulate_keystrokes("up");
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).selection().head(),
                1,
                "up arrow moved nothing"
            );
        });
    }

    #[gpui::test]
    async fn clicking_places_the_caret(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "hello\nworld\n");
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.simulate_click(point(px(2.), px(8.)), gpui::Modifiers::default());
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).selection().head(),
                0,
                "click on the first line landed elsewhere"
            );
        });
        cx.simulate_click(point(px(2.), px(30.)), gpui::Modifiers::default());
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).selection().head(),
                6,
                "click on the second line landed elsewhere"
            );
        });
    }

    #[gpui::test]
    async fn hovering_the_task_widget_tracks_its_marker(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "\n- [ ] todo\n");
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        // Measure where the checkbox stand-in sits: the fragment's display
        // start on the shaped line. The caret sits on the empty first line, so
        // the task line renders collapsed.
        let checkbox_x = cx.update(|_, app| {
            let editor = editor.read(app);
            let line = &editor.plan().lines[1];
            let display = line
                .fragments
                .iter()
                .find_map(|fragment| match &fragment.kind {
                    crate::display::FragmentKind::Widget(WidgetKind::Task { .. }) => {
                        Some(fragment.display.start)
                    }
                    _ => None,
                })
                .expect("task widget fragment");
            let x = editor
                .layout
                .with_line(1, |wrapped, height| {
                    wrapped
                        .position_for_index(display, height)
                        .map(|position| position.x)
                })
                .flatten()
                .expect("checkbox position");
            x + px(2.)
        });

        cx.simulate_mouse_move(point(checkbox_x, px(34.)), None, gpui::Modifiers::default());
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).hover_task(),
                Some(3..6),
                "the checkbox marker should be marked hovered"
            );
        });

        cx.simulate_mouse_move(point(px(220.), px(34.)), None, gpui::Modifiers::default());
        cx.update(|_, app| {
            assert!(
                editor.read(app).hover_task().is_none(),
                "hover should clear away from the checkbox"
            );
        });
    }

    #[gpui::test]
    async fn insert_media_renders_a_card(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "hello");
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.place_caret_at_end(cx);
                editor.insert_media("shot", "assets/a.png", cx);
            });
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_, app| {
            let editor = editor.read(app);
            assert_eq!(
                editor.content(app),
                "hello\n\n![shot](assets/a.png)\n",
                "the reference sits on its own paragraph"
            );
            let plan = editor.plan();
            let media_line = plan
                .lines
                .iter()
                .find(|line| {
                    line.fragments.iter().any(|fragment| {
                        matches!(
                            fragment.kind,
                            crate::display::FragmentKind::Widget(
                                crate::display::WidgetKind::Media { .. }
                            )
                        )
                    })
                })
                .expect("the inserted reference projects as a media widget");
            assert!(
                media_line.text.starts_with(' '),
                "the widget substitutes the collapsed markup"
            );
        });
    }

    #[gpui::test]
    async fn clicking_an_audio_widget_asks_the_host_to_play(cx: &mut TestAppContext) {
        use std::cell::RefCell;
        use std::rc::Rc;

        let (editor, cx) = setup(cx, "\n![](assets/take.mp3)\n");
        let toggles: Rc<RefCell<Vec<(String, MediaKind)>>> = Rc::default();
        let sink = toggles.clone();
        // The subscription must outlive the update call: dropping it
        // unsubscribes.
        let _subscription = cx.update(|_, cx| {
            cx.subscribe(&editor, move |_, event: &EditorEvent, _| {
                if let EditorEvent::MediaToggle {
                    destination, kind, ..
                } = event
                {
                    sink.borrow_mut().push((destination.clone(), *kind));
                }
            })
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let y = cx.update(|_, app| {
            let editor = editor.read(app);
            editor.layout.line_top(1).expect("second line top") + px(10.)
        });
        cx.simulate_click(point(px(80.), y), gpui::Modifiers::default());
        cx.update(|_, _| {
            assert_eq!(
                toggles.borrow().as_slice(),
                &[("assets/take.mp3".to_string(), MediaKind::Audio)],
                "a click on the audio row toggles playback"
            );
        });
    }

    #[test]
    fn decoded_images_arrive_in_the_renderers_channel_order() {
        // A pure red pixel must reach the renderer as BGRA, or every decoded
        // image paints with red and blue swapped — a red cover reads blue.
        let dir = std::env::temp_dir().join(format!("dainiki-bgra-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let path = dir.join("red.png");
        let rgba = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
        rgba.save(&path).expect("write png");

        let loaded = super::load_render_image(&path).expect("decode");
        let bytes = loaded.as_bytes(0).expect("frame bytes");
        assert_eq!(
            &bytes[..4],
            &[0, 0, 255, 255],
            "red arrives as BGRA (blue, green, red, alpha)"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    async fn a_video_frame_paints_without_panicking(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "\n![](assets/clip.mp4)\n");
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let frame = gpui::RenderImage::new([image::Frame::new(
            image::RgbaImage::from_raw(2, 2, vec![0x40; 16]).expect("2x2 image"),
        )]);
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_media_frame("assets/clip.mp4", std::sync::Arc::new(frame), cx)
            });
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    }

    #[gpui::test]
    async fn bold_shortcut_wraps_unwraps_and_undos_in_one_step(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "hello");
        cx.simulate_keystrokes("cmd-a cmd-b");
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "**hello**");
        });

        cx.simulate_keystrokes("cmd-z");
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).content(app),
                "hello",
                "command was not one undo step"
            );
        });
        cx.simulate_keystrokes("cmd-shift-z");
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "**hello**");
        });

        // The parsed construct intercepts a second toggle and unwraps.
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.simulate_keystrokes("cmd-b");
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "hello", "unwrapped");
        });
    }

    #[gpui::test]
    async fn heading_shortcut_toggles_the_line(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "title\nbody");
        cx.simulate_keystrokes("cmd-1");
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "# title\nbody");
        });
        cx.simulate_keystrokes("cmd-1");
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "title\nbody");
        });
    }

    #[gpui::test]
    async fn toolbar_api_toggles_and_reports_active(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "title");
        cx.simulate_keystrokes("cmd-a");
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.toggle_inline_format(InlineFormat::Bold, cx)
            });
        });
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "**title**");
            assert!(
                editor.read(app).inline_format_active(InlineFormat::Bold),
                "toolbar should show bold active"
            );
        });
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.toggle_block_format(BlockFormat::Bullet, cx)
            });
        });
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "- **title**");
            assert!(editor.read(app).inline_format_active(InlineFormat::Bold));
        });
    }

    #[gpui::test]
    async fn enter_and_tab_insert_text_as_their_own_undo_steps(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "a");
        cx.simulate_keystrokes("right enter b");
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "a\nb");
            assert_eq!(editor.read(app).selection().head(), 3);
        });
        cx.simulate_keystrokes("tab");
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "a\nb\t");
        });
        cx.simulate_keystrokes("cmd-z cmd-z cmd-z cmd-z");
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "a", "undo steps are wrong");
        });
    }

    #[gpui::test]
    async fn block_format_active_reflects_the_line(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "one");
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.toggle_block_format(BlockFormat::Bullet, cx)
            });
        });
        cx.update(|_, app| {
            assert!(
                editor
                    .read(app)
                    .block_format_active(BlockFormat::Bullet, app)
            );
            assert!(
                !editor
                    .read(app)
                    .block_format_active(BlockFormat::Heading(1), app)
            );
        });
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.toggle_block_format(BlockFormat::Heading(1), cx)
            });
        });
        cx.update(|_, app| {
            assert!(
                editor
                    .read(app)
                    .block_format_active(BlockFormat::Heading(1), app)
            );
            assert!(
                !editor
                    .read(app)
                    .block_format_active(BlockFormat::Bullet, app)
            );
        });
    }

    #[gpui::test]
    async fn link_shortcut_inserts_placeholder(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "hello");
        cx.simulate_keystrokes("cmd-a cmd-k");
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "[hello]()");
            assert_eq!(editor.read(app).selection().head(), 8);
        });
    }

    #[gpui::test]
    async fn link_and_image_api_inserts(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "hello");
        cx.simulate_keystrokes("cmd-a");
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| editor.insert_link("https://x.dev", cx));
        });
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "[hello](https://x.dev)");
        });
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                editor.insert_image("shot", "assets/a.png", cx)
            });
        });
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).content(app),
                "[hello](https://x.dev)![shot](assets/a.png)"
            );
        });
        cx.simulate_keystrokes("cmd-z");
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "[hello](https://x.dev)");
        });
    }

    #[gpui::test]
    async fn typing_flows_through_incremental_index_into_the_display(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "");
        cx.simulate_input("**bold**");
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "**bold**");
            let plan = editor.read(app).plan();
            assert_eq!(
                plan.lines[0].text, "bold",
                "typed markers should collapse in the display"
            );
        });
    }

    #[gpui::test]
    async fn crlf_documents_edit_cleanly(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "line one\r\nline two\r\n");
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.simulate_keystrokes("cmd-1");
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "# line one\r\nline two\r\n");
        });
        cx.simulate_keystrokes("cmd-z");
        cx.simulate_input("X");
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "Xline one\r\nline two\r\n");
        });
    }

    #[gpui::test]
    async fn empty_documents_accept_every_basic_operation(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "");
        cx.simulate_input("hi");
        cx.simulate_keystrokes("enter");
        cx.simulate_input("there");
        cx.simulate_keystrokes("left left backspace");
        cx.update(|_, app| {
            // "there" minus the 'e' before 'r'.
            assert_eq!(editor.read(app).content(app), "hi\nthre");
            assert_eq!(editor.read(app).selection().head(), 5);
        });
        cx.simulate_keystrokes("cmd-z cmd-z cmd-z cmd-z");
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "");
        });
    }

    #[gpui::test]
    async fn copy_yields_markdown_source_even_with_collapsed_markup(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "a **bold** and `code` tail");
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.simulate_keystrokes("cmd-a cmd-c");
        let text = cx.read_from_clipboard().and_then(|item| item.text());
        assert_eq!(text.as_deref(), Some("a **bold** and `code` tail"));
        let _ = editor;
    }

    #[gpui::test]
    async fn slash_menu_applies_formatting_by_keyboard(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "");
        cx.simulate_input("/hea");
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_, app| {
            let menu = editor.read(app).slash_menu().expect("menu open");
            assert_eq!(
                menu.items,
                vec![
                    SlashItem::Heading1,
                    SlashItem::Heading2,
                    SlashItem::Heading3
                ]
            );
        });
        cx.simulate_keystrokes("down enter");
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "## ");
            assert_eq!(editor.read(app).selection().head(), 3);
        });
        cx.simulate_keystrokes("cmd-z");
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).content(app),
                "/hea",
                "one undo restores the query"
            );
        });
    }

    #[gpui::test]
    async fn slash_menu_geometry_and_click(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "");
        cx.simulate_input("/h");
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let panel = cx.update(|_, app| {
            let (menu, rect) = editor.read(app).slash_panel(app).expect("panel laid out");
            assert_eq!(menu.items.len(), 3, "headers match the query");
            assert!(rect.size.width > px(100.));
            rect
        });
        // Clicking the second row ("Heading 2") applies it.
        let row = panel.origin.y + px(4.) + px(SLASH_ROW_HEIGHT * 1.5);
        cx.simulate_click(
            point(panel.origin.x + px(20.), row),
            gpui::Modifiers::default(),
        );
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "## ");
            assert_eq!(editor.read(app).selection().head(), 3);
        });
    }

    #[gpui::test]
    async fn slash_menu_filters_dismisses_and_respects_code(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "");
        cx.simulate_input("/quo");
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_, app| {
            assert_eq!(
                editor.read(app).slash_menu().expect("menu").items,
                vec![SlashItem::Quote]
            );
            assert!(editor.read(app).slash_menu().is_some());
        });

        cx.simulate_keystrokes("escape");
        cx.simulate_input("te");
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_, app| {
            assert!(
                editor.read(app).slash_menu().is_none(),
                "dismissal is sticky"
            );
        });

        let (editor, cx) = setup(cx, "`a /b`");
        cx.simulate_keystrokes("cmd-a right");
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_, app| {
            assert!(
                editor.read(app).slash_menu().is_none(),
                "no menu inside code spans"
            );
        });
    }

    /// The harness mounts a fixed-height toolbar above the editor inside a
    /// flex column. If the middle row cannot shrink (CSS-style `min-height:
    /// auto`), the editor measures itself to its content and every scroll is
    /// clamped away. This mounts that exact structure.
    #[gpui::test]
    async fn a_toolbar_above_the_editor_keeps_it_scrollable(cx: &mut TestAppContext) {
        struct Harness {
            editor: Entity<MarkdownEditor>,
        }
        impl Render for Harness {
            fn render(
                &mut self,
                _window: &mut Window,
                _cx: &mut Context<Self>,
            ) -> impl IntoElement {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(div().h(px(40.)).w_full().child("toolbar"))
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.))
                            .w_full()
                            .flex()
                            .child(div().w_full().h_full().child(self.editor.clone())),
                    )
            }
        }

        let text = "line\n".repeat(120);
        let buffer = cx.new(|_| Buffer::from_text(&text));
        let theme = EditorTheme::default();
        let editor = cx.new(|cx| MarkdownEditor::new(buffer, theme, cx));
        let editor_for_view = editor.clone();
        let (_harness, cx) = cx.add_window_view(move |window, cx| {
            let focus_handle = editor_for_view.read(cx).focus_handle().clone();
            window.focus(&focus_handle, cx);
            Harness {
                editor: editor_for_view,
            }
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_, app| {
            let max_offset = editor.read(app).scroll.max_offset();
            assert!(
                max_offset > px(0.),
                "the editor measured itself to its content and lost its scroll range"
            );
        });
    }

    #[gpui::test]
    async fn backspace_and_undo_work(cx: &mut TestAppContext) {
        let (editor, cx) = setup(cx, "abc");
        cx.simulate_keystrokes("right right right backspace");
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "ab", "backspace did nothing");
        });
        cx.simulate_keystrokes("cmd-z");
        cx.update(|_, app| {
            assert_eq!(editor.read(app).content(app), "abc", "undo did nothing");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{autoscroll_step, deletion_range, utf16_to_byte_in};
    use crate::buffer::Buffer;
    use crate::input::Selection;
    use gpui::px;

    #[test]
    fn utf16_offsets_inside_text_round_down() {
        assert_eq!(utf16_to_byte_in("a🎉", 0), 0);
        assert_eq!(utf16_to_byte_in("a🎉", 1), 1);
        assert_eq!(utf16_to_byte_in("a🎉", 2), 1);
        assert_eq!(utf16_to_byte_in("a🎉", 3), 5);
        assert_eq!(utf16_to_byte_in("a🎉", 4), 5);
        assert_eq!(utf16_to_byte_in("héllo", 1), 1);
        assert_eq!(utf16_to_byte_in("héllo", 2), 3);
    }

    fn caret(offset: usize) -> Selection {
        Selection::caret(offset)
    }

    #[test]
    fn backspace_removes_one_grapheme() {
        let buffer = Buffer::from_text("a🎉b");
        assert_eq!(
            deletion_range(&buffer, caret(5), true, false, false),
            Some(1..5)
        );
        assert_eq!(
            deletion_range(&buffer, caret(1), true, false, false),
            Some(0..1)
        );
        assert_eq!(deletion_range(&buffer, caret(0), true, false, false), None);
    }

    #[test]
    fn delete_removes_one_grapheme_forward() {
        let buffer = Buffer::from_text("a🎉b");
        assert_eq!(
            deletion_range(&buffer, caret(1), false, false, false),
            Some(1..5)
        );
        assert_eq!(
            deletion_range(&buffer, caret(5), false, false, false),
            Some(5..6)
        );
        assert_eq!(deletion_range(&buffer, caret(6), false, false, false), None);
    }

    #[test]
    fn word_deletion_skips_like_word_movement() {
        let buffer = Buffer::from_text("foo bar baz");
        assert_eq!(
            deletion_range(&buffer, caret(7), true, true, false),
            Some(4..7)
        );
        assert_eq!(
            deletion_range(&buffer, caret(4), true, true, false),
            Some(0..4)
        );
        assert_eq!(
            deletion_range(&buffer, caret(3), false, true, false),
            Some(3..7)
        );
    }

    #[test]
    fn line_deletion_uses_line_bounds() {
        let buffer = Buffer::from_text("first\nsecond");
        assert_eq!(
            deletion_range(&buffer, caret(10), true, false, true),
            Some(6..10)
        );
        assert_eq!(
            deletion_range(&buffer, caret(6), false, false, true),
            Some(6..12)
        );
        assert_eq!(deletion_range(&buffer, caret(0), true, false, true), None);
    }

    #[test]
    fn deletion_prefers_the_selection() {
        let buffer = Buffer::from_text("hello world");
        let selection = Selection::new(11, 6);
        assert_eq!(
            deletion_range(&buffer, selection, true, false, false),
            Some(6..11)
        );
        assert_eq!(
            deletion_range(&buffer, selection, false, true, true),
            Some(6..11)
        );
    }

    #[test]
    fn autoscroll_steps_toward_pointer_past_edges() {
        let (top, bottom, margin) = (px(100.), px(700.), px(20.));
        assert_eq!(autoscroll_step(px(400.), top, bottom, margin), None);
        assert_eq!(autoscroll_step(px(120.), top, bottom, margin), None);
        assert_eq!(autoscroll_step(px(680.), top, bottom, margin), None);
        assert_eq!(
            autoscroll_step(px(110.), top, bottom, margin),
            Some(px(-20.))
        );
        assert_eq!(
            autoscroll_step(px(95.), top, bottom, margin),
            Some(px(-20.))
        );
        assert_eq!(
            autoscroll_step(px(690.), top, bottom, margin),
            Some(px(20.))
        );
    }

    #[test]
    fn deletion_crosses_line_boundaries() {
        let buffer = Buffer::from_text("ab\ncd");
        assert_eq!(
            deletion_range(&buffer, caret(3), true, false, false),
            Some(2..3)
        );
        assert_eq!(
            deletion_range(&buffer, caret(2), false, false, false),
            Some(2..3)
        );
    }
}
