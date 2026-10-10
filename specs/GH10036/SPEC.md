# Spec: Setting to open review panel maximised by default (GH-10036)

## Problem

Clicking the code-review button on the top right opens the review
panel as a side panel. Users on small screens (laptops, vertical
monitors) immediately maximise it via a second click. The two-click
flow is friction for the most common case.

## Goal

Add a boolean setting `code.editor.review_panel.open_maximized`
(default `false`) that, when enabled, opens the review panel
pre-maximised. The non-maximised default flow is unchanged.

The TOML key uses the existing `code.editor.*` namespace already in
use by other code-editor settings in this surface, with the
`review_panel` sub-namespace grouping panel-specific preferences.

### Canonical setting key

> **The single canonical TOML key is `code.editor.review_panel.open_maximized`.**
>
> This is the only spelling that appears anywhere in the
> implementation, the user-facing settings UI label, the
> documentation, the test fixtures, the migration code, the
> telemetry event payloads, or the changelog entry. Variants such as
> `editor.review_panel_open_maximized`,
> `code.editor.reviewPanel.openMaximized`,
> `code_editor.review_panel.open_maximized`, or any other casing /
> separator combination MUST NOT appear in any artifact shipped under
> this spec. Reviewers should grep the diff for those variants and
> reject any occurrence.
>
> The key is in `snake_case` segments separated by `.`, matching the
> convention already used for `code.editor.*` sibling keys in
> `app/src/terminal/general_settings.rs` (e.g., line 165's
> `code.editor.auto_open_code_review_pane_on_first_agent_change`)
> and `app/src/workspace/tab_settings.rs` (e.g., line 461's
> `code.editor.show_code_review_button`).
>
> **PR description / commit messages also follow the canonical key.**
> The PR opening this spec previously referenced
> `editor.review_panel_open_maximized` in its description; the
> author MUST edit the PR description and any associated commit
> body to use the canonical
> `code.editor.review_panel.open_maximized` before merge. Reviewers
> grep the PR description and commit messages for the rejected
> variants listed above and request changes if any survive.

## Behavior contract

- B1. New setting `code.editor.review_panel.open_maximized: bool`,
  default `false`, `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`,
  appended to the `GeneralSettings`
  `define_settings_group!` block in
  `app/src/terminal/general_settings.rs` immediately after
  `auto_open_code_review_pane_on_first_agent_change` (line 159).
  Code-review siblings already live in this group (the related
  tab-scoped flags `show_code_review_button` and
  `show_code_review_diff_stats` live in `TabSettings` at
  `app/src/workspace/tab_settings.rs:455/464`); the new field is
  global, not per-tab, so it goes with
  `auto_open_code_review_pane_on_first_agent_change`. There is
  intentionally no `review_panel` anchor in
  `app/src/settings/code.rs` and the V1 PR does not add one.
- B2. When `true`, clicking the code-review button opens the
  panel in maximised state directly. The Esc / un-maximise
  affordance still works to shrink it.
- B3. When `false`, no behavior change — the existing two-click
  flow is preserved exactly.
- B4. Setting is exposed in the existing **"Code Editor and Review"**
  category in the Code settings page (verified in
  `app/src/settings_view/code_page.rs` lines 314 and 438, where the
  category is built via
  `Category::new("Code Editor and Review", code_editor_review_widgets)`).
  The setting is appended to the `code_editor_review_widgets`
  `Vec` adjacent to the existing `show_code_review_button`,
  `show_code_review_diff_stats`, and
  `auto_open_code_review_pane_on_first_agent_change` widgets — NOT
  in a separate "Editor → Code review" subpage (that path does not
  exist in the current settings tree).

  The user-facing label is "Open review panel maximised by default."
  The settings widget is built and appended in BOTH branches of the
  `cfg(...)` split at lines 297/301 and 421/425 (so both feature
  configurations get the toggle), matching the surrounding pattern
  for sibling code-review widgets.

