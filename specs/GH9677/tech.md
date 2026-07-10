# Tech Spec: Render LaTeX math (`$...$` / `$$...$$`) in AI agent block output

**Issue:** [warpdotdev/warp#9677](https://github.com/warpdotdev/warp/issues/9677)
**Product spec:** [product.md](product.md)

## Context

Relevant existing pipeline (all line references at time of writing):

- **Shared markdown model:** `crates/markdown_parser/src/lib.rs` — inline content is `Vec<FormattedTextFragment>`; fragment styling is the flat `FormattedTextStyles` struct. The hand-rolled inline tokenizer lives in `crates/markdown_parser/src/markdown_parser.rs` (`parse_inline_token` alt-chain; code spans and backslash escapes have highest precedence).
- **AI agent output sectioning:** `app/src/ai/agent/util.rs` (`parse_markdown_into_text_and_code_sections`) splits streamed markdown into `AIAgentTextSection`s (`app/src/ai/agent/mod.rs`) with a line-based state machine that already withholds unterminated code fences.
- **Visual block rendering:** `app/src/ai/blocklist/block/view_impl/common.rs` renders sections; Mermaid diagrams (`render_mermaid_diagram_section`) are the precedent for "source → SVG → image" blocks, using `AssetSource::Async` (fetch memoized per asset id) → SVG bytes → `ImageType::Svg` (`usvg`) → `resvg` raster → `WarpImage`.
- **Mermaid asset glue:** `crates/editor/src/content/mermaid_diagram.rs` (`mermaid_asset_source`).

There is no existing math/KaTeX/MathML machinery in the codebase.

## Proposed changes

### 1. Parser: math spans in the shared markdown model

- `crates/markdown_parser/src/lib.rs`: new `MathMode { Inline, Display }`; new `FormattedTextStyles.math: Option<MathMode>` field; `FormattedTextFragment::math(latex, mode)` constructor. A math fragment's `text` is the raw LaTeX source (no delimiters), so `raw_text()`, copy, and any renderer unaware of math degrade to exactly today's behavior. `inline_to_markdown` round-trips math fragments back to `$...$`/`$$...$$`.
- `crates/markdown_parser/src/markdown_parser.rs`: new `InlineToken::Math` and `parse_math_span`, placed in the `parse_inline_token` alt-chain immediately after `code_span` (code wins; `\$` is consumed earlier by `parse_escape`). Pandoc `tex_math_dollars` rules: opener followed by non-whitespace; closer preceded by non-whitespace and not followed by a digit; backslash-escaped `$` inside a span does not close it; no valid closer → the parser fails and the `$` falls through to `unmatched_char` (literal text), which also yields streaming-safe behavior for free.

### 2. Typesetter: new `crates/math_render`

Thin wrapper over the RaTeX crates (`ratex-parser`/`ratex-layout`/`ratex-svg`, MIT-licensed, >99% KaTeX syntax coverage): `render_math_to_svg(latex, display, color, font_size) -> Result<String>`. Built with `standalone` + `embed-fonts` features so glyphs are emitted as SVG `<path>` outlines — the output is self-contained and renders through the existing `usvg`/`resvg` stack with no font database or network dependency. Errors are surfaced as `Result` so callers can fall back to raw source.

### 3. Asset glue: `crates/editor/src/content/math_block.rs`

`math_asset_source(latex, display, color, font_size) -> AssetSource` mirroring `mermaid_asset_source`. The asset id hashes latex + display + color + font size, so theme or font-size changes produce a new id (fresh render) instead of a stale wrongly-colored cache hit.

### 4. AI agent sections

- `app/src/ai/agent/mod.rs`: new `AgentOutputMath { latex, markdown_source }` and `AIAgentTextSection::Math` variant. All exhaustive matches extended (`is_empty`, `MarkdownTextSection` Display, copy formatting, `find.rs`, `link_detection.rs`, SDK driver output, block state-handle collection) — math behaves like Mermaid: its markdown source is used for copy/find/CLI output.
- `app/src/ai/agent/util.rs`: the section splitter recognizes a lone `$$` line (opens a multi-line math state, closed by the next lone `$$`) and whole-line `$$...$$`. EOF with an unterminated block flushes the raw lines back to plain text, so mid-stream content stays literal until the closing fence arrives (mirrors the code-fence state machine). Inline `$...$` stays inside `PlainText` sections and is handled by the parser (change 1).

### 5. Rendering

`app/src/ai/blocklist/block/view_impl/common.rs`: `render_math_section` reuses `render_visual_markdown_block` (placeholder while loading → `WarpImage` with `FitWidth` sizing after load, centered). The SVG is generated at `monospace_font_size × 1.2` per em in the block's `text_color`, so equations match the theme and scale with the user's font size. `AssetState::FailedToLoad` → `render_visual_markdown_fallback` (raw source, monospace). Unlike Mermaid there is no card chrome or background canvas — an equation renders as a clean centered block.

### Tradeoffs

- **RaTeX vs. KaTeX-WASM vs. latex2mathml:** RaTeX is pure Rust (no JS runtime), emits self-contained SVG usable by the existing image pipeline, and tracks KaTeX coverage. latex2mathml would still need a MathML renderer, which the codebase lacks.
- **SVG-image blocks vs. native glyph runs:** the image pipeline gives correct typesetting immediately with theming and caching; inline (in-text-line) math needs baseline-aligned inline images or native glyph integration and is deferred (see product spec non-goals).
- **Sectioning at the splitter vs. the parser:** display math follows the same architecture as tables/Mermaid/code (extracted before `parse_markdown`), keeping the streaming "only complete blocks" logic in one place.

## Testing and validation

- **Parser invariants (product spec 3–7):** unit tests in `crates/markdown_parser/src/markdown_parser_tests.rs` — inline/display parsing, currency non-math, escaped dollars, code-span/code-block precedence, unclosed spans, empty math, tables/headers/lists containing math.
- **Splitter invariants (1, 2, 5, 6, 7):** unit tests in `app/src/ai/agent/util_tests.rs` — multi-line and single-line display blocks, unterminated-block fallback (streaming), `$$` inside code fences, empty math, inline math staying in plain text.
- **Typesetter:** unit tests in `crates/math_render` — SVG output with path glyphs, parse-error and invalid-color fallbacks.
- **Invariants 8–10 (fallback, theming, copy):** manual testing with screenshots/recording in the PR — agent responses containing display math in light and dark themes, invalid LaTeX fallback, copy yielding source.
- **Invariant 12 (no regressions):** full existing suites (`cargo nextest run`) pass unchanged.
