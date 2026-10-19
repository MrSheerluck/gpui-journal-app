//! Pure formatting commands: each maps `(text, selection)` to a single
//! [`Edit`] the editor applies as one undo step. No GPUI, no I/O.

use std::ops::Range;

use crate::input::Selection;

/// One formatting command's result: replace `range` with `new_text`, then
/// place `new_selection`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub new_text: String,
    pub new_selection: Selection,
}

/// Character-level formats toggled around the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineFormat {
    Bold,
    Italic,
    Strike,
    Highlight,
    Code,
}

impl InlineFormat {
    pub fn delimiter(self) -> &'static str {
        match self {
            InlineFormat::Bold => "**",
            InlineFormat::Italic => "*",
            InlineFormat::Strike => "~~",
            InlineFormat::Highlight => "==",
            InlineFormat::Code => "`",
        }
    }
}

/// Block-level formats applied to every line the selection touches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockFormat {
    Heading(u8),
    Bullet,
    Ordered,
    Task,
    Quote,
}

impl BlockFormat {
    fn prefix(self, index: usize) -> String {
        match self {
            BlockFormat::Heading(level) => format!("{} ", "#".repeat(level.clamp(1, 6) as usize)),
            BlockFormat::Bullet => "- ".to_string(),
            BlockFormat::Ordered => format!("{}. ", index + 1),
            BlockFormat::Task => "- [ ] ".to_string(),
            BlockFormat::Quote => "> ".to_string(),
        }
    }
}

/// Toggles an inline format around the selection:
/// - the selection including its delimiters → unwrap;
/// - the delimiters immediately around the selection (caret inside) → unwrap;
/// - otherwise wrap the selection's non-space core;
/// - a collapsed caret inserts a delimiter pair with the caret inside.
pub fn toggle_inline(text: &str, selection: Selection, format: InlineFormat) -> Option<Edit> {
    let delimiter = format.delimiter();
    let len = delimiter.len();
    let (start, end) = (selection.start(), selection.end());

    // The selection contains the delimiters.
    if end - start >= 2 * len {
        let inner = &text[start..end];
        if inner.starts_with(delimiter) && inner.ends_with(delimiter) {
            let stripped = inner[..inner.len() - len].to_string();
            let stripped = stripped[delimiter.len()..].to_string();
            let selection = shift_selection(selection, -(len as isize));
            return Some(Edit {
                range: start..end,
                new_text: stripped,
                new_selection: selection,
            });
        }
    }

    // The delimiters sit immediately outside the selection.
    if start >= len
        && end + len <= text.len()
        && text[start - len..start] == *delimiter
        && text[end..end + len] == *delimiter
    {
        return Some(Edit {
            range: start - len..end + len,
            new_text: text[start..end].to_string(),
            new_selection: shift_selection(selection, -(len as isize)),
        });
    }

    // Collapsed caret inside empty space: insert a pair, caret in between.
    if selection.is_caret() {
        return Some(Edit {
            range: start..end,
            new_text: format!("{delimiter}{delimiter}"),
            new_selection: Selection::caret(start + len),
        });
    }

    // Wrap the non-space core so trailing whitespace stays outside.
    let content = &text[start..end];
    let core_len = content.trim_end().len();
    let core = &content[..core_len];
    let trailing = &content[core_len..];
    let new_text = format!("{delimiter}{core}{delimiter}{trailing}");
    let selection =
        Selection::new_ordered(start + len, start + len + core.len(), selection.reversed());
    Some(Edit {
        range: start..end,
        new_text,
        new_selection: selection,
    })
}

