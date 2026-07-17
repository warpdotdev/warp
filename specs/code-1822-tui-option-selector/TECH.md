# TECH: Reusable TUI option selector over shared option snapshots

## Context

This slice builds on the frontend-neutral orchestration option snapshots
(base commit `d6da3b23`; see `specs/code-1822-option-snapshots/TECH.md` for the data
contract). At that base, `app/src/tui_export.rs` re-exports `OptionSnapshot`,
`OptionRow`, `OptionBadge`, `OptionSourceStatus`, and `OptionFooter`, but nothing in
`crates/warp_tui` renders them: the TUI has no single-select list primitive.

This PR adds that primitive — `TuiOptionSelector` — on top of the generic
`TuiEditorView::single_line` supplied by the preceding stack branch. The next slice
(the TUI orchestration permission/configuration card) embeds the selector to render
its per-field configuration pages; the same primitive is intended for future
AskUserQuestion and permission prompts, which is why it is snapshot-driven and knows
nothing about orchestration edit state.

## Proposed changes

### `crates/warp_tui/src/option_selector.rs`

A `TuiView` + `TypedActionView` (`TuiOptionSelector`) rendering one page:

- `OptionSelectorPage` owns the full renderable page configuration: a short field
  label, sequence position, full prompt, option snapshot, and search opt-in. The
  header renders the field label on the left, right-aligned `← n of m →` navigation
  state (boundary arrows muted), a blank separator row, and the bold prompt.
- Option list rendered from an `OptionSnapshot` (`warp::tui_export`): up to
  `MAX_VISIBLE_OPTION_ROWS` (6) rows visible at once with `↑` / `↓` overflow markers.
  Rows show a viewport-relative `(1)`-style number, the label, an optional badge
  suffix (`(default)` / `(recent)` / `(connected)`), and — for disabled rows — the
  `disabled_reason`. The selected row is bold magenta without an extra marker or
  background.
- Optional search: `set_page(page, ctx)` lazily creates a search editor only when
  `page.searchable` is true, then renders its pinned `Search:` row between the prompt
  and scroll viewport. Search is not a `SelectorItem`; the list starts focused on
  `selected_id` (or its first item) so digits remain immediate shortcuts. Up from the
  top item focuses search, Down from search returns to the first filtered item,
  Up from search selects the last filtered item, and Down from the last item
  focuses search. Typing a non-digit from the list focuses and seeds search.
  Filtering is case-insensitive substring matching over row labels; an empty
  result renders `No matches`. The pinned search editor remains visible while
  rows scroll.
- Status rows appended after the list per `OptionSourceStatus`: `Loading…` (dim),
  `Failed { message }` (error style, plus a selectable `↻ Retry` virtual row that
  emits `RetryRequested`), and `Empty { message }` (dim). Status rows are not
  navigable.
- Footer: `OptionFooter::CustomText { label }` appends a selectable entry that, when
  confirmed, embeds a one-line `TuiEditorView` in place of the entry.
  Submitting a value replaces the generic footer label with that value, keeps the
  footer selected, and pre-fills the value when it is edited again. A selected id
  not present in the fixed rows restores this custom value when a page is rebuilt.
  `OptionFooter::CreateNewAuthSecret` is ignored (resource creation is out of scope
  in the TUI).

State/API surface for the embedding host:

- `new(ctx)` then `set_page(page, ctx)` — atomically replaces the page configuration,
  resets the search query and selection to the snapshot's `selected_id` (falling back
  to the first item), and discards any in-progress custom-text editing.
- `refresh_snapshot(snapshot, ctx)` — in-place catalog refresh preserving the
  active selection when it still exists, else falling back to `selected_id`.
- `confirm_selected(ctx)` — the shared confirmation core used by the selector's
  Enter/Numpad Enter action and by hosts that need to combine confirmation with
  another interaction. Enabled rows emit `TuiOptionSelectorEvent::Confirmed { id }`;
  disabled rows stay selected so their reason remains visible; while the custom-text
  editor is active it validates (trimmed, non-empty — else an inline
  "Enter a value to continue." error) and emits `CustomTextSubmitted { value }`.
  While search owns focus, confirmation selects the first enabled filtered row,
  skipping disabled matches.
- `handle_back(ctx) -> bool` — the host's Escape path: cancels active custom-text
  editing or clears a non-empty search while search owns focus, and reports whether
  the key was consumed so the host only leaves the page when the selector had
  nothing to unwind.
- `TuiOptionSelectorEvent::LayoutInvalidated` — tells hosts with separately cached
  measurements to remeasure after scrolling changes overflow markers, a catalog
  refresh changes the row set, search changes the rendered rows, or the custom-text
  validation row toggles. `ctx.notify()` still refreshes the child itself; the event
  crosses the view boundary to invalidate the ancestor's cache, matching
  `TuiAIBlockEvent::LayoutInvalidated` prior art.

