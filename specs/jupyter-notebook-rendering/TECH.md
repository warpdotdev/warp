# Jupyter Notebook Rendering — TECH

Render `.ipynb` (Jupyter) files as formatted notebooks instead of raw JSON. **Render-only**: no kernel, no cell execution, no editing of outputs. Outputs are display-only (text, tracebacks, and embedded images). Behind a feature flag.

## Context
Warp already has a non-code "rendered file" surface, `FileNotebookView` (`app/src/notebooks/file/mod.rs`), which is misleadingly named — "Notebook" refers to Warp Drive rich-text notebooks, not Jupyter. It loads a file's text and feeds it straight to the markdown parser via `reset_with_markdown` (`crates/editor/src/model.rs:899`). The same renderer already supports images: `![alt](src)` serializes to `<img>` (`crates/editor/src/content/markdown_tests.rs:264`), and base64 data-URIs are used today for Mermaid (`app/src/notebooks/editor/model.rs:137`).

Two things block `.ipynb` rendering today:

1. **Routing.** `FileNotebookView` is only reached via `FileTarget::MarkdownViewer`, which `resolve_file_target_with_editor_choice` (`app/src/util/openable_file_type.rs:196`) returns only when `is_markdown_file` is true. That predicate (`crates/warp_util/src/file_type.rs:130`) matches only `md`/`markdown`. An `.ipynb` is reported as `application/json`, classified `OpenableFileType::Text` (`openable_file_type.rs:143`), and routed to `FileTarget::CodeEditor` → opens as raw JSON in `CodeView`.
2. **Content handling.** Even if routed there, `FileNotebookView::set_content` (`app/src/notebooks/file/mod.rs:329`, and the remote path at `:630`) passes raw bytes to `reset_with_markdown`. For an `.ipynb` that's JSON, so it would render JSON-as-markdown, not cells.

Relevant existing wiring this feature reuses:
- Open flow: `Workspace::open_file_with_target` `FileTarget::MarkdownViewer` arm (`app/src/workspace/view.rs:5986`) → `open_file_notebook` → `FilePane::new` (`app/src/pane_group/pane/file_pane.rs:43`) → `FileNotebookView`.
- Rendered⇄Raw toggle: `FileNotebookView` "Raw" emits `ReplaceWithCodePane` (`notebooks/file/mod.rs:1069`); `CodeView` "Rendered" (`CodeViewAction::RenderMarkdown`, `code/view.rs:2249`) emits `ReplaceWithFilePane`. Both are gated on `is_markdown_file` (`code/view.rs:284`, `:2069`; `notebooks/file/mod.rs:702`, `:1165`).

## Proposed changes
The only substantial new code is an `.ipynb` → `FormattedText` converter (the new `ipynb_parser` crate); everything else is detection + a conversion hook in the editor + extending the "is renderable" predicate.

**1. Detection — `crates/warp_util/src/file_type.rs`**
Add `is_jupyter_notebook_file(path)` matching the `ipynb` extension (sibling to `is_markdown_file`).

**2. Routing — `app/src/util/openable_file_type.rs`**
- **Decision (revised during implementation):** do *not* add an `OpenableFileType::JupyterNotebook` variant. That variant is matched exhaustively across many file-classification surfaces (the "Open in Warp" banner in `app/src/terminal/view/open_in_warp.rs`, menu/banner/workspace code), so a new variant would force churn across all of them and widen the blast radius for a flag-gated v1. `.ipynb` keeps its current classification (`OpenableFileType::Text`) so it stays openable and, with the flag off, behaves exactly as today. The terminal "Open in Warp" banner (`app/src/terminal/view/open_in_warp.rs`) was switched from an `OpenableFileType` match to the `renders_in_warp_notebook_viewer(path)` helper, so with the flag on it now also suggests `.ipynb` and opens it in the notebook viewer; with the flag off the helper returns false and the banner behaves exactly as today.
- Instead, gate the render decision at the single chokepoint that chooses between the notebook viewer and the code editor: in `resolve_file_target_with_editor_choice` (and `resolve_file_target_to_open_in_warp`), when `is_jupyter_notebook_file(path)` is true, the file is openable, and the feature flag is enabled, return `FileTarget::MarkdownViewer(layout)` **unconditionally** (i.e. not gated on `prefer_markdown_viewer` or `editor_choice`, since rendering-instead-of-JSON is the whole point). When the flag is off, fall through to today's behavior (opens as JSON).
- Add a small `renders_in_warp_notebook_viewer(path)` helper (markdown OR flag-enabled jupyter) used by the Rendered/Raw toggle gates in step 5 and the view-header gate in step 4.

