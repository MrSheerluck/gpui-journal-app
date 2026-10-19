//! Display projection: turns source text plus the construct index into
//! displayable lines — visible text, hidden markup, style runs — with an
//! exact byte mapping between source and display offsets.
//!
//! Pure: no GPUI, no I/O. The element shapes and paints what this module
//! produces; the view feeds it the text, the index and the selection.

use std::ops::Range;

use crate::input::Selection;
use crate::markdown::{Construct, ConstructKind, MarkdownIndex};

/// Line-level style. Inline styles arrive with their construct families.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    #[default]
    Body,
    Heading {
        level: u8,
    },
    /// A blockquote line: dimmed content behind a `▎` bar.
    Quote,
    /// A line inside a fenced code block; markers are the ``` lines.
    Code {
        marker: bool,
    },
}

/// Inline character-level styles carried as display runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineStyle {
    Emphasis,
    Strong,
    Strikethrough,
    Code,
    Highlight,
    Link,
}

/// A styled stretch of a projected line's display text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyleRun {
    pub display: Range<usize>,
    pub style: InlineStyle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FragmentKind {
    /// Text the user sees; maps linearly from source to display.
    Visible,
    /// Markup the live preview hides while collapsed; zero width in display.
    Hidden,
    /// A rendered stand-in for markup: bullet glyph, task checkbox, image.
    Widget(WidgetKind),
}

/// Widgets that replace source markup while collapsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WidgetKind {
    /// A `-`/`*`/`+` marker rendered as a painted dot.
    Bullet,
    /// A `>` marker rendered as a painted vertical bar.
    QuoteBar,
    /// A `[ ]`/`[x]` marker rendered as a painted checkbox and toggled by clicking.
    Task { checked: bool },
    /// A `---` rule rendered as a full-width painted line.
    Rule,
    /// A whole-line `![alt](path)` rendered as the media it points at.
    Media { kind: MediaKind, path: String },
}

/// What a media destination is, by its file extension. The classification is
/// pure so the projection decides rendering and the host decides playback
/// from the same answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Audio,
    Video,
    /// A whole-line reference the editor does not render inline: a file chip.
    File,
}

/// The file a media destination names: CommonMark wraps destinations that
/// contain spaces in angle brackets (`<assets/a b.png>`), and some writers
/// percent-encode them instead; both name the same file on disk.
pub fn destination_path(destination: &str) -> String {
    let inner = destination
        .trim()
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
        .unwrap_or(destination.trim());
    if inner.contains("%20") {
        inner.replace("%20", " ")
    } else {
        inner.to_string()
    }
}

/// Classifies a media destination by extension, case-insensitively and
/// ignoring URL query/fragment suffixes. Unknown extensions are `File`.
pub fn media_kind(destination: &str) -> MediaKind {
    let path = destination_path(destination);
    let path = path.split(['?', '#']).next().unwrap_or(&path);
    let extension = path
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "tif" | "heic" | "avif" => {
            MediaKind::Image
        }
        "mp3" | "m4a" | "aac" | "wav" | "aif" | "aiff" | "flac" | "ogg" | "oga" | "opus" => {
            MediaKind::Audio
        }
        "mp4" | "mov" | "m4v" | "webm" | "avi" | "mkv" => MediaKind::Video,
        _ => MediaKind::File,
    }
}

/// One piece of a projected line. Fragments tile both the source range and
/// the display text completely and in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fragment {
    /// Absolute source bytes.
    pub source: Range<usize>,
    /// Display bytes within [`ProjectedLine::text`].
    pub display: Range<usize>,
    pub kind: FragmentKind,
}

/// A buffer line as the preview shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectedLine {
    /// The buffer line's content range (no trailing newline).
    pub source: Range<usize>,
    /// Visible text to shape and paint.
    pub text: String,
    /// Source ↔ display mapping, in display order.
    pub fragments: Vec<Fragment>,
    /// Inline styles over the display text; may overlap when nested.
    pub runs: Vec<StyleRun>,
    pub style: Style,
}

impl ProjectedLine {
    /// The display byte for a source byte within this line. Offsets inside
    /// hidden markup or a widget collapse to its display boundary.
    pub fn display_for_source(&self, source: usize) -> usize {
        for fragment in &self.fragments {
            if source < fragment.source.start {
                return fragment.display.start;
            }
            if source <= fragment.source.end {
                return match fragment.kind {
                    FragmentKind::Hidden | FragmentKind::Widget(_) => fragment.display.start,
                    FragmentKind::Visible => {
                        fragment.display.start + (source - fragment.source.start)
                    }
                };
            }
        }
        self.text.len()
    }