- B5. The setting respects the existing maximise/restore state
  machine implemented in `app/src/workspace/view.rs`. The relevant
  pieces of the existing state machine are:
  - `PaneGroup::is_right_panel_maximized` (line 910) — the in-memory
    flag for the active pane group.
  - `RightPanelSnapshot::is_maximized` (lines 3852, 10030–10035) —
    the persisted prior state, restored on tab restore via
    `apply_right_panel_snapshot` (line 3847).
  - `toggle_right_panel_maximized` (line 8280) — toggles the flag
    and calls `view.set_maximized(...)` (lines 8284–8288).

  The new setting interacts with this state machine by these rules:

  1. Switching the setting at runtime does NOT retroactively
     maximise an already-open panel — only the next open uses the
     new default. (Same as before.)
  2. **Tab/session restore takes precedence over the setting.** When
     a tab is restored with a `RightPanelSnapshot` (i.e., the panel
     was open at suspend time), the panel reopens to
     `right_panel_snapshot.is_maximized` regardless of the setting
     value. Reasoning: the snapshot encodes the user's last explicit
     choice for that specific tab; the setting only affects the
     "fresh open" path where there is no prior state to honor.
  3. **Closing then reopening within a session uses the setting.**
     Once the panel is closed (no `RightPanelSnapshot` is in effect
     for the next open action), the next open through
     `setup_code_review_panel` (line 8001) or
     `open_code_review_panel_from_arg` (line 8055) consults the
     setting and seeds `is_right_panel_maximized` from it.
  4. **"Previously maximised before being closed" specifically.** If
     the user maximises the panel, then closes it, and then reopens
     it later (without a tab restore in between), the reopen behavior
     is governed by the setting, not by the prior in-session
     maximise state. Rationale: the in-memory `is_right_panel_maximized`
     flag is per-pane-group and is not retained across an explicit
     close; the only cross-close persistence is `RightPanelSnapshot`,
     which only applies on tab/session restore. This matches the
     spirit of the setting: it is a default for fresh opens, and the
     close-then-reopen sequence is a "fresh open" for our purposes.

  Acceptance B5 is the conjunction of B5.1–B5.4. Each sub-rule has
  a corresponding test in the Test plan.

#### Reconciliation with the existing state machine (B5 pre-flight)

The reviewer flagged that the default-off contract may conflict with
what the current right-panel state machine does today for the
close-then-reopen-after-maximise sequence. This subsection makes the
conflict (or lack thereof) explicit and pins the V1 outcome:

1. **Current behavior (master, pre-V1):** The in-memory flag
   `PaneGroup::is_right_panel_maximized` (view.rs:910) is owned by
   the active pane group. When the panel is closed (the right-panel
   slot becomes `None`), the pane group does NOT retain the flag for
   a subsequent reopen — the next open call into
   `setup_code_review_panel` (view.rs:8001) constructs a fresh right
   panel and `set_maximized` is invoked only via the snapshot path or
   an explicit user toggle. There is no "remember in-session
   maximise across an explicit close" code path today.
2. **Implication for the default-off contract:** Because today's
   close-then-reopen already drops to side-panel (no in-memory
   carry-over and no setting yet), the V1 default-off behavior is
   pixel-identical to the current master behavior on this sequence.
   B3 ("no behavior change when setting is `false`") therefore holds
   without a regression.
3. **Pre-flight verification step in the V1 PR.** Before adding the
   setting, the V1 PR adds a regression test
   `T_close_reopen_baseline` against current master behavior that
   asserts close-then-reopen lands in side-panel state with no
   snapshot and no setting. The test stays in the tree after the
   setting lands and continues to pass with setting OFF, codifying
   that V1 preserves the pre-V1 contract.
4. **What does change.** The only new behavior with setting ON is
   that the fresh-open seed for `is_right_panel_maximized` reads
   `true` from the setting before `setup_code_review_panel` returns.
   The state machine itself is not modified; the new code only
   writes to `is_right_panel_maximized` on the same fresh-open path
   the snapshot writer already uses (apply_right_panel_snapshot at
   view.rs:3847).
5. **If the pre-flight test reveals the current code DOES carry the
   flag across close (i.e., the description in (1) is wrong),** the
   V1 PR is blocked. The spec author must update B5.4 to match the
   real state machine — either by preserving the pre-V1 close-then-
   reopen-maximise behavior under setting OFF (treating the
   in-memory flag as authoritative) or by explicitly calling out
   that V1 introduces a deliberate behavior change. This decision is
   logged in the PR description and approved by a code-review owner
   before the setting code lands.

