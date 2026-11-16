# Markdown Table Styling Consistency — Product Spec
Linear: none provided
Figma: House of Agents — https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7451-99490&t=NvrWl7bhDEC5kpKF-1
Reference styling baseline: PR #23908 — https://github.com/warpdotdev/warp-internal/pull/23908

## Summary
Make rendered Markdown tables use the same blockless visual treatment across every Warp surface that renders Markdown tables.

The block list styling introduced in PR #23908 should become the visual baseline for Markdown tables everywhere they render in Warp, including Markdown notebooks and the Markdown editor. This is a consistency and presentation change, not a redesign of Markdown table semantics or editing behavior.

## Problem
Warp currently renders the same Markdown table with noticeably different visual treatments depending on where the user sees it.

After PR #23908, AI block list tables use a lighter, blockless presentation based on the House of Agents design: no outer container, no full border, no vertical dividers, no filled header row, and more typography-driven hierarchy. Other Markdown-rendering surfaces still use the older table treatment, which creates visible inconsistency in:

- border and container chrome
- row density
- header emphasis
- background fills
- overall visual weight

This makes the same Markdown content feel like different components in different parts of the product. Users moving between notebooks, the Markdown editor, AI documents, comment editors, file notebooks, and AI output should not have to mentally re-parse the same table because Warp styled it differently in each surface.

## Goals
- Make rendered Markdown tables in every current Warp Markdown-rendering surface match the visual treatment now used in the AI block list.
- Treat the merged block list styling from PR #23908 as the visual source of truth for this work.
- Preserve existing Markdown table semantics, supported inline formatting, alignment rules, and editability.
- Preserve existing selection, cursor, link, copy, and scrolling behavior unless a change is required to achieve the styling consistency.
- Ensure the same Markdown table feels like the same component across all Warp Markdown-rendering surfaces.
- Ensure future Warp surfaces that adopt Markdown table rendering inherit this same style by default rather than introducing another table treatment.

## Non-goals
- Changing Markdown table parsing rules or supported syntax.
- Introducing new Markdown table editing capabilities.
- Changing block list table styling again as part of this work.
- Redesigning non-table Markdown blocks.
- Converting non-table text into tables.
- Broadly restyling every table-like UI in Warp outside Markdown-rendered tables.

## Figma / design references
- Figma: House of Agents — https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7451-99490&t=NvrWl7bhDEC5kpKF-1
- PR baseline: https://github.com/warpdotdev/warp-internal/pull/23908
- Existing related spec: `specs/blocklist-markdown-table-rendering/PRODUCT.md`

The Figma node shows the blockless table treatment that now exists in the AI block list and should be reused for Markdown tables across Warp.

## User experience

### Scope
This feature applies to rendered GitHub Flavored Markdown tables in:
- Markdown notebooks
- the Markdown editor
- file notebooks and other rendered Markdown file views
- AI documents and other notebook-backed planning/editor surfaces
- Markdown comment editors and other lightweight rendered Markdown editors
- any other current or future Warp surface that renders Markdown tables through the shared Markdown rendering stack or through a dedicated Markdown table renderer

If a Warp surface renders a valid Markdown table as a table, it should adopt the updated styling described here.

This feature does not change the appearance of raw Markdown source when the user is viewing or editing literal source text rather than a rendered table presentation.

### Visual consistency rule
The user-visible rule is simple:

The same valid Markdown table should look materially the same in the AI block list and in every other Warp surface that renders Markdown tables.

The goal is not approximate similarity. The goal is one shared visual treatment for Markdown-rendered tables across these surfaces.

### Styling invariants
Rendered Markdown tables in all Warp Markdown-rendering surfaces should adopt the same structural styling introduced for block list tables in PR #23908.

That means:

- the table renders directly in the Markdown flow rather than inside a card-like or boxed container
- the table does not show a rounded outer container
- the table does not show a full perimeter border
- the table does not show vertical divider lines between columns
- the header row does not use a filled background distinct from the table body
- body rows do not use alternating zebra-striping backgrounds
- the header row uses stronger typography than body rows
- header text uses primary text color
- body text uses secondary text color
- row separators are thin horizontal dividers only
- row spacing matches the block list treatment, with a more open vertical rhythm than the older notebook/editor style
- the table should feel visually lighter and less boxed-in than the previous notebook/editor treatment

The blockless Figma version is the intended design target.

### Typography and spacing
The table should continue to use the surrounding Markdown typography system for font family and sizing, but with the same hierarchy used in the block list treatment:

- headers are bold or semibold and visually prominent
- body cells are regular weight
- body text uses the same size as surrounding Markdown body text
- vertical padding should match the block list style closely, approximately 12px per row

If theme-specific token values differ across surfaces, the relationship must still remain the same:

- header text is visually stronger than body text
- divider lines are subtle
- the table has no filled container or boxed background treatment

### Functional behavior that must remain unchanged
This restyle should not change what Markdown tables mean or how users interact with them.

The following behaviors should remain unchanged unless a specific implementation constraint requires an adjustment:

- valid table detection
- column alignment behavior
- inline Markdown rendering inside cells
- text selection within a cell
- text selection across cells and rows
- cursor placement and text editing behavior in editable contexts
- link rendering and interaction
- copy behavior
- horizontal overflow handling for wide tables
- surrounding Markdown flow before and after the table

This is a styling consistency feature, not a behavior change feature.

### Alignment and inline formatting parity
All currently supported Markdown table content should continue to render correctly after the restyle.

At minimum, every affected Markdown-rendering surface must continue to support:

- left, center, and right column alignment
- bold text
- italic text
- bold-italic text
- inline code
- strikethrough
- links
- escaped pipe characters rendered as literal pipes
- multiline cell content, if already supported today

Changing the visual style must not collapse centered or right-aligned columns back to left alignment.

### Editing contexts
Some Markdown table surfaces are editable and some are read-only. The restyle must not make editing feel worse anywhere editing is supported today.

If a user can place a cursor in a rendered table cell today, they should still be able to do so after the change.

If a user can:

- click into a cell
- move the cursor with keyboard navigation
- select text across table content
- type, delete, or paste within the table

those interactions must continue to work with the new styling.

The visual update must not reduce hit targets, make selections harder to read, or create ambiguity about which cell is active.

### Wide tables
Wide tables should remain readable and usable.

If a table requires horizontal overflow handling, the updated styling must preserve the user’s ability to:

- access off-screen columns
- read header and cell content clearly
- select text after horizontally scrolling
- edit content in editable surfaces if that behavior is supported today

This feature should not introduce clipping, truncation, or layout breakage that did not exist before.

### Invalid or unsupported table-like content
This feature does not change fallback behavior.

If content is not a valid Markdown table and currently falls back to normal Markdown text rendering, it should continue to do so. Likewise, table-looking content inside code blocks should continue to render as code, not as a table.

### Existing and newly rendered content
The updated style should apply consistently to:

- newly created notebooks and documents
- existing notebooks and documents when reopened
- tables already present in saved content
- new tables inserted after the feature ships
- any current or future surface that renders Markdown tables once it adopts the shared renderer or shared style source

Users should not have to migrate or rewrite table Markdown to get the new treatment.

## Success criteria
- A Markdown table rendered in any current Warp Markdown-rendering surface uses the same blockless styling pattern as the AI block list table from PR #23908.
- Warp no longer shows two competing visual treatments for rendered Markdown tables across its current Markdown surfaces.
- Other Markdown-rendering surfaces no longer show the older table chrome such as a full outer border, vertical dividers, zebra striping, or a filled header background.
- Header rows remain visually distinct through typography and text color rather than heavy background treatment.
- Body rows use secondary text color and subtle horizontal separators, matching the block list treatment.
- The same Markdown table shown in the AI block list and in other Warp Markdown-rendering surfaces looks materially identical aside from surface-specific typography or layout constraints.
- Alignment behavior remains correct for left-, center-, and right-aligned columns.
- Inline Markdown inside cells continues to render correctly.
- Editing and selection behavior in editable Markdown surfaces does not regress.
- Wide tables remain usable and readable.
- Invalid Markdown table input continues to fall back to existing non-table behavior.

## Validation
- Manual validation with the same Markdown table rendered in the AI block list and in multiple editor-backed surfaces, including:
  - AI block list
  - Markdown notebook
  - Markdown editor
- Spot-check validation in other current Markdown-rendering surfaces that use the shared rich-text Markdown renderer, such as AI documents, file notebooks, and comment editors where practical.
- Visual comparison against the Figma node and the current AI block list implementation from PR #23908.
- Manual validation that affected Markdown-rendering surfaces no longer show an outer border, vertical dividers, zebra striping, or a filled header row.
- Manual validation that header typography, body text color, divider treatment, and row spacing match the block list treatment closely.
- Manual validation of left-, center-, and right-aligned columns.
- Manual validation of inline formatting inside cells: bold, italic, bold-italic, inline code, strikethrough, links, and escaped pipes.
- Manual validation of cursor placement, selection, typing, deletion, and paste behavior in editable contexts.
- Manual validation of wide tables that require horizontal scrolling.
- Regression validation that malformed table-like content and fenced code blocks continue to avoid table rendering.
- Screenshot-based review confirming that the editor-backed result is visually consistent with the merged block list result.

## Open questions
None currently. New Warp surfaces that render Markdown tables should inherit this style by default.