    /// The source byte for a display byte within this line. Display positions
    /// inside collapsed markup or widgets resolve to the next visible
    /// boundary; positions past the end resolve to the line end.
    pub fn source_for_display(&self, display: usize) -> usize {
        for fragment in &self.fragments {
            if fragment.kind != FragmentKind::Visible {
                continue;
            }
            if display < fragment.display.start {
                return fragment.source.start;
            }
            if display <= fragment.display.end {
                let within = (display - fragment.display.start).min(fragment.source.len());
                return fragment.source.start + within;
            }
        }
        self.source.end
    }

    /// The widget under a display byte, if any: its source range and kind.
    pub fn widget_at_display(&self, display: usize) -> Option<(Range<usize>, WidgetKind)> {
        for fragment in &self.fragments {
            if let FragmentKind::Widget(kind) = &fragment.kind
                && fragment.display.start <= display
                && display < fragment.display.end
            {
                return Some((fragment.source.clone(), kind.clone()));
            }
        }
        None
    }
}

/// The whole document, one projected line per buffer line.
#[derive(Clone, Debug, Default)]
pub struct DisplayPlan {
    pub lines: Vec<ProjectedLine>,
}

impl DisplayPlan {
    /// Projects every buffer line. Structure (hidden markup, heading styles)
    /// follows the construct index and the current selection.
    pub fn compute(text: &str, index: &MarkdownIndex, selection: Selection) -> DisplayPlan {
        let mut lines = Vec::new();
        let mut start = 0;
        loop {
            let end = text[start..]
                .find('\n')
                .map(|offset| start + offset)
                .unwrap_or(text.len());
            let content_end = if text[start..end].ends_with('\r') {
                end - 1
            } else {
                end
            };
            lines.push(project_line(text, start..content_end, index, selection));
            if end == text.len() {
                break;
            }
            start = end + 1;
        }
        DisplayPlan { lines }
    }

    /// The projected line containing `source`.
    pub fn line_for_source(&self, source: usize) -> Option<usize> {
        self.lines
            .iter()
            .position(|line| line.source.start <= source && source <= line.source.end)
            .or_else(|| self.lines.len().checked_sub(1))
    }
}

/// The plan's reveal rule for inline constructs: a collapsed caret reveals
/// only when it is strictly inside the construct; a ranged selection reveals
/// when it intersects.
pub fn revealed(construct: &Construct, selection: Selection) -> bool {
    if selection.is_caret() {
        construct.range.start < selection.head() && selection.head() < construct.range.end
    } else {
        selection.start() < construct.range.end && construct.range.start < selection.end()
    }
}

/// Reveal rule for whole-line constructs (headings): the line shows its
/// source whenever the caret touches it at all.
fn line_revealed(range: &Range<usize>, selection: Selection) -> bool {
    if selection.is_caret() {
        range.start <= selection.head() && selection.head() <= range.end
    } else {
        selection.start() <= range.end && range.start <= selection.end()
    }
}

