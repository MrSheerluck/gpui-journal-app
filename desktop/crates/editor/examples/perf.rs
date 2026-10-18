//! Baseline measurement harness for the pure editor pipeline.
//!
//! Run in release mode:
//!   cargo run --release --example perf
//!
//! Reports, for a ~10k-char mixed-construct document and a 2,000-line
//! document: full text copy, tree-sitter index rebuild and display-plan
//! construction — the three O(document) costs on every keystroke.

use std::time::Instant;

use dainiki_editor::buffer::Buffer;
use dainiki_editor::display::DisplayPlan;
use dainiki_editor::input::Selection;
use dainiki_editor::markdown::MarkdownIndex;
use tree_sitter::Parser;

fn sample_10k() -> String {
    let mut text = String::new();
    let mut n = 0;
    while text.len() < 10_000 {
        n += 1;
        text.push_str(&format!(
            "## Section {n}\n\nA journal paragraph with **bold**, *italic*, `code`, ==highlight==, ~~strike~~ and a [link](https://example.com/{n}) inside it. The quick brown fox jumps over the lazy dog. 日本語のテキストも含まれています。\n\n- [ ] task {n}\n- [x] done {n}\n\n> quoted line {n}\n\n"
        ));
    }
    text
}

fn sample_2000_lines() -> String {
    let mut text = String::new();
    for n in 0..2_000 {
        text.push_str(&format!(
            "Line {n} with some **bold** text and a longer trailing sentence to wrap naturally.\n\n"
        ));
    }
    text
}

fn time<T>(label: &str, iterations: u32, mut f: impl FnMut() -> T) -> f64 {
    // Warm up.
    f();
    let start = Instant::now();
    for _ in 0..iterations {
        std::hint::black_box(f());
    }
    let per_op = start.elapsed().as_secs_f64() * 1000.0 / f64::from(iterations);
    println!("{label:<44} {per_op:>8.3} ms");
    per_op
}

fn main() {
    for (name, text) in [
        ("10k chars", sample_10k()),
        ("2,000 lines", sample_2000_lines()),
    ] {
        println!(
            "\n=== {name} ({} bytes, {} lines) ===",
            text.len(),
            text.lines().count()
        );

        time("content(): full rope copy", 200, || {
            let buffer = Buffer::from_text(&text);
            buffer.slice(0..buffer.len_bytes())
        });

        let mut index = MarkdownIndex::new();
        time("index.update(): full reparse", 100, || {
            index.update(&text);
        });

        let mut block_parser = Parser::new();
        block_parser
            .set_language(&tree_sitter_md::LANGUAGE.into())
            .expect("block grammar");
        let mut inline_parser = Parser::new();
        inline_parser
            .set_language(&tree_sitter_md::INLINE_LANGUAGE.into())
            .expect("inline grammar");
        time("  block grammar only", 100, || {
            block_parser.parse(&text, None)
        });
        {
            // Incremental block parse variants.
            let mut doc = text.clone();
            let mid = doc.len() / 2;
            let mut tree = block_parser.parse(&doc, None).expect("tree");
            let point = |text: &str, offset: usize| {
                let row = text[..offset].bytes().filter(|b| *b == b'\n').count();
                let line_start = text[..offset].rfind('\n').map(|ix| ix + 1).unwrap_or(0);
                tree_sitter::Point {
                    row,
                    column: offset - line_start,
                }
            };
            time("  block incremental + edit()", 100, || {
                let edit = tree_sitter::InputEdit {
                    start_byte: mid,
                    old_end_byte: mid,
                    new_end_byte: mid + 1,
                    start_position: point(&doc, mid),
                    old_end_position: point(&doc, mid),
                    new_end_position: point(&doc, mid + 1),
                };
                doc.insert(mid, 'x');
                tree.edit(&edit);
                tree = block_parser.parse(&doc, Some(&tree)).expect("tree");
                doc.remove(mid);
            });
            let mut doc = text.clone();
            let mut tree = block_parser.parse(&doc, None).expect("tree");
            time("  block diff-parse (no edit())", 100, || {
                doc.insert(mid, 'x');
                tree = block_parser.parse(&doc, Some(&tree)).expect("tree");
                doc.remove(mid);
            });
        }
        time("  inline reparses only", 100, || {
            let tree = block_parser.parse(&text, None).expect("tree");
            let mut regions = 0usize;
            let mut cursor = tree.walk();
            let mut stack = vec![tree.root_node()];
            while let Some(node) = stack.pop() {
                if node.kind() == "inline" {
                    regions += 1;
                    let source = &text[node.start_byte()..node.end_byte()];
                    let _ = inline_parser.parse(source, None);
                    continue;
                }
                for child in node.children(&mut cursor) {
                    stack.push(child);
                }
            }
            std::hint::black_box(regions);
        });

        index.update(&text);
        time("DisplayPlan::compute()", 200, || {
            DisplayPlan::compute(&text, &index, Selection::caret(0))
        });

        let mut incremental = MarkdownIndex::new();
        let mut doc = text.clone();
        incremental.update(&doc);
        let mid = doc.len() / 2;
        time("index.update_edited(): 1-char edit", 200, || {
            doc.insert(mid, 'x');
            incremental.update_edited(&doc, mid..mid, 1);
            doc.remove(mid);
            incremental.update_edited(&doc, mid..mid + 1, 0);
        });

        let plan = DisplayPlan::compute(&text, &index, Selection::caret(0));
        time("plan lookups: 10 caret placements", 200, || {
            for position in 0..10 {
                let source = position * (text.len() / 10);
                if let Some(line_ix) = plan.line_for_source(source)
                    && let Some(line) = plan.lines.get(line_ix)
                {
                    let _ = line.display_for_source(source.min(line.source.end));
                    let _ = line.source_for_display(0);
                }
            }
        });
    }
}
