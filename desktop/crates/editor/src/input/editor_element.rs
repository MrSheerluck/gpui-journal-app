use std::{cell::RefCell, rc::Rc};

use gpui::{
    App, AvailableSpace, Bounds, Element, ElementId, Entity, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, SharedString, Size, Window, WrappedLine,
};

use crate::buffer::Buffer;

pub struct EditorElement {
    buffer: Entity<Buffer>,
}

impl EditorElement {
    pub fn new(buffer: Entity<Buffer>) -> Self {
        Self { buffer }
    }
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = EditorLayout;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        _cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let line_height = window.pixel_snap(
            text_style
                .line_height
                .to_pixels(font_size.into(), window.rem_size()),
        );

        let state = EditorLayout::default();
        let measure_state = state.clone();
        let buffer = self.buffer.clone();
        let layout_id = window.request_measured_layout(
            Default::default(),
            move |known_dimensions, available_space, window, cx| {
                let wrap_width = known_dimensions.width.or(match available_space.width {
                    AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                });

                let buffer = buffer.read(cx);
                let mut lines = Vec::with_capacity(buffer.len_lines());
                let mut size: Size<Pixels> = Size::default();
                for ix in 0..buffer.len_lines() {
                    let text = buffer.line_text(ix);
                    let shaped = window.text_system().shape_text(
                        SharedString::new(text.as_str()),
                        font_size,
                        &[text_style.to_run(text.len())],
                        wrap_width,
                        None,
                    );
                    let line = match shaped {
                        Ok(mut shaped) => shaped.pop().unwrap_or_default(),
                        Err(error) => {
                            tracing::warn!(line = ix, error = %error, "failed to shape line");
                            WrappedLine::default()
                        }
                    };
                    let line_size = line.size(line_height);
                    size.height += line_size.height;
                    size.width = size.width.max(line_size.width).ceil();
                    lines.push(line);
                }

                *measure_state.0.borrow_mut() = Some(EditorLayoutInner {
                    lines,
                    line_height,
                    bounds: None,
                });

                size
            },
        );

        (layout_id, state)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let mut inner = request_layout.0.borrow_mut();
        let inner = inner
            .as_mut()
            .expect("measured layout must run before prepaint");
        inner.bounds = Some(bounds);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let inner = request_layout.0.borrow();
        let Some(inner) = inner.as_ref() else {
            return;
        };
        let Some(bounds) = inner.bounds else {
            return;
        };
        let align = window.text_style().text_align;

        let mut origin = bounds.origin;
        for line in &inner.lines {
            if let Err(error) =
                line.paint(origin, inner.line_height, align, Some(bounds), window, cx)
            {
                tracing::warn!(error = %error, "failed to paint a line");
            }
            origin.y += line.size(inner.line_height).height;
        }
    }
}

#[derive(Clone, Default)]
pub struct EditorLayout(Rc<RefCell<Option<EditorLayoutInner>>>);

struct EditorLayoutInner {
    lines: Vec<WrappedLine>,
    line_height: Pixels,
    bounds: Option<Bounds<Pixels>>,
}