## Acceptance criteria

- A1. With setting OFF (default), starting from no prior state
  (no `RightPanelSnapshot`, no in-memory `is_right_panel_maximized`):
  clicking code-review opens the side-panel; clicking the maximise
  control (or invoking the existing
  `workspace:toggle_maximize_code_review_panel` action) maximises
  it. Pixel-equivalent to today.

  **OFF + previously maximised then closed (no tab restore in
  between)**: with the setting OFF, the close-then-reopen sequence
  reopens as a side-panel (per B5.4). The in-memory
  `is_right_panel_maximized` flag does not survive an explicit
  close because no `RightPanelSnapshot` was created — closing the
  panel is not a tab/session boundary. This is by design: the
  setting is the authoritative default for fresh opens, and a
  fresh open after an explicit close is a fresh open. There is no
  "remember last in-session maximise state across an explicit
  close" behavior in V1; the only persistence channel is
  `RightPanelSnapshot`, which only fires on tab/session restore.

  Codifies the OFF interaction with the existing maximise/restore
  state machine: setting + snapshot are orthogonal, snapshot
  always wins on restore, setting always wins on fresh open
  (including close-then-reopen).
- A2. With setting ON, starting from no prior state: clicking
  code-review opens the panel already maximised in a single click.
  Invoking `workspace:toggle_maximize_code_review_panel` (Esc / the
  maximise control) restores the side-panel state.
- A3. Toggling the setting at runtime while a non-maximised panel
  is open does NOT auto-maximise it.
- A4. Tab/session restore: a tab whose `RightPanelSnapshot` has
  `is_maximized = true` reopens maximised on app restart even when
  the setting is OFF. A tab whose snapshot has
  `is_maximized = false` reopens as a side-panel on restart even
  when the setting is ON. The snapshot is authoritative for
  restored tabs; the setting only governs fresh opens. (Codifies
  B5.2 and prevents a regression where the new setting would
  override a saved per-tab state.)
- A5. Close-then-reopen within a session: maximise the panel, close
  it, then reopen via the code-review button (or
  `OpenCodeReviewPanel`). With the setting OFF, the reopen is a
  side-panel. With the setting ON, the reopen is maximised.
  Codifies B5.4.

## Implementation sketch

### Where the setting lives in code

Code-review settings in this repo are NOT consolidated into a
single `define_settings_group!(CodeSettings, ...)` block in
`app/src/settings/code.rs`. They are split across two existing
groups, which the V1 PR adopts unchanged:

- `TabSettings` in `app/src/workspace/tab_settings.rs` (line 445
  onward) defines `show_code_review_button` (line 455) and
  `show_code_review_diff_stats` (line 464), both keyed under
  `code.editor.*`.
- `GeneralSettings` in `app/src/terminal/general_settings.rs`
  (line 159) defines
  `auto_open_code_review_pane_on_first_agent_change`, also keyed
  under `code.editor.*`.

The new `review_panel_open_maximized` field is appended to the
`GeneralSettings` group in `app/src/terminal/general_settings.rs`
immediately after
`auto_open_code_review_pane_on_first_agent_change`. That is the
existing home for a non-tab-scoped code-review preference and
matches the sync semantics required by B1
(`SyncToCloud::Globally(RespectUserSyncSetting::Yes)`). It is the
only group whose toml prefix is `code.editor.*` AND whose values
are not per-tab. Tab-scoped placement (`TabSettings`) is rejected
because the maximize default is global UX, not a per-tab
preference.

```rust
// app/src/terminal/general_settings.rs — appended to the
// GeneralSettings group, after
// `auto_open_code_review_pane_on_first_agent_change` at line 159.
review_panel_open_maximized: ReviewPanelOpenMaximized {
    type: bool,
    default: false,
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "code.editor.review_panel.open_maximized",
    description: "Whether the code review panel opens already maximised.",
},
```

The earlier draft of this spec referenced an `app/src/settings/code.rs`
location that does not host the code-review siblings. That path
remains a no-op for this change; do not edit it. The grep test in
T8 enforces this by failing if `review_panel_open_maximized`
appears in any file other than
`app/src/terminal/general_settings.rs` (definition) and
`app/src/settings_view/code_page.rs` (widget).

