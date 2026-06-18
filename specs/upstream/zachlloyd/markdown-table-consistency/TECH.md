# Markdown Table Styling Consistency — Tech Spec
Product spec: `specs/zachlloyd/markdown-table-consistency/PRODUCT.md`

## Problem
PR #23908 updated AI block list Markdown tables to the new blockless visual treatment, but the rest of Warp’s Markdown table renderers still use the older boxed table styling. On the current branch, Markdown table appearance is owned by two separate rendering paths:

- editor-backed Markdown surfaces use `RichTextStyles.table_style`
- AI block list tables build a separate WarpUI `TableConfig` inline

That split creates two technical problems:

1. the shared editor-backed renderer still paints outer borders, vertical dividers, filled header backgrounds, and alternating row backgrounds
2. future surfaces can drift again because there is no single shared source for Markdown table chrome

The implementation should make every current Markdown table renderer inherit the same structural style while preserving surface-specific typography and existing editing/selection behavior.

## Relevant code
- `specs/zachlloyd/markdown-table-consistency/PRODUCT.md` — approved product behavior
- `specs/blocklist-markdown-table-rendering/TECH.md` — prior AI block list table implementation plan
- `app/src/notebooks/editor/mod.rs (145-215)` — `rich_text_styles`; current shared editor-backed table style source
- `crates/editor/src/render/model/mod.rs (424-568)` — `RichTextStyles` and `TableStyle`
- `crates/editor/src/render/element/table.rs:1-220` — editor-backed Markdown table painting path
- `app/src/ai/blocklist/block/view_impl/common.rs (1140-1270)` — `render_table_section`; current AI block list table renderer and inline `TableConfig`
- `crates/warpui_core/src/elements/table/mod.rs (117-239)` — shared `TableConfig`, `RowBackground`, and `TableVerticalSizing`
- `crates/warpui_core/src/elements/table/mod.rs (537-759)` — intrinsic width measurement logic
- `crates/warpui_core/src/elements/table/mod.rs (1013-1211)` — viewported row layout and painting behavior
- `app/src/notebooks/file/mod.rs (230-278)` — file notebook view using `rich_text_styles`
- `app/src/ai/ai_document_view.rs (292-321)` — AI document view fallback editor using `rich_text_styles`
- `app/src/ai/document/ai_document_model.rs (783-801)` — AI document model creating notebook-backed editors with `rich_text_styles`
- `app/src/code/editor/comment_editor.rs (505-544)` — comment editor using `rich_text_styles`
- `app/src/code/editor/view.rs (2357-2381)` — `code_text_styles`; clones `rich_text_styles` before overriding non-table text settings

## Current state

### Editor-backed Markdown tables
Most rendered Markdown surfaces in the app are backed by `NotebooksEditorModel` and the shared editor renderer. Their table appearance comes from `rich_text_styles()` in `app/src/notebooks/editor/mod.rs`, which currently sets:

- `border_color = theme.surface_3()`
- `header_background = theme.surface_2()`
- `cell_background = theme.background()`
- `alternate_row_background = Some(theme.surface_2())`
- `cell_padding = 6.`

That style is then consumed by `RenderableTable` in `crates/editor/src/render/element/table.rs`. The renderer currently always paints:

- row backgrounds for header and body
- an outer top/bottom/left/right border
- horizontal row dividers
- vertical column dividers

This is the old notebook/editor table treatment that the new product spec wants to replace.

### Surfaces that inherit the editor-backed table style
The old table style is not limited to notebooks. The same `rich_text_styles()` entry point is reused by multiple surfaces:

- notebooks
- file notebooks
- AI documents / planning docs
- Markdown comment editors

In addition, `code_text_styles()` starts from `rich_text_styles()` before overriding paragraph-level settings, so any code-editor surface that ends up rendering Markdown tables also inherits the same table style unless it explicitly overrides it.

