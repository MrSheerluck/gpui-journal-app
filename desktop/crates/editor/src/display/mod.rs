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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FragmentKind {
    /// Text the user sees; maps linearly from source to display.
    Visible,
    /// Markup the live preview hides while collapsed; zero width in display.
    Hidden,
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
    pub style: Style,
}

impl ProjectedLine {
    /// The display byte for a source byte within this line. Offsets inside
    /// hidden markup collapse to the hidden fragment's display boundary.
    pub fn display_for_source(&self, source: usize) -> usize {
        for fragment in &self.fragments {
            if source < fragment.source.start {
                return fragment.display.start;
            }
            if source <= fragment.source.end {
                return match fragment.kind {
                    FragmentKind::Hidden => fragment.display.start,
                    FragmentKind::Visible => {
                        fragment.display.start + (source - fragment.source.start)
                    }
                };
            }
        }
        self.text.len()
    }

    /// The source byte for a display byte within this line. Display positions
    /// inside collapsed text resolve to the nearest visible boundary.
    pub fn source_for_display(&self, display: usize) -> usize {
        for fragment in &self.fragments {
            if fragment.kind == FragmentKind::Hidden {
                continue;
            }
            if display <= fragment.display.end {
                let within = (display - fragment.display.start).min(fragment.source.len());
                return fragment.source.start + within;
            }
        }
        self.source.end
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
    let heading = index.constructs_in(source.clone()).find(|construct| {
        matches!(construct.kind, ConstructKind::Heading { .. })
            && !line_revealed(
                &(construct.range.start.max(source.start)..construct.range.end.min(source.end)),
                selection,
            )
    });

    let Some(heading) = heading else {
        return plain_line(text, source);
    };
    let ConstructKind::Heading { level } = heading.kind else {
        return plain_line(text, source);
    };

    // Hidden markup: from the leading marker through the gap before the
    // content. A trailing marker (e.g. `# Title #`) is hidden as well.
    let content = heading.inner.first().cloned();
    let mut fragments = Vec::new();
    let mut display = 0;
    let mut cursor = source.start;

    let hidden_end = content
        .as_ref()
        .map(|range| range.start)
        .unwrap_or(source.end);
    if cursor < hidden_end {
        fragments.push(Fragment {
            source: cursor..hidden_end,
            display: display..display,
            kind: FragmentKind::Hidden,
        });
        cursor = hidden_end;
    }
    if let Some(content) = &content {
        let visible_end = content.end.min(source.end);
        let visible = &text[cursor..visible_end];
        fragments.push(Fragment {
            source: cursor..visible_end,
            display: display..display + visible.len(),
            kind: FragmentKind::Visible,
        });
        display += visible.len();
        cursor = visible_end;

        if let Some(trailing) = heading
            .markers
            .iter()
            .find(|marker| marker.start >= visible_end)
            && cursor < trailing.end
        {
            fragments.push(Fragment {
                source: cursor..trailing.end,
                display: display..display,
                kind: FragmentKind::Hidden,
            });
            cursor = trailing.end;
        }
    }
    if cursor < source.end {
        let visible = &text[cursor..source.end];
        fragments.push(Fragment {
            source: cursor..source.end,
            display: display..display + visible.len(),
            kind: FragmentKind::Visible,
        });
    }

    let text_visible: String = fragments
        .iter()
        .filter(|fragment| fragment.kind == FragmentKind::Visible)
        .map(|fragment| &text[fragment.source.clone()])
        .collect();
    ProjectedLine {
        source,
        text: text_visible,
        fragments,
        style: Style::Heading { level },
    }
}

fn plain_line(text: &str, source: Range<usize>) -> ProjectedLine {
    let mut fragments = Vec::new();
    if source.start <= source.end {
        fragments.push(Fragment {
            source: source.clone(),
            display: 0..source.end - source.start,
            kind: FragmentKind::Visible,
        });
    }
    ProjectedLine {
        text: text[source.clone()].to_string(),
        fragments,
        source,
        style: Style::Body,
    }
}

#[cfg(test)]
mod tests {
    use super::{DisplayPlan, FragmentKind, Style, revealed};
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
}
