*Spec: Markdown table rendering parity in plan / notebook documents*

Linear: APP-4917 — https://linear.app/warpdotdev/issue/APP-4917
Target repo: `warpdotdev/warp` (client app). Researched at commit `f74bfd97bb5460a8b3d9a9dfad7edc411303d48f` (`master`); all file references below are pinned to that SHA.

== SUMMARY ==

Suraj Gupta reported that plan / Warp Drive notebook documents feel like a different editing experience from general `.md` text files, and specifically that **plans do not render Markdown tables** the way `.md` files do. The ticket asks us to decide between (a) unifying plan/notebook editing with general `.md` editing, or (b) — if (a) is too complex / semantically different — delivering Markdown-table rendering parity in plan documents.

**Decision: pursue (b).** Full editor unification is rejected (see *Design alternatives*): plan/notebook surfaces and the general Markdown/code editor are built on deliberately different models and persistence, and merging them is a large, high-risk effort with no proportionate user benefit. Instead this spec locks in **Markdown-table rendering parity** across every notebook-backed surface (AI planning documents, Warp Drive notebooks, file-backed `.md` notebooks, Markdown comment editors) and the independent TUI plan renderer, and makes the `MarkdownTables` rollout explicit.

**Important framing from the investigation:** the table infrastructure already exists and is wired end-to-end, and `FeatureFlag::MarkdownTables` is a **default Cargo feature** (`app/Cargo.toml`), so it is compiled in and enabled at startup on **all channels** — it is not globally off. The reported "plans don't render tables" is therefore most consistent with a stale/older build at report time, or with the residual gaps this spec closes, rather than missing infrastructure. This spec's primary product value is **guaranteeing and regression-locking parity** (with deterministic tests + UI verification) and removing the remaining inconsistencies, not building a table renderer from scratch.

*Key design choices:*
1. Scope to table-rendering parity, not editor unification — the notebook (`NotebooksEditorModel`, block model, Warp Drive/SQLite/version persistence, agent-diff streaming) and code editor (`CodeEditorView`, line/character model, LSP, file-bytes persistence, raw source) are semantically distinct; unifying them is out of proportion to the request.
2. Treat the existing shared rich-text/table stack (`Buffer::from_markdown` GFM parsing, `BufferBlockStyle::Table`, `TableStyle`, `MarkdownTableAppearance`, GFM/HTML round-trip) as the single source of truth; do not add another isolated renderer.
3. Resolve the GUI-vs-TUI parse gate divergence by making the TUI honor the same `MarkdownTables` flag as the GUI, and make the flag rollout explicit (promote toward removal only in a follow-up) so behavior is deterministic.

== PRODUCT ==

*Summary:* A valid GFM Markdown table authored (by a user or the agent) in a plan / Warp Drive / file-backed notebook document renders as a real table — visually consistent with the AI block-list and general `.md` rendered view — with source text preserved and round-trippable, and a deterministic readable fallback for malformed/unsupported tables. Raw source-editing surfaces (the code editor) are unchanged.

*Key design choices:* (as above) — table parity over unification; reuse shared stack; explicit flag rollout.

*Behavior* (numbered, testable invariants from the user's/consumer's view):

1. **Default / happy path (GUI notebook surfaces).** With `MarkdownTables` enabled (its default state on all channels), a document containing a valid GFM table opened or loaded in an AI planning document (`AIDocumentView`), a Warp Drive notebook, or a file-backed `.md` notebook (`FileNotebookView`, Rendered mode) renders that content as a table — a header row plus body rows with column dividers/row separators per the shared table appearance — not as literal `|`-delimited text.

2. **Agent-authored plans.** When the agent creates or edits a plan via `CreateDocuments`/`EditDocuments` (applied through `apply_diffs` → `reset_with_markdown`), any valid GFM table in the resulting document renders as a table in the plan pane, identical to a table typed by a user, once the applied document is re-parsed.