/// Toggles a block format across every line the selection touches:
/// a second toggle strips it; a different block prefix is swapped out.
pub fn toggle_block(text: &str, selection: Selection, format: BlockFormat) -> Option<Edit> {
    let lines = covered_lines(text, selection);
    if lines.is_empty() {
        return None;
    }
    let range = lines[0].start..lines[lines.len() - 1].end;
    let all_match = lines
        .iter()
        .filter(|line| !line.is_blank)
        .all(|line| line.block == Some(format));
    let stripped: Vec<String> = lines
        .iter()
        .map(|line| strip_block_prefix(&text[line.start..line.end]))
        .collect();

    let mut new_text = String::new();
    let mut number = 0usize;
    for (ix, content) in stripped.iter().enumerate() {
        if ix > 0 {
            new_text.push('\n');
        }
        if all_match {
            // Toggle off: keep the rest of the line as-is.
            new_text.push_str(content);
            continue;
        }
        if content.is_empty() {
            // Blank lines keep their blankness (no dangling prefix).
            continue;
        }
        let prefix = match format {
            BlockFormat::Ordered => {
                number += 1;
                format.prefix(number - 1)
            }
            other => other.prefix(0),
        };
        new_text.push_str(&prefix);
        new_text.push_str(content);
    }

    let new_selection = Selection::new_ordered(
        range.start,
        range.start + new_text.len(),
        selection.reversed(),
    );
    Some(Edit {
        range,
        new_text,
        new_selection,
    })
}

/// Wraps the selection in a fenced code block, or inserts an empty one.
pub fn insert_code_fence(text: &str, selection: Selection) -> Option<Edit> {
    let (start, end) = (selection.start(), selection.end());
    if selection.is_caret() {
        let indent = line_prefix(&text[..start]);
        return Some(Edit {
            range: start..end,
            new_text: format!("```\n\n```\n{indent}"),
            new_selection: Selection::caret(start + 4),
        });
    }
    let content = &text[start..end];
    let new_text = format!("```\n{content}\n```");
    let selection = Selection::new_ordered(start + 4, start + 4 + content.len(), false);
    Some(Edit {
        range: start..end,
        new_text,
        new_selection: selection,
    })
}

/// Inserts a horizontal rule on its own line after the current line.
pub fn insert_rule(text: &str, selection: Selection) -> Option<Edit> {
    let start = selection.head().min(text.len());
    let line_end = text[start..]
        .find('\n')
        .map(|offset| start + offset)
        .unwrap_or(text.len());
    let insert_at = if line_end == start {
        // Caret on an empty line: replace it.
        start
    } else {
        line_end
    };
    let new_text = if insert_at == start {
        "---\n".to_string()
    } else {
        "\n\n---\n".to_string()
    };
    let caret = insert_at + new_text.len();
    Some(Edit {
        range: insert_at..insert_at,
        new_text,
        new_selection: Selection::caret(caret),
    })
}

/// The block format of every line the selection touches — the toolbar's
/// active-state input.
pub fn selection_block_formats(text: &str, selection: Selection) -> Vec<Option<BlockFormat>> {
    covered_lines(text, selection)
        .into_iter()
        .map(|line| line.block)
        .collect()
}

/// The lines a selection touches; a selection ending exactly at a line start
/// does not include that line.
fn covered_lines(text: &str, selection: Selection) -> Vec<LineInfo> {
    let mut all = Vec::new();
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
        all.push(LineInfo {
            start,
            end: content_end,
            is_blank: text[start..content_end].trim().is_empty(),
            block: existing_block(&text[start..content_end]),
        });
        if end == text.len() {
            break;
        }
        start = end + 1;
    }

    let line_at = |offset: usize| -> usize {
        let ix = all.partition_point(|line| line.start <= offset);
        ix.saturating_sub(1).min(all.len() - 1)
    };
    let first = line_at(selection.start());
    let mut last = line_at(selection.end());
    if !selection.is_caret() && last > first && selection.end() == all[last].start {
        last -= 1;
    }
    all[first..=last].to_vec()
}

#[derive(Clone)]
struct LineInfo {
    start: usize,
    end: usize,
    is_blank: bool,
    block: Option<BlockFormat>,
}

