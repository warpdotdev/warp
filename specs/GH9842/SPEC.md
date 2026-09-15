# Spec: Editable prompt suggestions (GH-9842)

## Problem

Prompt suggestions in Warp are all-or-nothing: clicking one
sends the suggested prompt as-is. There's no way to insert it
into the input as a *draft* the user can edit, refine, and then
send themselves. This makes suggestions feel rigid for the
common case where the suggestion is "almost right."

## Goal

Add an alternate affordance on each prompt-suggestion entry that
inserts the suggestion into the agent input as editable text
without dispatching it. The user edits, then submits manually.

## Behavior contract

- B1. Each prompt-suggestion entry exposes two affordances. **Both
  entry points are required in V1** — shipping only one is
  insufficient. They dispatch the same `InsertPromptSuggestionAsDraft`
  action:
  - **Primary click (existing):** sends the suggestion as-is.
    Matches today's interaction; no pixel-parity requirement, but
    the existing card padding and icon-button base style are
    preserved.
  - **New: insert-as-draft entry points (V1 ships BOTH):**
    - **B1a. Edit icon.** A small pencil glyph rendered at the
      trailing edge of each suggestion card. See B7 for affordance
      details.
    - **B1b. Modifier-click.** Alt-click on macOS and Linux,
      Ctrl-click on Windows. Plain (no modifier) click continues
      to send.
- B2. Existing-draft handling. Insert-as-draft REPLACES any existing
  draft in the agent input. If the input is empty, the suggestion
  text becomes the input. If the input has user text (length > 0),
  show a confirm dialog: *"Replace your current draft with this
  suggestion?"* with **[Replace]** and **[Cancel]** buttons.
  - Replace clears the existing draft and inserts the suggestion
    text. The caret lands at the end of the inserted text.
  - Cancel makes no change to the input or draft.
  - The choice is NOT persisted across inserts ("don't ask again"
    is intentionally absent in V1). Each insert with non-empty
    draft prompts again.
  - A future TECH may relax to caret-insert or append-with-newline;
    V1 is replace-with-confirm only.
- B3. Cursor lands immediately after the inserted suggestion text.
- B4. The insert dispatches NO send action. The user must
  manually send via Enter / Cmd-Enter as usual.
- B5. The existing inline-banner prompt-suggestion model
  (`app/src/terminal/view/inline_banner/prompt_suggestions.rs`)
  is the data source; no schema change.
- B6. Telemetry. V1 adds a NEW telemetry event,
  `TelemetryEvent::PromptSuggestionInsertedAsDraft`, that mirrors
  the payload shape of the existing
  `TelemetryEvent::PromptSuggestionAccepted` event (verified in
  `app/src/server/telemetry/events.rs`). The Warp client follows
  an event-per-action pattern (separate `PromptSuggestionAccepted`,
  `StaticPromptSuggestionAccepted`, `PromptSuggestionShown`, etc.);
  V1 stays consistent with that pattern instead of introducing a
  unified `prompt_suggestion.action` event with an `action_type`
  field.
  - **Telemetry events (verified from
    `app/src/server/telemetry/events.rs`):**
    - `PromptSuggestionAccepted { id: String,
      view: PromptSuggestionViewType,
      interaction_source: InteractionSource }` — existing.
    - `StaticPromptSuggestionAccepted { id: String,
      view: PromptSuggestionViewType,
      interaction_source: InteractionSource }` — existing.
    - `PromptSuggestionShown { ... }` — existing.
    - `StaticPromptSuggestionsBannerShown { ... }` — existing.
    - `ZeroStatePromptSuggestionUsed { ... }` — existing.
    - `PromptSuggestionInsertedAsDraft { id: String,
      view: PromptSuggestionViewType,
      interaction_source: InteractionSource }` — **new in V1**.
  - The new event reuses the same three fields (`id`,
    `view`, `interaction_source`) as `PromptSuggestionAccepted`. No
    additional payload fields. `interaction_source` is `Button` for
    edit-icon click and `Keybinding` for modifier-click.
  - For static prompt suggestions inserted as draft, V1 emits
    `PromptSuggestionInsertedAsDraft` (single new event covers both
    static and dynamic suggestions; the suggestion `id` is enough to
    join with prior `*Shown` events on the server side).
  - The fictional `prompt_suggestion.action` event with `category`,
    `source`, and `action_type` is NOT introduced — it conflicts
    with the verified event-per-action shape currently in code.
  - No private payload fields are added; opt-out behavior matches
    the existing `PromptSuggestionAccepted` event exactly.
