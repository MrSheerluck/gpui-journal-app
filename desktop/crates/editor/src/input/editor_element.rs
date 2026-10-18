use std::ops::Range;
use std::{cell::RefCell, rc::Rc};

use gpui::{
    App, AvailableSpace, Bounds, DefiniteLength, DispatchPhase, Element, ElementId,
    ElementInputHandler, Entity, FontStyle, FontWeight, GlobalElementId, Hsla, InspectorElementId,
    IntoElement, LayoutId, Length, MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, Size,
    StrikethroughStyle, Style, TextAlign, TextRun, TextStyle, Window, WrappedLine, fill, point, px,
};

use crate::{
    display::{FragmentKind, InlineStyle, ProjectedLine, Style as LineStyle, WidgetKind},
    editor::MarkdownEditor,
    input::scroll::{EditorScrollState, clamp_offset},
    theme::EditorTheme,
};

const THUMB_WIDTH: f32 = 6.;
const THUMB_MIN_HEIGHT: f32 = 24.;
const THUMB_MARGIN: f32 = 4.;

pub struct EditorElement {
    view: Entity<MarkdownEditor>,
    scroll: Rc<EditorScrollState>,
    layout: EditorLayout,
}

impl EditorElement {
    pub fn new(
        view: Entity<MarkdownEditor>,
        scroll: Rc<EditorScrollState>,
        layout: EditorLayout,
    ) -> Self {
        Self {
            view,
            scroll,
            layout,
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
        let base_font_size = text_style.font_size.to_pixels(window.rem_size());
        let base_line_height = window.pixel_snap(
            text_style
                .line_height
                .to_pixels(base_font_size.into(), window.rem_size()),
        );

        let state = self.layout.clone();
        let measure_state = self.layout.clone();
        let view = self.view.clone();
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

                let (plan, theme) = {
                    let view = view.read(cx);
                    (view.plan(), view.theme().clone())
                };

                let mut lines = Vec::with_capacity(plan.lines.len());
                let mut content_size: Size<Pixels> = Size::default();
                for projected in &plan.lines {
                    let scale = match projected.style {
                        LineStyle::Body | LineStyle::Quote | LineStyle::Code { .. } => 1.0,
                        LineStyle::Heading { level } => theme.heading_scale(level),
                    };
                    let font_size = base_font_size * scale;
                    let line_height = base_line_height * scale;
                    let mut style = text_style.clone();
                    style.font_size = font_size.into();
                    match projected.style {
                        LineStyle::Quote => style.color = theme.text.opacity(0.65),
                        LineStyle::Code { marker } => {
                            style.font_family = theme.mono_font_family.clone();
                            if marker {
                                style.color = theme.text.opacity(0.35);
                            }
                        }
                        _ => {}
                    }
                    let runs = shape_runs(projected, &style, &theme);
                    let shaped = window.text_system().shape_text(
                        SharedString::new(projected.text.as_str()),
                        font_size,
                        &runs,
                        wrap_width,
                        None,
                    );
                    let line = match shaped {
                        Ok(mut shaped) => shaped.pop().unwrap_or_default(),
                        Err(error) => {
                            tracing::warn!(error = %error, "failed to shape line");
                            WrappedLine::default()
                        }
                    };
                    let line_size = line.size(line_height);
                    content_size.height += line_size.height;
                    content_size.width = content_size.width.max(line_size.width).ceil();
                    lines.push(LaidLine {
                        wrapped: line,
                        height: line_height,
                    });
                }

                *measure_state.0.borrow_mut() = Some(EditorLayoutInner {
                    lines,
                    body_line_height: base_line_height,
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

        let (theme, selection_state, caret_visible, focus_handle, marked_range, plan) = {
            let view = self.view.read(cx);
            (
                view.theme().clone(),
                view.selection(),
                view.caret_visible(),
                view.focus_handle().clone(),
                view.marked_range(),
                view.plan(),
            )
        };
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        let drag_view = self.view.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
            if phase == DispatchPhase::Bubble {
                drag_view.update(cx, |editor, cx| editor.on_mouse_move(event.position, cx));
            }
        });
        let up_view = self.view.clone();
        window.on_mouse_event(move |_event: &MouseUpEvent, phase, _window, cx| {
            if phase == DispatchPhase::Bubble {
                up_view.update(cx, |editor, cx| editor.end_drag(cx));
            }
        });

        let focused = focus_handle.is_focused(window);
        let caret_source = if focused && selection_state.is_caret() {
            Some(selection_state.head())
        } else {
            None
        };

        // Reveal the caret before painting: while focused with a collapsed
        // caret, keep it inside the viewport with one line of margin.
        if let Some(source) = caret_source
            && let Some(plan_index) = plan.line_for_source(source)
            && let Some(laid) = inner.lines.get(plan_index)
            && let Some(projected) = plan.lines.get(plan_index)
        {
            let mut caret_top = Pixels::ZERO;
            for line in &inner.lines[..plan_index] {
                caret_top += line.wrapped.size(line.height).height;
            }
            if let Some(segment) = laid
                .wrapped
                .position_for_index(projected.display_for_source(source), laid.height)
            {
                caret_top += segment.y;
            }
            let target = reveal_offset(
                caret_top,
                laid.height,
                bounds.size.height,
                self.scroll.offset(),
                laid.height,
            );
            self.scroll.set_offset(target);
        }

        let viewport_bottom = bounds.origin.y + bounds.size.height;
        let max_offset = (inner.content_height - bounds.size.height).max(Pixels::ZERO);
        let offset = clamp_offset(self.scroll.offset(), max_offset);
        let scroll_progress = if max_offset > Pixels::ZERO {
            offset.as_f32() / max_offset.as_f32()
        } else {
            0.0
        };

        let show_selection = !selection_state.is_caret();
        let selection = (selection_state.start(), selection_state.end());
        let show_caret = caret_visible && !show_selection && focused;

        window.paint_layer(bounds, |window| {
            let mut line_origin = point(bounds.origin.x, bounds.origin.y - offset);
            for (ix, laid) in inner.lines.iter().enumerate() {
                let height = laid.height;
                let line_bottom = line_origin.y + height;
                if line_bottom > bounds.origin.y && line_origin.y < viewport_bottom {
                    if let Some(projected) = plan.lines.get(ix) {
                        if matches!(projected.style, LineStyle::Code { marker: false }) {
                            let block = Bounds {
                                origin: line_origin,
                                size: Size {
                                    width: bounds.size.width,
                                    height,
                                },
                            };
                            window.paint_quad(fill(block, theme.code_block_background));
                        }
                        if show_selection {
                            paint_selection(
                                &laid.wrapped,
                                line_origin,
                                height,
                                projected,
                                selection,
                                theme.selection,
                                window,
                            );
                        }
                        if let Some(marked) = &marked_range {
                            let start = marked.start.max(projected.source.start);
                            let end = marked.end.min(projected.source.end);
                            if start < end {
                                paint_marked(
                                    &laid.wrapped,
                                    line_origin,
                                    height,
                                    projected.display_for_source(start)
                                        ..projected.display_for_source(end),
                                    theme.text,
                                    window,
                                );
                            }
                        }
                    }
                    if let Err(error) =
                        laid.wrapped
                            .paint(line_origin, height, align, Some(bounds), window, cx)
                    {
                        tracing::warn!(error = %error, "failed to paint a line");
                    }
                    if let Some(projected) = plan.lines.get(ix)
                        && projected
                            .fragments
                            .iter()
                            .any(|fragment| fragment.kind == FragmentKind::Widget(WidgetKind::Rule))
                    {
                        let rule = Bounds {
                            origin: point(bounds.origin.x, line_origin.y + height / 2.),
                            size: Size {
                                width: bounds.size.width,
                                height: px(1.),
                            },
                        };
                        window.paint_quad(fill(rule, theme.text.opacity(0.25)));
                    }
                    if show_caret
                        && let Some(source) = caret_source
                        && let Some(projected) = plan.lines.get(ix)
                        && projected.source.start <= source
                        && source <= projected.source.end
                    {
                        paint_caret(
                            &laid.wrapped,
                            line_origin,
                            height,
                            projected.display_for_source(source),
                            theme.caret,
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
            window.paint_quad(fill(thumb, theme.scrollbar).corner_radii(THUMB_WIDTH / 2.));
        }

        if let Some((url, position)) = self.view.read(cx).hover_link() {
            paint_tooltip(&url, position, bounds, &theme, window, cx);
        }
    }
}

/// Paints the link hover tooltip near the cursor, clamped to the element.
fn paint_tooltip(
    url: &str,
    position: Point<Pixels>,
    bounds: Bounds<Pixels>,
    theme: &EditorTheme,
    window: &mut Window,
    cx: &mut App,
) {
    let mut style = window.text_style().clone();
    style.font_size = px(12.).into();
    style.color = theme.tooltip_text;
    let Ok(mut shaped) = window.text_system().shape_text(
        SharedString::new(url),
        px(12.),
        &[style.to_run(url.len())],
        None,
        None,
    ) else {
        return;
    };
    let Some(line) = shaped.pop() else {
        return;
    };
    let line_height = window.pixel_snap(px(16.));
    let text_size = line.size(line_height);
    let padding = px(8.);
    let mut origin = point(position.x + px(14.), position.y + px(20.));
    origin.x = origin
        .x
        .min(bounds.origin.x + bounds.size.width - text_size.width - padding * 2.)
        .max(bounds.origin.x);
    origin.y = origin
        .y
        .min(bounds.origin.y + bounds.size.height - line_height - padding)
        .max(bounds.origin.y);
    let backdrop = Bounds {
        origin: point(origin.x - padding, origin.y - px(4.)),
        size: Size {
            width: text_size.width + padding * 2.,
            height: line_height + px(8.),
        },
    };
    window.paint_quad(fill(backdrop, theme.tooltip_background).corner_radii(6.));
    if let Err(error) = line.paint(
        point(origin.x, origin.y),
        line_height,
        TextAlign::Left,
        None,
        window,
        cx,
    ) {
        tracing::warn!(error = %error, "failed to paint tooltip");
    }
}

/// Paints the selection intersection of one line: highlight quads per wrapped
/// segment, plus a newline sliver when the selection continues past the line.
fn paint_selection(
    line: &WrappedLine,
    origin: Point<Pixels>,
    line_height: Pixels,
    projected: &crate::display::ProjectedLine,
    selection: (usize, usize),
    color: Hsla,
    window: &mut Window,
) {
    let source = projected.source.clone();
    if selection.0 > source.end {
        return;
    }
    let spans_past = selection.1 > source.end;
    let start = selection.0.max(source.start);
    let end = selection.1.min(source.end);
    if start < end || (spans_past && selection.0 <= source.end) {
        let display = projected.display_for_source(start)..projected.display_for_source(end);
        for quad in range_quads(line, origin, line_height, display) {
            window.paint_quad(fill(quad, color));
        }
    }
    if spans_past
        && selection.0 <= source.end
        && let Some(position) = line.position_for_index(projected.text.len(), line_height)
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

/// Paints the IME composition underline over a display range.
fn paint_marked(
    line: &WrappedLine,
    origin: Point<Pixels>,
    line_height: Pixels,
    display: Range<usize>,
    color: Hsla,
    window: &mut Window,
) {
    for quad in range_quads(line, origin, line_height, display) {
        let underline = Bounds {
            origin: point(quad.origin.x, quad.origin.y + line_height - px(2.)),
            size: Size {
                width: quad.size.width,
                height: px(2.),
            },
        };
        window.paint_quad(fill(underline, color));
    }
}

/// Paints the caret bar for one line at a display byte offset.
fn paint_caret(
    line: &WrappedLine,
    origin: Point<Pixels>,
    line_height: Pixels,
    display: usize,
    color: Hsla,
    window: &mut Window,
) {
    let Some(position) = line.position_for_index(display, line_height) else {
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

/// Highlight rectangles for a display byte range within one line, one per
/// wrapped segment the range covers.
fn range_quads(
    line: &WrappedLine,
    origin: Point<Pixels>,
    line_height: Pixels,
    display: Range<usize>,
) -> Vec<Bounds<Pixels>> {
    let content_len = line.len();
    let clip_start = display.start.min(content_len);
    let clip_end = display.end.min(content_len);

    let mut quads = Vec::new();
    if clip_start < clip_end {
        let starts = segment_starts(line);
        let unwrapped = &line.unwrapped_layout;
        for (segment, pair) in starts.windows(2).enumerate() {
            let start = clip_start.max(pair[0]);
            let end = clip_end.min(pair[1]);
            if start >= end {
                continue;
            }
            let base = unwrapped.x_for_index(pair[0]);
            let x0 = unwrapped.x_for_index(start) - base;
            let x1 = unwrapped.x_for_index(end) - base;
            quads.push(Bounds {
                origin: point(origin.x + x0, origin.y + line_height * segment as f32),
                size: Size {
                    width: x1 - x0,
                    height: line_height,
                },
            });
        }
    }
    quads
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

/// The scroll offset that keeps a caret of `caret_height` at content position
/// `caret_top` visible within the viewport, holding `margin` on both edges.
/// Returns `current` when the caret is already comfortably visible.
fn reveal_offset(
    caret_top: Pixels,
    caret_height: Pixels,
    viewport_height: Pixels,
    current: Pixels,
    margin: Pixels,
) -> Pixels {
    if caret_top - current < margin {
        caret_top - margin
    } else if caret_top + caret_height - current > viewport_height - margin {
        caret_top + caret_height + margin - viewport_height
    } else {
        current
    }
}

#[derive(Default, Clone, Copy)]
struct InlineFlags {
    emphasis: bool,
    strong: bool,
    strike: bool,
    code: bool,
    highlight: bool,
    link: bool,
}

/// Builds styled text runs for a projected line: fonts carry emphasis, strong
/// and code; decoration fields carry strike-through and background chips.
/// Overlapping runs (nesting) compose by flag union.
fn shape_runs(line: &ProjectedLine, base: &TextStyle, theme: &EditorTheme) -> Vec<TextRun> {
    if line.runs.is_empty() {
        return vec![base.to_run(line.text.len())];
    }
    let len = line.text.len();
    let mut points: Vec<usize> = vec![0, len];
    for run in &line.runs {
        points.push(run.display.start.min(len));
        points.push(run.display.end.min(len));
    }
    points.sort_unstable();
    points.dedup();

    let mut runs = Vec::with_capacity(points.len().saturating_sub(1));
    for pair in points.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if start >= end {
            continue;
        }
        let mut flags = InlineFlags::default();
        for run in &line.runs {
            if run.display.start <= start && start < run.display.end {
                match run.style {
                    InlineStyle::Emphasis => flags.emphasis = true,
                    InlineStyle::Strong => flags.strong = true,
                    InlineStyle::Strikethrough => flags.strike = true,
                    InlineStyle::Code => flags.code = true,
                    InlineStyle::Highlight => flags.highlight = true,
                    InlineStyle::Link => flags.link = true,
                }
            }
        }
        let mut style = base.clone();
        if flags.strong {
            style.font_weight = FontWeight::BOLD;
        }
        if flags.emphasis {
            style.font_style = FontStyle::Italic;
        }
        if flags.code {
            style.font_family = theme.mono_font_family.clone();
            style.background_color = Some(theme.code_background);
        } else if flags.highlight {
            style.background_color = Some(theme.highlight_background);
        }
        if flags.strike {
            style.strikethrough = Some(StrikethroughStyle {
                thickness: px(1.),
                color: None,
            });
        }
        if flags.link {
            style.color = theme.link;
        }
        runs.push(style.to_run(end - start));
    }
    runs
}

#[derive(Clone, Default)]
pub struct EditorLayout(Rc<RefCell<Option<EditorLayoutInner>>>);

impl EditorLayout {
    /// Runs `f` over the wrapped line at `ix` together with its height.
    pub(crate) fn with_line<R>(
        &self,
        ix: usize,
        f: impl FnOnce(&WrappedLine, Pixels) -> R,
    ) -> Option<R> {
        let inner = self.0.borrow();
        let inner = inner.as_ref()?;
        inner
            .lines
            .get(ix)
            .map(|laid| f(&laid.wrapped, laid.height))
    }

    /// The body line height (computed from the window text style).
    pub(crate) fn body_line_height(&self) -> Option<Pixels> {
        let inner = self.0.borrow();
        let inner = inner.as_ref()?;
        Some(inner.body_line_height)
    }

    pub(crate) fn bounds(&self) -> Option<Bounds<Pixels>> {
        let inner = self.0.borrow();
        let inner = inner.as_ref()?;
        inner.bounds
    }

    /// Content-space y of the top of the line at `ix`, before scroll offset.
    pub(crate) fn line_top(&self, ix: usize) -> Option<Pixels> {
        let inner = self.0.borrow();
        let inner = inner.as_ref()?;
        if ix >= inner.lines.len() {
            return None;
        }
        let mut top = Pixels::ZERO;
        for line in &inner.lines[..ix] {
            top += line.wrapped.size(line.height).height;
        }
        Some(top)
    }

    /// The line containing content-space `y`, with the y within that line.
    pub(crate) fn line_at_y(&self, y: Pixels) -> Option<(usize, Pixels)> {
        let inner = self.0.borrow();
        let inner = inner.as_ref()?;
        if y < Pixels::ZERO {
            return None;
        }
        let mut top = Pixels::ZERO;
        for (ix, line) in inner.lines.iter().enumerate() {
            let height = line.wrapped.size(line.height).height;
            if y < top + height {
                return Some((ix, y - top));
            }
            top += height;
        }
        None
    }
}

struct LaidLine {
    wrapped: WrappedLine,
    height: Pixels,
}

struct EditorLayoutInner {
    lines: Vec<LaidLine>,
    body_line_height: Pixels,
    content_height: Pixels,
    bounds: Option<Bounds<Pixels>>,
}

#[cfg(test)]
mod tests {
    use super::reveal_offset;
    use gpui::px;

    #[test]
    fn reveal_keeps_visible_carets_in_place() {
        assert_eq!(
            reveal_offset(px(300.), px(20.), px(600.), px(100.), px(20.)),
            px(100.)
        );
        assert_eq!(
            reveal_offset(px(120.), px(20.), px(600.), px(100.), px(20.)),
            px(100.)
        );
        assert_eq!(
            reveal_offset(px(560.), px(20.), px(600.), px(100.), px(20.)),
            px(100.)
        );
    }

    #[test]
    fn reveal_pulls_carets_past_the_margins() {
        assert_eq!(
            reveal_offset(px(110.), px(20.), px(600.), px(100.), px(20.)),
            px(90.)
        );
        assert_eq!(
            reveal_offset(px(700.), px(20.), px(600.), px(100.), px(20.)),
            px(140.)
        );
        assert_eq!(
            reveal_offset(px(10.), px(20.), px(600.), px(0.), px(20.)),
            px(-10.)
        );
    }
}