/// The block prefix of a line, if any.
fn existing_block(line: &str) -> Option<BlockFormat> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    if (1..=6).contains(&hashes) && trimmed.chars().nth(hashes) == Some(' ') {
        return Some(BlockFormat::Heading(hashes as u8));
    }
    if trimmed.starts_with("> ") {
        return Some(BlockFormat::Quote);
    }
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        if rest.starts_with("[ ] ") || rest.starts_with("[x] ") || rest.starts_with("[X] ") {
            return Some(BlockFormat::Task);
        }
        return Some(BlockFormat::Bullet);
    }
    let digits = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digits > 0 && trimmed[digits..].starts_with(". ") {
        return Some(BlockFormat::Ordered);
    }
    None
}

/// Strips any block prefix, returning the content.
fn strip_block_prefix(line: &str) -> String {
    let trimmed = line.trim_start();
    let indent_len = line.len() - trimmed.len();
    let indent = &line[..indent_len];

    if let Some(block) = existing_block(line) {
        let stripped = match block {
            BlockFormat::Heading(level) => trimmed[(level as usize + 1)..].to_string(),
            BlockFormat::Quote => trimmed[2..].to_string(),
            BlockFormat::Bullet => trimmed[2..].to_string(),
            BlockFormat::Task => trimmed[6..].to_string(),
            BlockFormat::Ordered => {
                let digits = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
                trimmed[digits + 2..].to_string()
            }
        };
        return format!("{indent}{stripped}");
    }
    line.to_string()
}

/// Inserts a Markdown link. A selection becomes the label; a collapsed caret
/// uses the URL as the label. An empty URL inserts `[]()` with the caret
/// inside the parentheses, ready for typing a target.
pub fn insert_link(text: &str, selection: Selection, url: &str) -> Edit {
    let url = url.trim();
    let (start, end) = (selection.start(), selection.end());
    let label = if selection.is_caret() {
        url
    } else {
        &text[start..end]
    };
    let new_text = format!("[{label}]({url})");
    let caret = if url.is_empty() {
        // Just inside the closing parenthesis.
        start + new_text.len() - 1
    } else {
        start + new_text.len()
    };
    Edit {
        range: start..end,
        new_text,
        new_selection: Selection::caret(caret),
    }
}

/// Inserts an image reference at the caret, replacing any selection.
pub fn insert_image(selection: Selection, alt: &str, path: &str) -> Edit {
    let (start, end) = (selection.start(), selection.end());
    let new_text = format!("![{alt}]({path})");
    let caret = start + new_text.len();
    Edit {
        range: start..end,
        new_text,
        new_selection: Selection::caret(caret),
    }
}

/// Inserts a media reference on its own paragraph, replacing any selection.
///
/// Attachments render as cards only when the reference is a whole line, so
/// this adds the paragraph breaks an inline insert would miss: a leading
/// blank line when the caret is mid-text, a single break when it sits at the
/// start of a line, and a trailing newline when text follows.
pub fn insert_media(text: &str, selection: Selection, alt: &str, path: &str) -> Edit {
    let (start, end) = (selection.start(), selection.end());
    let reference = format!("![{alt}]({path})");
    let before = &text[..start];
    let after = &text[end..];

    let mut new_text = String::new();
    if !before.is_empty() {
        if before.ends_with("\n\n") {
            // Already at the start of a fresh paragraph.
        } else if before.ends_with('\n') {
            new_text.push('\n');
        } else {
            new_text.push_str("\n\n");
        }
    }
    new_text.push_str(&reference);
    // The reference needs its own line; text that follows already starts one.
    if after.is_empty() || !after.starts_with('\n') {
        new_text.push('\n');
    }

    let caret = start + new_text.len();
    Edit {
        range: start..end,
        new_text,
        new_selection: Selection::caret(caret),
    }
}

