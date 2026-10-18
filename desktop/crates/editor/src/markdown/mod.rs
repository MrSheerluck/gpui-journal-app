//! Markdown construct index.
//!
//! The block grammar parses the document; every `inline` region it reports is
//! re-parsed with the inline grammar. The walk produces byte ranges for every
//! construct the live preview collapses or styles. Pure: no GPUI, no I/O.
//!
//! Grammar notes (tree-sitter-md 0.5.1, verified by the census test):
//! - `**strong**` arrives as adjacent one-byte `emphasis_delimiter` nodes;
//!   adjacent markers are coalesced so a marker is the whole `**` run.
//! - `~~strike~~` is parsed as nested single-tilde emphasis; the index folds
//!   that back into a strikethrough construct.

use std::ops::Range;

use tree_sitter::{Node, Parser, Tree};

/// Everything the live preview currently understands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConstructKind {
    /// `# …` — the marker is the `#` run.
    Heading { level: u8 },
    /// `*text*`
    Emphasis,
    /// `**text**`
    Strong,
    /// `~~text~~`
    Strikethrough,
    /// `==text==` — Obsidian-style highlight, not part of the grammar.
    Highlight,
    /// `` `code` `` — the code span including its backtick delimiters.
    CodeSpan,
    /// `[text](url)` — inner carries [text, destination].
    Link,
    /// `![alt](url)` — inner carries [alt, destination].
    Image,
    /// A `-`/`*`/`+` marker of a bullet list item.
    ListMarker,
    /// An `1.`/`1)` marker of an ordered list item.
    OrderedMarker,
    /// `- [ ]`/`- [x]` marker with its checked state.
    TaskMarker { checked: bool },
    /// A `>` blockquote marker.
    QuoteMarker,
    /// A fenced code block; markers are the open/close delimiter runs.
    Fence,
    /// `---`, `***`, `___`
    ThematicBreak,
}

/// One construct: its kind, the byte range of its whole markup, and the byte
/// ranges of the marker pieces that the live preview hides when collapsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Construct {
    pub kind: ConstructKind,
    pub range: Range<usize>,
    pub markers: Vec<Range<usize>>,
    /// Inner pieces (link text, destination, heading content) where meaningful.
    pub inner: Vec<Range<usize>>,
}

pub struct MarkdownIndex {
    block_parser: Parser,
    inline_parser: Parser,
    block_tree: Option<Tree>,
    constructs: Vec<Construct>,
}

impl MarkdownIndex {
    pub fn new() -> Self {
        let mut block_parser = Parser::new();
        block_parser
            .set_language(&tree_sitter_md::LANGUAGE.into())
            .expect("markdown block grammar loads");
        let mut inline_parser = Parser::new();
        inline_parser
            .set_language(&tree_sitter_md::INLINE_LANGUAGE.into())
            .expect("markdown inline grammar loads");
        Self {
            block_parser,
            inline_parser,
            block_tree: None,
            constructs: Vec::new(),
        }
    }

    /// Re-parses `text` and rebuilds the construct list.
    pub fn update(&mut self, text: &str) {
        let Some(tree) = self.block_parser.parse(text, None) else {
            return;
        };
        let root = tree.root_node();
        let mut constructs = Vec::new();
        self.walk_blocks(root, text, &mut constructs);
        let inline_nodes = collect_kind_nodes(root, "inline");
        for inline in inline_nodes {
            let start = inline.start_byte();
            let source = &text[start..inline.end_byte()];
            if let Some(tree) = self.inline_parser.parse(source, None) {
                self.walk_inline(tree.root_node(), start, text, &mut constructs);
            }
        }
        constructs.sort_by_key(|construct| (construct.range.start, construct.range.end));
        self.block_tree = Some(tree);
        self.constructs = constructs;
        let mut highlights = Vec::new();
        scan_highlights(text, &self.constructs, &mut highlights);
        self.constructs.extend(highlights);
        self.constructs
            .sort_by_key(|construct| (construct.range.start, construct.range.end));
    }