3. **TUI plan parity.** The TUI plan renderer (`tui_plan_view.rs`) renders valid GFM tables in plan documents, consistent with the GUI plan surface for the same content. The TUI must honor the same `FeatureFlag::MarkdownTables` gate as the GUI: enabled builds parse/render tables, while disabled builds use the documented non-table fallback, so the two surfaces cannot disagree for the same build.

4. **Alignment + inline formatting.** Left / center / right column alignment is preserved, and inline formatting inside cells (bold, italic, bold-italic, inline code, strikethrough, links, and escaped `\|` rendered as a literal pipe) renders correctly on every surface that renders the table.

5. **Source recoverability / round-trip.** Rendering a table never loses the source: serializing a rendered table back to Markdown produces a valid GFM table (`| … |` rows, alignment separators `---` / `:---:` / `---:`, escaped pipes) that re-parses to the same table. Copying a table (or a sub-range of it) yields the expected plain-text/HTML clipboard content.

6. **Persistence / version restore.** Saving a plan/notebook containing a table and reopening it, and restoring an earlier AI-document version that contains a table, both reproduce the same rendered table (because content persists as Markdown and re-parses on load).

7. **Deterministic fallback.** Content that is not a valid GFM table (malformed rows, ragged columns, or table-looking text inside a fenced code block) does **not** render as a table; it falls back to the existing readable rendering (paragraph text, or code inside a code block) with no panic, blank block, or dropped text.

8. **Flag-disabled behavior.** With `MarkdownTables` disabled, every affected surface falls back to the deterministic non-table rendering of invariant 7 for all table content — i.e. the flag cleanly gates *table lowering/layout* only, with no crash or content loss, and no half-rendered tables.

9. **Raw / code surfaces unchanged.** Viewing or editing literal Markdown source — the `CodeEditorView` (general `.md` as code) and the file notebook's Raw mode (which switches to a code pane) — continues to show raw source text and is explicitly out of scope; this spec does not turn raw-source editing into rendered tables.

10. **No collateral regressions.** Non-table notebook blocks (headings, lists, task lists, code/command blocks, mermaid, embedded workflows, images), selection/cursor/keyboard navigation, links, and the file-notebook Rendered/Raw toggle all continue to behave as they do today.

== TECH ==

*Context (how the area works today, commit-pinned):*

- **Shared notebook stack.** AI planning documents, Warp Drive notebooks, and file-backed `.md` notebooks all use `NotebooksEditorModel` + `RichTextEditorView`:
  - AI docs: `app/src/ai/ai_document_view.rs:38-40,347-355,837-864` (`RichTextEditorView` over a `NotebooksEditorModel`; editable vs `Selectable` via `set_editor_model`).
  - File notebooks: `app/src/notebooks/file/mod.rs:36,84,252-268,338-353` (`RichTextEditorView`, `set_content` → `reset_with_markdown`; `MarkdownDisplayMode::{Rendered,Raw}` at `:72-77,731-750`; Raw switches to a code pane, not an in-place raw view).
  - AI document model: `app/src/ai/document/ai_document_model.rs:1008-1031` (`create_editor_model` → `reset_with_markdown`), `:1076-1099` (`create_new_version_and_apply_diffs` → `apply_diffs`), `:961-988` (persisted-content reset).