- B7. Edit-icon affordance & accessibility.
  - Rendered as a 16×16 pencil glyph at the trailing edge of each
    suggestion card.
  - Keyboard-focusable; participates in the suggestion list's
    existing roving-focus model.
  - On focus or hover, surfaces a tooltip: *"Click to send,
    [⌥/Ctrl]-click or this icon to edit first"* (with the modifier
    matching the host OS).
  - Activating via Enter / Space while focused dispatches
    `InsertPromptSuggestionAsDraft`, identical to mouse click.
  - Pixel parity with the existing card layout is NOT required; the
    icon uses the existing icon-button base style and the card's
    existing padding.

#### Accessibility contract (native Warp UI)

Warp's UI is native (GPUI-rendered), so the accessibility
contract is expressed entirely in terms of Warp's native
accessibility primitives — verified against the existing
codebase. No web/DOM primitives are referenced or required.

- **Accessibility label.** The edit icon implements the
  `accessibility_label()` trait method already used by Warp
  search items, slash-command items, profile pickers, etc.
  (see `app/src/terminal/input/{prompts,profiles,
  slash_commands,...}/search_item.rs::accessibility_label`,
  `app/src/search/data_source.rs`). The value MUST be the
  literal string `"Edit suggestion before sending"`. This
  value is the assistive-tech-readable name surfaced through
  Warp's native AT bridge:
  - macOS → NSAccessibility `accessibilityLabel`.
  - Windows → UIA `Name` property.
  - Linux → AT-SPI `accessible-name` /
    `accessibility_content_text`.
- **Focus / keyboard order.** The edit icon participates in
  Warp's native focus system via `FocusHandle` /
  `PaneFocusHandle`. The suggestion list's existing
  roving-focus state is extended to treat the edit icon as a
  focusable peer of the suggestion card, with the order
  suggestion-card → edit-icon → next-suggestion-card →
  next-edit-icon under Tab, reversed under Shift+Tab.
  Activation via `KeyboardAction::Confirm` (Enter) and a
  Space binding routed through the same handler dispatches
  `InsertPromptSuggestionAsDraft`.
- **Validation target.** Accessibility behavior is validated
  against the native AT bridge for each platform:
  - macOS: Accessibility Inspector reports the icon with
    role = button, label = `"Edit suggestion before
    sending"`, and VoiceOver announces the label on focus.
  - Windows: Inspect.exe / Accessibility Insights reports
    the equivalent UIA `Name` property and a button-like
    `LocalizedControlType`.
  - Linux: Accerciser / `dogtail` reports the equivalent
    AT-SPI `accessible-name`.
  Automated coverage in CI uses Warp's existing accessibility
  testing harness — the same one exercised by the
  `accessibility_content_text` and search-bar accessibility
  tests (e.g. `app/src/search/search_bar.rs` and
  `app/src/search/command_search/searcher_test.rs::
  accessibility_label`).
- B8. Sequential composition (NOT out of scope in V1). Users may
  insert multiple suggestions in a row. Each insert applies the
  B2 replace-with-confirm rule against the current draft. The
  suggestion banner remains visible after insert-as-draft; it
  does NOT auto-dismiss. The banner dismisses only on:
  - The user manually sending the prompt (Enter / Cmd-Enter), OR
  - The user manually closing the banner, OR
  - Context loss (conversation switch, tab close, model change).

## Acceptance criteria

- A1. Plain click on a suggestion → sends (existing behavior).
- A2. **Edit icon affordance.** Clicking the pencil icon on a
  suggestion card inserts the suggestion text into the agent
  input, fires no send action, and lands the caret at the end of
  the inserted text. The icon is keyboard-focusable through
  Warp's native focus system (it acquires a `FocusHandle` and
  participates in the suggestion list's roving-focus order),
  exposes the native accessibility label
  `"Edit suggestion before sending"` via the existing
  `accessibility_label()` trait method (verified against
  `app/src/search/data_source.rs` and friends), and Enter/Space
  while focused dispatches the same insert action. Validation
  happens through the native AT bridge (Accessibility Inspector
  on macOS, Inspect / Accessibility Insights on Windows,
  Accerciser on Linux), NOT through DOM-based tooling.
- A3. **Modifier-click affordance.** Alt-click on macOS/Linux and
  Ctrl-click on Windows on a suggestion card inserts the
  suggestion text identically to A2.
- A4. **Both affordances ship together.** A V1 build with only one
  affordance present fails this spec.
- A5. Existing-draft replace flow. With a non-empty draft in the
  input, insert-as-draft shows the replace-confirm dialog. Choosing
  Replace clears the draft and inserts the suggestion at the end;
  choosing Cancel makes no change.
- A6. Empty-draft insert. With an empty input, insert-as-draft
  inserts directly with no confirm dialog.
- A7. **Sequential composition.** After insert-as-draft, the
  suggestion banner remains visible; clicking another suggestion
  prompts the replace-confirm dialog again. Banner dismisses only
  on send, manual close, or context loss.
