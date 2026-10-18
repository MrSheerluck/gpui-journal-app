use std::rc::Rc;

use gpui::{Context, Entity, Pixels, Render, ScrollWheelEvent, Window, div, prelude::*};

use crate::{
    buffer::Buffer,
    input::{EditorElement, EditorLayout, EditorScrollState},
    theme::EditorTheme,
};

pub struct MarkdownEditor {
    buffer: Entity<Buffer>,
    theme: EditorTheme,
    scroll: Rc<EditorScrollState>,
    layout: EditorLayout,
}

impl MarkdownEditor {
    pub fn new(buffer: Entity<Buffer>, theme: EditorTheme) -> Self {
        Self {
            buffer,
            theme,
            scroll: Rc::new(EditorScrollState::default()),
            layout: EditorLayout::default(),
        }
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
}

impl Render for MarkdownEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .overflow_hidden()
            .text_color(self.theme.text)
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(EditorElement::new(
                self.buffer.clone(),
                self.theme.clone(),
                self.scroll.clone(),
                self.layout.clone(),
            ))
    }
}