- **General Markdown/code editing** uses the separate `CodeEditorView` (`app/src/code/editor/view.rs`), a line/character code model with LSP, syntax highlighting, vim, find/replace, and file-bytes persistence — it shows raw source, not rendered tables. `code_text_styles()` clones `rich_text_styles()` then overrides paragraph settings (see prior spec `specs/zachlloyd/markdown-table-consistency/TECH.md:31,61`).
- **GFM parse gate.** `Buffer::from_markdown` picks `parse_markdown_with_gfm_tables` when `FeatureFlag::MarkdownTables.is_enabled()`, else `parse_markdown` (`crates/editor/src/content/buffer.rs:843-874`; `from_ipynb` mirrors this at `:883-899`). This is the single point where Markdown text is lowered into blocks; both `reset_with_markdown` and `apply_diffs` route here (`crates/editor/src/model.rs:924-940`; `app/src/notebooks/editor/model.rs:310-312,1808-1854`).
- **Table lowering + layout.** Parsed tables become `BufferBlockStyle::Table { alignments, cache }`; `layout_text_block` short-circuits to `layout_table_block` when the block is a `Table` and `MarkdownTables` is enabled (`crates/editor/src/content/edit.rs:994-1003`). Core lowering: `crates/editor/src/content/core.rs`.
- **Rendering + appearance.** Renderer: `crates/editor/src/render/element/table.rs`; `TableStyle`/`RichTextStyles`: `crates/editor/src/render/model/mod.rs`. Shared appearance + style mapping: `app/src/notebooks/editor/mod.rs:45-59,158-198,276` (`MarkdownTableAppearance`, `markdown_table_style`, wired into `rich_text_styles`). `markdown_table_count` telemetry: `app/src/notebooks/editor/model.rs:242-244`.
- **Round-trip.** Table → GFM Markdown: `crates/editor/src/content/markdown.rs:1270-1291` (`append_gfm_table_row`, `alignment_to_gfm_separator`, `escape_gfm_table_cell`). Table → HTML + clipboard/partial selection: `crates/editor/src/content/buffer.rs:2374-2520`; table → HTML: `markdown.rs:1199-1245`.
- **TUI plan renderer.** `crates/warp_tui/src/tui_plan_view.rs:7,118-136` calls `parse_markdown_with_gfm_tables` **unconditionally** (no `MarkdownTables` gate) and re-parses the full document on each sync.
- **Flag rollout.** `FeatureFlag::MarkdownTables` defined at `crates/warp_features/src/lib.rs:507-508`; enabled from the Cargo feature via `app/src/features.rs:164-165` inside `enabled_features()` (`:8-25`), and `markdown_tables` is listed in the `default` feature set (`app/Cargo.toml:510,652`, feature declared at `:800`). It is **not** in `DOGFOOD_FLAGS`/`PREVIEW_FLAGS`/`RELEASE_FLAGS` — it does not need to be, because the default Cargo feature enables it on every build/channel. It is not in `RUNTIME_FEATURE_FLAGS`, so it cannot be toggled at runtime.
- **Prior art.** `specs/zachlloyd/markdown-table-consistency/PRODUCT.md` + `TECH.md` document the blockless cross-surface table *styling* work; that baseline is present in the current checkout. This ticket is about *rendering parity + rollout*, building on that styling baseline.

*Design alternatives:*

