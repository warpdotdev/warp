*Spec: Native CSV viewer for Warp's file viewer*

== PRODUCT ==

*Summary:* When a user opens a `.csv` file in Warp's file viewer, offer a **Raw / Rendered** toggle (mirroring the Markdown/Jupyter viewer). **Rendered** mode parses the CSV and displays it as a rich, read-only table reusing Warp's existing GUI table component — with a header row, intrinsic (auto-fit) columns, striped rows, vertical virtualization for large files, and horizontal scrolling for wide files. **Raw** mode shows the file as plain text in the code editor (today's behavior). The feature is gated behind a new runtime feature flag, default off, and degrades gracefully: malformed or oversized CSVs fall back to Raw instead of blocking file access.

*Key design choices:*
1. **Reuse the existing GUI `Table` component** (`crates/warpui_core/src/elements/gui/table/mod.rs`) for Rendered mode — intrinsic column widths, striped `RowBackground`, `SumTree` row virtualization, and horizontal scroll are already built. We do **not** render CSV as a GFM markdown table through the editor's `LaidOutTable` path; that path is for markdown buffers and would not give the virtualization/scroll behavior the requester asked for.
2. **Mirror the Jupyter notebook rendering precedent** (`FeatureFlag::JupyterNotebookRendering`): detect `.csv` by extension → route into the existing notebook/file viewer (`FileNotebookView`) when the flag is on, with the same Raw/Rendered toggle UX. The existing toggle already switches Raw → a `CodePane` (raw text), so Raw mode is free.
3. **Rendered is read-only; editing happens in Raw.** This matches the existing toggle semantics (Raw opens the code editor) and the `.ipynb` precedent (render-only). A dedicated CSV table editor is out of scope for v1.
4. **Graceful fallback, never a blank view.** Parse errors, unsupported shapes, and oversized files fall back to Raw (code editor) — mirroring `ipynb_parser`'s "render-only with a raw fallback" philosophy. No parse failure may block the user from seeing the file.
5. **v1 scope: `.csv` (comma-delimited) only, detected by extension.** TSV and other delimiters, and content-heuristic detection, are explicit follow-ups (see Out of scope). The `csv` crate (already a dependency) is used for RFC-4180-correct parsing.

*Behavior* (numbered, testable invariants from the user's view):
1. With the feature flag enabled, opening a `.csv` file in Warp shows a **Rendered / Raw** segmented toggle in the pane header (identical control to the Markdown/Jupyter viewer). With the flag disabled, `.csv` opens exactly as today: raw text in the code editor, no toggle.
2. **Rendered** mode displays the CSV as a read-only table: the first row renders as a **header row** (fixed at the top of the viewport while the body scrolls), and subsequent rows render as data rows with **striped** (alternating) row backgrounds.
3. Columns auto-size to their widest cell (**intrinsic** widths); when there are more columns than fit the pane width, the table **scrolls horizontally** so all columns are reachable.
4. **RFC-4180 parsing is correct:** fields containing quoted commas (`"a,b"`), escaped/doubled quotes (`"He said ""hi"""`), and quoted values spanning multiple physical lines render as single cell values (not split across cells/rows).
5. **Ragged rows** (rows with fewer or more fields than the header) render without panicking or dropping the file: short rows pad with empty cells to the header width; long rows are preserved (overflow columns rendered, not truncated silently).
6. **Raw** mode shows the file verbatim as plain text in the code editor (byte-for-byte the current behavior); toggling Rendered→Raw and Raw→Rendered switches panes and preserves the file/scroll context the existing Markdown toggle already preserves.
7. **Large-but-valid CSVs** within the configured row/byte cap render smoothly via the table's `SumTree` row virtualization (only visible rows are laid out/painted); scrolling is responsive.
8. **Oversized or malformed CSVs fall back to Raw:** a CSV that exceeds the row or byte cap, or that the parser rejects, opens in the code editor (Raw) rather than rendering a broken/blank table or hanging the UI. The user can always see and edit the raw content.
9. The toggle is **read-only in Rendered**; the user switches to Raw to edit. No edit actions are offered on the rendered table in v1.
10. Toggling Rendered↔Raw emits the existing notebook toggle telemetry (mirrored for a CSV source), so usage is observable.
11. The feature is **GUI-only** (the notebook/file viewer and the `Table` component are GUI surfaces). It has no effect on the headless TUI front-end, and is gated off where the notebook viewer is unavailable (wasm / non-`local_fs` builds behave as today).

*Open design questions (resolved here for v1):*
- **Editing in Rendered mode:** read-only; edit via Raw. (v1)
- **Large-file UX:** cap rows and bytes; over-cap → Raw fallback; within-cap → virtualized table. Thresholds are named constants (see Tech), tuned conservatively and easy to adjust. (v1)
- **TSV / other delimiters:** out of scope for v1 (`.csv` comma only). (Follow-up)
- **Detection:** extension-only for v1 (`.csv`, case-insensitive), mirroring `is_jupyter_notebook_file`. Content heuristics are a follow-up. (v1)

*Out of scope for v1:* TSV/PSV/other delimiters; content-heuristic CSV detection; editing/sorting/filtering in Rendered mode; column resize/reorder; CSV export; rendering CSVs embedded in AI/agent output (the blocklist table path already handles markdown tables there). These may be follow-ups behind the same flag.

== TECH ==

*Context (all paths commit-pinned to `warpdotdev/warp` @ `a554fbe9901884999b8c06d49b0cf0141febc969`):*

File-type detection and routing:
- `crates/warp_util/src/file_type.rs:148` — `is_jupyter_notebook_file` (extension-based `.ipynb`); the pattern to mirror for a new `is_csv_file`. `crates/warp_util/src/file_type.rs:260` — `"csv" | "tsv" | ...` already classifies `.csv` as a text extension, so `.csv` already opens as `OpenableFileType::Text`.
- `app/src/util/openable_file_type.rs:78` — `renders_in_warp_notebook_viewer(path)`: returns true for markdown always and `.ipynb` when `JupyterNotebookRendering` is enabled. This is the gate that decides a file gets the notebook viewer + Raw/Rendered toggle.
- `app/src/util/openable_file_type.rs:216` — `resolve_file_target_with_editor_choice`: step 0 (`:231`) routes `.ipynb` → `FileTarget::MarkdownViewer` when the flag is on (unconditional, not gated on `prefer_markdown_viewer`). `app/src/util/openable_file_type.rs:176` — `resolve_file_target_to_open_in_warp` has the same `.ipynb` clause (`:187`).

Notebook/file viewer (the view we reuse for Rendered):
- `app/src/notebooks/file/mod.rs:74` — `MarkdownDisplayMode { Rendered, Raw }` (the toggle state).
- `app/src/notebooks/file/mod.rs:80` — `FileNotebookView` (read-only notebook viewer backed by a file). Fields: `markdown_display_mode` (`:98`), `display_mode_segmented_control: ViewHandle<MarkdownToggleView>` (`:99`).
- `app/src/notebooks/file/mod.rs:338` — `set_content`: branches on `render_as_ipynb` (`:340-341`) to call `editor.reset_with_ipynb` vs `editor.reset_with_markdown` (`:343-347`). This is where CSV content is loaded.
- `app/src/notebooks/file/mod.rs:732` — `shows_markdown_toggle`: returns true for markdown + `.ipynb` (flag-gated). This gates the toggle's visibility in the header (`:1198`, rendered at `:1209`).
- `app/src/notebooks/file/mod.rs:960` — `render_body`: for `FileState::Loaded` it renders `ChildView::new(&self.editor)` (`:965`). This is the render branch to extend for CSV Rendered mode.
- `app/src/notebooks/file/mod.rs:1086` — `ToggleMarkdownDisplayMode` handler: `Rendered` stays in the view (`:1094-1097`); `Raw` emits `PaneEvent::ReplaceWithCodePane` (`:1098-1108`, `:1102`) — i.e. Raw mode already opens the code editor pane. This gives CSV Raw mode for free.
- `app/src/view_components/markdown_toggle_view.rs:19` — `MarkdownToggleView` (segmented control with labels "Rendered"/"Raw" at `:37-38`). Reused as-is; labels are format-agnostic.
- `app/src/notebooks/editor/view.rs:1653` — `RichTextEditorView::reset_with_ipynb` (and `:1647` `reset_with_markdown`); the editor load API (precedent for how content enters the viewer).

The reusable GUI table component:
- `crates/warpui_core/src/elements/gui/table/mod.rs:121` — `TableColumnWidth::Intrinsic` (auto-fit to widest cell).
- `crates/warpui_core/src/elements/gui/table/mod.rs:169` — `RowBackground::striped(even, odd)` (`:193`) and `color_for_row` (`:200`).
- `crates/warpui_core/src/elements/gui/table/mod.rs:211` — `TableConfig` (`fixed_header`, `vertical_sizing`, `measure_body_cells_for_intrinsic_widths`, `row_background`, borders/dividers/padding).
- `crates/warpui_core/src/elements/gui/table/mod.rs:434` — `TableStateHandle::new(row_count, row_render_fn)`; `:494` `set_row_count`, `:501` `set_row_render_fn`, `:484` `scroll_to_row`. `row_render_fn` is `Fn(usize, &AppContext) -> Vec<Box<dyn Element>>` (`:503`) — one element per cell.
- `crates/warpui_core/src/elements/gui/table/mod.rs:542` — `Table::new(state, unconstrained_width, unconstrained_height)`; `:582` `with_headers(Vec<TableHeader>)`; `:589` `with_row_count`; `:596` `with_row_render_fn`; `:604` `with_config`. `SumTree` row virtualization is built in (`:267`, `:539-541`).
- Proven reuse: `app/src/ai/blocklist/block/view_impl/common.rs` already builds tables from this component (behind `BlocklistMarkdownTableRendering`), and `crates/warpui/examples/table-sample/root_view.rs` is a usage example.

Feature-flag infrastructure (the precedent to copy):
- `crates/warp_features/src/lib.rs:503` — `JupyterNotebookRendering` enum variant (doc at `:501-503`); added to `DOGFOOD_FLAGS` at `:981`.
- `app/src/features.rs:162` — `#[cfg(feature = "jupyter_notebook_rendering")] FeatureFlag::JupyterNotebookRendering,` (compile-time cargo feature → runtime flag bridge).
- `app/Cargo.toml:794` — `jupyter_notebook_rendering = []` cargo feature definition (and `:795` `markdown_tables = []`); the cargo feature is enabled in the appropriate default feature group so the flag is compiled in. `app/Cargo.toml:95` — `csv = "1.3.1"` is **already** a workspace dependency (currently used by `app/src/app_menus.rs:6`), so no new dependency is required.

Render-only parser with raw fallback (the philosophy to mirror):
- `crates/ipynb_parser/src/lib.rs:50` — `ipynb_to_formatted_text` returns `Result<FormattedText, IpynbError>`; `:83` — `raw_fallback_formatted_text` for the fallback path. The CSV parser follows the same "render-only, never blank, fall back to raw on any failure" contract.

*Proposed changes:*

1. **New feature flag `CsvViewerRendering`** (default off, dogfood-on):
   - Add `CsvViewerRendering` variant to the `FeatureFlag` enum in `crates/warp_features/src/lib.rs` (next to `JupyterNotebookRendering`, `:503`), with a doc comment ("Renders `.csv` files as a read-only table in Warp's file viewer instead of raw text.").
   - Add `FeatureFlag::CsvViewerRendering` to `DOGFOOD_FLAGS` (`:981` region) so dogfood builds exercise it.
   - Add the compile-time bridge `#[cfg(feature = "csv_viewer_rendering")] FeatureFlag::CsvViewerRendering,` in `app/src/features.rs` (next to the `jupyter_notebook_rendering` bridge at `:162`).
   - Add the `csv_viewer_rendering = []` cargo feature in `app/Cargo.toml` (next to `:794`) and enable it in the same default feature group that enables `jupyter_notebook_rendering`. Follow the `add-feature-flag` skill conventions.

2. **CSV file-type detection** (`crates/warp_util/src/file_type.rs`):
   - Add `pub fn is_csv_file(path: impl AsRef<Path>) -> bool` mirroring `is_jupyter_notebook_file` (`:148`): true when the extension is `.csv` (case-insensitive). Export it alongside `is_jupyter_notebook_file`/`is_markdown_file`.

3. **Routing** (`app/src/util/openable_file_type.rs`):
   - Extend `renders_in_warp_notebook_viewer` (`:78`) with a third clause: `|| (FeatureFlag::CsvViewerRendering.is_enabled() && is_csv_file(path))`.
   - Extend both routing functions with a CSV clause mirroring the `.ipynb` step-0 clause: in `resolve_file_target_with_editor_choice` (`:231` region) and `resolve_file_target_to_open_in_warp` (`:187` region), when the flag is enabled and `is_csv_file(path)`, return `FileTarget::MarkdownViewer(layout)` (unconditional, like `.ipynb`). Keep the existing markdown/preference logic intact.

4. **CSV parser module** (new, e.g. `app/src/notebooks/file/csv.rs`, gated `#[cfg(feature = "csv_viewer_rendering")]`):
   - A render-only parser wrapping `csv::Reader` (flexible, RFC-4180) that turns file text into a row-oriented structure: `struct CsvTable { header: Vec<String>, rows: Vec<Vec<String>> }` (or an `Arc`-shared, clone-cheap shape the `row_render_fn` closure can capture).
   - A result enum: `enum CsvRender { Table(CsvTable), FallbackToRaw { reason: CsvFallbackReason } }` with reasons `TooManyRows`, `TooManyBytes`, `ParseError`, `Empty`. Public entry `fn parse_csv_for_render(content: &str) -> CsvRender`.
   - Named, conservative caps as constants (e.g. `const MAX_CSV_ROWS: usize = 50_000;` `const MAX_CSV_BYTES: usize = 10 * 1024 * 1024;`) checked before/during parsing; over-cap → `FallbackToRaw`. Ragged rows are normalized to the header width (pad short, keep long).
   - Mirror `ipynb_parser`'s contract: never panic, never return a blank view — any failure yields `FallbackToRaw`.

5. **Rendered CSV table in `FileNotebookView`** (`app/src/notebooks/file/mod.rs`):
   - Add a CSV render path. In `set_content` (`:338`), detect CSV (flag on + `is_csv_file`): run `parse_csv_for_render(content)` and cache the `CsvRender` on the view (new field, e.g. `csv_render: Option<Arc<CsvRender>>`); skip the markdown/ipynb editor reset for CSV (the editor is not used to render CSV). For non-CSV, behavior is unchanged.
   - In `render_body` (`:960`), when `FileState::Loaded` and a CSV `CsvRender::Table` is cached and `markdown_display_mode == Rendered`, render a `Table` element (from `crates/warpui_core/.../table/mod.rs`) instead of `ChildView::new(&self.editor)` (`:965`): build `TableHeader`s with `TableColumnWidth::Intrinsic` from the header row, a `TableConfig` with `fixed_header: true`, `RowBackground::striped(...)` using theme colors, `vertical_sizing: Viewported`, `measure_body_cells_for_intrinsic_widths: true`; a `TableStateHandle` holding the row count with a `row_render_fn` that returns one `Text`/`StyledText` element per cell for the given row index (capturing the shared `CsvTable`). On `CsvRender::FallbackToRaw`, render the existing editor child view as raw text (or emit `ReplaceWithCodePane` to land in the code editor) so the user sees the raw file.
   - Extend `shows_markdown_toggle` (`:732`) to include CSV: `|| (FeatureFlag::CsvViewerRendering.is_enabled() && self.is_csv_file())`. Add an `is_csv_file` helper mirroring `is_jupyter_notebook_file` (`:724`). The existing `ToggleMarkdownDisplayMode` handler (`:1086`) already does the right thing: Rendered stays in the view (re-renders the table); Raw emits `ReplaceWithCodePane` (`:1102`) → code editor with raw text.
   - Theme/colors: source header/row/background colors from `Appearance`/theme tokens (not hardcoded `TableConfig::default()` colors), per the GUI UI guidelines. Cell text uses the editor/text styles already used by the notebook viewer.
   - Telemetry: emit the existing notebook toggle telemetry on CSV Rendered↔Raw (mirror `NotebookTelemetryAction` usage), tagging the source as a CSV file.

6. **Tests:**
   - `csv` parser unit tests (`app/src/notebooks/file/csv_tests.rs` or `mod_tests.rs` style per AGENTS.md): quoted commas, doubled/escaped quotes, multiline quoted values, ragged rows (short + long), empty file, over-`MAX_CSV_ROWS` and over-`MAX_CSV_BYTES` → `FallbackToRaw`, parse-error → `FallbackToRaw`. These fail before the parser exists and pass after.
   - `is_csv_file` unit test in `crates/warp_util` (case-insensitive `.csv`; `.tsv`/`.txt`/no-extension false).
   - Routing tests in `app/src/util/openable_file_type_tests.rs` mirroring the `.ipynb` tests (`:118-152`): with `CsvViewerRendering` overridden on, `.csv` → `FileTarget::MarkdownViewer` and `renders_in_warp_notebook_viewer` true; with it off, `.csv` → `CodeEditor` and renders-in-viewer false.
   - The GUI `Table` component already has `crates/warpui_core/src/elements/gui/table/mod_tests.rs`; add a focused render/layout test for a CSV-shaped table (intrinsic columns + horizontal overflow) only if the existing tests don't already cover that shape — otherwise rely on the existing ones.

*Tradeoffs / notes:*
- We reuse `MarkdownDisplayMode` and `MarkdownToggleView` as-is (labels "Rendered"/"Raw" are format-agnostic). Renaming to a format-neutral `DisplayMode`/`DisplayToggleView` is a larger refactor and is **not** required for v1; noted as an optional follow-up.
- Routing CSV to `FileNotebookView` (rather than a brand-new `CsvViewerView`) maximally reuses the header/toggle/pane/telemetry machinery. The cost is one new render branch in `render_body` and a cached `CsvRender` field; the editor handle stays but is unused for CSV.
- The `csv` crate is already a dependency, so no new dependency review is needed.

*Validation & verification criteria* (must ALL pass before merge):
1. **CSV parser correctness** — new unit test `parse_csv_quoted_and_multiline` (in `app/src/notebooks/file/csv_tests.rs` or equivalent) asserts the exact parsed rows for a fixture containing quoted commas (`"a,b"`), doubled quotes (`"x ""y"" z"`), and a multiline quoted value spanning two physical lines. Fails before the parser module exists; passes after. — verifies behavior invariants #4 and #5.
2. **Ragged-row handling** — new unit test `parse_csv_ragged_rows` asserts a header of N columns plus a short row (padded to N) and a long row (preserved at N+k) parse without panic and with the expected cell counts. — verifies invariant #5.
3. **Large-file fallback** — new unit test `parse_csv_over_cap_falls_back_to_raw` feeds a synthetic CSV exceeding `MAX_CSV_ROWS` and one exceeding `MAX_CSV_BYTES`; both return `CsvRender::FallbackToRaw` (no panic, no OOM, no partial table). — verifies invariant #8.
4. **Malformed-file fallback** — new unit test `parse_csv_malformed_falls_back_to_raw` feeds input that triggers a `csv` parse error and asserts `FallbackToRaw { reason: ParseError }`; the function never panics. — verifies invariant #8.
5. **`is_csv_file` detection** — new unit test in `crates/warp_util` asserts `is_csv_file("a.CSV")`/`"a.csv"` are true and `is_csv_file("a.tsv")`/`"a.txt"`/`"README"` are false. — verifies invariant #1 / extension-only detection.
6. **Routing with flag on** — new test in `app/src/util/openable_file_type_tests.rs` (mirroring the `.ipynb` test at `:118-152`) overrides `FeatureFlag::CsvViewerRendering.override_enabled(true)` and asserts `resolve_file_target_with_editor_choice` for a `.csv` path returns `FileTarget::MarkdownViewer(_)` and `renders_in_warp_notebook_viewer` returns true. — verifies invariant #1.
7. **Routing with flag off** — companion test asserts that with the flag off, a `.csv` path resolves to `FileTarget::CodeEditor(_)` (today's behavior) and `renders_in_warp_notebook_viewer` returns false. — verifies invariant #1 (default-off).
8. **No regression to Markdown/Jupyter viewers** — `cargo nextest run -p warp --no-fail-fast` (or `./script/presubmit`) shows the existing notebook/file-viewer tests still pass: `app/src/notebooks/file/mod_tests.rs`, `app/src/util/openable_file_type_tests.rs`, `app/src/notebooks/editor/*_tests.rs`, and `crates/ipynb_parser`. — verifies invariants #6 and that the toggle/shared code is intact.
9. **`./script/presubmit` passes** unconditionally (fmt + clippy + tests + build) on the `warp` repo. Additionally `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings` is clean, per `AGENTS.md`. — verifies the change builds and lints under all features (so the `csv_viewer_rendering` cfg bridge compiles in both states).
10. **Visual proof (computer-use video)** — a screen recording, captured via `computer_use` (or `test-warp-ui`/`verify-ui-change-in-cloud`) against a dogfood build with `CsvViewerRendering` on, demonstrating: (a) opening a `.csv` and seeing the **Rendered/Raw** toggle; (b) the rendered table with a header row, striped rows, and intrinsic columns; (c) horizontal scrolling for a wide (many-column) CSV; (d) smooth vertical scroll on a large-but-within-cap CSV; (e) toggling to **Raw** → code editor with raw text, and back to **Rendered**; (f) a malformed or oversized CSV falling back to Raw. The video is attached to the PR body **and** posted to the originating Slack thread (#factory-server, ts 1784042336.528929). Media is not committed to the branch. — verifies invariants #1–#8 (user-facing). Per `factory-ui-verification`, missing/invalid proof is a blocking review finding.
11. **Default-off confirmation** — the visual proof (or a second short clip) shows that with the flag off, the same `.csv` opens as raw text with no toggle, confirming no behavior change when the feature is disabled. — verifies invariant #1 and the flag gate.