/// Items offered by the slash menu. Every item applies without the user
/// typing any Markdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlashItem {
    Heading1,
    Heading2,
    Heading3,
    Bullet,
    Ordered,
    Task,
    Quote,
    CodeBlock,
    Divider,
    Link,
    Image,
}

impl SlashItem {
    pub const ALL: [SlashItem; 11] = [
        SlashItem::Heading1,
        SlashItem::Heading2,
        SlashItem::Heading3,
        SlashItem::Bullet,
        SlashItem::Ordered,
        SlashItem::Task,
        SlashItem::Quote,
        SlashItem::CodeBlock,
        SlashItem::Divider,
        SlashItem::Link,
        SlashItem::Image,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SlashItem::Heading1 => "Heading 1",
            SlashItem::Heading2 => "Heading 2",
            SlashItem::Heading3 => "Heading 3",
            SlashItem::Bullet => "Bullet list",
            SlashItem::Ordered => "Numbered list",
            SlashItem::Task => "Task",
            SlashItem::Quote => "Quote",
            SlashItem::CodeBlock => "Code block",
            SlashItem::Divider => "Divider",
            SlashItem::Link => "Link",
            SlashItem::Image => "Image",
        }
    }

    /// Whether `label` matches the typed query (case-insensitive substring).
    pub fn matches(self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty() || self.label().to_lowercase().contains(&query)
    }
}

/// Applies a slash item over the line holding `query_start`: any typed
/// `/query` is removed and the block/insert it names is applied in one edit.
pub fn apply_slash(text: &str, caret: usize, query_start: usize, item: SlashItem) -> Edit {
    let caret = caret.min(text.len());
    let query_start = query_start.min(caret);
    let line_start = text[..query_start]
        .rfind('\n')
        .map(|ix| ix + 1)
        .unwrap_or(0);
    let line = &text[line_start..query_start];
    let content = strip_block_prefix(line);
    let content = content.trim();

    let (new_text, caret_offset) = match item {
        SlashItem::Heading1
        | SlashItem::Heading2
        | SlashItem::Heading3
        | SlashItem::Bullet
        | SlashItem::Ordered
        | SlashItem::Task
        | SlashItem::Quote => {
            let format = match item {
                SlashItem::Heading1 => BlockFormat::Heading(1),
                SlashItem::Heading2 => BlockFormat::Heading(2),
                SlashItem::Heading3 => BlockFormat::Heading(3),
                SlashItem::Bullet => BlockFormat::Bullet,
                SlashItem::Ordered => BlockFormat::Ordered,
                SlashItem::Task => BlockFormat::Task,
                SlashItem::Quote => BlockFormat::Quote,
                _ => unreachable!(),
            };
            let prefix = match format {
                BlockFormat::Heading(level) => format!("{} ", "#".repeat(level as usize)),
                BlockFormat::Bullet => "- ".to_string(),
                BlockFormat::Ordered => "1. ".to_string(),
                BlockFormat::Task => "- [ ] ".to_string(),
                BlockFormat::Quote => "> ".to_string(),
            };
            let line = format!("{prefix}{content}");
            let caret = line.len();
            (line, caret)
        }
        SlashItem::CodeBlock => {
            let text = if content.is_empty() {
                "```\n\n```\n".to_string()
            } else {
                format!("{content}\n```\n\n```\n")
            };
            let fence_start = text.len() - "```\n\n```\n".len();
            (text, fence_start + 4)
        }
        SlashItem::Divider => {
            let text = if content.is_empty() {
                "---\n".to_string()
            } else {
                format!("{content}\n\n---\n")
            };
            let caret = text.len();
            (text, caret)
        }
        SlashItem::Link => ("[]()".to_string(), 3),
        SlashItem::Image => ("![]()".to_string(), 5),
    };

    Edit {
        range: line_start..caret,
        new_text,
        new_selection: Selection::caret(line_start + caret_offset),
    }
}

