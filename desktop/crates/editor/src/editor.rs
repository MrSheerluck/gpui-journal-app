use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point, Render, ScrollWheelEvent,
    UTF16Selection, Window, div, point, prelude::*, px, size,
};

use crate::{
    buffer::Buffer,
    display::{DisplayPlan, WidgetKind},
    input::{
        EditorElement, EditorLayout, EditorScrollState, Selection,
        history::{Edit, History, MergeKind},
        movement,
    },
    markdown::{ConstructKind, MarkdownIndex},
    theme::EditorTheme,
};

const CARET_BLINK_INTERVAL: Duration = Duration::from_millis(530);
const AUTOSCROLL_INTERVAL: Duration = Duration::from_millis(16);

/// Events the host listens to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorEvent {
    /// The buffer text changed: typing, deletion, paste, or undo/redo.
    Edited,
}

impl EventEmitter<EditorEvent> for MarkdownEditor {}

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
    autoscroll_active: bool,
}

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
            autoscroll_active: false,
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

    /// The current display projection (cheap `Rc` clone).
    pub(crate) fn plan(&self) -> Rc<DisplayPlan> {
        self.plan.clone()
    }

    /// Re-projects the display when the text or selection changed since the
    /// last call. Called at the top of every render; a no-op otherwise.
    fn sync_display(&mut self, cx: &mut Context<Self>) {
        if self.revision == self.display_revision && self.selection == self.display_selection {
            return;
        }
        let text = self.content(cx);
        self.index.update(&text);
        self.plan = Rc::new(DisplayPlan::compute(&text, &self.index, self.selection));
        self.display_revision = self.revision;
        self.display_selection = self.selection;
    }

    pub fn scroll_by(&mut self, delta: Pixels, cx: &mut Context<Self>) {
        if self.scroll.scroll_by(delta) {
            cx.notify();
        }
    }

    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(window.line_height()).y;
        self.scroll_by(delta, cx);
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        if event.modifiers.platform
            && !event.modifiers.shift
            && let Some(url) = self.link_at(hit.offset, cx)
        {
            cx.open_url(&url);
            return;
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
        self.revision += 1;
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
        let url = self.link_position(position, cx);
        let previous = self.hover_link.clone();
        let next = url.map(|url| (url, position));
        let changed = match (&previous, &next) {
            (None, None) => false,
            (Some((before, at)), Some((after, to))) => {
                before != after || (at.x - to.x).abs() > px(2.) || (at.y - to.y).abs() > px(2.)
            }
            _ => true,
        };
        if changed {
            self.hover_link = next;
            cx.notify();
        }
    }

    fn link_position(&self, position: Point<Pixels>, cx: &mut Context<Self>) -> Option<String> {
        let hit = self.hit_test(position, cx)?;
        self.link_at(hit.offset, cx)
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
            offset: projected.source.start + projected.source_for_display(display),
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
        if keystroke.modifiers.platform {
            match keystroke.key.as_str() {
                "c" => {
                    self.copy_selection(cx);
                    cx.stop_propagation();
                    return;
                }
                "x" => {
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
        let selection = self.buffer.update(cx, |buffer, _| {
            if redo {
                self.history.redo(buffer)
            } else {
                self.history.undo(buffer)
            }
        });
        if let Some(selection) = selection {
            self.selection = selection;
            self.revision += 1;
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
            return Some(projected.source.start + projected.source_for_display(index));
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
            Some(target.source.start + target.source_for_display(index))
        } else {
            let ix = line_ix + 1;
            if ix >= buffer.len_lines() {
                return Some(buffer.len_bytes());
            }
            let target = self.plan.lines.get(ix)?;
            let index = self.layout.with_line(ix, |line, line_height| {
                snap_index(line.closest_index_for_position(point(x, Pixels::ZERO), line_height))
            })?;
            Some(target.source.start + target.source_for_display(index))
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

/// A hit-test result: the source offset for the caret and, when the click
/// landed on a widget, the widget's source range and kind.
struct PointHit {
    offset: usize,
    widget: Option<(Range<usize>, WidgetKind)>,
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
        let removed = self
            .buffer
            .update(cx, |buffer, _| buffer.replace(range.clone(), text));
        self.selection.collapse(caret);
        self.marked_range = mark_composition.then(|| range.start..range.start + text.len());
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
        self.revision += 1;
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