Focus and element-level input (via the private `SelectorInputElement` wrapper, active only
while the selector is rendered as the blocking interaction):
- The list and embedded editors are real focus zones. `set_page` focuses the selector;
  boundary arrows move focus between the selector and search editor.
- Enter and Numpad Enter dispatch `ConfirmSelected` from the selector element, so
  row, search-result, retry, and custom-text confirmation stay reusable host-agnostic
  behavior.
- Up/Down move the selection, scrolling to keep it visible. Search behaves as
  the final item in the cycle: Up from the first row focuses search, Up from
  search selects the last row, Down from the last row focuses search, and Down
  from search selects the first row.
- Digits 1-6 confirm the corresponding visible row — viewport-relative, so digit 1 is
  always the top visible row after scrolling. While search owns focus, digits are
  editor input instead.
- Row clicks select the row and confirm it when enabled via per-item persistent
  `MouseStateHandle`s (owned by the view, per the mouse-state ownership rule).
- Wheel scrolling moves the viewport without moving the selection.
- Search and custom text use the shared `TuiEditorView`; printable characters, cursor,
  selection, single-line paste, horizontal/word/line navigation, undo/redo, and
  kill/yank come from the shared editor layer. Escape remains host policy with a
  selector fallback: it cancels active custom editing; otherwise, it clears a
  non-empty search only while search owns focus. When neither editor has an
  interaction to unwind, it leaves the page.
- An element-level Escape fallback emits `Dismissed` for hosts without their own
  Escape binding; the embedding card's keymap normally consumes Escape first.

Selection reuses `InlineMenuSelection` and `keep_selected_visible` from
`crates/warp_tui/src/inline_menu.rs`.

### Generic editor dependency

The preceding stack branch adds `TuiEditorView::single_line` and the shared editor
interaction layer documented in
`specs/code-1822-tui-generic-editor-view/TECH.md`. The generic field owns focus,
single-line insertion/replacement policy, selection, kill/yank state, model-backed
editing, one-row cursor following, and stale-viewport clamping
(`crates/warp_tui/src/editor_view.rs` (37-202);
`crates/warp_tui/src/editor_interaction.rs` (15-559)).

This selector owns the surrounding search/custom-text labels, validation,
filtering, Enter/Escape behavior, and vertical navigation. The generic editor does
not register Up/Down or Shift-Up/Shift-Down bindings, so those keys propagate to
the selector's list/focus cycle instead of being consumed by the embedded field.

### `crates/warp_tui/src/tui_builder.rs`

Adds `option_selector_selected_style()`: bold, full-strength magenta text for the
selected option. The card slice adds its orchestration surface background and
remaining recipes (title glyph, selected metadata values, identity palette) itself.

### `crates/warp_tui/src/lib.rs`

Declares `mod option_selector` with a narrowly-scoped, commented
`#[allow(dead_code)]` on the module declaration, since nothing consumes the selector
until the card slice; that slice removes the allow.

## Testing and validation

- `crates/warp_tui/src/option_selector_tests.rs` covers: field label/position/prompt
  rendering and initial selection from `selected_id`; Up/Down + Enter confirmation;
  selector-element handling for Enter and Numpad Enter;
  digit confirmation, including viewport-relative digits in scrolled lists; scrolling
  to keep the selection visible with overflow markers; disabled rows being
  selectable but not confirmable via Enter, digit, or click; Loading/Empty status
  rows being non-selectable; the Failed state's keyboard-reachable Retry row;
  custom-text trim/validate/submit, submitted-value rendering/re-editing/restoration,
  and Backspace; Back cancelling custom-text editing before leaving the page; the
  ignored `CreateNewAuthSecret` footer; snapshot-refresh
  selection preservation and selected-value fallback; lazy search-editor creation;
  `LayoutInvalidated` emission when overflow markers or custom-text validation change
  rendered height; badge rendering;
  and paste falling through from the list while the custom-text editor consumes it
  using only the first line;
  searchable pages starting on the selected row; boundary focus handoff; numeric
  shortcuts remaining active from the list; digit-containing queries; filtering,
  no-match rendering, Enter confirmation from focused search, and clear-on-Escape.
- Tests host the selector under `test_fixtures::TestHostView` in a headless TUI
  window and render to lines (see the `tui-testing` conventions).
- Commands: `./script/format`,
  `cargo nextest run -p warp_tui -E 'test(option_selector)'`,
  `cargo nextest run -p warp_tui`, and
  `cargo clippy -p warp_tui --tests -- -D warnings`.

## Follow-ups

The TUI orchestration card slice embeds `TuiOptionSelector` for its configuration
pages (host, environment, harness, model, API key, location), adds the remaining
orchestration theming recipes, and removes the module-level `allow(dead_code)`.