fn project_line(
    text: &str,
    source: Range<usize>,
    index: &MarkdownIndex,
    selection: Selection,
) -> ProjectedLine {
    let mut pieces: Vec<(Range<usize>, Piece)> = Vec::new();
    let mut pending_runs: Vec<(Range<usize>, InlineStyle)> = Vec::new();
    let mut line_style = Style::Body;

    for construct in index.constructs_in(source.clone()) {
        let reveal_range =
            construct.range.start.max(source.start)..construct.range.end.min(source.end);
        match construct.kind {
            ConstructKind::Heading { level } => {
                if line_revealed(&reveal_range, selection) {
                    continue;
                }
                line_style = Style::Heading { level };
                if let Some(content) = construct.inner.first() {
                    // Everything outside the content range is markup: the
                    // leading marker plus gap, and the trailing marker.
                    let content_start = content.start.min(source.end);
                    if source.start < content_start {
                        pieces.push((source.start..content_start, Piece::Hide));
                    }
                    let tail_start = content.end.min(source.end).max(source.start);
                    if tail_start < source.end {
                        pieces.push((tail_start..source.end, Piece::Hide));
                    }
                } else {
                    pieces.push((source.clone(), Piece::Hide));
                }
            }
            ConstructKind::ListMarker => {
                if !line_revealed(&source, selection) {
                    pieces.push((
                        reveal_range,
                        Piece::Glyph {
                            text: "• ".to_string(),
                            widget: WidgetKind::Bullet,
                        },
                    ));
                }
            }
            ConstructKind::TaskMarker { checked } => {
                if !line_revealed(&source, selection) {
                    pieces.push((
                        reveal_range,
                        Piece::Glyph {
                            text: if checked { "☑".into() } else { "☐".into() },
                            widget: WidgetKind::Task { checked },
                        },
                    ));
                }
            }
            ConstructKind::OrderedMarker => {
                // Ordered markers render as-is.
            }
            ConstructKind::QuoteMarker => {
                if !line_revealed(&source, selection) {
                    if line_style == Style::Body {
                        line_style = Style::Quote;
                    }
                    let marker = &text[reveal_range.clone()];
                    let barred = pieces.iter().any(|(_, piece)| {
                        matches!(
                            piece,
                            Piece::Glyph {
                                widget: WidgetKind::QuoteBar,
                                ..
                            }
                        )
                    });
                    // One bar per line: a nested level — a second `>` on the
                    // same line, whether it sits in its own marker node or in
                    // one run like `> > ` — keeps its width as indentation, so
                    // a nested quote reads as an indented quote instead of two
                    // bars side by side.
                    let bar_len = if barred {
                        0
                    } else {
                        marker[1..]
                            .find('>')
                            .map(|second| 1 + second)
                            .unwrap_or(marker.len())
                    };
                    let bar_end = reveal_range.start + bar_len;
                    if bar_len > 0 {
                        pieces.push((
                            reveal_range.start..bar_end,
                            Piece::Glyph {
                                text: "▎ ".to_string(),
                                widget: WidgetKind::QuoteBar,
                            },
                        ));
                    }
                    if bar_end < reveal_range.end {
                        pieces.push((
                            bar_end..reveal_range.end,
                            Piece::Text(" ".repeat(reveal_range.end - bar_end)),
                        ));
                    }
                }
            }
            ConstructKind::ThematicBreak => {
                if !line_revealed(&source, selection) {
                    pieces.push((
                        reveal_range,
                        Piece::Glyph {
                            text: String::new(),
                            widget: WidgetKind::Rule,
                        },
                    ));
                }
            }
            ConstructKind::Fence => {
                let is_marker_line = construct
                    .markers
                    .iter()
                    .any(|marker| marker.start < source.end && source.start < marker.end);
                if is_marker_line {
                    if !line_revealed(&source, selection) {
                        line_style = Style::Code { marker: true };
                    }
                } else if line_style == Style::Body {
                    line_style = Style::Code { marker: false };
                }
            }
            ConstructKind::Link => {
                if !revealed(construct, selection)
                    && let Some(text_range) = construct.inner.first()
                {
                    let prefix_start = source.start.max(construct.range.start);
                    if prefix_start < text_range.start {
                        pieces.push((prefix_start..text_range.start, Piece::Hide));
                    }
                    let suffix_end = source.end.min(construct.range.end);
                    if text_range.end < suffix_end {
                        pieces.push((text_range.end..suffix_end, Piece::Hide));
                    }
                }
                if let Some(text_range) = construct.inner.first() {
                    let start = text_range.start.max(source.start);
                    let end = text_range.end.min(source.end);
                    if start < end {
                        pending_runs.push((start..end, InlineStyle::Link));
                    }
                }
            }
            ConstructKind::Image => {
                if !revealed(construct, selection) {
                    let destination = construct.inner.get(1).map(|range| &text[range.clone()]);
                    let cleaned = destination.map(destination_path);
                    let file_name = cleaned
                        .as_deref()
                        .and_then(|path| path.rsplit('/').next())
                        .unwrap_or("image");
                    let whole_line = is_whole_line(&construct.range, &source, text);
                    if whole_line {
                        let path = destination.unwrap_or_default().to_string();
                        pieces.push((
                            reveal_range,
                            Piece::Glyph {
                                text: " ".to_string(),
                                widget: WidgetKind::Media {
                                    kind: media_kind(&path),
                                    path,
                                },
                            },
                        ));
                    } else {
                        let label = match media_kind(destination.unwrap_or_default()) {
                            MediaKind::Image => format!("[image: {file_name}]"),
                            MediaKind::Audio => format!("[audio: {file_name}]"),
                            MediaKind::Video => format!("[video: {file_name}]"),
                            MediaKind::File => format!("[file: {file_name}]"),
                        };
                        pieces.push((reveal_range, Piece::Text(label)));
                    }
                }
            }
            ConstructKind::Emphasis
            | ConstructKind::Strong
            | ConstructKind::Strikethrough
            | ConstructKind::CodeSpan
            | ConstructKind::Highlight => {
                if !revealed(construct, selection) {
                    for marker in &construct.markers {
                        let start = marker.start.max(source.start);
                        let end = marker.end.min(source.end);
                        if start < end {
                            pieces.push((start..end, Piece::Hide));
                        }
                    }
                }
                if construct.markers.len() >= 2 {
                    let first = &construct.markers[0];
                    let last = &construct.markers[construct.markers.len() - 1];
                    let start = first.end.max(source.start);
                    let end = last.start.min(source.end);
                    if start < end {
                        pending_runs.push((start..end, inline_style(&construct.kind)));
                    }
                }
            }
        }
    }

    let (visible_text, fragments) = build_fragments(text, &source, pieces);
    let runs = map_runs(&fragments, pending_runs);

    ProjectedLine {
        source,
        text: visible_text,
        fragments,
        runs,
        style: line_style,
    }
}

