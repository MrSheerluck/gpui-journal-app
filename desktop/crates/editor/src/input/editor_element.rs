use std::{cell::RefCell, rc::Rc};

use gpui::{
    App, AvailableSpace, Bounds, DefiniteLength, Element, ElementId, Entity, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, Length, Pixels, Point, SharedString, Size, Style,
    Window, WrappedLine, fill, point, px,
};

use crate::{
    buffer::Buffer,
    input::{
        Selection,
        scroll::{EditorScrollState, clamp_offset},
    },
    theme::EditorTheme,
};

const THUMB_WIDTH: f32 = 6.;
const THUMB_MIN_HEIGHT: f32 = 24.;
const THUMB_MARGIN: f32 = 4.;

pub struct EditorElement {
    buffer: Entity<Buffer>,
    theme: EditorTheme,
    scroll: Rc<EditorScrollState>,
    layout: EditorLayout,
    selection: Selection,
    show_caret: bool,
}

impl EditorElement {
    pub fn new(
        buffer: Entity<Buffer>,
        theme: EditorTheme,
        scroll: Rc<EditorScrollState>,
        layout: EditorLayout,
        selection: Selection,
        show_caret: bool,
    ) -> Self {
        Self {
            buffer,
            theme,
            scroll,
            layout,
            selection,
            show_caret,
        }
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

        let state = self.layout.clone();
        let measure_state = self.layout.clone();
        let buffer = self.buffer.clone();
        let layout_id = window.request_measured_layout(
            Style {
                size: Size {
                    width: Length::Auto,
                    height: Length::Definite(DefiniteLength::Fraction(1.0)),
                },
                ..Default::default()
            },
            move |known_dimensions, available_space, window, cx| {
                let wrap_width = known_dimensions.width.or(match available_space.width {
                    AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                });

                let buffer = buffer.read(cx);
                let mut lines = Vec::with_capacity(buffer.len_lines());
                let mut content_size: Size<Pixels> = Size::default();
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
                    content_size.height += line_size.height;
                    content_size.width = content_size.width.max(line_size.width).ceil();
                    lines.push(line);
                }

                *measure_state.0.borrow_mut() = Some(EditorLayoutInner {
                    lines,
                    line_height,
                    content_height: content_size.height,
                    bounds: None,
                });

                let fill = Size {
                    width: wrap_width,
                    height: known_dimensions.height.or(match available_space.height {
                        AvailableSpace::Definite(height) => Some(height),
                        _ => None,
                    }),
                };
                Size {
                    width: fill.width.unwrap_or(content_size.width),
                    height: fill.height.unwrap_or(content_size.height),
                }
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
        self.scroll
            .record_layout(inner.content_height, bounds.size.height);
        self.scroll.clamp_to_layout();
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

        let viewport_bottom = bounds.origin.y + bounds.size.height;
        let max_offset = (inner.content_height - bounds.size.height).max(Pixels::ZERO);
        let offset = clamp_offset(self.scroll.offset(), max_offset);
        let scroll_progress = if max_offset > Pixels::ZERO {
            offset.as_f32() / max_offset.as_f32()
        } else {
            0.0
        };

        let show_selection = !self.selection.is_caret();
        let selection = (self.selection.start(), self.selection.end());
        let show_caret = self.show_caret && !show_selection;
        let caret = if show_caret {
            let buffer = self.buffer.read(cx);
            Some(buffer.offset_to_line(self.selection.head()))
        } else {
            None
        };
        let line_ranges: Vec<(usize, usize)> = if show_selection {
            let buffer = self.buffer.read(cx);
            (0..buffer.len_lines())
                .map(|ix| (buffer.line_start_byte(ix), buffer.line_end_byte(ix)))
                .collect()
        } else {
            Vec::new()
        };

        window.paint_layer(bounds, |window| {
            let mut line_origin = point(bounds.origin.x, bounds.origin.y - offset);
            for (ix, line) in inner.lines.iter().enumerate() {
                let height = line.size(inner.line_height).height;
                let line_bottom = line_origin.y + height;
                if line_bottom > bounds.origin.y && line_origin.y < viewport_bottom {
                    if let Some(&line_range) = line_ranges.get(ix) {
                        paint_selection(
                            line,
                            line_origin,
                            inner.line_height,
                            line_range,
                            selection,
                            self.theme.selection,
                            window,
                        );
                    }
                    if let Err(error) = line.paint(
                        line_origin,
                        inner.line_height,
                        align,
                        Some(bounds),
                        window,
                        cx,
                    ) {
                        tracing::warn!(error = %error, "failed to paint a line");
                    }
                    if let Some((caret_line, caret_local)) = caret
                        && ix == caret_line
                    {
                        paint_caret(
                            line,
                            line_origin,
                            inner.line_height,
                            caret_local,
                            self.theme.caret,
                            window,
                        );
                    }
                }
                line_origin.y += height;
            }
        });

        if max_offset > Pixels::ZERO {
            let viewport_height = bounds.size.height.as_f32();
            let content_height = inner.content_height.as_f32();
            let thumb_height = (viewport_height * (viewport_height / content_height))
                .max(THUMB_MIN_HEIGHT)
                .min(viewport_height);
            let travel = viewport_height - thumb_height;
            let thumb_y = bounds.origin.y + px(travel * scroll_progress);
            let thumb = Bounds {
                origin: point(
                    bounds.origin.x + bounds.size.width - px(THUMB_MARGIN + THUMB_WIDTH),
                    thumb_y,
                ),
                size: Size {
                    width: px(THUMB_WIDTH),
                    height: px(thumb_height),
                },
            };
            window.paint_quad(fill(thumb, self.theme.scrollbar).corner_radii(THUMB_WIDTH / 2.));
        }
    }
}

/// Paints the selection intersection of one line: highlight quads per wrapped
/// segment, plus a newline sliver when the selection continues past the line.
fn paint_selection(
    line: &WrappedLine,
    origin: Point<Pixels>,
    line_height: Pixels,
    line_range: (usize, usize),
    selection: (usize, usize),
    color: Hsla,
    window: &mut Window,
) {
    let (line_start, line_end) = line_range;
    if selection.0 > line_end {
        return;
    }
    let content_len = line_end - line_start;
    let clip_start = selection.0.saturating_sub(line_start).min(content_len);
    let clip_end = selection.1.saturating_sub(line_start).min(content_len);

    let starts = segment_starts(line);
    let unwrapped = &line.unwrapped_layout;
    if clip_start < clip_end {
        for (segment, pair) in starts.windows(2).enumerate() {
            let start = clip_start.max(pair[0]);
            let end = clip_end.min(pair[1]);
            if start >= end {
                continue;
            }
            let base = unwrapped.x_for_index(pair[0]);
            let x0 = unwrapped.x_for_index(start) - base;
            let x1 = unwrapped.x_for_index(end) - base;
            let quad = Bounds {
                origin: point(origin.x + x0, origin.y + line_height * segment as f32),
                size: Size {
                    width: x1 - x0,
                    height: line_height,
                },
            };
            window.paint_quad(fill(quad, color));
        }
    }

    if selection.1 > line_end
        && let Some(position) = line.position_for_index(content_len, line_height)
    {
        let sliver = Bounds {
            origin: point(origin.x + position.x, origin.y + position.y),
            size: Size {
                width: px(6.),
                height: line_height,
            },
        };
        window.paint_quad(fill(sliver, color));
    }
}

/// Paints the caret bar for one line at a byte offset within its text.
fn paint_caret(
    line: &WrappedLine,
    origin: Point<Pixels>,
    line_height: Pixels,
    local: usize,
    color: Hsla,
    window: &mut Window,
) {
    let Some(position) = line.position_for_index(local, line_height) else {
        return;
    };
    let caret = Bounds {
        origin: point(origin.x + position.x, origin.y + position.y),
        size: Size {
            width: px(2.),
            height: line_height,
        },
    };
    window.paint_quad(fill(caret, color));
}

/// Byte offsets where each wrapped segment of the line starts, terminated by
/// the line's total length.
fn segment_starts(line: &WrappedLine) -> Vec<usize> {
    let mut starts = Vec::with_capacity(line.wrap_boundaries().len() + 2);
    starts.push(0);
    for boundary in line.wrap_boundaries() {
        if let Some(glyph) = line.runs()[boundary.run_ix].glyphs.get(boundary.glyph_ix) {
            starts.push(glyph.index);
        }
    }
    starts.push(line.len());
    starts
}

#[derive(Clone, Default)]
pub struct EditorLayout(Rc<RefCell<Option<EditorLayoutInner>>>);

struct EditorLayoutInner {
    lines: Vec<WrappedLine>,
    line_height: Pixels,
    content_height: Pixels,
    bounds: Option<Bounds<Pixels>>,
}
