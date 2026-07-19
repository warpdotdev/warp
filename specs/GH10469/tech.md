# Tech Spec: Same-line layout for the Warp prompt

**Issue:** [warpdotdev/warp#10469](https://github.com/warpdotdev/warp/issues/10469)

**Product spec:** [`specs/GH10469/product.md`](product.md)

**Code researched at:** [`b491ddaf259257c544570322b4fb2bf4768b4676`](https://github.com/warpdotdev/warp/tree/b491ddaf259257c544570322b4fb2bf4768b4676)

## Context

The persisted model and most of the former same-line renderer still exist. A
custom `PromptConfiguration` stores `same_line_prompt_enabled` and its separator,
`CurrentPrompt` carries both values into each session, and the input editor still
supports top, left, and right decorator elements. Two newer decisions prevent
the behavior from reaching users: the prompt editor no longer mounts its
same-line row, and the rendering predicate suppresses Warp same-line for the
universal input presentation and whenever the AgentView feature is enabled,
rather than distinguishing terminal command mode from an actually active agent
input.

Relevant current code:

- [`app/src/context_chips/prompt.rs:89-148 @ b491dda`](https://github.com/warpdotdev/warp/blob/b491ddaf259257c544570322b4fb2bf4768b4676/app/src/context_chips/prompt.rs#L89-L148)
  persists the chip list, same-line boolean, and separator. The default prompt
  resolves to same-line off and no separator.
- [`app/src/context_chips/prompt.rs:161-227 @ b491dda`](https://github.com/warpdotdev/warp/blob/b491ddaf259257c544570322b4fb2bf4768b4676/app/src/context_chips/prompt.rs#L161-L227)
  updates `SessionSettings` and rebuilds the singleton prompt from saved values.
- [`app/src/terminal/session_settings.rs:299-316 @ b491dda`](https://github.com/warpdotdev/warp/blob/b491ddaf259257c544570322b4fb2bf4768b4676/app/src/terminal/session_settings.rs#L299-L316)
  cloud-syncs the Warp/PS1 selection and private saved prompt configuration.
- [`app/src/prompt/editor_modal.rs:286-368 @ b491dda`](https://github.com/warpdotdev/warp/blob/b491ddaf259257c544570322b4fb2bf4768b4676/app/src/prompt/editor_modal.rs#L286-L368)
  loads and saves the retained same-line/separator state and already emits toggle
  telemetry.
- [`app/src/prompt/editor_modal.rs:613-718 @ b491dda`](https://github.com/warpdotdev/warp/blob/b491ddaf259257c544570322b4fb2bf4768b4676/app/src/prompt/editor_modal.rs#L613-L718)
  contains the dead same-line checkbox/separator row; the active Warp section
  passes `None` for its configuration row.
- [`app/src/settings_view/appearance_page.rs:3767-3851 @ b491dda`](https://github.com/warpdotdev/warp/blob/b491ddaf259257c544570322b4fb2bf4768b4676/app/src/settings_view/appearance_page.rs#L3767-L3851)
  renders the saved prompt preview and opens the same prompt editor from
  Settings > Appearance > Input.
- [`app/src/context_chips/current_prompt.rs:1105-1151 @ b491dda`](https://github.com/warpdotdev/warp/blob/b491ddaf259257c544570322b4fb2bf4768b4676/app/src/context_chips/current_prompt.rs#L1105-L1151)
  propagates prompt and settings changes to session-owned chip state.
- [`app/src/context_chips/display.rs:384-445 @ b491dda`](https://github.com/warpdotdev/warp/blob/b491ddaf259257c544570322b4fb2bf4768b4676/app/src/context_chips/display.rs#L384-L445)
  renders interactive chips as a wrapping row for the current input stack and as
  a constrained single row for the older path.
- [`app/src/terminal/prompt_render_helper.rs:59-117 @ b491dda`](https://github.com/warpdotdev/warp/blob/b491ddaf259257c544570322b4fb2bf4768b4676/app/src/terminal/prompt_render_helper.rs#L59-L117)
  chooses placement. It returns above-input for universal input and its
  `FeatureFlag::AgentView` branch returns only the PS1 result, dropping the saved
  Warp same-line value even in ordinary terminal mode.
- [`app/src/terminal/prompt_render_helper.rs:398-581 @ b491dda`](https://github.com/warpdotdev/warp/blob/b491ddaf259257c544570322b4fb2bf4768b4676/app/src/terminal/prompt_render_helper.rs#L398-L581)
  turns the placement result into loading, PS1, or interactive context-chip
  prompt elements.
- [`app/src/terminal/input/universal.rs:25-93 @ b491dda`](https://github.com/warpdotdev/warp/blob/b491ddaf259257c544570322b4fb2bf4768b4676/app/src/terminal/input/universal.rs#L25-L93)
  renders the older universal input by adding the prompt and editor as separate
  column children.
- [`app/src/terminal/input/terminal.rs:24-69 @ b491dda`](https://github.com/warpdotdev/warp/blob/b491ddaf259257c544570322b4fb2bf4768b4676/app/src/terminal/input/terminal.rs#L24-L69)
  does the same for the current terminal-mode input when AgentView is available
  but inactive.
- [`app/src/terminal/input.rs:16280-16306 @ b491dda`](https://github.com/warpdotdev/warp/blob/b491ddaf259257c544570322b4fb2bf4768b4676/app/src/terminal/input.rs#L16280-L16306)
  selects CLI-agent, Agent View, current terminal, older universal, or classic
  rendering based on actual per-terminal state.
- [`app/src/terminal/input.rs:2854-2905 @ b491dda`](https://github.com/warpdotdev/warp/blob/b491ddaf259257c544570322b4fb2bf4768b4676/app/src/terminal/input.rs#L2854-L2905)
  installs same-line elements into the editor but already excludes an actively
  displayed Agent View.
- [`app/src/editor/view/element.rs:134-151 @ b491dda`](https://github.com/warpdotdev/warp/blob/b491ddaf259257c544570322b4fb2bf4768b4676/app/src/editor/view/element.rs#L134-L151)
  defines the decorator contract: the final prompt row is a one-line left notch
  and earlier prompt rows are a top section.
- [`app/src/editor/view/element.rs:1550-1589 @ b491dda`](https://github.com/warpdotdev/warp/blob/b491ddaf259257c544570322b4fb2bf4768b4676/app/src/editor/view/element.rs#L1550-L1589)
  lays out the notch width before laying out command text, so an unbounded chip
  row can consume the editor's usable width.
- [`app/src/ai/blocklist/agent_view/controller.rs:350-425 @ b491dda`](https://github.com/warpdotdev/warp/blob/b491ddaf259257c544570322b4fb2bf4768b4676/app/src/ai/blocklist/agent_view/controller.rs#L350-L425)
  owns the per-terminal active Agent View state that should replace the global
  feature flag as the user-visible mode input.

No data migration is needed: older custom prompt records already deserialize
into the retained fields. The change should consume those fields rather than
introducing a second setting.

## Proposed changes

### 1. Restore an accessible prompt-editor control

In `app/src/prompt/`, extract the same-line checkbox into a small focused child
view (for example `SameLinePromptControl`) and mount it as the Warp section's
configuration row in `editor_modal.rs`:

- The child owns a persistent `MouseStateHandle`, focus state, label activation,
  and typed `Toggle` action. Register `Space` for toggle and forward
  `Tab`/`Shift-Tab` navigation to the modal; keep Save and Cancel ownership in
  `EditorModal`.
- Give the view `CheckboxRole` accessibility content containing the label and
  checked state, and return the new state from `action_accessibility_contents`.
  The current raw checkbox element is mouse-driven and is not sufficient by
  itself for product invariants 16-17.
- Keep `EditorModal` as the source of truth for unsaved
  `same_line_prompt_enabled` and `warp_prompt_separator`. A child event updates
  those fields through the existing `ToggleSameLinePrompt` action, marks the
  modal dirty, converts `WarpDefault` to `Warp`, and enables/disables the
  existing separator dropdown.
- Pass the configuration row only to the Warp prompt section. PS1 selection
  must not mutate the retained Warp values. Reset continues to copy the default
  prompt's `false`/`None` values.
- Extend the Appearance prompt preview to place the separator after the chips
  when saved same-line is enabled. This keeps Settings from showing a preview
  that omits a saved visible choice.

Do not add a new public `Setting`: `PromptConfiguration` is already serialized,
synced, normalized, and observed. Reusing it is what provides invariants 1, 3,
6, 8, and 18 without a migration or a second source of truth.

### 2. Make placement depend on the active input surface

Replace the boolean-heavy `should_render_prompt_on_same_line` decision with a
small pure placement helper whose inputs are explicit: input presentation
(classic, older universal, or current terminal), terminal-versus-agent input
mode, PS1 state, and the saved Warp same-line value. It returns an enum such as
`PromptPlacement::{Above, Inline}` rather than a loosely interpreted boolean.

- Add the per-terminal `AgentViewController` handle to `PromptRenderHelper` (or
  pass a small `InputSurface` value from the renderer selected at
  `Input::render`). Do not use `FeatureFlag::AgentView` or
  `InputBoxType::Universal` alone as proxies for active agent input.
- PS1 keeps its existing classic-input same-line rule. A real AI/Agent Mode
  input and an active Agent View keep their current above/footer layouts.
- In terminal command mode, a saved Warp same-line value produces `Inline` in
  all three presentations: classic, the older universal input, and the current
  AgentView-enabled terminal input. This is the key compatibility fix for
  invariants 2-3 and 10.
- Update `render_universal_developer_input` and `render_terminal_input` to ask
  for placement. For `Inline`, omit their separate prompt column child and its
  prompt-to-editor top margin; the shared editor decorator owns the prompt. For
  `Above`, retain the current tree unchanged. The classic renderer continues
  using its existing decorator-versus-prompt-row split.
- Use the same placement result for `render_prompt`, editor decorator setup,
  prompt text/copy helpers, loading state, and notification paths. Computing it
  once per render avoids one path showing inline while another serializes or
  reserves space as above-input.
- Continue to install decorators only when the controller is inactive, as the
  existing closure in `input.rs` already does. Agent View transitions already
  notify and reset the editor height-shrink delay; preserve that path so exit
  restores the saved placement without stale height.

Keep terminal-model lock scope unchanged: the helper receives an already locked
model and reads the controller/settings outside any new nested model lock.

### 3. Bound the inline prompt and render its separator

Add an inline rendering mode to `PromptDisplay` rather than changing chip data
or creating a second set of display-chip views:

- `Above` retains the current wrapping row. `Inline` uses a one-line constrained
  row so it satisfies the editor's left-notch contract and retains the same
  `DisplayChip` handles, event subscriptions, menus, and focus restoration.
- Append a lightweight separator element sourced from
  `PromptType::separator(ctx)` only in `Inline` placement and only when it is not
  `None`. It is presentation, not a synthetic context chip, so copy menus and
  chip ordering remain unchanged.
- Derive a minimum usable command-editor width from the current editor metrics
  and `SizeInfo`, sufficient to keep the insertion cursor and adjacent command
  text visible, before constraining the inline prompt.
  Reuse each chip's existing text truncation and the prompt's horizontal clip as
  the last resort. If the pane cannot provide both the minimum editor width and
  one prompt element, return `Above` for that render without changing settings.
- Feed pane-size/font changes through the existing `InputRenderStateModel`
  notification path so placement is recomputed. Store no independent
  per-session width preference.
- Add a bounded-width field to the editor decorator layout rather than allowing
  the left notch to lay out against the entire editor constraint. The calculated
  notch width must be the same width used to offset the first text line and its
  cursor. Keep top-section behavior and PS1 multi-line splitting unchanged.

This approach deliberately avoids wrapping a multi-row `PromptDisplay` inside
`left_notch`: `EditorDecoratorElements` documents that the notch must be exactly
one line high. A bounded inline mode plus width fallback keeps that contract and
addresses invariants 11-12 without a broad editor-layout rewrite.

### 4. Preserve live context and session behavior

No changes are needed to chip generators or SSH transport. `CurrentPrompt`
already observes prompt settings and session metadata; `PromptDisplay` already
reuses child views unless chip values change. Ensure the prompt-settings handler
updates both `same_line_prompt_enabled` and `separator` before notifying so a
save repaints every open pane in one state transition.

Add a regression assertion for an existing enabled configuration created before
the feature restoration. The implementation must deserialize and render it
directly, not rewrite it on startup. Static prompt snapshots used by shared
session viewers continue carrying the same-line value, but changing shared
session product semantics is outside this issue.

## Testing and validation

### Automated tests

Add a separate test module for the pure placement helper covering the complete
truth table:

- Warp default/off versus custom/on in classic, older universal-terminal, and
  current terminal command inputs (product invariants 1-3 and 7).
- Universal terminal mode versus AI mode, and AgentView available/inactive
  versus an active Agent View (invariants 3 and 10).
- PS1 with classic-input eligibility and Warp settings present (invariant 8).
- Bootstrap/loading content uses the same result as live chips (invariant 15).

Extend `app/src/context_chips/prompt_tests.rs`:

- Deserialize a legacy custom configuration with same-line enabled and a
  separator, initialize `Prompt`, and assert both values survive normalization
  and settings propagation (invariants 3, 5-6, and 18).
- Toggle off/on and reset; assert chip order is unchanged, the separator is
  retained across the temporary disable, and reset returns `false`/`None`
  (invariants 4, 6-8).

Add `app/src/prompt/same_line_prompt_control_tests.rs` and focused editor-modal
tests:

- Exercise mouse/label and typed keyboard toggle paths, forward/backward focus
  traversal, Save, and Cancel (invariants 1, 6-8, 16, and 18).
- Assert focus accessibility content has `CheckboxRole`, label, checked state,
  and the post-toggle announcement (invariant 17).
- Open the modal from both Settings and the command palette with a pre-existing
  saved configuration and verify the displayed state is identical (invariant
  3).

Extend context-chip/editor render tests with constrained layout cases:

- Inline chips plus each separator produce one left-notch row in the configured
  order and leave the minimum editor width (invariants 2, 5, and 11).
- Long working-directory/Git values, many chips, and extremely narrow widths
  truncate or choose `Above` without overlap; widening selects `Inline` again
  without a settings write (invariants 11-12).
- Appearance, font-size, and live chip-value changes recompute layout while the
  editor buffer, selection, and focus remain unchanged (invariants 7 and 12-15).

Add one GUI integration test under `crates/integration` that configures a Warp
prompt with working-directory, Git, and SSH-context test values, enables
same-line, types but does not submit a command, resizes the pane through the
fallback boundary, enters/exits Agent View, and finally submits. Assert saved
element positions, input contents, prompt copy text, and the completed block.
This covers invariants 2, 7, and 9-15 in the real renderer.

Run at minimum:

```sh
cargo nextest run -p warp --lib context_chips::prompt::tests
cargo nextest run -p warp --lib prompt
cargo nextest run -p warp --lib terminal::prompt_render_helper
cargo nextest run -p integration --test integration same_line_prompt
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
```

### Manual validation

1. Capture before/after screenshots of Settings > Appearance > Input and the
   prompt editor with same-line off/on and each separator (invariants 1, 5-6,
   16-18).
2. Record a terminal command-input session showing chips and cursor on one row,
   a typed command surviving off/on changes, Agent Mode enter/exit, and command
   submission (invariants 2, 7, and 9-10).
3. Resize a split pane from wide to extremely narrow and back, then change font
   size and open/close a side panel. Verify no overlap, clipped cursor, stale
   row, or focus jump (invariants 11-12).
4. In a Git repository, change directory, checkout a branch, and activate an
   environment while text remains in the input. Verify live chip updates and
   menus (invariants 4 and 13).
5. Connect to an SSH host, enable the Warp prompt, and repeat the input/resize
   checks while the SSH and remote-directory chips update. Record this because
   local mocks do not prove remote context behavior (invariant 14).
6. With VoiceOver enabled, reach the checkbox using `Tab`, toggle with `Space`,
   and verify its label/state and state change are announced (invariants 16-17).
7. Seed a saved pre-restoration custom prompt with same-line enabled, launch the
   new build, and verify it renders inline before opening Settings (invariant 3).

## Risks and mitigations

- **A feature-availability flag is not active view state.** Centralize placement
  in the pure helper and test feature-available/inactive separately from active
  Agent View; do not add another call-site-specific flag check.
- **A wrapping chip row violates the one-line notch contract.** Give
  `PromptDisplay` an explicit bounded inline mode and test its height; fall back
  to the unchanged above-input renderer when the editor cannot retain minimum
  width.
- **Prompt updates can cause input-height jitter.** Reuse the existing editor
  notification and shrink-delay reset paths, and cover resize plus Agent View
  transitions in the integration test.
- **A second setting could drift from saved prompt data.** Keep the retained
  `PromptConfiguration` fields as the only persisted source and add legacy
  deserialization coverage.
- **Interactive chip menus can be clipped by the inline container.** Clip only
  the prompt row; keep menu overlays unbounded and verify branch/directory menus
  near the pane edge manually and in display-menu tests.

## Parallelization

Use `run_agents` after the placement enum and `PromptDisplay` inline contract are
agreed in a short coordinator-owned bootstrap commit. The implementation can
then split into two independent local worktrees, followed by a validation lane:

1. **Prompt settings/accessibility agent** — local, because it needs the checked
   out UI primitives and Rust tests. Work in `/tmp/warp-gh10469-settings` on
   branch `rasitakyol/gh10469-prompt-settings`; own `app/src/prompt/`, the
   Appearance prompt preview, and prompt configuration tests. Do not edit
   terminal renderer or editor layout files.
2. **Inline renderer agent** — local, because layout tests and Clippy require the
   full workspace. Work in `/tmp/warp-gh10469-render` on branch
   `rasitakyol/gh10469-inline-renderer`; own `prompt_render_helper.rs`,
   `context_chips/display.rs`, the narrow decorator-layout change, and their
   unit tests. Do not edit prompt-editor files.
3. **Integration/evidence agent** — local and starts only after the first two
   commits are integrated. Work in `/tmp/warp-gh10469-integration` on branch
   `rasitakyol/gh10469-integration`; own `crates/integration` coverage and the
   local/SSH manual evidence checklist. It reports renderer failures to the
   owning agent instead of patching their files.

All commits land in one implementation PR on
`rasitakyol/gh10469-same-line-prompt`; the coordinator cherry-picks the settings
and renderer commits, resolves only the shared module declarations, then starts
the integration lane from that combined head. Final format, workspace Clippy,
targeted tests, manual evidence, and product/tech spec updates run sequentially
on the combined branch. Separate PRs are not useful because both lanes implement
one saved-setting-to-render contract and must ship atomically.