    pub fn constructs(&self) -> &[Construct] {
        &self.constructs
    }

    /// The constructs intersecting `range`, in document order.
    pub fn constructs_in(&self, range: Range<usize>) -> impl Iterator<Item = &Construct> {
        self.constructs.iter().filter(move |construct| {
            construct.range.start < range.end && range.start < construct.range.end
        })
    }

    /// Block-level constructs: headings, list and quote markers, fences, rules.
    fn walk_blocks(&self, node: Node<'_>, text: &str, out: &mut Vec<Construct>) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "setext_heading" => {
                    let mut markers = Vec::new();
                    collect_children(
                        child,
                        &["setext_h1_underline", "setext_h2_underline"],
                        &mut markers,
                    );
                    let level = markers
                        .first()
                        .map(|marker| {
                            if text[marker.clone()].starts_with('=') {
                                1
                            } else {
                                2
                            }
                        })
                        .unwrap_or(1);
                    let mut inner = collect_descendant_kind_nodes(child, "inline");
                    inner.truncate(1);
                    trim_heading_inner(text, &mut inner);
                    out.push(Construct {
                        kind: ConstructKind::Heading { level },
                        range: child.byte_range(),
                        markers,
                        inner,
                    });
                }
                "atx_heading" => {
                    let level = atx_heading_level(child).unwrap_or(1);
                    let mut markers = Vec::new();
                    collect_children(
                        child,
                        &[
                            "atx_h1_marker",
                            "atx_h2_marker",
                            "atx_h3_marker",
                            "atx_h4_marker",
                            "atx_h5_marker",
                            "atx_h6_marker",
                        ],
                        &mut markers,
                    );
                    let mut inner = Vec::new();
                    collect_children(child, &["inline"], &mut inner);
                    trim_heading_inner(text, &mut inner);
                    out.push(Construct {
                        kind: ConstructKind::Heading { level },
                        range: child.byte_range(),
                        markers,
                        inner,
                    });
                }
                "fenced_code_block" => {
                    let mut markers = Vec::new();
                    collect_children(child, &["fenced_code_block_delimiter"], &mut markers);
                    out.push(Construct {
                        kind: ConstructKind::Fence,
                        range: child.byte_range(),
                        markers,
                        inner: Vec::new(),
                    });
                }
                "thematic_break" => {
                    out.push(Construct {
                        kind: ConstructKind::ThematicBreak,
                        range: child.byte_range(),
                        markers: Vec::new(),
                        inner: Vec::new(),
                    });
                }
                "list_item" => {
                    let mut cursor = child.walk();
                    for marker in child.children(&mut cursor) {
                        let kind = match marker.kind() {
                            "list_marker_minus" | "list_marker_star" | "list_marker_plus" => {
                                ConstructKind::ListMarker
                            }
                            "list_marker_dot" | "list_marker_parenthesis" => {
                                ConstructKind::OrderedMarker
                            }
                            "task_list_marker_checked" => {
                                ConstructKind::TaskMarker { checked: true }
                            }
                            "task_list_marker_unchecked" => {
                                ConstructKind::TaskMarker { checked: false }
                            }
                            _ => continue,
                        };
                        out.push(Construct {
                            kind,
                            range: marker.byte_range(),
                            markers: Vec::new(),
                            inner: Vec::new(),
                        });
                    }
                }
                "block_quote_marker" => {
                    out.push(Construct {
                        kind: ConstructKind::QuoteMarker,
                        range: child.byte_range(),
                        markers: Vec::new(),
                        inner: Vec::new(),
                    });
                }
                _ => {}
            }
            self.walk_blocks(child, text, out);
        }
    }

    /// Inline constructs: emphasis/strong/strike, code spans, links, images.
    fn walk_inline(&self, node: Node<'_>, offset: usize, text: &str, out: &mut Vec<Construct>) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let range = child.byte_range();
            let source = &text[shift(range.clone(), offset)];
            let kind = match child.kind() {
                "emphasis" => ConstructKind::Emphasis,
                "strong_emphasis" => ConstructKind::Strong,
                "strikethrough" => ConstructKind::Strikethrough,
                "code_span" => ConstructKind::CodeSpan,
                "inline_link"
                | "full_reference_link"
                | "collapsed_reference_link"
                | "shortcut_link" => ConstructKind::Link,
                "image" => ConstructKind::Image,
                _ => {
                    self.walk_inline(child, offset, text, out);
                    continue;
                }
            };

            // `~~text~~` parses as a strikethrough whose direct delimiters are
            // the outer tildes, with a nested strikethrough artifact holding
            // the inner pair. Fold it into one construct with two-tilde
            // markers and do not descend again.
            let folded_strike = kind == ConstructKind::Strikethrough
                && source.starts_with("~~")
                && source.ends_with("~~")
                && range.len() >= 4;

            let mut markers = Vec::new();
            collect_children_abs(
                child,
                &["emphasis_delimiter", "code_span_delimiter"],
                offset,
                &mut markers,
            );
            if folded_strike {
                markers = vec![
                    shift(range.start..range.start + 2, offset),
                    shift(range.end - 2..range.end, offset),
                ];
            }
            coalesce(&mut markers);

            let mut inner = Vec::new();
            collect_children_abs(
                child,
                &[
                    "link_text",
                    "link_destination",
                    "image_description",
                    "inline",
                ],
                offset,
                &mut inner,
            );

            out.push(Construct {
                kind,
                range: shift(range, offset),
                markers,
                inner,
            });

            // Strikethrough was folded from its doubled-tilde form; do not
            // descend into it again or the nested artifact would duplicate.
            if !folded_strike {
                self.walk_inline(child, offset, text, out);
            }
        }
    }
}

