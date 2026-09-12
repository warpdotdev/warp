# Support `<details>`/`<summary>` in markdown rendering — Product Spec

GitHub issue: https://github.com/warpdotdev/warp/issues/10259

Figma: none provided

## Summary

Render `<details>`/`<summary>` HTML blocks in Warp-rendered markdown as collapsible disclosure sections, matching GitHub-flavored markdown. Today both tags render as literal text with no toggle, so agent output, READMEs, and plans that use progressive disclosure show every hidden section expanded and inline.

## Goals

- Render a `<details>` block as a disclosure widget whose summary is always visible and whose body collapses and expands on user action.
- Support formatted markdown inside the summary and full markdown inside the body, including nested `<details>`.
- Degrade deterministically on malformed input, so no markdown input produces an undefined or inconsistent rendering.
- Make the disclosure interactive on the two surfaces where a user can act on it: the agent conversation view and the notebook/plan editor.

## Non-goals

- Supporting arbitrary HTML tags. This spec covers `<details>` and `<summary>` only; every other unconsumed tag keeps its current literal-text behavior.
- Preserving byte-exact markdown source through a round trip. Warp's rich-text pipeline re-serializes canonically, and this feature follows that policy.
- Adding a disclosure widget to Warp's headless TUI renderer, which has no pointer input and no per-view fold state. The TUI renders details content expanded.
- Authoring affordances for creating a `<details>` region from a toolbar or command. Users type the tags; the editor round-trips them.
- Persisting collapse state across sessions, panes, or restarts.

## Behavior

### Rendering

1. A `<details>` block whose opening tag starts a line renders as a disclosure section: a summary row followed by the body content. On an interactive surface the summary row carries a disclosure indicator; behavior 19 governs surfaces that are not interactive.

2. The body renders as ordinary markdown. Headings, lists, tables, code blocks, images, and nested `<details>` inside the body render exactly as they would outside it.

3. The summary renders as formatted inline markdown. Bold, italic, inline code, and links inside `<summary>` render with their normal styling and remain clickable.

4. A `<details>` block with the `open` attribute renders expanded on first render. Without `open`, it renders collapsed. `open` is a boolean attribute: its presence means expanded regardless of any value written on it, so `open="false"` renders expanded, matching HTML and GitHub. Other attributes on either tag are ignored and do not affect rendering.

5. Tag names match case-insensitively, so `<DETAILS>` and `<Summary>` behave as their lowercase spellings do. Whitespace is allowed between the tag name or its attributes and the closing `>`.

### Interaction

6. On an interactive surface, clicking the summary row toggles the section between collapsed and expanded. The section is keyboard focusable, and Enter or Space toggles it. The disclosure indicator reflects the current state.

7. The innermost interactive element under the pointer handles a click. A click on a link inside the summary (behavior 3) activates that link and does not toggle the section; a click anywhere else on the summary row toggles. Keyboard activation is unaffected: Enter or Space with the section focused toggles it, and a link inside the summary is reached by its own focus stop. This matches how a link nested in any other clickable row behaves.

8. A drag that begins on the summary row selects text and does not toggle. Toggling happens on release, and only when the pointer stayed within the row and did not move past the surface's drag threshold. Summary text is selectable, which behavior 10 requires of it, so a press on the row cannot commit to toggling before the drag is resolved.

9. Toggling a section changes only what is displayed. It never modifies document content — the `open` attribute in the source is untouched by toggling — never marks a buffer dirty, and never adds an undo entry. Collapse state lives only in the open view: closing and reopening a document renders every section at its default state (behavior 4).

10. Copying a selection that spans a details section yields the summary text and the full body text, including the body of a section that is currently collapsed. Collapse is a view state, so it never removes content from a copy, a save, or a serialization.

11. A collapsed body is not reachable by keyboard or by caret. Tab traversal skips focus stops inside it, including links, and arrow-key or click-to-place caret movement moves across the collapsed region rather than into it. The summary row keeps its focus stop while the section is collapsed, so a collapsed section is always keyboard-disclosable: Tab reaches it and Enter or Space expands it (behavior 6). Reaching hidden content requires expanding the section first, so focus and the caret never rest somewhere the user cannot see. This is a narrower rule than behavior 10's: copy reads through a collapsed region because a selection can legitimately span it, while focus and the caret are positions the user must be able to see to use.

12. Nested `<details>` sections render as independently toggleable sections. Collapsing an outer section hides its nested sections; expanding it restores each nested section to the state it already had. A nested section's own state is untouched by an ancestor's toggle — it is hidden along with the rest of the ancestor's body, not reset.

### Nesting guard

13. Nesting is supported to a depth of 64. A `<details>` opening tag at depth 65 or deeper renders as literal text, and its content renders as ordinary markdown. The now-unmatched `</details>` renders as literal text under behavior 14(b). An over-depth region's rendered text therefore contains the characters `<details>` and `</details>` themselves, where a within-depth region's rendered text contains only its summary and body; serialization writes those same literal characters back, so the round trip is stable. The depth is a fixed constant, so the same input always produces the same rendering. This bound is a stack-safety guard on the recursive body parse, not a product limit: it sits far above any realistic content — content in the wild rarely nests past 3 — and far below the depth at which the recursion could exhaust a stack. The tech spec derives the number.

### Malformed and unsupported input

