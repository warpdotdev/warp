# Markdown `<details>` / `<summary>` Support (GH-10259)

## Summary

Warp's markdown renderer should support standard HTML5 `<details>` and
`<summary>` tags so collapsible disclosure widgets work in any rendered
markdown surface (agent output, README previews, conversation list previews).
Today these tags are stripped or rendered as plain text, which breaks a common
GFM convention used heavily in agent outputs and open-source READMEs.

## Problem

- `<details>` / `<summary>` is part of GitHub Flavored Markdown's de facto
  surface and standard HTML5. Many sources Warp renders (READMEs, agent
  outputs, copy-pasted content) rely on it for progressive disclosure.
- In the current renderer, the tags are dropped or escaped, producing flat
  output that is hard to scan — particularly in long agent responses where
  authors deliberately collapse supporting detail.
- Adding the two tags is a small, well-bounded addition to the existing
  sanitizer allowlist; the cost of leaving it out is disproportionate to the
  cost of adding it.

## Goals

- Render `<details>` / `<summary>` as collapsible disclosure widgets in every
  markdown surface that renders in **interactive mode** per B9.1 (agent
  message bodies, README previews, changelog entries, login failure
  notifications, and any other surface where `FormattedText` is the primary
  read target). Surfaces that render in **non-interactive mode** per B9.1
  (notably the conversation list row preview) render `<details>` as inert
  inline summary text with no disclosure widget; this is by design and is
  the SOLE departure from the "collapsible disclosure widget" model. There
  is no third "partial widget" mode — every rendering surface picks exactly
  one of `Interactive` or `NonInteractivePreview` at the call site.
- Preserve the `open` attribute in interactive mode. In non-interactive
  mode the `open` attribute is ignored per B9.1 (the body is always
  collapsed and never emitted into the DOM in that mode).
- Support nested `<details>` UP TO a hard cap of 32 levels (B5, B8.1). Levels
  beyond the cap fall through to plain-text rendering deterministically.
- Keep sanitization safe — no event handler attributes, no broader HTML
  whitelist expansion.
- Treat the feature as untrusted-recursive-markup-safe: ALL resource limits
  (depth, count, recursive guard) are deterministic hard caps, not soft
  heuristics.
- Provide accessible behavior matching native `<details>` semantics.

## Non-Goals

- Not full HTML rendering. Only `<details>` and `<summary>` are added to the
  allowlist.
- No other tag whitelist expansion beyond what the renderer already supports.
- No animated disclosure transitions in V1.
- No persistence of open/closed state across re-renders in V1 (see B9 for
  V1.5 option).
- No new markdown surfaces — this only changes how existing surfaces render
  these tags.

## Behavior Contract

### B1. Recognized tags

- `<details>` — block-level container. May carry `open` attribute. No other
  attributes preserved.
- `<summary>` — exactly one is the visible always-rendered header. Additional
  `<summary>` siblings inside the same `<details>` are concatenated into the
  first.

### B2. Default state

- Collapsed unless `open` is present on `<details>`.

### B3. Toggle

- Click on the summary text or the chevron icon toggles open/closed.
- Both surfaces of the summary row are clickable; the entire row is the hit
  target.

### B4. Keyboard

- Tab focuses the summary.
- Enter and Space both toggle open/closed.
- Focus ring matches the existing focusable controls in the markdown surface
  (consistent with links, code-copy buttons, etc.).

### B5. Nested `<details>`

- Supported up to a HARD CAP of 32 nesting levels. Beyond the 32nd level,
  additional `<details>` markup falls through to plain-text rendering per
  B8.1 (the over-cap `<details>` and `<summary>` opening / closing tags
  appear as visible literal text; the inner body content still renders
  through the standard markdown pipeline). This is the SINGLE rule for
  nesting depth — no "arbitrary depth" support is promised anywhere in the
  spec; all references must agree with this 32-level cap.
- Each level (within the cap) renders an independent disclosure with proper
  indentation.
- Toggling a parent does not alter the open/closed state of children.
- The 32-level cap is a deterministic hard limit, not best-effort. See B8.1.

### B6. Inside `<summary>`

- Inline markdown is rendered: bold, italic, inline code, links.
- Block markdown (lists, headings, code blocks) inside `<summary>` is NOT
  supported. Block tokens that appear within a summary collapse to inline
  rendering, matching the HTML spec's content model for `<summary>`.