/// Shifts a slice-relative byte range into document coordinates.
fn shift(range: Range<usize>, offset: usize) -> Range<usize> {
    range.start + offset..range.end + offset
}

/// Finds `==text==` pairs the Markdown grammar does not know about, outside
/// code spans and fences. Pairs are matched left-to-right per line; empty
/// content is not a highlight.
fn scan_highlights(text: &str, existing: &[Construct], out: &mut Vec<Construct>) {
    let code_spans: Vec<&Construct> = existing
        .iter()
        .filter(|construct| {
            matches!(
                construct.kind,
                ConstructKind::CodeSpan | ConstructKind::Fence
            )
        })
        .collect();
    let inside_code = |offset: usize| {
        code_spans
            .iter()
            .any(|construct| construct.range.start <= offset && offset < construct.range.end)
    };

    let mut line_start = 0;
    loop {
        let line_end = text[line_start..]
            .find('\n')
            .map(|offset| line_start + offset)
            .unwrap_or(text.len());
        let line = &text[line_start..line_end];
        let mut open: Option<usize> = None;
        let mut cursor = 0;
        while let Some(found) = line[cursor..].find("==") {
            let position = cursor + found;
            cursor = position + 2;
            if inside_code(line_start + position) {
                continue;
            }
            match open {
                None => open = Some(position),
                Some(start) => {
                    open = None;
                    if position > start + 2 && !inside_code(line_start + position) {
                        // One range element by design; the lint misreads
                        // `vec![a..b]` as a mistaken `vec![x; n]`.
                        #[allow(clippy::single_range_in_vec_init)]
                        let inner = vec![line_start + start + 2..line_start + position];
                        out.push(Construct {
                            kind: ConstructKind::Highlight,
                            range: line_start + start..line_start + position + 2,
                            markers: vec![
                                line_start + start..line_start + start + 2,
                                line_start + position..line_start + position + 2,
                            ],
                            inner,
                        });
                    }
                }
            }
        }
        if line_end == text.len() {
            break;
        }
        line_start = line_end + 1;
    }
}

/// Merges adjacent or overlapping marker ranges (e.g. two one-byte `*` runs
/// of a `**` delimiter) into whole runs.
fn coalesce(markers: &mut Vec<Range<usize>>) {
    markers.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(markers.len());
    for range in markers.drain(..) {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    *markers = merged;
}

fn collect_kind_nodes<'tree>(node: Node<'tree>, kind: &str) -> Vec<Node<'tree>> {
    let mut found = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            found.push(child);
        }
        found.extend(collect_kind_nodes(child, kind));
    }
    found
}