- **(a) Unify plan/notebook editing with general `.md` editing (rejected).** Would require merging `NotebooksEditorModel` (block model; embedded workflows, command/mermaid child models; Warp Drive/SQLite persistence and AI version history; agent-diff streaming) with `CodeEditorView` (line/character model; LSP; syntax highlighting; vim; find/replace; file-bytes persistence; diff/code-review integration). These are semantically different surfaces (rendered rich-text vs raw source). Pros: one editing experience. Cons: very large blast radius across LSP, code review, diff viewer, vim, notebook blocks, and two persistence models; high regression risk; the user's actual pain (tables not rendering in plans) does not require it. **Not justified.**
- **(b) Markdown-table rendering parity in plan/notebook documents (chosen).** Reuse the existing shared parser/lowering/renderer/round-trip; verify and regression-lock parity across all notebook surfaces + TUI; reconcile the GUI/TUI parse gate; make the flag rollout explicit. Pros: directly fixes the report, low incremental code, deterministic to test. Cons: does not merge the two editors (acceptable — that is a non-goal).
- **Flag rollout sub-decision.** Options: (i) leave the flag as a default-on Cargo feature and only document + regression-test it; (ii) promote it toward removal (delete the flag and the disabled code paths) now that it ships on by default; (iii) add channel entries. Selected: **(i) for this ticket** — keep `MarkdownTables` as the single GUI/TUI gate, add the tests/UI verification and the explicit default-on documentation, and record flag removal as a follow-up (per the repo's remove-feature-flag process) rather than bundling flag removal into a parity fix.

*Proposed changes (implementation-time; this is a spec, not an implementation):*

1. **Verify parity, then close any gap.** Confirm that AI planning documents, Warp Drive notebooks, and file-backed `.md` notebooks all render a valid GFM table at HEAD (they share `from_markdown`, so they should). For any surface that does not, route it through the shared `reset_with_markdown`/`from_markdown` path rather than adding a bespoke renderer.
2. **Reconcile the GUI/TUI parse gate.** Update the TUI plan renderer to branch on `FeatureFlag::MarkdownTables`, using `parse_markdown_with_gfm_tables` when enabled and the ordinary `parse_markdown` path when disabled. This is the selected single gate for both front-ends; do not make table parsing unconditional in only one surface.
3. **Make the flag rollout explicit.** Keep `MarkdownTables` as the default-on Cargo feature and single runtime gate for this ticket. Add regression tests asserting enabled and disabled behavior, and document (in code near the flag and in the PR) that it ships on by default via the `default` Cargo feature. Flag removal is a separate follow-up and is not part of this implementation.
4. **Do not touch** `CodeEditorView` raw-source behavior or the file-notebook Raw (code-pane) mode.

*Open questions resolved:*
- *Unify vs. table parity?* → Table parity (b). Resolved from the codebase: the two editors are distinct models/persistence; unification is disproportionate. (Non-goal recorded.)
- *Is the missing rendering caused by the flag being off?* → No. Resolved from `app/Cargo.toml` + `app/src/features.rs`: `MarkdownTables` is a default Cargo feature enabled on all channels. Most likely a stale build at report time and/or the residual gaps closed here.
- *Do agent-streamed plans reparse tables?* → Yes. Resolved from `app/src/notebooks/editor/model.rs:1808-1854`: `apply_diffs` re-serializes to Markdown and calls `reset_with_markdown` → `from_markdown`.
- *Do tables round-trip for persistence/restore?* → Yes. Resolved from `crates/editor/src/content/markdown.rs:1270-1291` and `buffer.rs:2374-2520`.
- *Is the GUI/TUI gate consistent?* → No, today: GUI is flag-gated (`buffer.rs:850`), TUI is ungated (`tui_plan_view.rs:128`). The selected fix is for TUI to honor the GUI's `MarkdownTables` gate (proposed change 2), with flag-off fallback covered by tests.
- *Live-typed tables (not via reparse)?* → GFM lowering happens on full (re)parse (`from_markdown`), not on incremental keystroke insertion; `apply_diffs` and `reset_with_markdown` both reparse, so agent edits and document (re)loads lower tables. Keystroke-by-keystroke promotion of a newly typed table is explicitly out of scope for this ticket; the UI verification confirms that existing editable table interactions do not regress.

*Validation & verification criteria* (must ALL pass before merge):

Deterministic regression tests (name the test; each must fail before the fix if it targets a real gap, and pass after):

1. **Parser — valid GFM tables.** Extend `crates/editor/src/content/markdown_tests.rs`: a valid GFM table parses into a table with correct headers, rows, and per-column alignment (left/center/right). (Verifies invariants 1, 4.)
2. **Parser — malformed/unsupported input.** Malformed / ragged / non-table pipe content and a table-looking block inside a fenced code block do **not** parse as a table and fall back deterministically (paragraph / code). No panic. (Verifies invariant 7.)
3. **Parser — inline formatting + escaped pipes in cells.** Bold, italic, bold-italic, inline code, strikethrough, links, and escaped `\|` render as expected inside cells. (Verifies invariant 4.)
4. **Round-trip — table → GFM → table.** Extend `markdown_tests.rs`: serializing a parsed table back to Markdown (`append_gfm_table_row` / `alignment_to_gfm_separator` / `escape_gfm_table_cell`) yields valid GFM that re-parses to an equal table, preserving alignment and escaped pipes. (Verifies invariant 5.)
5. **Buffer/layout — flag ON.** Extend `crates/editor/src/content/edit_tests.rs` / `buffer_tests.rs` with a `MarkdownTables` override enabled: a `Table` block is created and laid out via `layout_table_block` (borders/dividers/sizing per `TableStyle`), and the table-block layout cache invalidates correctly on edit. (Verifies invariants 1, 4.)
6. **Buffer/layout — flag OFF.** Same input with `MarkdownTables` overridden off: no `Table` block is produced; content falls back to paragraph rendering with no dropped text and no panic. (Verifies invariant 8.)
7. **Clipboard / partial selection.** Copying a whole table and a sub-range yields the expected plain-text (tab/newline-delimited) and HTML (`<table>`) output (`buffer.rs:2374-2520`, `markdown.rs:1199-1245`). (Verifies invariant 5.)
8. **`apply_diffs` reparse lowers tables.** In `app/src/notebooks/editor/model_tests.rs`, applying a diff whose insertion adds a GFM table (flag on) results in an editor whose content contains a `Table` block. (Verifies invariant 2.)
9. **Persistence / version restore.** A notebook/AI-document whose serialized Markdown contains a table reproduces the same rendered `Table` block after `reset_with_markdown` from persisted content and after restoring an earlier version. (Verifies invariant 6.)
10. **TUI plan parity + gate reconciliation.** Extend `crates/warp_tui/src/tui_plan_view_tests.rs`: a plan document containing a GFM table renders a table (render-to-lines), and the TUI parse decision matches the reconciled single gate chosen in proposed change 2. (Verifies invariant 3.)
11. **Non-table regressions.** Existing table/notebook/editor tests continue to pass (headings, lists, task lists, code/command blocks, mermaid, embedded items, selection/cursor/links) — run `crates/editor` and `app` notebook/editor test suites. (Verifies invariant 10.)
12. **Presubmit.** `./script/presubmit` passes (fmt, clippy `-D warnings`, workspace tests) at repo default settings. Include TUI feature coverage where the change touches `crates/warp_tui` (e.g. the relevant `--features` build/test used by presubmit).

User-facing UI verification (per `factory-verification`; capture screenshot/visual proof for each surface, attached to the task and the PR):

13. **AI planning document.** In a running client, open/generate a plan containing a valid GFM table (including left/center/right alignment and at least one cell with bold + inline code + a link + an escaped pipe); confirm it renders as a table matching the shared appearance, not raw `|` text.
14. **Warp Drive notebook.** Open a Warp Drive notebook containing the same table; confirm identical rendering.
15. **File-backed `.md` notebook (Rendered).** Open a `.md` file that renders in the notebook viewer with the same table; confirm identical rendering in Rendered mode, and confirm Raw mode still shows raw source (code pane) unchanged.
16. **Agent-authored plan.** Have the agent create/edit a plan with a table via `CreateDocuments`/`EditDocuments`; confirm the table renders in the plan pane after the edit is applied.
17. **TUI plan.** In the TUI (`./script/run-tui`), render a plan containing the same table; confirm it renders as a table consistent with the GUI.
18. **Fallback + wide-table checks.** Confirm malformed table-like content and a fenced code block containing pipes do not render as tables; confirm a wide table remains readable/scrollable and selectable; confirm cursor/selection/link interactions in an editable notebook table are unchanged.
19. **Flag-off sanity.** With `MarkdownTables` disabled for the build, confirm every surface degrades to readable non-table text with no crash or content loss.

Reproduction closure: the exact report ("plan/notebook documents don't render Markdown tables like `.md` files do") no longer reproduces — criteria 13–17 demonstrate the same GFM table rendering identically in plan/notebook surfaces and the `.md` rendered view; and the parser/round-trip/gate tests (1–10) lock the behavior so it cannot silently regress.
