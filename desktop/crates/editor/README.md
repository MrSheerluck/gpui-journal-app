# dainiki-editor

Dainiki's live-preview Markdown editor: an Obsidian-style editing surface built
directly on GPUI primitives, with zero dependency on any GPUI application
framework.

## Why this crate exists

Dainiki edits Markdown files in place, with the source as the single source of
truth (see `docs/decisions/0001-editor-data-model.md`). Off-the-shelf GPUI text
components either reject inline decoration in editor mode or hide their layout
behind private APIs, so the editor surface is owned here and built on the raw
GPUI snapshot (see `docs/decisions/0005-editor-surface-gpui-primitives.md`).

## Boundary rule (enforced)

`dainiki-editor` depends on:

- `gpui-pre` (the pinned GPUI snapshot, `=0.3.9`), `ropey`, `tree-sitter`,
  `tree-sitter-md`, `unicode-segmentation`, `image`, `tracing` — and nothing
  else.

It must **never** depend on `gpui-kit`, `gpui-base` or `gpui-component`; those
are consumed only by the `dainiki-desktop` app for chrome and the reading view.
CI checks this with:

```sh
cargo tree -p dainiki-editor | grep -E 'gpui-kit|gpui-base|gpui-component' && exit 1 || true
```

## Architecture (one direction, all derived state)

```text
source (Rope) ──► MarkdownIndex (tree-sitter constructs, byte ranges)
      │                    │
      │         selection  ▼
      └──────► DisplayPlan = Vec<ProjectedLine>     [pure function of
                     │   fragments: Visible/Hidden/Widget   (text, index, selection)]
                     ▼
               shaped lines (TextSystem::shape_line, per-line height)
                     ▼
               EditorElement paints runs and widgets; owns caret, IME, scroll
```

- `buffer/` — the rope-backed text: byte-offset accessors, UTF-16 conversion,
  replace/slice. Pure.
- `markdown/` — the construct index: block tree-sitter parse plus per-region
  inline re-parses, with incremental edits (`tree.edit` + per-region caches).
  Pure.
- `display/` — the projection: visible text, hidden markup (as explicit
  zero-width fragments, so caret mapping stays exact), widget stand-ins
  (bullets, checkboxes, images) and style runs. Pure.
- `commands/` — formatting transforms `(text, selection) -> Edit`. Pure.
- `input/` — the GPUI side: the custom `Element`, caret/selection geometry,
  scroll state, undo history. Thin by design.
- `editor.rs` — the `MarkdownEditor` facade: events, input handling (keys, IME,
  mouse), command application, image loading.

## Performance

Measured with `cargo run --release --example perf`; results and methodology in
`docs/benchmarks/editor-index.md`. Headline: a keystroke on a 10k-character
entry costs ~1.4 ms across the index, projection and lookups (budget: 16 ms).

## Testing

- Unit tests across the pure layers (movement, commands, projection, index,
  history, buffer).
- Differential test proving incremental index updates equal full reparses
  across edit shapes.
- `#[gpui::test]` interaction tests: typing, arrows, clicks, shortcuts,
  undo/redo, CRLF, empty docs, clipboard, incremental display flow.

## Attribution

No code has been copied from `gpui-kit` or other projects; behavior follows
Zed's GPUI conventions through its public API. See `NOTICE` for dependency
licenses.