### B7. Inside `<details>` body (after `<summary>`)

- Full markdown rendering, including code blocks, lists, blockquotes, nested
  details, links, images.

### B7.1. Malformed `<details>` handling

Untrusted markdown commonly contains malformed HTML. The renderer must remain
robust and never drop content silently. The following cases are explicit:

- **`<details>` with NO `<summary>` child**: render with the default fallback
  summary text `"Details"`. Body content renders normally. The block is NOT
  dropped.
- **`<summary>` not as the first child of `<details>`**: hoist the FIRST
  `<summary>` element to the front. Any inline content that preceded it
  becomes part of the body, in original order. Subsequent `<summary>`
  siblings within the same `<details>` are concatenated into the first
  summary (with a single space separator), per B1.
- **Orphan `<summary>`** (a `<summary>` outside any `<details>` ancestor):
  rendered as inline plain text — the tag itself is stripped, but inner
  inline-markdown is preserved (e.g. `<summary>**hi**</summary>` becomes
  bold `hi`).
- **Unclosed `<details>`**: closed at parse time at the next surface boundary
  (end of input, end of containing block); the partial block emits normally
  with whatever body content was collected.
- **Unclosed `<summary>` inside `<details>`**: closed at the next sibling
  block or at `</details>`, whichever comes first.
- **Nested `<summary>` inside another `<summary>`**: the inner `<summary>` is
  treated as inline plain text within the outer summary (tag stripped, inner
  inline-markdown preserved).
- **Empty `<summary>`** (no text content): render with the fallback summary
  `"Details"`.
- **Block-level elements inside `<summary>`** (lists, headings, code blocks):
  collapse to inline per B6; preserve text content but strip block formatting.

These rules are enforced AT PARSE TIME, before sanitization, so the
sanitizer's allowlist (B8) operates on a normalized tree.

### B8. Sanitization

- Allowlist gains `<details>` and `<summary>` only.
- Attribute allowlist (DETERMINISTIC — sanitizer drops anything not on the
  list, regardless of value):
  - `details`: `[open]` only.
  - `summary`: empty — NO attributes preserved.
- All other attributes — `on*` event handlers (`onclick`, `onmouseover`,
  `onfocus`, …), `style`, `id`, `class`, `aria-*` (any aria attribute
  appearing in INPUT markdown), `data-*`, custom attributes — are
  unconditionally stripped before the renderer sees the tree.
- The sanitizer runs AFTER parse-time normalization (B7.1) and BEFORE the
  renderer; the renderer therefore never inspects user-provided attributes.
- Existing sanitizer behavior for other tags is unchanged.

### B8.2. Renderer-generated ARIA identifiers

The accessibility hookup for the custom-widget path (B10) requires
`aria-controls` to point at a unique body region per `<details>`. To keep
this safe under untrusted input:

- The renderer GENERATES `id` and `aria-controls` values AFTER sanitization,
  using a per-surface counter (e.g. `warp-details-1`, `warp-details-2`, …,
  `warp-details-N`). These IDs are unique within a single rendered surface.
- ANY `id` or `aria-*` attribute present in the INPUT markdown is stripped by
  the sanitizer (B8) before the renderer runs. Input-side identifiers can
  NEVER appear on the rendered DOM and therefore CANNOT be used to forge
  cross-element references.
- The renderer-generated `aria-controls` value is the ONLY way a `<summary>`
  toggle is wired to a body region.
- When the view layer renders native `<details>` / `<summary>` semantics,
  identifier generation is skipped entirely (the browser/native widget owns
  the relationship). The custom-widget path is the only path that emits
  generated `aria-controls` IDs.

### B8.1. Resource limits (security — deterministic hard caps)

`<details>` is a recursive container. Untrusted markdown can therefore embed
deeply nested or extremely high-cardinality `<details>` trees that could
exhaust the stack or starve rendering. The renderer MUST defend against this
with EXACT, deterministic limits — no soft caps, no best-effort approximations,
no probabilistic backoff.