14. Malformed and unsupported input degrades deterministically, and markup the parser does not consume renders as visible literal text rather than being silently dropped. Each rule below states what a reader sees and what a save writes back: an unconsumed tag survives a round trip as the literal characters of that tag, so no edit or serialization can delete a details tag the user typed.

    a. A `<details>` with no matching `</details>` takes the rest of the enclosing content as its body. A nested unclosed `<details>` ends where its parent ends. Serialization writes the opening `<details>` and no closing tag, so the round trip reproduces the same unclosed region.

    b. A `</details>` with no open `<details>` renders as literal text, and serialization writes the literal characters `</details>`.

    c. A `<details>` or `</details>` tag that does not start a line renders as literal text and does not open or close a section. Leading whitespace is permitted before an opening tag; any other preceding character on the line, including a backtick, disqualifies it.

    d. A self-closing `<details/>` opens a section with no distinct closing tag, so it degrades under 14(a).

    e. A `<details>` with no `<summary>` renders with the literal summary label `Details`. The label is a rendering substitute, not content: serialization writes no `<summary>` element, so the round trip reproduces a summary-less region rather than materializing the word `Details` into the document.

    f. Only a `<summary>` that opens the details body is the summary. A `<summary>` appearing after body content renders as literal text, as does each `<summary>` after the first.

    g. A `<summary>` with no matching `</summary>` takes the rest of the details body as its summary, leaving the body empty. When the enclosing `<details>` is itself unclosed under 14(a), the summary ends at the end of the enclosing content, so a single unclosed `<summary>` can consume the remainder of the document into one summary row. Serialization writes the opening `<summary>` and no closing tag, so the round trip reproduces the same unclosed summary and the body stays empty.

    h. A `</summary>` with no open `<summary>` renders as literal text, and serialization writes the literal characters `</summary>`.

    i. Markdown block structure inside `<summary>` is not honored: a code fence, a nested `<details>`, or a nested `<summary>` inside a summary renders as literal inline text, and a multi-line summary renders as one line. GitHub renders block content inside a summary as real blocks; Warp models a summary as a single inline run, so this diverges — see "Divergences from GitHub rendering".

15. Tags inside a code region are content, not markup. A `<details>`, `</details>`, `<summary>`, or `</summary>` line inside a fenced code block in the body renders verbatim in the code block and does not open or close any section. A tag inside an inline code span likewise never opens or closes a section, because it fails the line-start rule in 14(c).

16. A code fence opened inside a details body and never closed leaves the rest of the body inside that fence, so no later `</details>` closes the section and the section degrades under 14(a).

### Block-level scope

17. A `<details>` opens a section only at document level or at the block level of a details body.

    a. A `<details>` indented under a list item terminates the list and opens a section as its own block. GitHub nests it inside the list item and continues the list; Warp diverges — see "Divergences from GitHub rendering".

    b. A `<details>` inside a blockquote does not open a section and renders as literal text, as does its `</details>`. GitHub nests the disclosure section inside the blockquote; Warp diverges — see "Divergences from GitHub rendering".

### Surfaces

18. On an interactive surface, a details section exposes disclosure semantics to assistive technology: the summary is an activatable control that reports its expanded or collapsed state, and the association between that control and the body it controls uses renderer-generated identifiers, never identifiers taken from the markdown input.

19. On surfaces that render markdown without user interaction — banners, modals, settings pages, changelog entries, the command palette, and the TUI — a details section renders as its summary row followed by its fully expanded body, with no disclosure indicator and no toggle. No content is hidden on a surface where the user cannot reveal it.

### Streaming and editing

20. While markdown streams in, a `<details>` whose closing tag has not yet arrived renders progressively under behavior 14(a), and content above it does not reflow as the block grows. A section the user has toggled keeps the state the user chose as further content arrives: a streaming update never reverts a section to its `open` default, and the arrival of the closing tag is not itself a state change.

21. Editing inside or around a details region in the notebook/plan editor always leaves a well-defined document. The buffer accepts every edit — markdown is a plain-text format and the editor never rejects a keystroke because of what it would produce — so deleting one of the region's boundaries degrades under the same rules as behavior 14(a) and 14(b) rather than producing an undefined state. The nesting guard in behavior 13 applies where content is read back out, not to the edits themselves.

    Collapse state follows the region across edits rather than being reset by them. An edit inside a collapsed body leaves the section collapsed — the section does not spring open because its hidden text changed — and an edit that moves or removes one of the region's boundaries carries the collapse to whatever region the rebalancing rules then define, dropping it when no region remains. Because collapse is view state (behavior 9), losing it is never a content loss.

22. Existing rendering is unchanged for documents containing neither tag. Markdown that contains no `<details>` and no `<summary>` renders exactly as it does today.

## Divergences from GitHub rendering

GitHub-flavored markdown is the reference implementation for this feature, and Warp matches it everywhere except the three cases below. These are not decisions to remain divergent. Each one requires a structural change to the parser's intermediate representation, which is a flat sequence of lines with no container blocks; this spec scopes the implementation to the subset that representation supports today, and convergence on all three is future work.

| Case | GitHub | Warp | Structural reason |
|---|---|---|---|
| `<details>` indented under a list item (behavior 17(a)) | Nests inside the list item; the list continues afterwards | Terminates the list, then opens a section as its own block | List items are flat lines carrying an indent level, not containers with child blocks, so there is no list-item body for a section to nest inside |
| `<details>` inside a blockquote (behavior 17(b)) | Nests inside the blockquote | Renders as literal text | The intermediate representation has no blockquote variant at all, so there is nothing to nest into |
| Block content inside `<summary>` (behavior 14(i)) | Renders as real blocks, so a heading in a summary is a heading | Renders as literal inline text on one line | A summary is modeled as a single inline run, in both the parser IR and the editor buffer |

An unindented `<details>` following a list item terminates the list on both GitHub and Warp, and is not a divergence.