/// Items matching a slash query, favorites first.
pub fn slash_items(query: &str) -> Vec<SlashItem> {
    SlashItem::ALL
        .into_iter()
        .filter(|item| item.matches(query))
        .collect()
}

/// Removes an enclosing delimiter pair around a selection: `open` and `close`
/// are the marker ranges (e.g. of a parsed strong construct).
pub fn unwrap_delimiters(
    text: &str,
    selection: Selection,
    open: Range<usize>,
    close: Range<usize>,
) -> Edit {
    Edit {
        range: open.start..close.end,
        new_text: text[open.end..close.start].to_string(),
        new_selection: shift_selection(selection, -(open.len() as isize)),
    }
}

/// The whitespace prefix of the current line (used for fence indentation).
fn line_prefix(before: &str) -> String {
    let line_start = before.rfind('\n').map(|ix| ix + 1).unwrap_or(0);
    before[line_start..]
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .collect()
}

/// Moves a selection by `delta` bytes, preserving direction.
fn shift_selection(selection: Selection, delta: isize) -> Selection {
    let shift = |offset: usize| (offset as isize + delta).max(0) as usize;
    Selection::new(shift(selection.anchor()), shift(selection.head()))
}

#[cfg(test)]
mod tests {
    use super::{
        BlockFormat, Edit, InlineFormat, insert_code_fence, insert_rule, toggle_block,
        toggle_inline,
    };
    use crate::input::Selection;

    fn apply(text: &str, edit: &Edit) -> String {
        let mut output = text.to_string();
        output.replace_range(edit.range.clone(), &edit.new_text);
        output
    }

    /// True when the letters of every word in `before` three letters or
    /// longer still run in order through `after`. Toggles may add, remove or
    /// renumber markup — `#`, `- [ ]`, `1. `, `**` — but none may drop
    /// written prose.
    fn words_preserved(before: &str, after: &str) -> bool {
        let written: Vec<char> = before
            .split(|ch: char| !ch.is_alphabetic())
            .filter(|word| word.chars().count() >= 3)
            .flat_map(|word| word.chars())
            .collect();
        let mut survivors = after.chars().filter(|ch| ch.is_alphabetic());
        written
            .into_iter()
            .all(|letter| survivors.any(|survivor| survivor == letter))
    }