- **Maximum nesting depth: HARD CAP of 32 levels** of `<details>` per
  rendered surface. The 33rd-level `<details>` (and every deeper level)
  renders as PLAIN TEXT — the literal `<details>` and `<summary>` opening /
  closing tags become visible text in the output, and any inner markdown
  body still renders normally through the existing markdown pipeline (no
  content is silently dropped). This bound is exact: the 32nd-level widget
  renders, the 33rd does not. There is NO grace, NO heuristic, and NO
  approximation — implementations MUST treat 32 as an exact threshold.
- **Maximum `<details>` count per rendered surface: HARD CAP of 1000.**
  Beyond the 1000th `<details>`, additional `<details>` blocks render as
  plain text, same fallback as the depth cap. Like the depth cap, this is
  exact: the 1000th block renders as a widget, the 1001st renders as plain
  text. Implementations MUST NOT defer the cutoff, sample, batch, or
  otherwise shift the boundary.
- **Implementation MUST be iterative or guarded recursion**. The depth/count
  bookkeeping must be enforced inside the parser/renderer; it MUST NOT rely
  on unbounded native recursion that risks a stack overflow on adversarial
  input. Either implement with an explicit work stack, or use guarded
  recursion that checks the depth counter before recursing and falls
  through to the plain-text fallback when the cap is hit.
- **Enforcement boundary**: the depth and count limits are enforced AT or
  BEFORE the renderer — not solely in the sanitizer — so that a future
  change to the sanitizer can't accidentally remove the bound. This makes
  rendering a function of the post-parse tree, not of sanitizer state.
- **Mutual / template-style recursion** (e.g. content that, after expansion,
  would re-introduce nested `<details>`): bounded by the same depth counter
  that wraps the recursive render call, so it cannot escape via indirection.
- **Determinism contract**: ALL resource limits in this section are
  deterministic hard caps. The terms "soft cap", "best-effort",
  "approximate", "probabilistic", "sampled", "approximate threshold" are
  EXPLICITLY DISALLOWED in any conformant implementation. Tests MUST verify
  the exact boundary (the 32nd level renders; the 33rd does not. The 1000th
  renders; the 1001st does not).

### B9. Open/close state persistence

- V1: state is local to the rendered instance and is NOT persisted across
  re-renders of the same content (e.g. scroll-out and back, theme switch).
- V1.5 (deferred — see Open Questions): persist via stable summary-text hash
  scoped to the rendering surface.

### B9.1. Surface-mode rendering (interactive vs non-interactive)

The renderer is used in surfaces that have very different interaction
contracts. Surfaces fall into one of two MODES, decided per render call:

- **Interactive mode.** Default for surfaces where the rendered
  `FormattedText` is the primary content the user reads and acts on
  (agent message bodies, README previews, changelog entries, login
  failure notifications). Full B3/B4/B10 disclosure behavior applies:
  click/keyboard toggle, focus ring, native-or-custom-widget rendering.