/// What a source piece becomes in the display: nothing (hidden), replacement
/// text, or a widget glyph standing in for markup (bullet, checkbox, image).
enum Piece {
    Hide,
    Text(String),
    Glyph { text: String, widget: WidgetKind },
}

/// True when `range` spans the whole line content except surrounding spaces.
fn is_whole_line(range: &Range<usize>, line: &Range<usize>, text: &str) -> bool {
    let before = &text[line.start..range.start];
    let after = &text[range.end..line.end];
    before.trim().is_empty() && after.trim().is_empty()
}

fn inline_style(kind: &ConstructKind) -> InlineStyle {
    match kind {
        ConstructKind::Emphasis => InlineStyle::Emphasis,
        ConstructKind::Strong => InlineStyle::Strong,
        ConstructKind::Strikethrough => InlineStyle::Strikethrough,
        ConstructKind::CodeSpan => InlineStyle::Code,
        ConstructKind::Highlight => InlineStyle::Highlight,
        _ => unreachable!("only inline constructs carry inline styles"),
    }
}

/// Splits a line into visible and hidden fragments around the hidden ranges
/// and widgets, in source order.
fn build_fragments(
    text: &str,
    source: &Range<usize>,
    pieces: Vec<(Range<usize>, Piece)>,
) -> (String, Vec<Fragment>) {
    let mut pieces: Vec<(Range<usize>, Piece)> = pieces
        .into_iter()
        .map(|(range, piece)| {
            (
                range.start.max(source.start)..range.end.min(source.end),
                piece,
            )
        })
        .filter(|(range, _)| range.start < range.end)
        .collect();
    pieces.sort_by_key(|(range, _)| range.start);
    let mut merged: Vec<(Range<usize>, Piece)> = Vec::new();
    for (range, piece) in pieces {
        if let Some((last, _)) = merged.last()
            && range.start < last.end
        {
            continue; // overlapping markup: keep the first piece
        }
        merged.push((range, piece));
    }

    let mut output = String::new();
    let mut fragments = Vec::new();
    let mut display = 0;
    let mut cursor = source.start;
    for (range, piece) in merged {
        if cursor < range.start {
            output.push_str(&text[cursor..range.start]);
            fragments.push(Fragment {
                source: cursor..range.start,
                display: display..display + (range.start - cursor),
                kind: FragmentKind::Visible,
            });
            display += range.start - cursor;
        }
        match piece {
            Piece::Hide => {
                fragments.push(Fragment {
                    source: range.clone(),
                    display: display..display,
                    kind: FragmentKind::Hidden,
                });
            }
            Piece::Text(replacement) => {
                output.push_str(&replacement);
                fragments.push(Fragment {
                    source: range.clone(),
                    display: display..display + replacement.len(),
                    kind: FragmentKind::Visible,
                });
                display += replacement.len();
            }
            Piece::Glyph {
                text: glyph,
                widget,
            } => {
                output.push_str(&glyph);
                fragments.push(Fragment {
                    source: range.clone(),
                    display: display..display + glyph.len(),
                    kind: FragmentKind::Widget(widget),
                });
                display += glyph.len();
            }
        }
        cursor = range.end;
    }
    if cursor < source.end {
        output.push_str(&text[cursor..source.end]);
        fragments.push(Fragment {
            source: cursor..source.end,
            display: display..display + (source.end - cursor),
            kind: FragmentKind::Visible,
        });
    }
    (output, fragments)
}

/// Maps source-space style runs through the fragments, splitting them when
/// hidden markup interrupts the visible text.
fn map_runs(fragments: &[Fragment], pending: Vec<(Range<usize>, InlineStyle)>) -> Vec<StyleRun> {
    let mut runs = Vec::new();
    for (range, style) in pending {
        for fragment in fragments {
            if fragment.kind != FragmentKind::Visible {
                continue;
            }
            let start = range.start.max(fragment.source.start);
            let end = range.end.min(fragment.source.end);
            if start >= end {
                continue;
            }
            runs.push(StyleRun {
                display: fragment.display.start + (start - fragment.source.start)
                    ..fragment.display.start + (end - fragment.source.start),
                style,
            });
        }
    }
    runs.sort_by_key(|run| (run.display.start, run.display.end));
    runs
}

#[cfg(test)]
mod tests {
    use super::{DisplayPlan, FragmentKind, InlineStyle, Style, WidgetKind, revealed};
    use crate::buffer::Buffer;
    use crate::input::Selection;
    use crate::markdown::MarkdownIndex;

    fn plan_for(text: &str, selection: Selection) -> DisplayPlan {
        let mut index = MarkdownIndex::new();
        index.update(text);
        DisplayPlan::compute(text, &index, selection)
    }