This means a change to the shared editor-backed style source will propagate to multiple surfaces automatically.

### AI block list tables
The AI block list no longer uses the old style. `render_table_section()` in `app/src/ai/blocklist/block/view_impl/common.rs` builds a WarpUI `Table` with an inline `TableConfig` that already matches the desired structural treatment:

- `outer_border: false`
- `column_dividers: false`
- `row_dividers: true`
- transparent header/background fills
- `cell_padding: 12.`
- `TableVerticalSizing::ExpandToContent`
- `measure_body_cells_for_intrinsic_widths: true`

That renderer is already the visual baseline from PR #23908, but it is defined inline inside the block list path rather than shared with editor-backed renderers.

### Ownership problem
Today there is no shared “Markdown table appearance” abstraction. The editor-backed path and block list path both describe the same visual decisions in different structures:

- `TableStyle` for the editor renderer
- `TableConfig` plus per-cell text styling for WarpUI `Table`

As a result:

- the two renderers can drift
- future surfaces may pick one path and forget to copy the latest style
- structural decisions like “no outer border” or “no column dividers” are not represented in the editor-backed style model yet

## Proposed changes

### 1. Introduce a shared Markdown table appearance helper
Add a small shared helper at the app layer that defines the canonical Markdown table chrome for the new blockless treatment.

The helper should represent the structural and color decisions that must stay aligned across renderers:

- divider color
- header text color
- body text color
- transparent vs filled backgrounds
- whether outer borders are shown
- whether column dividers are shown
- whether row dividers are shown
- row striping behavior
- cell padding

Typography should remain surface-specific. The helper should define the table chrome and text-color hierarchy, not force every surface to use the same font family or font size.

This gives us one source of truth that both renderers can map from.

### 2. Extend `TableStyle` so the editor renderer can express the blockless treatment
`crates/editor/src/render/model/mod.rs` currently cannot represent some of the structural choices already used by the block list renderer.

Extend `TableStyle` with the missing structural controls needed by the product spec:

- `outer_border: bool`
- `column_dividers: bool`
- `row_dividers: bool`

Keep the existing color and typography fields. The important change is letting the editor-backed renderer express “only horizontal separators, no outer border, no vertical dividers” directly instead of baking those assumptions into painting logic.

### 3. Update the editor-backed table painter to honor the expanded style model
Update `crates/editor/src/render/element/table.rs` so painting follows `TableStyle` rather than hardcoded table chrome assumptions.

Specifically:

- `paint_backgrounds()` should respect transparent header/body backgrounds and the absence of alternating row backgrounds
- `paint_borders()` should paint only the borders/dividers enabled by `TableStyle`
- the renderer should continue using the existing text layout, selection, cursor, and alignment logic unchanged

This keeps the behavioral parts of the editor renderer stable while changing only the visual treatment.

### 4. Make `rich_text_styles()` produce the new Markdown table style
Update `app/src/notebooks/editor/mod.rs` so `rich_text_styles()` returns the blockless Markdown table style from the new shared helper.

That means the shared editor-backed path should move from the current boxed style to:

- no outer border
- no column dividers
- row dividers only
- transparent header background
- transparent body background
- no alternating row backgrounds
- header/body text colors matching the block list hierarchy
- cell padding aligned with the block list treatment

This change is the main propagation point for notebooks, file notebooks, AI documents, comment editors, and any other surface using the shared editor-backed Markdown renderer.

### 5. Refactor the block list table renderer to consume the same shared appearance helper
Update `app/src/ai/blocklist/block/view_impl/common.rs` so the block list no longer constructs its structural table chrome entirely inline.

The block list should still keep its surface-specific text settings where needed:

- AI font family
- AI font size
- AI font weight
- AI selection color

But the structural table configuration should come from the same shared appearance helper used by `rich_text_styles()`. That keeps the existing block list result visually unchanged while preventing future drift.