/// Inline nodes of `node` at any depth, as byte ranges.
fn collect_descendant_kind_nodes(node: Node<'_>, kind: &str) -> Vec<Range<usize>> {
    collect_kind_nodes(node, kind)
        .into_iter()
        .map(|node| node.byte_range())
        .collect()
}

/// The grammar keeps a closed ATX heading's trailing `#` run inside the
/// inline content; trim it (with its leading spaces) from the content range.
fn trim_heading_inner(text: &str, inner: &mut [Range<usize>]) {
    if let Some(range) = inner.first_mut() {
        let content = &text[range.clone()];
        let without_hashes = content.trim_end().trim_end_matches('#').trim_end();
        if without_hashes.len() < content.len() {
            range.end = range.start + without_hashes.len();
        }
    }
}

fn collect_children(node: Node<'_>, kinds: &[&str], out: &mut Vec<Range<usize>>) {
    collect_children_abs(node, kinds, 0, out);
}

fn collect_children_abs(
    node: Node<'_>,
    kinds: &[&str],
    offset: usize,
    out: &mut Vec<Range<usize>>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if kinds.contains(&child.kind()) {
            out.push(shift(child.byte_range(), offset));
        }
    }
}

fn atx_heading_level(node: Node<'_>) -> Option<u8> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if let Some(level) = kind
            .strip_prefix("atx_h")
            .and_then(|rest| rest.strip_suffix("_marker"))
            .and_then(|level| level.parse::<u8>().ok())
        {
            return Some(level);
        }
    }
    None
}