    #[test]
    fn line_ranges_match_the_buffer() {
        let text = "# One\n\nsecond\r\nthird\n";
        let plan = plan_for(text, Selection::default());
        let buffer = Buffer::from_text(text);
        assert_eq!(plan.lines.len(), buffer.len_lines());
        for (ix, line) in plan.lines.iter().enumerate() {
            assert_eq!(
                line.source,
                buffer.line_start_byte(ix)..buffer.line_end_byte(ix),
                "line {ix}"
            );
        }
    }

    #[test]
    fn headings_collapse_markers_when_the_caret_is_elsewhere() {
        let text = "# Title\nbody\n";
        let plan = plan_for(text, Selection::caret(9));
        let heading = &plan.lines[0];
        assert_eq!(heading.text, "Title");
        assert_eq!(heading.style, Style::Heading { level: 1 });
        assert_eq!(heading.fragments[0].kind, FragmentKind::Hidden);
        assert_eq!(heading.fragments[0].source, 0..2);

        let body = &plan.lines[1];
        assert_eq!(body.text, "body");
        assert_eq!(body.style, Style::Body);
    }

    #[test]
    fn headings_reveal_when_the_caret_touches_the_line() {
        let text = "# Title\nbody\n";
        for caret in [0, 1, 4, 7] {
            let plan = plan_for(text, Selection::caret(caret));
            let heading = &plan.lines[0];
            assert_eq!(heading.text, "# Title", "caret {caret}");
            assert_eq!(heading.style, Style::Body, "caret {caret}");
        }
        let plan = plan_for(text, Selection::caret(8));
        assert_eq!(plan.lines[0].text, "Title");
    }

    #[test]
    fn selections_reveal_touched_lines() {
        let text = "# Title\nbody\n";
        let plan = plan_for(text, Selection::new(0, 2));
        assert_eq!(plan.lines[0].text, "# Title");
        let plan = plan_for(text, Selection::new(8, 11));
        assert_eq!(plan.lines[0].text, "Title");
    }

    #[test]
    fn hidden_offsets_collapse_to_their_boundary() {
        let text = "# Title\n";
        let plan = plan_for(text, Selection::caret(20));
        let heading = &plan.lines[0];
        assert_eq!(heading.display_for_source(0), 0);
        assert_eq!(heading.display_for_source(1), 0);
        assert_eq!(heading.display_for_source(2), 0);
        assert_eq!(heading.display_for_source(3), 1);
        assert_eq!(heading.display_for_source(7), 5);
        assert_eq!(heading.source_for_display(0), 2);
        assert_eq!(heading.source_for_display(1), 3);
        assert_eq!(heading.source_for_display(5), 7);
        assert_eq!(heading.source_for_display(99), 7);
    }

    #[test]
    fn mapping_roundtrips_on_visible_text() {
        let text = "# French — café\n";
        let plan = plan_for(text, Selection::caret(20));
        let heading = &plan.lines[0];
        for source in 2..=text.trim_end().len() {
            let display = heading.display_for_source(source);
            assert_eq!(
                heading.source_for_display(display),
                source,
                "source {source}"
            );
        }
    }

    #[test]
    fn revealed_caret_rule_is_strict_for_inline() {
        let text = "a **b** c\n";
        let mut index = MarkdownIndex::new();
        index.update(text);
        let strong = index
            .constructs()
            .iter()
            .find(|construct| construct.range == (2..7))
            .expect("strong construct");
        assert!(!revealed(strong, Selection::caret(2)));
        assert!(revealed(strong, Selection::caret(3)));
        assert!(revealed(strong, Selection::caret(6)));
        assert!(!revealed(strong, Selection::caret(7)));
        assert!(revealed(strong, Selection::new(0, 3)));
    }

    #[test]
    fn empty_and_plain_lines_pass_through() {
        let text = "plain\n\n# H\nend";
        let plan = plan_for(text, Selection::caret(0));
        assert_eq!(plan.lines[0].text, "plain");
        assert_eq!(plan.lines[1].text, "");
        assert_eq!(plan.lines[3].text, "end");
        assert_eq!(plan.lines.len(), 4);
    }

    #[test]
    fn inline_markers_collapse_and_styles_run() {
        let text = "Plain *em* and **strong** and `code` and ~~gone~~ and ==hot==.\n";
        let plan = plan_for(text, Selection::caret(0));
        let line = &plan.lines[0];
        assert_eq!(line.text, "Plain em and strong and code and gone and hot.");
        let styles: Vec<(std::ops::Range<usize>, InlineStyle)> = line
            .runs
            .iter()
            .map(|run| (run.display.clone(), run.style))
            .collect();
        assert_eq!(
            styles,
            vec![
                (6..8, InlineStyle::Emphasis),
                (13..19, InlineStyle::Strong),
                (24..28, InlineStyle::Code),
                (33..37, InlineStyle::Strikethrough),
                (42..45, InlineStyle::Highlight),
            ]
        );
    }

