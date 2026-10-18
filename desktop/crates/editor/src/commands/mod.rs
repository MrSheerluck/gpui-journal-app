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
/// uses the URL as the label so the result is immediately useful.
pub fn insert_link(text: &str, selection: Selection, url: &str) -> Edit {
    let url = url.trim();
    let (start, end) = (selection.start(), selection.end());
    let label = if selection.is_caret() {
        url
    } else {
        &text[start..end]
    };
    let new_text = format!("[{label}]({url})");
    let caret = start + new_text.len();
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

        let edit = super::insert_image(Selection::caret(0), "shot", "assets/shot.png");
        assert_eq!(apply("", &edit), "![shot](assets/shot.png)");
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