impl Default for MarkdownIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{Construct, ConstructKind, MarkdownIndex};
    use std::ops::Range;

    const FIXTURE: &str = "# One\n\n## Two\n\nPlain *em* and **strong** and `code` and ~~strike~~.\n\n- a\n- [ ] task\n- [x] done\n\n1. ordered\n\n> quoted\n\n```rust\nfn main() {}\n```\n\n---\n\n[text](https://example.com) and ![alt](assets/x.png)\n";

    fn index_of(text: &str) -> MarkdownIndex {
        let mut index = MarkdownIndex::new();
        index.update(text);
        index
    }

    fn kinds(index: &MarkdownIndex) -> Vec<ConstructKind> {
        index
            .constructs()
            .iter()
            .map(|construct| construct.kind.clone())
            .collect()
    }

    fn find(index: &MarkdownIndex, predicate: impl Fn(&Construct) -> bool) -> Construct {
        index
            .constructs()
            .iter()
            .find(|construct| predicate(construct))
            .cloned()
            .unwrap_or_else(|| panic!("construct not found in {:?}", kinds(index)))
    }

    #[test]
    fn headings_carry_level_marker_and_content() {
        let index = index_of(FIXTURE);
        let one = find(&index, |c| c.kind == ConstructKind::Heading { level: 1 });
        assert_eq!(one.markers, vec![0..1]);
        assert_eq!(one.inner, vec![2..5]);
        let two = find(&index, |c| c.kind == ConstructKind::Heading { level: 2 });
        assert_eq!(two.markers, vec![7..9]);
    }

    #[test]
    fn inline_constructs_are_found_once_with_whole_markers() {
        let index = index_of(FIXTURE);
        let emphasis: Vec<_> = index
            .constructs()
            .iter()
            .filter(|c| c.kind == ConstructKind::Emphasis)
            .collect();
        assert_eq!(emphasis.len(), 1, "{emphasis:#?}");

        let strong = find(&index, |c| c.kind == ConstructKind::Strong);
        assert_eq!(strong.markers, vec![30..32, 38..40]);

        let code = find(&index, |c| c.kind == ConstructKind::CodeSpan);
        assert_eq!(code.markers, vec![45..46, 50..51]);

        let strike = find(&index, |c| c.kind == ConstructKind::Strikethrough);
        assert_eq!(strike.range, 56..66);
        assert_eq!(strike.markers, vec![56..58, 64..66]);
        assert_eq!(
            index
                .constructs()
                .iter()
                .filter(|c| c.kind == ConstructKind::Strikethrough)
                .count(),
            1
        );
    }

    #[test]
    fn links_and_images_carry_text_and_destination() {
        let index = index_of(FIXTURE);
        let link = find(&index, |c| c.kind == ConstructKind::Link);
        assert_eq!(link.inner, vec![150..154, 156..175]);
        assert_eq!(&FIXTURE[150..154], "text");
        assert_eq!(&FIXTURE[156..175], "https://example.com");

        let image = find(&index, |c| c.kind == ConstructKind::Image);
        assert_eq!(&FIXTURE[image.inner[0].clone()], "alt");
        assert_eq!(&FIXTURE[image.inner[1].clone()], "assets/x.png");
    }

    #[test]
    fn list_task_and_quote_markers() {
        let index = index_of(FIXTURE);
        let bullets: Vec<_> = index
            .constructs()
            .iter()
            .filter(|c| c.kind == ConstructKind::ListMarker)
            .map(|c| c.range.clone())
            .collect();
        assert_eq!(bullets, vec![69..71, 73..75, 84..86]);

        let tasks: Vec<_> = index
            .constructs()
            .iter()
            .filter_map(|c| match c.kind {
                ConstructKind::TaskMarker { checked } => Some((c.range.clone(), checked)),
                _ => None,
            })
            .collect();
        assert_eq!(tasks, vec![(75..78, false), (86..89, true)]);

        let ordered = find(&index, |c| c.kind == ConstructKind::OrderedMarker);
        assert_eq!(ordered.range, 96..99);

        let quote = find(&index, |c| c.kind == ConstructKind::QuoteMarker);
        assert_eq!(quote.range, 108..110);
    }

    #[test]
    fn fences_and_rules() {
        let index = index_of(FIXTURE);
        let fence = find(&index, |c| c.kind == ConstructKind::Fence);
        assert_eq!(fence.markers, vec![118..121, 139..142]);
        let rule = find(&index, |c| c.kind == ConstructKind::ThematicBreak);
        assert_eq!(rule.range, 144..148);
    }

    #[test]
    fn constructs_in_reports_overlaps() {
        let index = index_of(FIXTURE);
        let heading: Vec<Range<usize>> = index
            .constructs_in(0..6)
            .map(|construct| construct.range.clone())
            .collect();
        assert_eq!(heading, vec![0..6]);
        assert_eq!(index.constructs_in(210..250).count(), 0);
    }

    #[test]
    fn highlights_are_scanned_outside_code() {
        let text = "a ==hot== b and `==not==` and ==x==\n";
        let index = index_of(text);
        let highlights: Vec<_> = index
            .constructs()
            .iter()
            .filter(|c| c.kind == ConstructKind::Highlight)
            .map(|c| c.range.clone())
            .collect();
        assert_eq!(highlights, vec![2..9, 30..35]);
        let first = find(&index, |c| c.kind == ConstructKind::Highlight);
        assert_eq!(first.markers, vec![2..4, 7..9]);
        assert_eq!(first.inner, vec![4..7]);
        assert_eq!(&text[4..7], "hot");
    }

    #[test]
    fn highlight_edges_do_not_match() {
        let text = "==== \nempty ==\n==ok==\n";
        let index = index_of(text);
        let highlights: Vec<_> = index
            .constructs()
            .iter()
            .filter(|c| c.kind == ConstructKind::Highlight)
            .map(|c| c.range.clone())
            .collect();
        assert_eq!(
            highlights,
            vec![15..21],
            "wrong pairs matched: {highlights:?}"
        );
    }

    #[test]
    fn plain_text_has_no_constructs() {
        let index = index_of("Just a sentence.\n\nAnd another one.\n");
        assert!(index.constructs().is_empty(), "{:?}", kinds(&index));
    }
}
