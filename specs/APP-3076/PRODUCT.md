# Block List Markdown Table Rendering — Product Spec
Linear: none provided
Figma: House of Agents — https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7451-99490&t=NvrWl7bhDEC5kpKF-1

## Summary
Render valid GitHub Flavored Markdown tables in AI block list responses as formatted inline tables instead of raw pipe-delimited text. The rendered table should appear directly in the response flow, match the table rendering capabilities already supported in notebooks, and preserve correct text selection behavior.

## Problem
AI responses often include Markdown tables to compare options, summarize data, or present structured results. In the AI block list, those tables are currently much harder to read because the response is shown as plain text table markup rather than as a formatted table. That makes scanning difficult, weakens visual hierarchy, and creates an inconsistent experience with notebooks, where tables already support richer rendering behavior.

This is especially problematic when table cells include inline Markdown such as bold text, inline code, strikethrough, or links. When those constructs are not rendered as formatted content inside the table, the response becomes noisy and less useful.

Selection behavior is also critical. Users need to be able to select text from rendered tables naturally, including partial selections, cross-cell selections, and selections that begin before or after the table in surrounding prose.

## Goals
- Render valid GFM pipe tables inline within AI block list responses.
- Make rendered block list tables visually and behaviorally consistent with the Markdown table primitives already supported in notebooks.
- Support the same inline formatting features inside table cells that notebook tables support.
- Preserve normal text selection and copy behavior for rendered table content.
- Keep surrounding AI response content unchanged before and after the table.
- Fall back gracefully to normal text rendering when the content is not a valid table.

## Non-goals
- Adding table editing capabilities to the AI block list.
- Supporting non-GFM table syntaxes.
- Expanding notebook table feature support as part of this work.
- Introducing a block-list-specific table feature set that diverges from notebooks.
- Converting arbitrary aligned plain text into a rendered table when it is not valid Markdown table syntax.

## User Experience

### Scope
This feature applies to Markdown-formatted AI responses rendered in the AI block list.

If a response contains a valid GitHub Flavored Markdown pipe table, that table should render inline as a formatted table in the response body. Content before and after the table should continue to render as normal Markdown text in the same response.

This behavior applies equally to:
- newly streamed AI responses
- previously rendered responses reopened from history
- transcript or restored views that display the same block list content

### Table detection
A response section should render as a table only when it is a valid GFM pipe table.

A valid table must:
- include a header row
- include a separator row
- have a consistent column structure for rows that belong to the table

If content does not form a valid table, it should remain plain Markdown text. The product should prefer not rendering a table over rendering an incorrect or partially broken table.

Tables inside fenced code blocks must continue to render as code, not as tables.

### Inline rendering behavior
When a valid table is detected, it should render as a formatted table directly in the AI block list rather than as a raw text block containing `|` characters and alignment markers.

The rendered table should behave as part of the normal response flow:
- it appears between the preceding and following response content
- it uses block list styling appropriate for AI output
- it does not require opening a notebook or a separate viewer
- it does not introduce a separate interaction mode

The baseline visual target for inline AI block list tables is the House of Agents design linked above, specifically node `7451:99490`.

The table may use horizontal scrolling when necessary to remain readable within the available block width. If horizontal overflow occurs, the table should remain fully usable and readable without truncating content unpredictably.

Wide tables should have their own horizontal scrollbar so users can scroll sideways within the table itself.

Tall tables should not introduce a separate vertical scrollbar. Vertical movement through table content should happen through the normal block list scroll behavior so the table remains part of the surrounding response flow rather than becoming a nested vertically scrolling region.

### Visual design requirements
Inline AI block list tables should match the House of Agents design:
- the table renders directly in the response flow rather than inside a card-like container
- there is no rounded outer container and no filled table background
- the header row uses bold text and primary text color while body rows use secondary text color
- table cell text uses the same font family and font size as the surrounding block list response content
- rows are separated by thin horizontal dividers
- columns do not show vertical divider lines
- the table does not show a full outer border around the perimeter
- row spacing should match the design’s more open presentation, approximately 12px vertical cell padding

These presentation rules do not change the underlying Markdown, copy-response behavior, or the supported inline Markdown formatting inside cells.

### Rendering parity with notebooks
Rendered block list tables should support the same table rendering primitives already supported in notebooks. The block list should not invent a reduced or alternate table dialect.

At minimum, if notebooks support these constructs within table cells, the AI block list should render them the same way:
- left, center, and right column alignment
- bold text
- italic text
- bold-italic text
- inline code
- strikethrough
- links
- escaped pipe characters rendered as literal pipes

Alignment must remain visibly correct even when a cell's content is narrower than its column. Center-aligned and right-aligned columns should render centered and right-justified within the computed column width rather than collapsing back to left alignment.

More generally, the user-visible rule is:

If a piece of Markdown table content is supported and rendered in notebooks, it should render equivalently in the AI block list unless there is a clear product reason not to. This spec assumes parity is the default.

### Links inside table cells
Links inside table cells should render as links, using the same interaction model users already get for links in other AI Markdown output.

Single-click behavior should match normal link behavior in the block list.