- **Non-interactive mode.** Used for surfaces where the rendered
  text is a SECONDARY summary or preview, NOT the primary read target
  (notably the **conversation list row preview**, where the row's
  click target is "open this conversation" — NOT "toggle a disclosure
  inside the row's preview"). In this mode:
  - `<details>` always renders **with the body collapsed and inert**.
    The summary text is shown as plain inline text; the body is not
    rendered into the DOM at all.
  - The summary chevron is NOT shown (no toggle affordance).
  - The summary element is NOT focusable, NOT keyboard-activatable,
    has NO `role="button"`, and NO `aria-expanded` /
    `aria-controls`. It is rendered as inert inline content with
    accessible name = the summary text.
  - The row's existing click handler (e.g. "open this conversation")
    is preserved and is the ONLY click target for the row. A click
    on the rendered summary text routes to the row's handler — never
    to a toggle.
  - The `open` attribute on `<details>` is IGNORED in this mode (the
    body is always collapsed and never rendered into the preview), so
    a long agent-authored body cannot accidentally bloat the preview
    line height.
  - All other markdown (paragraphs, inline code, bold/italic) renders
    normally; only `<details>` is mode-shifted.
- **Mode selection.** The rendering site picks the mode explicitly via
  the `FormattedText` rendering options (e.g. a
  `DetailsRenderMode { Interactive, NonInteractivePreview }` parameter
  on the renderer call). The conversation list preview surfaces — and
  any future preview surface where the rendered text is not the primary
  click target — pass `NonInteractivePreview`. All other surfaces use
  the default `Interactive`. This selection is decided at the call site,
  not inferred from heuristics.
- **Resource limits unchanged.** B8.1's depth + count caps still apply
  in non-interactive mode; over-cap blocks fall through to plain-text
  rendering exactly as in interactive mode.

### B10. Accessibility

- Render as native `<details>` / `<summary>` element when the underlying view
  layer supports them, OR an equivalent ARIA pattern when using a custom
  widget:
  - `role="group"` on the disclosure container.
  - **Summary toggle is an activatable control with a focusable target.**
    The custom-widget path MUST give the summary element ALL of:
    - `role="button"` on the summary element (disclosure-button
      pattern). This is the activatable role; without it, AT users
      cannot interact with the toggle.
    - `tabindex="0"` so the summary participates in the tab order
      (the native `<summary>` is implicitly focusable; the custom
      path is not, so an explicit tabindex is required).
    - `aria-expanded="true"` when the body is open, `aria-expanded="false"`
      when collapsed. The attribute is updated on every toggle.
    - `aria-controls` pointing at the body region's renderer-generated
      `id` per B8.2 (the only way the summary is wired to the body).
    - Keyboard activation handlers for both Enter and Space (B4) on the
      summary element. The handlers MUST call `preventDefault` for
      Space so the page does not scroll while focus is on the toggle.
    - Visible focus ring matching B4's "consistent with links,
      code-copy buttons, etc." requirement; the focus ring MUST appear
      on the summary element itself, not on a parent or descendant, so
      keyboard users can see the focused toggle.
  - The native-element path (`<details>` / `<summary>`) inherits all of
    the above from the browser/native widget and the custom-widget
    requirements above DO NOT apply.
- Screen readers must announce open/closed state and the accessible name from
  the rendered summary text.

## Settings / API surface

No new settings. No new API.

The change is internal to the markdown rendering pipeline:

- Parser allowlist update.
- Sanitizer attribute allowlist update.
- Renderer adds a `details` block component.

## Acceptance Criteria

- A1. `<details>` with no `open` attribute renders collapsed; only the
  summary text is visible.
- A2. `<details open>` renders expanded by default.
- A3. Click on summary toggles between collapsed and expanded.
- A4. Tab focuses the summary; Enter and Space each toggle the state.
- A5. Three-deep nested `<details>` renders correctly with independent state
  per level and proper indentation.
- A6. Inline markdown (bold, italic, inline code, links) inside `<summary>`
  renders correctly.
- A7. Block markdown (lists, headings, code blocks) inside `<summary>`
  collapses to inline rendering — no nested block tokens.
- A8. Full markdown — including code blocks, lists, links, images, and nested
  `<details>` — renders inside the body.
- A9. Sanitizer rejects `<script>`, event handler attributes (`onclick`,
  `onmouseover`, …), and any non-`open` attribute on `<details>` /
  `<summary>`. Specifically: `id`, `class`, `style`, `aria-*`, `data-*` on
  input markup are stripped before the renderer runs.
- A10. Accessibility tree audit shows correct `aria-expanded` /
  `aria-controls` / accessible name (or native semantics when using
  `<details>`/`<summary>` directly). On the custom-widget path,
  `aria-controls` values are RENDERER-GENERATED (e.g. `warp-details-1`),
  unique within the surface, and never derived from input markup.
- A_hard_cap_32. The 32-level nesting depth cap is exact: rendering input
  with 33 nested `<details>` produces 32 disclosure widgets and 1 plain-text
  fallback at the over-cap level. The boundary is deterministic — the same
  input always yields the same split.
- A_hard_cap_1000. The 1000-element count cap is exact: rendering input
  with 1001 sibling `<details>` blocks produces 1000 disclosure widgets and
  1 plain-text fallback at the 1001st block. The boundary is deterministic;
  there is no soft, best-effort, sampled, or probabilistic behavior.
- A_resource_limits_deterministic. Identical input always produces an
  identical split between widget-rendered and plain-text-fallback
  `<details>` blocks across runs. Implementations MUST NOT shift the
  boundary based on load, randomness, or backoff.

## Implementation Pointers

> Paths verified against the worktree at spec time. Warp uses an in-house
> markdown parser (`crates/markdown_parser`) built on top of `html5ever`
> (see imports in `crates/markdown_parser/src/html_parser.rs`). The renderer
> is a Warp UI element (`FormattedTextElement`), not browser HTML. Modules
> that don't yet exist are marked `(new)` so reviewers can distinguish
> net-new files from edits to existing files.

- Markdown parser entry points (block-level pass; this is where
  `<details>` must be recognized as a block container and `<summary>` as
  inline-only):
  `crates/markdown_parser/src/markdown_parser.rs`.
- HTML-in-markdown pass (today's allowlist of inline/phrasing element tags
  lives here as `PHRASING_ELEMENT_TAGS`; top-level skips live as
  `TOP_LEVEL_ELEMENT_TAGS_TO_SKIP`. Add `details` as a recognized block
  container and `summary` as a recognized inline header; this is also where
  the malformed-input normalization in B7.1 is enforced):
  `crates/markdown_parser/src/html_parser.rs`.
- Markdown parser library entry / shared types
  (`FormattedText`, `FormattedTextLine`, `FormattedTextFragment` — extend
  with a `Details { open, summary, body }` variant or equivalent):
  `crates/markdown_parser/src/lib.rs`.
- Renderer / view-layer element (UI side of `FormattedText` — add a
  disclosure widget rendering for the new `Details` variant; this is where
  the click handler, focus ring, and `aria-expanded` / `aria-controls`
  hookup live). The custom-widget path generates `aria-controls` / matching
  `id` per B8.2 using a per-surface counter (`warp-details-1`,
  `warp-details-2`, …). When the native `<details>` / `<summary>` semantics
  path is taken, the renderer SKIPS identifier generation:
  `crates/warpui_core/src/elements/formatted_text_element.rs`.
- Renderer call sites — B9.1 mode selection. B9.1 makes `DetailsRenderMode`
  an EXPLICIT per-call parameter, so every existing `FormattedText` render
  site must opt in to `Interactive` or `NonInteractivePreview`. The default
  for a freshly-added render site is `Interactive`; the
  `NonInteractivePreview` call sites are enumerated below and MUST pass the
  mode explicitly (the renderer MUST NOT infer mode from surface heuristics,
  per B9.1 "decided at the call site, not inferred from heuristics"):
  - **`Interactive` (default mode)** — these surfaces render `FormattedText`
    as the primary read target and pass `Interactive` (either explicitly or
    by relying on the default):
    `app/src/resource_center/section_views/changelog_section.rs` (changelog
    entry bodies),
    `app/src/launch_configs/save_modal.rs` (launch-config save modal
    description),
    `app/src/auth/login_failure_notification.rs` (login failure
    notification body),
    `app/src/changelog_model.rs` (changelog model render entry point),
    and any agent message body / README preview render site reachable from
    `crates/warpui_core/src/elements/formatted_text_element.rs`.
  - **`NonInteractivePreview` (MUST pass explicitly)** — these surfaces
    render `FormattedText` as a SECONDARY preview where the row/cell's own
    click target is the primary interaction. They MUST pass
    `NonInteractivePreview` at the call site:
    - Conversation list row preview (the canonical case from B9.1). The
      conversation-list row renders a one-line preview of the latest
      message; the row's click target is "open this conversation", and
      the preview MUST NOT introduce a competing toggle target. Search
      anchor for reviewers: the conversation-list row view that renders
      the latest-message preview via `FormattedText` (the renderer call
      that produces the inline preview text inside the row).
    - Any future preview surface where `FormattedText` is rendered inside
      a row/cell whose own click handler owns the primary interaction
      (e.g. search-result row previews, history-list row previews). When
      adding such a surface, the call site MUST pass
      `NonInteractivePreview`; reviewers should flag any new render site
      that omits the mode argument and accepts the `Interactive` default
      where the surrounding row owns the click.
  - Reviewers should spot-check that (a) the new `Details` variant is
    exercised everywhere `FormattedText` is rendered, and (b) every
    `NonInteractivePreview` call site above is wired with the explicit
    mode argument and not relying on the default.
- Sanitization / attribute allowlist (today's allowlist for HTML-in-markdown
  is implemented inside `html_parser.rs`; this is where `details: [open]`
  and `summary: []` are wired. The sanitizer here MUST drop `on*` event
  handlers, `style`, `id`, `class`, ALL `aria-*` (any aria attribute
  appearing on input), and `data-*` per B8 — only the `open` attribute on
  `<details>` survives onto the renderer's tree):
  `crates/markdown_parser/src/html_parser.rs`.
- `(new module)` Resource-limit guard for B8.1 — depth + count counters,
  single source of truth, used by both the parser block-handler and the
  renderer fallback path. The caps are EXACT (32 nesting, 1000 count); the
  guard exposes a single `should_fallback(depth, count) -> bool` that
  returns the same result every call for the same inputs (deterministic).
  Suggested location: `crates/markdown_parser/src/details_limits.rs`. The
  renderer's recursive rendering of `Details` MUST consult this guard (or
  accept depth as a parameter) and switch to the plain-text fallback once a
  cap is hit. Implementations MUST NOT introduce randomness, sampling, or
  load-aware behavior here.
- Existing parser tests (extend with `<details>`/`<summary>` cases,
  including the malformed-input matrix from B7.1 and the resource-limit
  matrix from B8.1):
  `crates/markdown_parser/src/markdown_parser_tests.rs`,
  `crates/markdown_parser/src/html_parser_tests.rs`.
- Tokenizer note: ensure `<summary>` content is parsed inline regardless of
  whether the source contains block-level markdown — handle the collapse in
  the tokenizer or renderer, not via post-hoc string trimming.

## Tests

- T1. `<details>...</details>` with no `open` renders collapsed; only the
  summary is in the visible DOM.
- T2. `<details open>` renders expanded; body contents are visible.
- T3. Click on summary toggles open ↔ closed.
- T4. Tab navigation reaches the summary; Enter and Space each toggle.
- T5. Three-deep nested `<details>` parses and renders correctly with
  independent state per level.
- T6. Inline markdown inside `<summary>` (bold, italic, inline code, link)
  renders as expected.
- T7. Block markdown inside `<summary>` (e.g. a list) collapses to inline
  rendering — no `<ul>` / `<h1>` / `<pre>` emitted inside the summary.
- T8. Full markdown inside the body: code block, ordered list, link, image,
  nested `<details>`.
- T9. Sanitizer fuzz: payloads like `<details onclick=…>`,
  `<summary><script>…</script></summary>`,
  `<details style="display:none">` are stripped to safe output.
- T_sanitizer_strips_id. Input `<details id="evil">…</details>` and
  `<summary id="evil-sum">…</summary>` are sanitized so the rendered tree
  carries NO input-side `id` attribute. Any renderer-generated `id` is the
  only `id` present and follows the `warp-details-N` pattern.
- T_sanitizer_strips_aria_input. Input `<details aria-controls="x"
  aria-expanded="true">…</details>` and `<summary aria-label="…">…</summary>`
  are sanitized: NO input-side `aria-*` attribute survives onto the rendered
  DOM. Renderer-generated `aria-controls` / `aria-expanded` are the only
  aria attributes present.
- T_renderer_generates_aria_controls. The custom-widget path emits a unique
  renderer-generated `aria-controls` / matching `id` per `<details>`
  rendered, following `warp-details-N` (or equivalent unique pattern). 50
  `<details>` blocks in one surface produce 50 distinct ID pairs, and
  `aria-controls` on the summary always equals the `id` on the body region.
- T10. Accessibility tree audit on rendered output asserts correct
  `aria-expanded` / `aria-controls` / accessible name (or native
  `<details>` / `<summary>` semantics).
- T11. Markdown content outside any `<details>` block is unchanged versus
  baseline, ruling out renderer regressions.
- T12. Depth cap (B8.1) — exact boundary: a `<details>` tree nested 33
  levels deep renders the first 32 levels as proper disclosure widgets;
  the 33rd level renders as plain text — its `<details>` and `<summary>`
  tags are visible literal text in the output and the inner body still
  renders through the standard markdown pipeline. The render call returns
  within bounded stack usage (no overflow on adversarial input). The test
  asserts the EXACT split: 32 widgets, 1 plain-text fallback. Repeating the
  test produces an identical split (no run-to-run variance).
- T12.1. Depth cap edge: a tree nested EXACTLY 32 levels deep renders all
  32 levels as widgets (no plain-text fallback). Asserts the cap is
  inclusive on the widget side, exclusive on the fallback side.
- T13. Count cap (B8.1) — exact boundary: a surface containing 1001 sibling
  `<details>` blocks renders the first 1000 as widgets; the 1001st renders
  as plain text per the same fallback as T12. Asserts the EXACT split:
  1000 widgets, 1 plain-text fallback. Repeating the test produces an
  identical split.
- T13.1. Count cap edge: a surface containing EXACTLY 1000 `<details>`
  blocks renders all 1000 as widgets (no fallback). Asserts the cap is
  inclusive on the widget side.
- T14. Mutual / template-style recursion (B8.1): an input crafted so that
  expansion would re-introduce nested `<details>` after depth-32 is
  bounded by the same depth counter; the run terminates and the over-cap
  level renders as plain text.
- T15. Malformed input matrix (B7.1): one assertion per case —
  `<details>` with no `<summary>` (fallback `"Details"`); `<summary>`
  preceded by inline content (summary hoisted, prefix becomes body);
  orphan `<summary>` outside any `<details>` (rendered inline, tag
  stripped); unclosed `<details>` (closed at boundary, body emits
  partially); unclosed `<summary>` (closed at sibling block); nested
  `<summary>` inside `<summary>` (inner treated as inline plain text);
  empty `<summary>` (fallback `"Details"`); block element inside
  `<summary>` (collapsed to inline).
- T_multi_summary_concat. Multi-`<summary>`-sibling normalization (B1
  + B7.1): input `<details><summary>One</summary><summary>Two</summary>
  <summary>Three</summary>body</details>` is normalized at parse time
  to a single `<details>` with one summary whose text is `One Two
  Three` (concatenated with single-space separators) and body
  `body`. Inline markdown inside each sibling summary is preserved
  through the concatenation (e.g. bold and links remain rendered).
  Asserts the concatenated count equals exactly the number of
  sibling `<summary>` tags MINUS zero (i.e. all are folded; none are
  dropped). Verified for both 2-sibling and 5-sibling inputs.
- T_summary_hoist_with_concat. Combined hoist + concat case from B7.1:
  input `<details>prefix<summary>A</summary>middle<summary>B</summary>
  body</details>` normalizes to a single summary `A B`, body
  `prefix middle body` (in original order, prefix and middle become
  body content). Asserts ordering is preserved.
- T_non_interactive_mode. Non-interactive (preview) rendering per
  B9.1: input `<details><summary>S</summary>body</details>` rendered
  via the conversation-list-preview surface emits the summary text
  inline with NO chevron, NO focusable element, NO \`role=\"button\"\`,
  NO \`aria-expanded\`, NO \`aria-controls\`, and NO body region in
  the DOM. A click on the rendered summary triggers the row's
  parent click handler, NOT a toggle. The B9.1 \`open\`-ignored rule
  is exercised: \`<details open>...</details>\` renders identically
  to the closed form in this mode (the \`open\` attribute does not
  cause body emission).
- T_custom_widget_activatable_role. Custom-widget accessibility
  contract per B10: when the renderer takes the custom-widget path
  (i.e. native \`<details>\`/\`<summary>\` is not used), the rendered
  summary element has \`role=\"button\"\`, \`tabindex=\"0\"\`,
  \`aria-expanded\` reflecting current state, and an
  \`aria-controls\` ID matching the body region's renderer-generated
  \`id\`. Tab focuses the summary; Enter and Space each toggle the
  state and update \`aria-expanded\`; Space activation does NOT
  scroll the surrounding scroll container. Focus ring is visible on
  the summary itself, not on a parent or descendant.

## Open Questions

- Persist open/closed state across re-renders by stable summary-text hash?
  Suggested for V1.5; defer until V1 ships and we observe whether
  re-render flicker is a real complaint. Persistence scope would be the
  rendering surface (e.g. one agent message), not global.
- Should we render a chevron icon adjacent to the summary, or rely on the
  view layer's native disclosure indicator? Suggested: rely on the view
  layer when using native `<details>`; emit a chevron only in the custom
  ARIA widget path to keep visual parity.

## Telemetry

No new events. If usage signals are needed later, a single
`markdown.details.toggled` event scoped to user-initiated toggles (not
programmatic re-renders) would be sufficient. Out of scope for V1.