### 6. Preserve surface-specific typography
The product goal is shared styling, not identical typography across unrelated surfaces.

The shared appearance helper should therefore be mapped differently by each renderer:

- editor-backed surfaces continue using their surface’s `font_family` / `font_size` in `TableStyle`
- block list continues using its AI-output text settings

What must remain shared is the blockless table chrome and the text hierarchy relationship, not every literal font token.

### 7. Treat the shared helper as the default for future Markdown table renderers
Document in the code by naming and placement that this helper is the default source for Markdown table appearance in Warp.

The goal is that a new Markdown-rendering surface should not invent its own `TableConfig` or `TableStyle` values for tables unless it has a clear product reason to diverge.

## End-to-end flow
1. A surface creates a rendered Markdown editor or table view.
2. If it is editor-backed, it obtains `RichTextStyles` from `rich_text_styles()` or `code_text_styles()`.
3. `rich_text_styles()` builds `table_style` from the shared Markdown table appearance helper.
4. The editor renderer lays out and paints table content using the updated `TableStyle`, which now supports the blockless chrome.
5. If it is the AI block list path, `render_table_section()` builds its WarpUI `Table` using the same shared appearance helper, while keeping block-list-specific typography and selection settings.
6. The user sees the same structural Markdown table treatment across surfaces, with existing interaction behavior preserved.

## Risks and mitigations

### Risk: editor-backed behavior regressions
Changing `RenderableTable` painting could accidentally affect selection visibility, cursor readability, or perceived cell hit areas.

Mitigation:
- keep layout, selection, cursor, and alignment code unchanged
- scope the change to styling and border/background painting
- validate editable surfaces manually after the visual update

### Risk: block list drifts again later
If the block list keeps hand-authoring its own `TableConfig`, the two renderers can diverge again even after this change.

Mitigation:
- refactor both paths to read from the same shared appearance helper
- avoid leaving structural table chrome duplicated inline

### Risk: future surfaces bypass the shared style
Even after current surfaces are fixed, a new renderer could hardcode another table style.

Mitigation:
- make the helper discoverable and clearly named as the canonical Markdown table appearance
- reference it directly from both existing rendering paths so future work sees the pattern

### Risk: typography becomes unintentionally identical everywhere
If the shared helper carries too much font data, it could flatten legitimate surface-specific typography differences.

Mitigation:
- keep the helper focused on chrome and text hierarchy
- let each renderer keep its own font family, size, and weight choices where appropriate

## Testing and validation

### Shared-style tests
- Add focused tests for the shared Markdown table appearance helper so its structural defaults are explicit:
  - no outer border
  - no column dividers
  - row dividers enabled
  - transparent backgrounds
  - no alternating row striping
  - updated padding

### Editor-backed renderer coverage
- Add or update tests around editor-backed Markdown tables to ensure table rendering still occurs and existing Markdown table behavior does not regress.
- Keep existing geometry/selection-oriented table tests in `crates/editor/src/render/element/table_tests.rs` passing after the style changes.

### Surface propagation checks
- Manual validation in:
  - Markdown notebook
  - Markdown editor
  - file notebook
  - AI document / planning document
  - Markdown comment editor
  - AI block list
- Confirm these surfaces all show the same blockless table chrome.

### Behavior regression checks
- Manual validation of:
  - left/center/right alignment
  - inline formatting inside cells
  - cursor placement in editable contexts
  - selection within and across cells
  - link rendering and interaction
  - wide-table overflow handling

### Visual validation
- Screenshot-based comparison against the Figma node and the AI block list implementation from PR #23908
- Manual confirmation that the older notebook/editor chrome is gone everywhere in scope

## Follow-ups
- If additional Markdown table renderers appear outside the current editor-backed path and block list path, route them through the shared appearance helper rather than creating another table style definition.
- If we later want stricter visual parity between surfaces, we can consider a deeper shared text-style adapter, but that should be a follow-up after the chrome is unified.
