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

use tree_sitter::{InputEdit, Node, Parser, Point, Tree};

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
    /// The text the current trees were parsed from; needed to build edit
    /// positions for incremental updates.
    last_text: String,
    /// Parsed inline regions, keyed by their byte range in the current text.
    inline_regions: Vec<InlineRegion>,
    constructs: Vec<Construct>,
}

struct InlineRegion {
    range: Range<usize>,
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
            last_text: String::new(),
            inline_regions: Vec::new(),
            constructs: Vec::new(),
        }
    }

    /// Re-parses `text` and rebuilds the construct list.
    pub fn update(&mut self, text: &str) {
        self.rebuild(text, None);
    }

    /// Applies a known edit `old_range` (as it was before the edit) that was
    /// replaced by `new_len` bytes, reusing both trees for everything the
    /// edit did not touch.
    pub fn update_edited(&mut self, text: &str, old_range: Range<usize>, new_len: usize) {
        self.rebuild(text, Some((old_range, new_len)));
    }

    fn rebuild(&mut self, text: &str, edit: Option<(Range<usize>, usize)>) {
        let t0 = std::time::Instant::now();
        let input_edit = edit.as_ref().and_then(|(old_range, new_len)| {
            let new_len = *new_len;
            let old_len = self.last_text.len();
            if old_range.start > old_range.end || old_range.end > old_len {
                return None;
            }
            Some(InputEdit {
                start_byte: old_range.start,
                old_end_byte: old_range.end,
                new_end_byte: old_range.start + new_len,
                start_position: point_in(&self.last_text, old_range.start),
                old_end_position: point_in(&self.last_text, old_range.end),
                new_end_position: point_in(text, old_range.start + new_len),
            })
        });

        let tree = match (input_edit, self.block_tree.take()) {
            (Some(edit), Some(mut old_tree)) => {
                old_tree.edit(&edit);
                self.block_parser
                    .parse(text, Some(&old_tree))
                    .unwrap_or(old_tree)
            }
            (_, _) => match self.block_parser.parse(text, None) {
                Some(tree) => tree,
                None => return,
            },
        };

        let root = tree.root_node();
        let t1 = std::time::Instant::now();
        let mut constructs = Vec::new();
        self.walk_blocks(root, text, &mut constructs);

        // Incremental inline pass: shift kept regions by the edit, drop the
        // ones the edit touched, and re-parse only regions without a cached
        // tree.
        let regions = collect_kind_nodes(root, "inline");
        let mut kept = std::mem::take(&mut self.inline_regions);
        if let Some((old_range, new_len)) = edit {
            let delta = new_len as isize - (old_range.end - old_range.start) as isize;
            kept.retain_mut(|region| {
                if region.range.end <= old_range.start {
                    true
                } else if region.range.start >= old_range.end {
                    region.range = shift_signed(&region.range, delta);
                    for construct in &mut region.constructs {
                        shift_construct(construct, delta);
                    }
                    true
                } else {
                    false
                }
            });
        } else {
            kept.clear();
        }

        let mut inline_regions = Vec::with_capacity(regions.len());
        let mut kept_iter = kept.into_iter().peekable();
        for region in regions {
            let range = region.byte_range();
            // Both sequences are sorted; skip kept entries that no longer
            // correspond to any region.
            while let Some(next) = kept_iter.peek() {
                if next.range.end < range.start {
                    kept_iter.next();
                } else {
                    break;
                }
            }
            if let Some(next) = kept_iter.peek()
                && next.range == range
            {
                inline_regions.push(kept_iter.next().expect("peeked"));
                continue;
            }
            let source = &text[range.clone()];
            if let Some(tree) = self.inline_parser.parse(source, None) {
                let mut constructs = Vec::new();
                self.walk_inline(tree.root_node(), range.start, text, &mut constructs);
                inline_regions.push(InlineRegion { range, constructs });
            }
        }
        for region in &inline_regions {
            constructs.extend(region.constructs.iter().cloned());
        }
        let t2 = std::time::Instant::now();
        self.inline_regions = inline_regions;

        constructs.sort_by_key(|construct| (construct.range.start, construct.range.end));
        // CommonMark rejects bare destinations with spaces, so the grammar
        // yields no Image for the most common macOS attachment names
        // (`assets/Screen Recording ….mov`). Accept whole-line references
        // anyway; destinations the grammar did parse overlap an existing
        // Image and are skipped.
        add_spaced_whole_line_images(text, &mut constructs);
        self.block_tree = Some(tree);
        self.constructs = constructs;
        let mut highlights = Vec::new();
        scan_highlights(text, &self.constructs, &mut highlights);
        self.constructs.extend(highlights);
        self.constructs
            .sort_by_key(|construct| (construct.range.start, construct.range.end));
        self.last_text.clear();
        self.last_text.push_str(text);
        let t3 = std::time::Instant::now();
        if std::env::var_os("SMBL_PERF_DEBUG").is_some() {
            eprintln!(
                "rebuild block={:.2}ms walk={:.2}ms tail={:.2}ms",
                (t1 - t0).as_secs_f64() * 1000.0,
                (t2 - t1).as_secs_f64() * 1000.0,
                (t3 - t2).as_secs_f64() * 1000.0,
            );
        }
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
                // Continuation lines carry their `>` in a `block_continuation`
                // node rather than a `block_quote_marker`: the empty quote
                // line, and a lazy continuation glued to the previous
                // paragraph. The same `>` is a quote's marker wherever it
                // stands, so it collapses the same way. Lists share the node
                // kind for their own continuations, hence the `>` test.
                "block_continuation" if text[child.byte_range()].contains('>') => {
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
            // `![]` parses with no description node at all. Keep the
            // `[alt, destination]` convention by inserting an empty
            // description ahead of the destination.
            if kind == ConstructKind::Image
                && inner.len() == 1
                && let Some(destination) = inner.first().cloned()
            {
                inner.insert(0, destination.start..destination.start);
            }

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

/// Appends an `Image` construct for every whole-line `![alt](destination)`
/// whose destination contains spaces — a form CommonMark rejects, which is
/// exactly how macOS names attachments. Constructs the grammar already found
/// (angle-bracketed `<…>` or percent-encoded destinations) overlap the line
/// and are left in place.
fn add_spaced_whole_line_images(text: &str, out: &mut Vec<Construct>) {
    // For a spaced destination the grammar emits a partial `Image` fragment
    // that stops short of the closing paren; drop every Image whose source is
    // not a complete reference before re-recognising the whole-line form.
    out.retain(|construct| {
        construct.kind != ConstructKind::Image
            || is_complete_image_reference(&text[construct.range.clone()])
    });

    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let without_newline = line.trim_end_matches(['\n', '\r']);
        let leading = without_newline.len() - without_newline.trim_start().len();
        let content = without_newline.trim_end();
        let content = &content[leading..];
        if let Some((alt, destination)) = spaced_whole_line_image(content) {
            let start = line_start + leading;
            let range = start..start + content.len();
            let already = out.iter().any(|construct| {
                construct.kind == ConstructKind::Image
                    && construct.range.start < range.end
                    && range.start < construct.range.end
            });
            if !already {
                let alt_start = start + 2;
                let destination_start = alt_start + alt.len() + 2;
                out.push(Construct {
                    kind: ConstructKind::Image,
                    range,
                    markers: Vec::new(),
                    inner: vec![
                        alt_start..alt_start + alt.len(),
                        destination_start..destination_start + destination.len(),
                    ],
                });
            }
        }
        line_start += line.len();
    }
    out.sort_by_key(|construct| (construct.range.start, construct.range.end));
}

/// Whether a construct's source is a complete `![…](…)` reference. The
/// grammar can emit truncated fragments; only complete ones are trusted.
fn is_complete_image_reference(source: &str) -> bool {
    source.starts_with("![") && source.contains("](") && source.ends_with(')')
}

/// `![alt](destination)` with spaces in the destination: the bare form the
/// grammar drops. Parentheses in either part would make the boundaries
/// ambiguous, so those stay literal text.
fn spaced_whole_line_image(content: &str) -> Option<(&str, &str)> {
    let rest = content.strip_prefix("![")?;
    let (alt, rest) = rest.split_once("](")?;
    let destination = rest.strip_suffix(')')?;
    if destination.is_empty()
        || !destination.contains(' ')
        || destination.contains(['(', ')'])
        || alt.contains(['[', ']'])
    {
        return None;
    }
    Some((alt, destination))
}

/// Shifts a byte range by a signed delta, clamped at zero.
fn shift_signed(range: &Range<usize>, delta: isize) -> Range<usize> {
    let start = (range.start as isize + delta).max(0) as usize;
    let end = (range.end as isize + delta).max(0) as usize;
    start..end
}

/// Shifts every range of a construct by a signed delta.
fn shift_construct(construct: &mut Construct, delta: isize) {
    construct.range = shift_signed(&construct.range, delta);
    for marker in &mut construct.markers {
        *marker = shift_signed(marker, delta);
    }
    for inner in &mut construct.inner {
        *inner = shift_signed(inner, delta);
    }
}

/// The tree-sitter point (byte-based column) for a byte offset.
fn point_in(text: &str, offset: usize) -> Point {
    let offset = offset.min(text.len());
    let row = text[..offset].bytes().filter(|byte| *byte == b'\n').count();
    let line_start = text[..offset].rfind('\n').map(|ix| ix + 1).unwrap_or(0);
    Point {
        row,
        column: offset - line_start,
    }
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
    fn empty_alt_images_still_carry_a_destination() {
        let index = index_of("![](assets/x.png)\n\n");
        let image = find(&index, |c| c.kind == ConstructKind::Image);
        assert_eq!(image.inner.len(), 2, "alt range + destination");
        assert!(image.inner[0].is_empty(), "the alt range is empty");
        assert_eq!(
            &"![](assets/x.png)\n\n"[image.inner[1].clone()],
            "assets/x.png"
        );
    }

    #[test]
    fn spaced_whole_line_images_are_recognised() {
        // The exact shape macOS attachments take: the bare destination has
        // spaces, which CommonMark rejects, so the grammar alone would leave
        // the line as literal text.
        let text = "Ok Testing this journal write now #thoughts\n\n\
             ![Screenshot 2026-10-18 at 08.11.33](assets/Screenshot 2026-10-18 at 08.11.33-1.png)\n\n\
             ![Screen Recording](assets/Clip-1.mov)\n";
        let index = index_of(text);
        let images: Vec<&Construct> = index
            .constructs()
            .iter()
            .filter(|c| c.kind == ConstructKind::Image)
            .collect();
        assert_eq!(images.len(), 2, "the spaced reference plus the clean one");
        let spaced = images
            .iter()
            .find(|image| text[image.inner[1].clone()].contains(' '))
            .expect("the spaced reference is recognised");
        assert_eq!(
            &text[spaced.inner[1].clone()],
            "assets/Screenshot 2026-10-18 at 08.11.33-1.png"
        );
        assert_eq!(
            &text[spaced.inner[0].clone()],
            "Screenshot 2026-10-18 at 08.11.33"
        );
        // The whole line is covered, so the projection treats it as whole-line.
        assert_eq!(
            &text[spaced.range.clone()],
            "![Screenshot 2026-10-18 at 08.11.33](assets/Screenshot 2026-10-18 at 08.11.33-1.png)"
        );
    }

    #[test]
    fn inline_spaced_images_stay_text() {
        // The fallback is whole-line only: a spaced reference inside a
        // sentence stays literal text (nothing to card there).
        let text = "text ![a](assets/a b.png) more\n";
        let index = index_of(text);
        assert!(
            index
                .constructs()
                .iter()
                .all(|c| c.kind != ConstructKind::Image),
            "inline spaced references are not images"
        );
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
    fn incremental_update_matches_full_update() {
        let base = "# Title\n\nA **bold** word and [link](https://x.dev).\n\n- [ ] task\n\n```rs\nfn main() {}\n```\n\n==hot== tail\n";
        let scenarios: Vec<(usize, Range<usize>, &str)> = vec![
            (base.find("A **").unwrap(), 0..0, "very "),
            (base.find("**bold**").unwrap() + 2, 2..2, "very "),
            (base.find("- [ ]").unwrap() + 2, 2..5, "[x]"),
            (base.find("```rs").unwrap(), 0..0, "> "),
            (base.find("tail").unwrap(), 0..5, ""),
            (base.len(), 0..0, "\n# New\n"),
            (0, 0..0, "pre "),
        ];
        for (position, old_range, replacement) in scenarios {
            let mut edited = base.to_string();
            edited.replace_range(
                position + old_range.start..position + old_range.end,
                replacement,
            );

            let mut full = MarkdownIndex::new();
            full.update(&edited);
            let mut incremental = MarkdownIndex::new();
            incremental.update(base);
            incremental.update_edited(
                &edited,
                position + old_range.start..position + old_range.end,
                replacement.len(),
            );

            assert_eq!(
                incremental.constructs().to_vec(),
                full.constructs().to_vec(),
                "constructs diverged for edit at {position} with {replacement:?}"
            );
        }
    }

    #[test]
    fn plain_text_has_no_constructs() {
        let index = index_of("Just a sentence.\n\nAnd another one.\n");
        assert!(index.constructs().is_empty(), "{:?}", kinds(&index));
    }
}
