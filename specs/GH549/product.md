# Notebook editor: Raw/Rendered toggle for Mermaid code blocks

## Summary
When a notebook code block's language is set to `Mermaid`, Warp shows a Raw/Rendered icon-button toggle in the block footer. The toggle defaults to Raw only when the user explicitly creates or converts a Mermaid code block in an ordinary editable notebook, which keeps in-progress source text editable. Planning documents and rendered Markdown file views default Mermaid blocks to Rendered so diagrams are visible by default. Selecting Rendered renders the source as a full-width diagram whose height is derived from the loaded SVG's aspect ratio; if rendering fails, an error frame is shown. The language dropdown also shows branded icons for each language.

Reference: GitHub issue `warpdotdev/warp-external#549`.

## Problem
The notebook code block language dropdown exposes `Mermaid` as a selectable language. Today, as soon as the user picks `Mermaid`, Warp unconditionally switches the block into its Mermaid diagram rendering path. This makes ordinary code, plain notes, or work-in-progress diagrams appear as a broken or empty diagram frame instead of staying readable and editable as normal text.

## Goals / Non-goals

**Goals:**
- Mermaid code blocks default to Raw (source text) only when the user explicitly creates or converts a Mermaid code block in an ordinary editable notebook.
- Mermaid code blocks in planning documents and rendered Markdown file views default to Rendered diagrams.
- A Raw/Rendered segmented control lets the user explicitly opt in to diagram rendering per block.
- Rendered mode shows the rendered SVG diagram, or an error frame when the source is invalid.
- The authored Mermaid source is always preserved in the buffer, regardless of display mode.

**Non-goals:**
- Making the Raw/Rendered choice persistent across sessions or round-trips to markdown.
- Reworking the code-block language dropdown, the list of supported languages, or the styling of non-Mermaid code blocks.
- Changing the Mermaid render pipeline's theme, caching, or `mermaid_to_svg` conversion behavior.
- Changing Mermaid behavior in other non-notebook surfaces such as read-only agent output.

## Figma
Figma: https://www.figma.com/design/Lvb72IUdZsYXHj4pjMj6uu/Render-images-and-mermaid-diagrams?node-id=48-4195

## Behavior

The following invariants apply to notebook code blocks whose language is set to `Mermaid` (via the dropdown or via markdown round-trip).

**Language selection**

1. The `Mermaid` option remains available in the code block language dropdown.
2. Picking `Mermaid` sets the block's language to Mermaid in the buffer; the block serializes as a ```` ```mermaid ```` fenced code block on markdown export, regardless of display mode.
3. Picking `Mermaid` in an ordinary editable notebook does not, on its own, trigger diagram rendering — the block opens in Raw mode.

**The language dropdown**

4. The code block language dropdown button and menu are wide enough to display language names and their icons without truncation (wider than the current narrow implementation).
5. Each language option in the dropdown displays a branded icon alongside its text label (e.g., the Go gopher, Python snake, Rust gear, etc.). Languages without an available branded icon fall back to a generic code icon.

**The Raw/Rendered toggle**

6. A Mermaid-labeled block displays a Raw/Rendered toggle in the block footer using two icon buttons — a code-brackets icon (`<>`) for Raw and a dataflow/graph icon for Rendered. These form a segmented-control-style pair.
7. The active button shows a visible background highlight (surface overlay) to indicate which mode is selected; the inactive button shows no background. The Raw button is highlighted in Raw mode; the Rendered button is highlighted in Rendered mode.
8. The footer leaves a clear horizontal gap between the `Mermaid` language label and the Raw/Rendered segmented control, so the label and buttons do not visually crowd each other.
9. In ordinary editable notebooks, the toggle defaults to Raw when the user creates a new code block and sets its language to Mermaid, or changes an existing code block's language to Mermaid.
10. The toggle is visible whenever the block's language is Mermaid, regardless of which mode is active — including when the diagram is successfully rendered. The toggle must not disappear when the block switches to Rendered mode.
11. The Raw/Rendered choice is per-block and per-session only — it is not persisted to the notebook file or round-tripped through markdown export.

**Raw mode**

12. In Raw mode the block renders as an ordinary notebook code block: editable source text, the standard code-block chrome (border, copy button, language dropdown), and `Mermaid` shown as the selected language.
13. All ordinary code-block editing behaviors apply in Raw mode: click to place a cursor, select, type, paste, copy, cut, backspace/delete individual characters, and undo/redo.
14. The buffer content is the source of truth and is preserved regardless of display mode.

**Rendered mode — successful render**

15. When the user selects Rendered, Warp attempts to render the block's current source as a Mermaid diagram using the existing SVG rendering pipeline.
16. While the async render is in progress the block shows a "Rendering Mermaid diagram…" placeholder inside a full-width diagram frame.
17. Before the Mermaid SVG has loaded, the diagram frame uses the full available code-block content width and a stable placeholder height that does not depend on the raw Mermaid source text height.
18. On a successful render the block shows the rendered diagram inside the diagram frame. The rendered frame uses the full available code-block content width, and its height is derived from the loaded SVG's aspect ratio at that full width. The rendered height must not be derived from the raw source text height, and loaded diagrams must not be capped to their intrinsic SVG width when additional block width is available.
19. A block that starts in Rendered mode, including a planning-document Mermaid block or rendered Markdown file Mermaid block that renders by default, must relayout to this same full-width/aspect-ratio height as soon as the initial SVG load completes. The user must not need to toggle Raw/Rendered to get the correct natural-width height.
20. In Rendered mode, the block does not show a text insertion cursor/caret over the diagram. Clicking the rendered diagram may select the block or interact with footer controls, but it must not leave a flashing text cursor inside the diagram frame.

**Rendered mode — failed render**

21. If the Mermaid source cannot be parsed or rendered, the block shows an error frame in place of the diagram. The error frame displays a message such as "Error rendering Mermaid diagram. Please check syntax."
22. The error frame uses the same full-width frame and border style as the diagram frame — it does not fall back to code-block view or use raw source text height.
23. The Raw/Rendered toggle remains visible and functional in the error state. The user can switch back to Raw to edit and fix the source.

**Round-trip and export**

24. The block is persisted and exported as a ```` ```mermaid ```` fenced code block regardless of the current display mode.
25. Reopening an ordinary editable notebook opens Mermaid blocks in Raw mode (the toggle resets to Raw on every open). Opening the same markdown as a rendered Markdown file view opens Mermaid blocks in Rendered mode.

**Planning documents**

26. AI planning documents use the same underlying Mermaid block UI and markdown serialization, but Mermaid blocks default to Rendered rather than Raw.
27. A rendered Mermaid block in a planning document uses the same full-width/aspect-ratio sizing behavior and must not show a flashing text cursor over the diagram.
28. Users can switch a planning-document Mermaid block back to Raw for the current session; the underlying markdown remains a fenced Mermaid code block.

**Rendered Markdown file views**

29. Directly opened Markdown files default to the rendered Markdown view rather than raw Markdown source when Warp opens them in the Markdown viewer.
30. Mermaid blocks in the rendered Markdown file view default to Rendered diagrams rather than Mermaid source text.
31. Users can switch a rendered Markdown file back to Raw from the pane header Markdown toggle; Raw mode opens the file in the code editor. Returning to the rendered Markdown view defaults Mermaid blocks to Rendered again for that view.

**Feature flag**

32. This behavior is gated by the existing `FeatureFlag::MarkdownMermaid` flag. When the flag is off, Mermaid blocks render as ordinary code blocks with no toggle and no diagram rendering.