- The settings UI widget is added in
  `app/src/settings_view/code_page.rs`. Both `code_editor_review_widgets`
  branches (around lines 297/301 and 421/425) extend their `Vec`
  with the new toggle. Place the new widget adjacent to the
  existing `show_code_review_button` /
  `auto_open_code_review_pane_on_first_agent_change` widgets within
  the `Category::new("Code Editor and Review", ...)` grouping (lines
  314, 438). Do NOT introduce a new category or settings page.

- The runtime entry points are `setup_code_review_panel` (line
  8001 of `app/src/workspace/view.rs`) and
  `open_code_review_panel_from_arg` (line 8055). At each entry
  point, before the `pane_group.update(...)` block that reads
  `is_right_panel_maximized`, branch on the source of the open:

  - If the open is part of a tab restore that supplied a
    `RightPanelSnapshot` (i.e., `apply_right_panel_snapshot` at line
    3847 has already seeded `pane_group.is_right_panel_maximized`),
    do nothing extra — the snapshot wins (B5.2 / A4).
  - Otherwise (fresh open, including close-then-reopen — see
    B5.4 / A5), read `code_settings.review_panel_open_maximized`
    and write it into `pane_group.is_right_panel_maximized`
    before calling `view.set_maximized(...)`. The existing
    `set_maximized` call at line 8123 / 8288 then performs the
    visible toggle. We do NOT introduce a new
    `OpenCodeReviewPanelMaximized` action variant; we seed the
    state on the existing path.

#### Full button-click → open-and-maximise call chain

The reviewer flagged that `setup_code_review_panel` and
`open_code_review_panel_from_arg` are too far downstream to be the
sole "where the seed lands" answer. The button is upstream; the
state transition that opens-and-maximises is downstream. This
section enumerates the full chain end-to-end so the implementer
modifies the one node that owns the shared transition:

1. **Top-bar button — visual entry point.** The code-review button
   on the top right is rendered by the top-bar widget in
   `app/src/terminal/title_bar/` (the renderer for the button
   already exists; the implementer locates it via `git grep -n
   show_code_review_button app/src/terminal/title_bar/`). On click,
   it dispatches `workspace::OpenCodeReviewPanel` (the existing
   action used by the keyboard shortcut and command palette).
2. **Action handler — `OpenCodeReviewPanel`.** The handler lives in
   `app/src/workspace/view.rs` and is registered alongside the
   other workspace actions (locate via `git grep -n
   "OpenCodeReviewPanel" app/src/workspace/view.rs`). The handler
   calls `setup_code_review_panel(...)` after resolving the active
   pane group.
3. **`setup_code_review_panel` (view.rs:8001) — the shared
   transition owner.** This function is the single node that every
   open path (button click, keyboard action, command-palette entry,
   `open_code_review_panel_from_arg` for URL/IPC entries, and the
   review-skill auto-open in
   `auto_open_code_review_pane_on_first_agent_change`) funnels
   through. It owns:
   - constructing the `RightPanel` instance,
   - writing `pane_group.is_right_panel_maximized`,
   - calling `view.set_maximized(...)` (lines 8123 / 8288).

   Because every open path funnels here, this is the correct and
   only seed point for the setting. The implementer reads
   `code_settings.review_panel_open_maximized` here, gated by the
   "no snapshot already applied" branch from the bullet above.
4. **Toggle action — `workspace:toggle_maximize_code_review_panel`
   (right_panel.rs lines 351, 407, 1042).** Unchanged by V1. This
   is the in-session maximise/restore affordance referenced in A2's
   "Esc / un-maximise" path. It does not read the setting.
5. **What the implementer must NOT do.** Do not seed the setting in
   the button's click handler, in the keyboard-shortcut dispatcher,
   in `OpenCodeReviewPanel`'s action handler, or in
   `open_code_review_panel_from_arg` directly. Seeding upstream of
   `setup_code_review_panel` would (a) duplicate the seed logic
   across every open path, and (b) bypass the snapshot precedence
   in B5.2 / A4. The shared transition node is the only correct
   site.