**3. Converter — `crates/ipynb_parser` (new crate)**
Pure, no UI deps, unit-testable in isolation. Rather than emitting a Markdown string that is re-parsed, it builds the renderer's `FormattedText` directly so untrusted notebook data (language tags, image payloads) lives in struct fields and can never break out of a Markdown fence or image URL:
```rust path=null start=null
pub fn ipynb_to_formatted_text(json: &str, gfm_tables: bool) -> Result<FormattedText, IpynbError>;
pub fn raw_fallback_formatted_text(content: &str) -> FormattedText;
```
- `gfm_tables` selects the GFM-table-aware Markdown parser for markdown cells (mirrors `Buffer::from_markdown`; the caller threads the `MarkdownTables` flag state through).
- `serde` structs for nbformat v4: top-level `cells[]` + language from `metadata.language_info.name` (falling back to `metadata.kernelspec.language`); per cell `cell_type`, `source` (string-or-`Vec<String>`), and code-cell `outputs[]`. An explicit, supported `nbformat == 4` is required, otherwise it returns `Err`.
- Conversion rules:
  - markdown cell → parsed into `FormattedText` lines.
  - code cell → `FormattedTextLine::CodeBlock` tagged with the notebook language, sanitized first (`sanitize_language`: trimmed, ≤32 chars, ASCII alphanumerics plus `+#-_.`); anything else yields an unhighlighted block.
  - raw cell → unhighlighted code block (its contents can't inject markdown).
  - `stream` / `execute_result`|`display_data` `text/plain` / `error.traceback` → plain code block (strip ANSI from tracebacks). Text beyond `MAX_TEXT_OUTPUT_CHARS` (100k) is truncated with an `[output truncated]` marker.
  - `display_data`/`execute_result` with `image/png`|`image/jpeg` → `FormattedTextLine::Image` with a `data:<mime>;base64,…` source. The payload is whitespace-stripped, size-bounded (`MAX_IMAGE_DATA_CHARS`, 8 MiB → `[output image omitted: exceeds size limit]`), and validated as base64; invalid data renders `[output image omitted: invalid base64 data]` instead of being embedded.
  - `text/html` and other rich MIME → skipped in v1 (see Follow-ups).
- On parse/format failure, return `Err`; callers fall back to `raw_fallback_formatted_text` (raw contents in a single code block — never blank, never re-interpreted as markdown).
- Depends on `serde_json`, `base64`, `thiserror`, and `markdown_parser`.

**4. View + buffer hook — `app/src/notebooks/file/mod.rs`, `crates/editor`**
- Add a dedicated `ContentFormat::Ipynb` path through the editor rather than reusing the Markdown reset: `InitialBufferState::ipynb` → `Buffer::from_ipynb` (`crates/editor/src/content/buffer.rs`) calls `ipynb_parser::ipynb_to_formatted_text` and falls back to `raw_fallback_formatted_text` on error; `reset_with_ipynb` (`crates/editor/src/model.rs`) drives it.
- In `set_content` (`:334`), when the flag is on and the backing path is `.ipynb`, call `reset_with_ipynb`; otherwise `reset_with_markdown`. This single hook covers both local and remote (`:641`) load paths since both call `set_content`.
- Replace the `is_markdown_file()`-gated header check with `shows_markdown_toggle` (markdown OR flag-enabled jupyter, `:728`) so the Rendered/Raw header toggle appears for `.ipynb`.

**5. Toggle parity — `app/src/code/view.rs`**
Extend the two `is_markdown_file` gates (`update_markdown_mode_segmented_control` `:284`; overflow `is_md` `:2069`) to also accept `.ipynb`. This makes "Raw" mode (JSON in `CodeView`) offer a "Rendered" toggle back to `FileNotebookView`. The existing `RenderMarkdown`→`ReplaceWithFilePane` path then works unchanged (functionally correct; the action name becomes a minor misnomer — rename optional).

**6. Feature flag — `crates/warp_features/src/lib.rs`**
Add `FeatureFlag::JupyterNotebookRendering` to the `FeatureFlag` enum (the actual enum lives in `crates/warp_features/src/lib.rs`, re-exported via `warp_core::features`; the WARP.md `warp_core/src/features.rs` reference is stale). Default-on for dogfood via `DOGFOOD_FLAGS`, add the matching `app/Cargo.toml` `[features]` entry + the `#[cfg(feature = "...")]` bridge arm in `app/src/features.rs::enabled_features`, per the `add-feature-flag` skill. Gate the routing/conversion in steps 2–5.

**Tradeoff — reuse the rich-text renderer vs. dedicated cell view.** A dedicated cell-based view is the right architecture *if execution is ever the goal*, but it is a large new subsystem and none of it is needed for render-only. Reusing the existing `FormattedText` rendering surface ships the user-visible win (no more raw JSON) at a fraction of the cost. The converter builds `FormattedText` directly rather than round-tripping through a Markdown string, which keeps untrusted notebook data out of the Markdown grammar (no fence/image-URL injection). If execution is later prioritized, the converter's parsing + output-MIME logic carries over; the rendering surface would be rebuilt.

## Testing and validation
These map to the behavior invariants in `PRODUCT.md` (each maps to a check below):

1. Opening an `.ipynb` (flag on) renders cells, not JSON. → integration: open a fixture `.ipynb`, assert a `FilePane`/`FileNotebookView` is created (not a `CodePane`); mirror `notebooks/file/mod_tests.rs`.
2. Markdown cells render as formatted text; code cells render as code blocks tagged with the (sanitized) kernel language. → unit tests on `ipynb_to_formatted_text` (golden `FormattedText` output).
3. Text outputs (stream, `text/plain`, error tracebacks) render as preformatted text; ANSI stripped. → unit tests.
4. `image/png`/`image/jpeg` outputs render as images via base64 data-URI. → unit test asserting a `FormattedTextLine::Image` with a `data:image/png;base64,…` source; manual screenshot to confirm the renderer displays the data-URI.
5. Malformed/non-v4 `.ipynb` falls back to raw JSON, never blank/panicking. → unit test feeding invalid JSON returns `Err`; view-level test asserts raw content shown.
6. Rendered⇄Raw toggle works both directions for `.ipynb` (Raw shows JSON in `CodeView`, Rendered returns to `FileNotebookView`). → view tests extending the markdown toggle tests.
7. Flag off ⇒ unchanged behavior (`.ipynb` opens as JSON in `CodeView`). → `openable_file_type_tests.rs` assertions for both flag states (using `FeatureFlag::JupyterNotebookRendering.override_enabled(..)`).

Add `.ipynb` cases to `app/src/util/openable_file_type_tests.rs` (resolve to `MarkdownViewer` when the flag is on, `CodeEditor` when off) and `is_jupyter_notebook_file` cases alongside the markdown predicate tests. Run `./script/format` and `cargo clippy` (per WARP.md) before PR.

## Parallelization
The feature is small (~350–550 LOC) and the steps are mostly sequential: routing (step 2) and the view hook (step 4) both depend on the converter's signature (step 3) and the predicate from step 1. The cleanly isolatable unit is the converter, which has zero UI/codebase coupling behind a frozen interface (`ipynb_to_formatted_text(&str, bool) -> Result<FormattedText, _>`).

Recommended default: **do it in a single PR sequentially** — coordination overhead outweighs the wall-clock savings at this size. If parallelism is still wanted, freeze the converter signature first, then split into two local agents on separate worktrees off `master`:

- **converter** (local) — owns the `ipynb_parser` crate + its unit tests only. Worktree `../warp-ipynb-converter`, branch `oz/ipynb-converter`. No other files.
- **wiring** (local) — owns steps 1, 2, 4, 5, 6 against the agreed signature (stubs the converter until merge). Worktree `../warp-ipynb-wiring`, branch `oz/ipynb-wiring`.

Merge `converter` first, then rebase `wiring` and land a single combined PR. Validation (presubmit + the integration test) is owned by `wiring` after merge.

## Risks and mitigations
- **Data-URI image rendering.** The converter emits a structured `FormattedTextLine::Image` with a `data:<mime>;base64,…` source; confirm the rich-text renderer displays embedded data-URIs (it already does so for Mermaid at `notebooks/editor/model.rs:137`).
- **Large outputs / huge embedded images** can bloat the buffer. Cap per-output text length and skip/oversized-image-placeholder beyond a threshold.
- **nbformat v3 vs v4** differ in `source`/`outputs` shape. Target v4 (dominant); anything that fails to parse falls back to raw JSON (invariant 5).

## Follow-ups
- `text/html` / table / LaTeX output fidelity.
- A dedicated `prefer_notebook_viewer` setting (mirror `prefer_markdown_viewer`).
- Telemetry for notebook opens.
- Cell execution on a Python kernel — separate, much larger effort (new cell-based view + Jupyter ZeroMQ protocol client + kernel process management).