Text selection must still work correctly in cells that contain links:
- click-and-drag should create a text selection rather than unexpectedly activating the link
- selecting across linked and non-linked text should behave the same as selection elsewhere in the response

### Selection behavior
Rendered tables must support normal text selection. This is a core requirement, not a best-effort enhancement.

Selection should work for:
- text within a single cell
- text spanning multiple cells in a row
- text spanning multiple rows
- selections that begin before the table and continue into the table
- selections that begin in the table and continue into following prose
- selections in horizontally scrolled tables

Selection should operate on rendered text content, not on table chrome such as borders, padding, or layout spacing.

When text is copied from a rendered table selection, the copied result should reflect the selected textual content in reading order. It should not include visual-only layout artifacts.

This feature should not regress existing selection behavior elsewhere in the block list.

### Streaming behavior
Because AI responses stream into the block list over time, table rendering should behave predictably during streaming.

The intended experience is:
- once the content is sufficient to identify a valid table, the table renders as a table
- as additional table rows stream in, the rendered table extends naturally
- streaming updates should not unexpectedly clear an active text selection
- the transition from plain incoming text to rendered table should feel stable rather than visually noisy

If the response later stops matching a valid table boundary, subsequent content should render as normal response content after the table rather than corrupting the rendered table.

### Multiple tables and surrounding content
A single AI response may contain:
- multiple tables
- prose before, between, and after tables
- code blocks adjacent to tables
- headings, lists, or other Markdown outside the table

Each table should render independently in the correct place within the response. Non-table content should continue to use its existing rendering behavior.

### Invalid or malformed table content
If a candidate table is malformed, ambiguous, or incomplete, the UI should fall back to rendering the original content as ordinary Markdown text rather than attempting a degraded table rendering.

Examples of content that should not render as a formatted table include:
- lines with pipe characters but no valid separator row
- content inside fenced code blocks
- structures that look table-like but do not resolve into a consistent table section

When a valid table ends, the next non-table content should resume normal rendering immediately after it.

### Large and wide tables
Some AI responses will contain wide tables or cells with long text.

For these cases:
- the table should remain legible inside the block list
- horizontal overflow should be handled gracefully with a table-local horizontal scrollbar
- the user should still be able to select and copy text from off-screen columns by scrolling
- vertical overflow should be handled by the block list's normal scrolling rather than by a table-local vertical scrollbar
- wide content should not break surrounding layout or cause the rest of the response to render incorrectly

### Interaction model
Rendered tables in the block list are read-only presentation of AI output.

Users should not be able to directly edit the rendered table in place. Existing higher-level actions on the AI response should continue to behave as they do today unless explicitly changed by a follow-up spec.

### Copy behavior
Block-level “copy response” behavior should copy the original Markdown source for the AI response, not a rendered or reformatted table export.

This means:
- a copied response preserves the original Markdown table syntax the model returned
- rendering a table inline in the block list does not change what block-level copy response returns
- manual text selection and copy from the rendered output may still copy the selected rendered text content, but response-level copy should preserve the original Markdown source

## Success Criteria
- A valid GFM table in an AI response renders inline in the block list as a formatted table instead of raw pipe-delimited text.
- Column alignment in the block list matches the alignment expressed by the Markdown and matches notebook table behavior.
- Inline Markdown that notebooks support inside table cells also renders correctly inside block list tables.
- Links inside cells render and behave like links without breaking text selection.
- Users can select text naturally within and across rendered table cells and rows.
- Users can extend a selection across table boundaries into surrounding prose and vice versa.
- Wide tables remain usable through a table-local horizontal scrollbar without breaking layout or selection.
- Tall tables do not create a nested vertical scrolling region and instead scroll with the surrounding block list content.
- Block-level copy response preserves the original Markdown source, including original table syntax.
- Invalid table-like content falls back to normal text rendering rather than rendering an incorrect table.
- Multiple tables in one response render independently in the correct order.
- Streamed responses and restored responses render the same table content consistently.

## Validation
- Manual validation with a simple two-column GFM table in an AI response.
- Manual validation with alignment coverage: left, center, and right aligned columns in the same table.
- Manual validation with cell formatting coverage: bold, italic, bold-italic, inline code, strikethrough, links, and escaped pipes.
- Manual validation that text can be selected inside one cell, across multiple cells, across multiple rows, and across table-to-prose boundaries.
- Manual validation that click-and-drag on linked text selects text rather than unexpectedly opening the link.
- Manual validation of a wide table that requires horizontal scrolling, confirming the table shows its own horizontal scrollbar and that readability and selection still work.
- Manual validation of a tall table, confirming it does not show its own vertical scrollbar and instead scrolls with the surrounding block list.
- Manual validation of a response containing prose, a table, more prose, and a second table.
- Manual validation that a fenced code block containing table-looking Markdown still renders as code.
- Manual validation that malformed or incomplete table syntax falls back to plain Markdown text.
- Manual validation that block-level copy response returns the original Markdown source for a response containing a table.
- Regression validation that the same Markdown table content renders equivalently in notebooks and in the AI block list wherever the notebook renderer already supports that content.