    #[test]
    fn one_revealed_inline_keeps_others_collapsed() {
        let text = "**a** and **b**\n";
        let plan = plan_for(text, Selection::caret(12));
        let line = &plan.lines[0];
        assert_eq!(line.text, "a and **b**");
        let styles: Vec<(std::ops::Range<usize>, InlineStyle)> = line
            .runs
            .iter()
            .map(|run| (run.display.clone(), run.style))
            .collect();
        assert_eq!(
            styles,
            vec![(0..1, InlineStyle::Strong), (8..9, InlineStyle::Strong),]
        );
    }

    #[test]
    fn nested_emphasis_produces_overlapping_runs() {
        let text = "**bold *and em* tail**\n";
        let plan = plan_for(text, Selection::caret(0));
        let line = &plan.lines[0];
        assert_eq!(line.text, "bold and em tail");
        let styles: Vec<(std::ops::Range<usize>, InlineStyle)> = line
            .runs
            .iter()
            .map(|run| (run.display.clone(), run.style))
            .collect();
        assert_eq!(
            styles,
            vec![
                (0..5, InlineStyle::Strong),
                (5..11, InlineStyle::Strong),
                (5..11, InlineStyle::Emphasis),
                (11..16, InlineStyle::Strong),
            ]
        );
    }

    #[test]
    fn list_markers_collapse_to_bullets() {
        let text = "- item\n* another\n";
        let plan = plan_for(text, Selection::caret(17));
        assert_eq!(plan.lines[0].text, "• item");
        assert_eq!(plan.lines[1].text, "• another");
        let (range, widget) = plan.lines[0].widget_at_display(0).expect("bullet widget");
        assert_eq!(range, 0..2);
        assert_eq!(widget, WidgetKind::Bullet);

        // Caret on the line reveals the source markers.
        let plan = plan_for(text, Selection::caret(3));
        assert_eq!(plan.lines[0].text, "- item");
    }

    #[test]
    fn task_markers_collapse_to_checkboxes() {
        let text = "- [ ] todo\n- [x] done\n";
        let plan = plan_for(text, Selection::caret(23));
        assert_eq!(plan.lines[0].text, "• ☐ todo");
        assert_eq!(plan.lines[1].text, "• ☑ done");
        let (range, widget) = plan.lines[1].widget_at_display(4).expect("task widget");
        assert_eq!(range, 13..16);
        assert_eq!(widget, WidgetKind::Task { checked: true });

        let plan = plan_for(text, Selection::caret(2));
        assert_eq!(plan.lines[0].text, "- [ ] todo");
    }

    #[test]
    fn ordered_markers_render_as_source() {
        let text = "1. first\n2. second\n";
        let plan = plan_for(text, Selection::caret(20));
        assert_eq!(plan.lines[0].text, "1. first");
        assert_eq!(plan.lines[1].text, "2. second");
    }

    #[test]
    fn quotes_collapse_to_bars_and_dim() {
        let text = "> quoted line\n\nafter\n";
        let plan = plan_for(text, Selection::caret(16));
        assert_eq!(plan.lines[0].text, "▎ quoted line");
        assert_eq!(plan.lines[0].style, Style::Quote);
        let (range, widget) = plan.lines[0].widget_at_display(0).expect("bar widget");
        assert_eq!(range, 0..2);
        assert_eq!(widget, WidgetKind::QuoteBar);

        let plan = plan_for(text, Selection::caret(3));
        assert_eq!(plan.lines[0].text, "> quoted line");
        assert_eq!(plan.lines[0].style, Style::Body);
    }

    #[test]
    fn rules_collapse_to_empty_widgets() {
        let text = "before\n\n---\n\nafter\n";
        let plan = plan_for(text, Selection::caret(0));
        assert_eq!(plan.lines[2].text, "");
        assert!(
            plan.lines[2]
                .fragments
                .iter()
                .any(|fragment| { fragment.kind == FragmentKind::Widget(WidgetKind::Rule) })
        );

        let plan = plan_for(text, Selection::caret(9));
        assert_eq!(plan.lines[2].text, "---");
    }

    #[test]
    fn display_offsets_inside_widgets_map_to_boundaries() {
        let text = "- [ ] todo\n";
        let plan = plan_for(text, Selection::caret(20));
        let line = &plan.lines[0];
        // Display 0..4 is the bullet, 4..7 the checkbox; the first visible
        // text starts at display 7.
        assert_eq!(line.source_for_display(0), 5);
        assert_eq!(line.source_for_display(6), 5);
        assert_eq!(line.source_for_display(7), 5);
        assert_eq!(line.source_for_display(8), 6);
        assert_eq!(line.source_for_display(99), 10);
    }