The V1 PR's diff therefore touches exactly: the settings group in
`general_settings.rs`, the widget vector in `code_page.rs`, and a
single read-then-write block inside `setup_code_review_panel` in
`view.rs`. The top-bar button renderer is not modified; the
existing dispatch path is reused.

- The action `workspace:toggle_maximize_code_review_panel` (right_panel.rs
  lines 351, 407, 1042) is unmodified — it continues to flip
  `is_right_panel_maximized` for the current panel and is the
  affordance referenced in A2's "Esc / un-maximise" path.

## Test plan

- T1. Setting round-trips through TOML under the canonical
  `code.editor.review_panel.open_maximized` key (no other
  spelling).
- T2. Fresh open with setting ON seeds
  `is_right_panel_maximized = true`; with setting OFF it stays
  `false`. Asserted by driving `setup_code_review_panel` directly
  with no prior `RightPanelSnapshot` and inspecting the
  `PaneGroup` state.
- T3. Snapshot test: with setting OFF, the rendered tree on the
  fresh-open path is identical to the current build (regression
  guard for the default-off requirement).
- T4. Runtime toggle while a non-maximised panel is open does NOT
  retroactively maximise it (A3). Asserts the in-memory
  `is_right_panel_maximized` flag is unchanged after a settings
  write.
- T5. Tab restore overrides the setting (A4 / B5.2). With setting
  OFF, restore a tab whose `RightPanelSnapshot` has
  `is_maximized = true` and assert the restored panel is maximised.
  With setting ON, restore a tab whose snapshot has
  `is_maximized = false` and assert the restored panel is a
  side-panel.
- T6. Close-then-reopen within a session uses the setting (A5 /
  B5.4). Maximise the panel, close it, then reopen via
  `OpenCodeReviewPanel`. With setting OFF, the reopen is a
  side-panel; with ON, the reopen is maximised.
- T7. The settings widget is appended to the `code_editor_review_widgets`
  `Vec` in BOTH `cfg(...)` branches of
  `app/src/settings_view/code_page.rs` (lines 297/301 and 421/425),
  inside the `Category::new("Code Editor and Review", ...)` group
  — not in any new category.
- T8. Grep test — scoped to implementation artifacts only.

  **Scope:** The grep runs against the following paths:
  - `app/**/*.rs`
  - `crates/**/*.rs`
  - `app/**/*.toml`, `crates/**/*.toml` (settings TOML fixtures only)
  - `app/**/snapshots/**`, `crates/**/snapshots/**`
  - The PR description and commit messages (via the
    `T_pr_description_audit` script invoked as a CI step, not a
    Cargo test).

  **Explicitly excluded from the grep:** `specs/**` (this spec file
  and every other spec prose document), `docs/**` markdown, and any
  `CHANGELOG.md` prose that discusses the rejected variants for
  historical context. Spec prose is permitted to enumerate the
  rejected key spellings because that is how the rejection list is
  communicated to reviewers and future implementers; an in-spec
  occurrence is documentation, not a leak into shipped artifacts.

  **Assertion:** Within the scoped paths above, no occurrence of
  `editor.review_panel_open_maximized`,
  `code.editor.reviewPanel.openMaximized`,
  `code_editor.review_panel.open_maximized`, or any other
  case/separator variant appears. Only the canonical
  `code.editor.review_panel.open_maximized` is present in scoped
  artifacts.

  **Why the scoping matters:** Without the spec/docs exclusion, the
  grep would self-fail because this spec lists the rejected
  spellings in the "Canonical setting key" section as part of the
  rejection contract. The scope above narrows the contract to "no
  rejected variant ships in code, config, fixtures, snapshots, PR
  metadata, or changelog entries" while permitting spec prose to
  document what is rejected. The implementation is a simple
  `ripgrep` invocation with `--glob '!specs/**' --glob '!docs/**'
  --glob '!CHANGELOG.md'` against the variant list; the script
  source lives at `scripts/audit_review_panel_key.sh` and is
  invoked from both a Cargo test (for the Rust scope) and a CI
  step (for the PR-description scope).

## Out of scope

- Per-window or per-project override of the maximise default.
- Remembering the user's last maximise state across sessions
  (separate UX decision; this setting is just a default, not a
  state restore).