- A8. Telemetry. The new `PromptSuggestionInsertedAsDraft` event
  fires exactly once per insert with `id`, `view`, and
  `interaction_source` populated identically to the existing
  `PromptSuggestionAccepted` event. No new payload fields are
  introduced. Respects global telemetry opt-out (matching
  `PromptSuggestionAccepted` opt-out behavior).

## Implementation pointers

- Suggestion render is in
  `app/src/terminal/view/inline_banner/prompt_suggestions.rs`.
- `TerminalAction::ResolvePromptSuggestion(...)` (search:
  `app/src/terminal/view/init.rs`) is today's "send-it-now" path.
  Add a sibling `InsertPromptSuggestionAsDraft(PromptSuggestion)`
  action. Keep the full `PromptSuggestion` available until
  telemetry is emitted so identifiers can be derived from existing
  fields.
- **V1 input behavior — replace, NOT insert-at-caret.** V1 sets the
  agent input's text via the existing `replace_buffer_content` API
  (verified: see `Input::replace_buffer_content` used in
  `app/src/workspace/view.rs` and `app/src/pane_group/mod.rs`).
  Concretely:
  - When the input buffer is empty, V1 calls
    `input.replace_buffer_content(&suggestion.prompt, ctx)`. The
    suggestion text becomes the entire draft content.
  - When the input buffer has user text (length > 0), V1 first
    shows the B2 replace-confirm dialog. On `Replace`, V1 calls
    `input.replace_buffer_content(&suggestion.prompt, ctx)`,
    replacing the entire prior draft. On `Cancel`, no buffer
    mutation occurs.
  - In both branches the caret lands at the end of the inserted
    text (existing behavior of `replace_buffer_content`).
  - The earlier "insert at caret" wording is removed from V1 — V1
    is replace-only. Insert-at-caret (preserve surrounding draft,
    insert at current cursor position) is **deferred to V1.5** as
    an additional mode toggled by a future setting; out of scope
    here.
- The agent-input editor lives in
  `app/src/ai/blocklist/agent_view/agent_input_footer/...`. The
  `Input::replace_buffer_content(text: &str, ctx)` entry point is
  the single API V1 uses; no new editor APIs are introduced.

## Test plan

- T1. Modifier-click (Alt/Ctrl per OS) on a suggestion fixture
  dispatches `InsertPromptSuggestionAsDraft`; no send action fires.
- T2. Clicking the edit icon on the same fixture dispatches the
  same action through the same path.
- T3. Edit icon: keyboard activation via Enter and Space when the
  icon is focused dispatches the insert action. Native
  accessibility assertions (NOT DOM):
  - The icon's `accessibility_label()` returns the literal
    string `"Edit suggestion before sending"` (verified by
    direct call against the same trait used by
    `app/src/search/data_source.rs`,
    `app/src/terminal/input/{prompts,profiles,
    slash_commands,...}/search_item.rs::accessibility_label`).
  - The icon registers a `FocusHandle` and is reachable in the
    suggestion list's roving-focus order without injecting a
    `tabindex` attribute (Warp has no DOM).
  - Native AT-bridge validation: macOS Accessibility Inspector,
    Windows Inspect / Accessibility Insights, and Linux
    Accerciser report the icon with role = button and the
    expected accessible name.
- T4. Plain click still dispatches the existing send action.
- T5. Empty-draft path: insert-as-draft against an empty input
  fills the input with the suggestion text, caret at end, no
  confirm dialog shown.
- T6. Non-empty-draft path: insert-as-draft against a non-empty
  input shows the replace-confirm dialog. Replace clears the
  draft and inserts; Cancel preserves the existing draft.
- T7. Sequential composition: two consecutive insert-as-draft
  actions each prompt confirm (when draft non-empty); banner
  remains visible across both.
- T8. Telemetry: `PromptSuggestionInsertedAsDraft` event fires
  exactly once per insert. Payload (`id`, `view`,
  `interaction_source`) matches the existing
  `PromptSuggestionAccepted` shape. `interaction_source = Button`
  for edit-icon click; `interaction_source = Keybinding` for
  modifier-click. Respects global telemetry opt-out.

## Out of scope

- **Insert-at-caret (V1.5).** Inserting suggestion text at the
  current caret position while preserving the surrounding draft.
  Deferred from V1; tracked as a follow-up mode togglable from
  settings.
- Caret-insert / append-with-newline behavior for non-empty drafts
  (V1 is replace-with-confirm; future TECH may relax).
- A "don't ask again" persistence option for the replace-confirm
  dialog.
- Drag-and-drop of suggestions into the input.
- A "save this edited suggestion" path (turning it into a
  reusable rule).
- Pixel parity with any specific design mock — the icon and card
  use the existing icon-button base style and card padding.