    #[test]
    fn links_collapse_to_styled_text() {
        let text = "see [the docs](https://example.com) now\n";
        let plan = plan_for(text, Selection::caret(0));
        let line = &plan.lines[0];
        assert_eq!(line.text, "see the docs now");
        let styles: Vec<(std::ops::Range<usize>, InlineStyle)> = line
            .runs
            .iter()
            .map(|run| (run.display.clone(), run.style))
            .collect();
        assert_eq!(styles, vec![(4..12, InlineStyle::Link)]);

        let plan = plan_for(text, Selection::caret(8));
        assert_eq!(
            plan.lines[0].text,
            "see [the docs](https://example.com) now"
        );
    }

    #[test]
    fn whole_line_images_collapse_to_a_widget() {
        let text = "before\n\n![alt](assets/x.png)\n\nafter\n";
        let plan = plan_for(text, Selection::caret(0));
        let line = &plan.lines[2];
        assert_eq!(line.text, " ");
        let (_, widget) = line.widget_at_display(0).expect("image widget");
        assert_eq!(
            widget,
            WidgetKind::Media {
                kind: super::MediaKind::Image,
                path: "assets/x.png".into()
            }
        );

        let plan = plan_for(text, Selection::caret(10));
        assert_eq!(plan.lines[2].text, "![alt](assets/x.png)");
    }

    #[test]
    fn inline_images_fall_back_to_a_labeled_chip() {
        let text = "text ![a](p.png) more\n";
        let plan = plan_for(text, Selection::caret(0));
        assert_eq!(plan.lines[0].text, "text [image: p.png] more");
    }

    #[test]
    fn media_classification_follows_the_extension() {
        use super::{MediaKind, media_kind};
        assert_eq!(media_kind("assets/x.png"), MediaKind::Image);
        assert_eq!(media_kind("assets/X.JPEG"), MediaKind::Image);
        assert_eq!(media_kind("assets/x.png?v=2"), MediaKind::Image);
        assert_eq!(media_kind("assets/take.mp3"), MediaKind::Audio);
        assert_eq!(media_kind("assets/take.M4A"), MediaKind::Audio);
        assert_eq!(media_kind("assets/clip.mp4"), MediaKind::Video);
        assert_eq!(media_kind("assets/clip.MOV#t=3"), MediaKind::Video);
        assert_eq!(media_kind("assets/notes.txt"), MediaKind::File);
        assert_eq!(media_kind("assets/no_dot"), MediaKind::File);
        assert_eq!(media_kind("assets/v1.2/x"), MediaKind::File);
        // Spaced destinations: angle-bracketed and percent-encoded forms.
        assert_eq!(
            media_kind("<assets/Screen Shot 2026.png>"),
            MediaKind::Image
        );
        assert_eq!(
            media_kind("assets/Screen%20Recording.mov"),
            MediaKind::Video
        );
    }

    #[test]
    fn destination_path_unwraps_brackets_and_escapes() {
        use super::destination_path;
        assert_eq!(destination_path("assets/a.png"), "assets/a.png");
        assert_eq!(destination_path("<assets/a b.png>"), "assets/a b.png");
        assert_eq!(destination_path("assets/a%20b.png"), "assets/a b.png");
        assert_eq!(destination_path("  assets/a.png  "), "assets/a.png");
        assert_eq!(destination_path("<assets/a b.png>"), "assets/a b.png");
        assert_eq!(destination_path(""), "");
    }

    #[test]
    fn bracketed_whole_line_media_becomes_a_widget() {
        let text = "![Shot](<assets/a b.png>)\n";
        let plan = plan_for(text, Selection::caret(30));
        let line = &plan.lines[0];
        let (_, widget) = line.widget_at_display(0).expect("media widget");
        assert_eq!(
            widget,
            WidgetKind::Media {
                kind: super::MediaKind::Image,
                path: "<assets/a b.png>".into()
            }
        );
        // Inline fallback label uses the cleaned file name.
        let plan = plan_for("see ![s](<assets/a b.png>) here\n", Selection::caret(0));
        assert_eq!(plan.lines[0].text, "see [image: a b.png] here");
    }

    #[test]
    fn whole_line_media_carries_its_kind() {
        let text = "![](assets/take.mp3)\n";
        let plan = plan_for(text, Selection::caret(0));
        let (_, widget) = plan.lines[0].widget_at_display(0).expect("media widget");
        assert_eq!(
            widget,
            WidgetKind::Media {
                kind: super::MediaKind::Audio,
                path: "assets/take.mp3".into()
            }
        );

        let text = "![](assets/clip.mp4)\n";
        let plan = plan_for(text, Selection::caret(0));
        let (_, widget) = plan.lines[0].widget_at_display(0).expect("media widget");
        assert_eq!(
            widget,
            WidgetKind::Media {
                kind: super::MediaKind::Video,
                path: "assets/clip.mp4".into()
            }
        );
    }

