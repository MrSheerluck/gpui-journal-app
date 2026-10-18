use std::rc::Rc;

use gpui::{
    Context, Entity, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Render,
    ScrollWheelEvent, Window, div, point, prelude::*,
};

use crate::{
    buffer::Buffer,
    input::{EditorElement, EditorLayout, EditorScrollState, Selection, movement},
    theme::EditorTheme,
};

pub struct MarkdownEditor {
    buffer: Entity<Buffer>,
    theme: EditorTheme,
    scroll: Rc<EditorScrollState>,
    layout: EditorLayout,
    selection: Selection,
    focus_handle: FocusHandle,
}

impl MarkdownEditor {
    pub fn new(buffer: Entity<Buffer>, theme: EditorTheme, cx: &mut Context<Self>) -> Self {
        Self {
            buffer,
            theme,
            scroll: Rc::new(EditorScrollState::default()),
            layout: EditorLayout::default(),
            selection: Selection::default(),
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
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
        _event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
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
        cx.notify();
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
        let (x, y, line_height) = self.layout.with_line(line_ix, |line, line_height| {
            line.position_for_index(local, line_height)
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
            return Some(buffer.line_start_byte(line_ix) + index);
        }

        if direction < 0 {
            if line_ix == 0 {
                return Some(0);
            }
            let ix = line_ix - 1;
            let last_y = line_height * (segments(ix) - 1) as f32;
            let index = self.layout.with_line(ix, |line, line_height| {
                snap_index(line.closest_index_for_position(point(x, last_y), line_height))
            })?;
            Some(buffer.line_start_byte(ix) + index)
        } else {
            let ix = line_ix + 1;
            if ix >= buffer.len_lines() {
                return Some(buffer.len_bytes());
            }
            let index = self.layout.with_line(ix, |line, line_height| {
                snap_index(line.closest_index_for_position(point(x, Pixels::ZERO), line_height))
            })?;
            Some(buffer.line_start_byte(ix) + index)
        }
    }
}

fn snap_index(index: Result<usize, usize>) -> usize {
    match index {
        Ok(index) | Err(index) => index,
    }
}

impl Render for MarkdownEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let show_caret = self.focus_handle.is_focused(window);
        div()
            .size_full()
            .overflow_hidden()
            .text_color(self.theme.text)
            .track_focus(&self.focus_handle)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(EditorElement::new(
                self.buffer.clone(),
                self.theme.clone(),
                self.scroll.clone(),
                self.layout.clone(),
                self.selection,
                show_caret,
            ))
    }
}