    #[test]
    fn toggles_never_drop_words() {
        use super::{insert_code_fence, insert_rule};

        let documents = [
            "hello world\n",
            "## Code\n\n```rust\nfn main() {}\n```\n\nafter\n",
            "> a\n>\n> b\n",
            "> **bold** and `code`\n",
            "> > nested quote\n",
            "- one\n- two\n",
            "1. one\n2. two\n",
            "- [ ] task\n- [x] done\n",
            "a ==swipe== b\n",
            "```\ncode only\n```\n",
        ];
        let inline = [
            InlineFormat::Bold,
            InlineFormat::Italic,
            InlineFormat::Strike,
            InlineFormat::Highlight,
            InlineFormat::Code,
        ];
        let block = [
            BlockFormat::Heading(1),
            BlockFormat::Heading(2),
            BlockFormat::Heading(3),
            BlockFormat::Bullet,
            BlockFormat::Ordered,
            BlockFormat::Task,
            BlockFormat::Quote,
        ];
        for text in documents {
            let line_starts: Vec<usize> = std::iter::once(0)
                .chain(text.match_indices('\n').map(|(ix, _)| ix + 1))
                .filter(|ix| *ix <= text.len())
                .collect();
            for start in line_starts.iter().copied() {
                let ends = [
                    start,
                    (start + 1).min(text.len()),
                    (start + 4).min(text.len()),
                    text.len(),
                ];
                for end in ends {
                    if !text.is_char_boundary(start) || !text.is_char_boundary(end) || end < start {
                        continue;
                    }
                    let selection = Selection::new(start, end.max(start));
                    for format in inline {
                        if let Some(edit) = toggle_inline(text, selection, format) {
                            assert!(
                                words_preserved(text, &apply(text, &edit)),
                                "{format:?} over {selection:?} in {text:?} dropped words: {:?}",
                                apply(text, &edit)
                            );
                        }
                    }
                    for format in block {
                        if let Some(edit) = toggle_block(text, selection, format) {
                            assert!(
                                words_preserved(text, &apply(text, &edit)),
                                "{format:?} over {selection:?} in {text:?} dropped words: {:?}",
                                apply(text, &edit)
                            );
                        }
                    }
                    if let Some(edit) = insert_code_fence(text, selection) {
                        assert!(
                            words_preserved(text, &apply(text, &edit)),
                            "fence over {selection:?} in {text:?} dropped words: {:?}",
                            apply(text, &edit)
                        );
                    }
                    if let Some(edit) = insert_rule(text, selection) {
                        assert!(
                            words_preserved(text, &apply(text, &edit)),
                            "rule over {selection:?} in {text:?} dropped words: {:?}",
                            apply(text, &edit)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn bold_wraps_and_unwraps_by_selection() {
        let text = "hello world";
        let wrapped = toggle_inline(text, Selection::new(0, 5), InlineFormat::Bold).unwrap();
        assert_eq!(apply(text, &wrapped), "**hello** world");
        assert_eq!(wrapped.new_selection, Selection::new(2, 7));

        let again =
            toggle_inline("**hello** world", wrapped.new_selection, InlineFormat::Bold).unwrap();
        assert_eq!(apply("**hello** world", &again), "hello world");
        assert_eq!(again.new_selection, Selection::new(0, 5));
    }

    #[test]
    fn caret_directly_between_delimiters_unwraps() {
        let edit = toggle_inline("****", Selection::caret(2), InlineFormat::Bold).unwrap();
        assert_eq!(apply("****", &edit), "");
        assert_eq!(edit.new_selection, Selection::caret(0));
    }

    #[test]
    fn collapsed_caret_gets_a_pair() {
        let edit = toggle_inline("ab", Selection::caret(1), InlineFormat::Italic).unwrap();
        assert_eq!(apply("ab", &edit), "a**b");
        assert_eq!(edit.new_selection, Selection::caret(2));
    }

    #[test]
    fn wrapping_excludes_trailing_whitespace() {
        let text = "hello ";
        let edit = toggle_inline(text, Selection::new(0, 6), InlineFormat::Bold).unwrap();
        assert_eq!(apply(text, &edit), "**hello** ");
        assert_eq!(edit.new_selection, Selection::new(2, 7));
    }

    #[test]
    fn nested_toggles_wrap_only_their_delimiters() {
        let text = "**bold and bold**";
        let selection = Selection::new(2, 6);
        let edit = toggle_inline(text, selection, InlineFormat::Italic).unwrap();
        assert_eq!(apply(text, &edit), "***bold* and bold**");
        let inner = Selection::new(3, 7);
        let undo = toggle_inline(&apply(text, &edit), inner, InlineFormat::Italic).unwrap();
        assert_eq!(apply(&apply(text, &edit), &undo), text);
    }

    #[test]
    fn reversed_selections_keep_direction() {
        let text = "hello";
        let selection = Selection::new(5, 0);
        let edit = toggle_inline(text, selection, InlineFormat::Bold).unwrap();
        assert_eq!(apply(text, &edit), "**hello**");
        assert!(edit.new_selection.reversed());
        assert_eq!(edit.new_selection, Selection::new(7, 2));
    }

    #[test]
    fn block_toggle_adds_and_removes_on_all_lines() {
        let text = "one\ntwo\nthree";
        let selection = Selection::new(0, 3);
        let edit = toggle_block(text, selection, BlockFormat::Bullet).unwrap();
        assert_eq!(apply(text, &edit), "- one\ntwo\nthree");

        let both = Selection::new(0, 7);
        let edit = toggle_block(text, both, BlockFormat::Bullet).unwrap();
        assert_eq!(apply(text, &edit), "- one\n- two\nthree");

        let all = Selection::new(0, text.len());
        let edit = toggle_block("one\ntwo", all, BlockFormat::Quote).unwrap();
        assert_eq!(apply("one\ntwo", &edit), "> one\n> two");
        let selection = Selection::new(0, 11);
        let off = toggle_block("> one\n> two", selection, BlockFormat::Quote).unwrap();
        assert_eq!(apply("> one\n> two", &off), "one\ntwo");
    }

    #[test]
    fn block_conversions_swap_prefixes() {
        let text = "- one\n- two";
        let selection = Selection::new(0, 11);
        let edit = toggle_block(text, selection, BlockFormat::Task).unwrap();
        assert_eq!(apply(text, &edit), "- [ ] one\n- [ ] two");

        let text = "- [ ] one\n- [ ] two";
        let selection = Selection::new(0, 17);
        let edit = toggle_block(text, selection, BlockFormat::Ordered).unwrap();
        assert_eq!(apply(text, &edit), "1. one\n2. two");

        let text = "1. one\n2. two";
        let selection = Selection::new(0, 13);
        let edit = toggle_block(text, selection, BlockFormat::Heading(2)).unwrap();
        assert_eq!(apply(text, &edit), "## one\n## two");

        let text = "## one\n## two";
        let selection = Selection::new(0, 13);
        let edit = toggle_block(text, selection, BlockFormat::Heading(2)).unwrap();
        assert_eq!(apply(text, &edit), "one\ntwo");
    }

    #[test]
    fn block_toggle_skips_blank_lines() {
        let text = "one\n\ntwo";
        let selection = Selection::new(0, 8);
        let edit = toggle_block(text, selection, BlockFormat::Quote).unwrap();
        assert_eq!(apply(text, &edit), "> one\n\n> two");
    }

    #[test]
    fn code_fence_wraps_or_inserts() {
        let text = "let x = 1;";
        let edit = insert_code_fence(text, Selection::new(0, 10)).unwrap();
        assert_eq!(apply(text, &edit), "```\nlet x = 1;\n```");
        assert_eq!(edit.new_selection, Selection::new(4, 14));

        let edit = insert_code_fence("", Selection::caret(0)).unwrap();
        assert_eq!(apply("", &edit), "```\n\n```\n");
        assert_eq!(edit.new_selection, Selection::caret(4));
    }

    #[test]
    fn link_and_image_inserts() {
        let text = "the docs";
        let edit = super::insert_link(text, Selection::new(4, 8), "https://example.com");
        assert_eq!(apply(text, &edit), "the [docs](https://example.com)");
        assert_eq!(edit.new_selection, Selection::caret(31));

        let edit = super::insert_link("", Selection::caret(0), "  https://x.dev  ");
        assert_eq!(apply("", &edit), "[https://x.dev](https://x.dev)");

        // Empty URL: caret lands inside the parentheses.
        let edit = super::insert_link("hello", Selection::new(0, 5), "");
        assert_eq!(apply("hello", &edit), "[hello]()");
        assert_eq!(edit.new_selection, Selection::caret(8));

        let edit = super::insert_image(Selection::caret(0), "shot", "assets/shot.png");
        assert_eq!(apply("", &edit), "![shot](assets/shot.png)");
    }

    #[test]
    fn insert_media_places_the_reference_on_its_own_paragraph() {
        // Empty document: the reference and a terminalline break.
        let edit = super::insert_media("", Selection::caret(0), "a", "assets/a.mp3");
        assert_eq!(apply("", &edit), "![a](assets/a.mp3)\n");
        assert_eq!(edit.new_selection, Selection::caret(19));

        // Mid-text: a paragraph break on both sides.
        let text = "hello";
        let edit = super::insert_media(text, Selection::caret(5), "a", "assets/a.mp3");
        assert_eq!(apply(text, &edit), "hello\n\n![a](assets/a.mp3)\n");

        // Already after a blank line: no extra leading break.
        let text = "hello\n\n";
        let edit = super::insert_media(text, Selection::caret(7), "a", "assets/a.mp3");
        assert_eq!(apply(text, &edit), "hello\n\n![a](assets/a.mp3)\n");

        // At the start of a line with content below: one break before, one
        // newline after.
        let text = "hello\nworld";
        let edit = super::insert_media(text, Selection::caret(6), "a", "assets/a.mp3");
        assert_eq!(apply(text, &edit), "hello\n\n![a](assets/a.mp3)\nworld");

        // Replacing a selection keeps the reference block-level.
        let text = "one two three";
        let edit = super::insert_media(text, Selection::new(4, 7), "a", "assets/a.mp3");
        assert_eq!(apply(text, &edit), "one \n\n![a](assets/a.mp3)\n three");
    }

    #[test]
    fn slash_blocks_apply_without_markdown_syntax() {
        // "/bul" typed on an empty line → a bullet item with the query gone.
        let text = "/bul";
        let edit = super::apply_slash(text, 4, 0, super::SlashItem::Bullet);
        assert_eq!(apply(text, &edit), "- ");
        assert_eq!(edit.new_selection, Selection::caret(2));

        // Swaps an existing block prefix: "## /quo" → "> ".
        let text = "## /quo";
        let edit = super::apply_slash(text, 7, 3, super::SlashItem::Quote);
        assert_eq!(apply(text, &edit), "> ");

        // Typed content is kept.
        let text = "my title /h1";
        let edit = super::apply_slash(text, 12, 9, super::SlashItem::Heading1);
        assert_eq!(apply(text, &edit), "# my title");
    }

    #[test]
    fn slash_inserts_code_block_divider_link_and_image() {
        let edit = super::apply_slash("/code", 5, 0, super::SlashItem::CodeBlock);
        assert_eq!(apply("/code", &edit), "```\n\n```\n");
        assert_eq!(
            edit.new_selection,
            Selection::caret(4),
            "caret inside the fence"
        );

        let text = "intro\n```\n\n```\n";
        let edit = super::apply_slash("intro /c", 8, 6, super::SlashItem::CodeBlock);
        assert_eq!(apply("intro /c", &edit), text);
        assert_eq!(
            edit.new_selection,
            Selection::caret(10),
            "inside the empty fence line"
        );

        let edit = super::apply_slash("/div", 4, 0, super::SlashItem::Divider);
        assert_eq!(apply("/div", &edit), "---\n");

        let edit = super::apply_slash("/link", 5, 0, super::SlashItem::Link);
        assert_eq!(apply("/link", &edit), "[]()");
        assert_eq!(
            edit.new_selection,
            Selection::caret(3),
            "inside the parentheses"
        );

        let edit = super::apply_slash("/img", 4, 0, super::SlashItem::Image);
        assert_eq!(apply("/img", &edit), "![]()");
        assert_eq!(edit.new_selection, Selection::caret(5));
    }

    #[test]
    fn slash_items_filter_by_label() {
        let items = super::slash_items("");
        assert_eq!(items.len(), super::SlashItem::ALL.len());
        let items = super::slash_items("hea");
        assert_eq!(
            items,
            vec![
                super::SlashItem::Heading1,
                super::SlashItem::Heading2,
                super::SlashItem::Heading3
            ]
        );
        assert!(super::slash_items("zzz").is_empty());
    }

    #[test]
    fn rule_inserts_on_its_own_line() {
        let text = "para";
        let edit = insert_rule(text, Selection::caret(2)).unwrap();
        assert_eq!(apply(text, &edit), "para\n\n---\n");

        let edit = insert_rule("", Selection::caret(0)).unwrap();
        assert_eq!(apply("", &edit), "---\n");
    }
}