    #[test]
    fn inline_media_chips_name_their_kind() {
        let plan = plan_for("a ![t](take.mp3) b\n", Selection::caret(0));
        assert_eq!(plan.lines[0].text, "a [audio: take.mp3] b");
        let plan = plan_for("a ![c](clip.mov) b\n", Selection::caret(0));
        assert_eq!(plan.lines[0].text, "a [video: clip.mov] b");
        let plan = plan_for("a ![n](notes.txt) b\n", Selection::caret(0));
        assert_eq!(plan.lines[0].text, "a [file: notes.txt] b");
    }

    #[test]
    fn atx_trailing_markers_hide_with_their_gap() {
        let text = "# Title #\n";
        let plan = plan_for(text, Selection::caret(10));
        assert_eq!(plan.lines[0].text, "Title");
    }

    #[test]
    fn setext_headings_collapse_the_underline_line() {
        let text = "Title\n=====\n\nbody\n";
        let plan = plan_for(text, Selection::caret(15));
        assert_eq!(plan.lines[0].text, "Title");
        assert_eq!(plan.lines[0].style, Style::Heading { level: 1 });
        assert_eq!(plan.lines[1].text, "");
        assert!(
            plan.lines[1]
                .fragments
                .iter()
                .all(|fragment| fragment.kind != FragmentKind::Visible)
        );

        let second = "Sub\n---\n\nbody\n";
        let plan = plan_for(second, Selection::caret(12));
        assert_eq!(plan.lines[0].style, Style::Heading { level: 2 });
    }

    #[test]
    fn escapes_and_unclosed_markers_stay_literal() {
        let text = "a \\*b\\* c and **unclosed and [link\n";
        let plan = plan_for(text, Selection::caret(0));
        assert_eq!(plan.lines[0].text, text.trim_end());
        assert!(plan.lines[0].runs.is_empty());
    }

    #[test]
    fn lazy_quote_continuation_does_not_crash() {
        let text = "> first\ncontinued\n";
        let plan = plan_for(text, Selection::caret(16));
        assert_eq!(plan.lines[0].style, Style::Quote);
        assert_eq!(plan.lines[1].style, Style::Body);
    }

    #[test]
    fn quote_continuations_collapse_their_markers() {
        // The grammar hands the first line a `block_quote_marker` and every
        // continuation line a `block_continuation`; both are the same `>` to
        // a reader, so both collapse to the bar.
        let text = "before\n\n> first\n>\n> third\n";
        let caret = text.len();
        let plan = plan_for(text, Selection::caret(caret));
        for ix in [2, 3, 4] {
            assert_eq!(
                plan.lines[ix].style,
                Style::Quote,
                "line {ix} reads as a quote"
            );
            assert!(
                !plan.lines[ix].text.contains('>'),
                "line {ix} keeps no marker: {:?}",
                plan.lines[ix].text
            );
        }

        // With the caret on that line the source comes back, as everywhere.
        let second_marker = text.find("\n>\n").expect("the empty quote line") + 2;
        let plan = plan_for(text, Selection::caret(second_marker));
        assert_eq!(plan.lines[3].text, ">");
    }

    #[test]
    fn nested_quote_levels_indent_instead_of_stacking_bars() {
        // The two lines differ in shape on purpose: the first has a marker
        // node per level, the second one run holding both (`> > `). Both
        // render the same way — one bar, the nested level as indentation.
        let text = "> > nested first\n> > nested second\n";
        let plan = plan_for(text, Selection::caret(text.len()));
        for line in &plan.lines[..2] {
            assert_eq!(line.style, Style::Quote);
            assert_eq!(
                line.fragments
                    .iter()
                    .filter(|fragment| matches!(
                        fragment.kind,
                        FragmentKind::Widget(WidgetKind::QuoteBar)
                    ))
                    .count(),
                1,
                "one bar per line: {:?}",
                line.text
            );
        }
        assert_eq!(plan.lines[0].text, "▎   nested first");
        assert_eq!(plan.lines[1].text, "▎   nested second");
    }

    #[test]
    fn fences_style_markers_and_content_lines() {
        let text = "before\n\n```rust\nfn main() {}\n```\n\nafter\n";
        let plan = plan_for(text, Selection::caret(0));
        assert_eq!(plan.lines[2].style, Style::Code { marker: true });
        assert_eq!(plan.lines[3].style, Style::Code { marker: false });
        assert_eq!(plan.lines[4].style, Style::Code { marker: true });
        assert_eq!(plan.lines[6].style, Style::Body);

        // Per-line reveal: the marker line under the caret shows source.
        let marker_line_start = text.find("```rust").unwrap();
        let plan = plan_for(text, Selection::caret(marker_line_start + 2));
        assert_eq!(plan.lines[2].style, Style::Body);
        assert_eq!(plan.lines[4].style, Style::Code { marker: true });
    }
}
