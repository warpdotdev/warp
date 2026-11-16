# APP-4271: Tech Spec — Vertical Tabs Summary v2

## Context

This is a follow-up to APP-3875 that changes how the Tabs / Summary card lays out its content. See `specs/APP-4271/PRODUCT.md` for the user-visible behavior and `specs/APP-3875/PRODUCT.md` / `specs/APP-3875/TECH.md` for the v1 baseline.

The v1 Summary path is fully contained in `app/src/workspace/view/vertical_tabs.rs`, with pure helpers covered by `vertical_tabs_tests.rs`. Most of the work for v2 is replacing the single-line title and working-directory rendering with per-line rendering, threading per-pane status info through the aggregation layer, sorting title lines so conversations come before non-conversation lines, and adding a per-line status icon prefix on conversation lines. No new settings, no new actions, no popup changes.

Key existing code to anchor against:

- `app/src/workspace/view/vertical_tabs.rs (777-790)` — `VerticalTabsSummaryData`, `VerticalTabsSummaryBranchEntry`. The `primary_labels: Vec<String>` shape is what changes.
- `app/src/workspace/view/vertical_tabs.rs (843-925)` — pure helpers for normalization, dedupe, branch coalescing, search fragments, and primary-label formatting (`format_summary_primary_labels`). The `•`-joining lives here.
- `app/src/workspace/view/vertical_tabs.rs (2638-2754)` — `build_vertical_tabs_summary_data`, the per-pane aggregation pass that needs to start carrying conversation-source info alongside each label.
- `app/src/workspace/view/vertical_tabs.rs (3494-3590)` — `render_summary_tab_item`, where the title line currently joins labels and the working-directory line currently joins directories. Branch lines and the `+ N more` overflow already render per-line; v2 mirrors that pattern for titles and directories.
- `app/src/workspace/view/vertical_tabs.rs (2251-2356)` — `resolve_icon_with_status_variant`, the existing source of truth for "is this a conversation pane, and if so which agent / what status." V2 should reuse this.
- `app/src/ui_components/icon_with_status.rs (29-145)` — `IconWithStatusVariant` and `render_icon_with_status`. The Oz/CLI agent variants with `status` are exactly what the per-line prefix needs; the only new piece is a smaller `IconWithStatusSizing` tuned for inline use next to 12pt text.
- `app/src/workspace/view/vertical_tabs.rs (109-132, 262-274)` — existing `IconWithStatusSizing` constants (`VERTICAL_TABS_SIZING`, `VERTICAL_TABS_AGENT_SIZING`) and `render_pane_icon_with_status`, which the new inline prefix sizing will sit beside.
- `app/src/workspace/view/vertical_tabs_tests.rs` — pure helper tests; existing patterns for `format_summary_primary_labels`, `coalesce_summary_branch_entries`, and `summary_search_text_fragments` give us the template for new tests.

## Proposed changes

### 1. Upgrade `primary_labels` to carry conversation status

Replace `primary_labels: Vec<String>` on `VerticalTabsSummaryData` with a richer per-label entry:

```rust path=null start=null
#[derive(Clone, Debug, PartialEq)]
struct VerticalTabsSummaryPrimaryLabel {
    text: String,
    status: Option<ConversationStatus>,
}
```

The v2 prefix is just a status pill (icon + 10%-opacity colored background), not a full agent-icon-with-status composite, so we only need to carry an `Option<ConversationStatus>` per label — no agent (Oz / CLI) discriminator.

`working_directories: Vec<String>` and `branch_entries: Vec<VerticalTabsSummaryBranchEntry>` are unchanged.

### 2. Plumb conversation status through `build_vertical_tabs_summary_data`

In the per-pane loop, tag each candidate primary label with its `Option<ConversationStatus>`:

- For terminal panes, extract a small helper `summary_conversation_status_for_terminal(...)` that returns the same status the focused-session row would show: CLI agent session status when the agent supports rich status, otherwise the Oz / ambient agent's `selected_conversation_status_for_display`. Plain terminals and CLI agents without rich status return `None`.
- For non-terminal pane types, `status: None`.

Replace `push_normalized_unique_summary_text` for the title region with `push_normalized_unique_summary_label(...)` that preserves the first-seen status alongside the first-seen display text. Keep dedupe semantics identical: dedupe by normalized text; if a later pane contributes the same normalized label, drop the duplicate (first-seen wins, matching invariant 14).

After the per-pane loop, run a stable sort `sort_summary_primary_labels_status_first(&mut primary_labels)` (`Vec::sort_by_key` keyed on `label.status.is_none()`) so labels with a known `ConversationStatus` move ahead of labels without one while preserving the first-seen relative order within each group. This satisfies invariant 5 — the visible 3-line cap then naturally prioritizes conversation lines, and any non-conversation lines spill into the `+ N more` overflow first (invariant 7).

The working-directory and branch helpers stay as-is.

### 3. Replace `format_summary_primary_labels` with a per-line API

`format_summary_primary_labels` currently joins labels with ` • ` and appends ` + N more`. Delete it and have `render_summary_tab_item` iterate the entries directly, capping at 3 and emitting a separate `+ N more` line — exactly the pattern branch lines use today.

Working-directory rendering changes the same way: iterate up to 3 entries, then emit `+ N more` if there are extras. Reuse `summary_overflow_count`.

### 4. Render conversation status pill prefix per title line

Reuse the existing status-pill renderer (`render_status_element`) from `app/src/ai/conversation_status_ui.rs`. It produces an icon over a 10%-opacity colored background with rounded corners — exactly the styling shown in the Figma mock and used today on the pane header / detail sidecar status pill.

Define an icon-size constant beside the other vertical-tabs sizing constants:

```rust path=null start=null
const VERTICAL_TABS_SUMMARY_STATUS_ICON_SIZE: f32 = 10.;
```

This pairs with `STATUS_ELEMENT_PADDING` (2px, defined in `conversation_status_ui.rs`) for an overall ~14px element next to a 12pt title.

In the title-region rendering loop:

- For each rendered title line: when `label.status` is `Some`, build a `Flex::row` with `render_status_element(status, VERTICAL_TABS_SUMMARY_STATUS_ICON_SIZE, appearance)` followed by the `Text` element.
- When `label.status` is `None` and at least one visible title line in the card has a status, render a fixed-width spacer (`icon_size + STATUS_ELEMENT_PADDING * 2`) so the text columns align across the region (invariant 32).
- When no visible title line has a status, no slot is reserved — plain text only (invariant 30).
- The `+ N more` overflow line never gets a prefix and never reserves a slot (invariant 15).

With the status-first sort from step 2, all visible status-bearing labels are at the front of the list. The `reserve_prefix_slot = visible_labels.iter().any(|l| l.status.is_some())` check therefore only ever turns on the spacer for non-conversation lines that share the visible region with at least one conversation line.

### 5. Lock region order in `render_summary_tab_item`

Today `render_summary_tab_item` already renders title → working dir → branches in that order. Make this contract explicit: the function takes `summary: &VerticalTabsSummaryData` and emits regions in the documented order, omitting empty regions entirely (invariants 1–3). No setting affects ordering.

The existing `render_title_override` short-circuit (when the user has set a custom tab title) should keep rendering the override as a single line above any other content, with no status icon prefix and no overflow line — custom titles aren't part of the work-label set.

### 6. Update summary search fragments

`summary_search_text_fragments` (vertical_tabs.rs 905-925) currently calls `summary.primary_labels.iter().cloned()`. With the new type, change it to `summary.primary_labels.iter().map(|entry| entry.text.clone())`. Search behavior stays unchanged — the conversation source is not searchable (invariant 35 covers labels, directories, branches, PR labels, and diff text, not status icons).

### 7. Tests

Extend `vertical_tabs_tests.rs` with pure helper coverage for the new behavior. Existing tests like `coalesce_summary_branch_entries_groups_by_repo_and_branch` and `summary_search_fragments_include_hidden_overflow_values` are the right templates.

New tests:

- `primary_labels_dedupe_preserves_first_seen_status` — given two panes that contribute the same normalized label where only the second has a status, the kept entry has `status: None` (first-seen wins).
- `primary_labels_preserve_status_through_aggregation` — `ConversationStatus` values round-trip through the aggregation pass intact.
- `sort_summary_primary_labels_moves_status_first_and_preserves_order` — a mixed input list interleaving status-bearing and non-status labels sorts to all status-bearing labels first (in first-seen order) followed by all non-status labels (in first-seen order).
- `summary_search_fragments_use_label_text_only` — `summary_search_text_fragments` returns the label text and ignores the status.
- `summary_overflow_count_caps_visible_region` — `summary_overflow_count` reports the remainder past a 3-line cap.
- Update existing assertions that reference `primary_labels: vec!["..."]` to construct `VerticalTabsSummaryPrimaryLabel { text, status: None }` via a small `fn label(text)` test helper (mechanical).

The render path itself is exercised manually — there is no element-tree snapshot harness for this view today, and adding one is out of scope.

### 8. Manual / UI validation

Mapped to PRODUCT.md invariants. Each row covers one or more invariants:

- Region order (1–3): open a tab with all three region kinds; confirm titles → directories → branches and that omitting a region collapses cleanly (e.g. a notebook-only tab with no terminals shows only a title line).
- Per-line titles + dedupe (4–6, 9): create a tab with multiple distinct work labels; confirm each renders on its own line. Add a duplicate normalized label (`  cargo   test  ` and `cargo test`) and confirm only one line.
- Title overflow (7–8): create >3 unique labels; confirm exactly 3 visible lines plus `+ N more` and end-ellipsis on long single lines.
- Status icon prefix (10–13): create a tab with a CLI agent, an Oz conversation, and a plain terminal command; confirm only the conversation lines have a status icon, status reflects current state, and the icon styling matches the Figma mock.
- Status-first sort (5, 7): create a tab where the first-created pane is a plain terminal and a later pane is an Oz conversation; confirm the conversation line still renders before the terminal line in the title region. Add enough non-conversation labels that some would normally be cut off; confirm the `+ N more` overflow includes the non-conversation labels first while the conversation labels remain visible.
- Prefix dedupe (14): two panes contribute the same conversation title with different statuses; the visible line shows the first-seen status.
- Overflow has no prefix (15): >3 conversation labels; confirm the `+ N more` line has no icon.
- Per-line directories (16–22): multi-directory tab renders each directory on its own line, deduped, capped at 3, with `+ N more`. Empty directory tab omits the region.
- Branches unchanged (23–25): re-run the v1 branch validation steps from APP-3875.
- Card icon and click (26–29): card-level pane-kind icon unchanged; clicking the card focuses the active pane.
- Mixed/missing data (30–34): tab with only non-conversation labels has no prefix slot; mixed tab has aligned text columns; single-pane tab still renders three single-line regions.
- Search (35): search for a hidden-overflow title, hidden-overflow directory, and hidden-overflow branch — all match.
- Settings popup (36): `View as = Tabs` + `Tab item = Summary` continues to hide `Density`, `Pane title as`, `Additional metadata`, `Show`; switching back restores them.

Run `./script/presubmit` (cargo fmt + clippy + tests) before opening the PR.

## Risks and mitigations

### Risk: Per-region prefix slot causes inconsistent alignment

If the prefix slot is reserved on some cards but not others, the eye sees subtle misalignment when scrolling the panel.

Mitigation: reserve the slot per-region (per card), not globally per panel. Within one card, all visible title lines share the same left edge for text. Across cards, alignment may differ — that's fine and matches how branch-line right-side badges already behave.

### Risk: Sort obscures pane creation order in the title region

Users may expect title lines to appear in pane creation order (matching the v1 first-seen order). Promoting conversation lines above plain ones changes that.

Mitigation: invariant 5 documents the new ordering explicitly (status-first, then first-seen within each group). The change is intentional — status-bearing lines are the most actionable and the visible cap of 3 needs to favor them. The card-level pane-kind icon, working-directory region, and branch region all keep their existing first-seen / coalesced ordering, so pane creation order is still discoverable for non-title metadata.

### Risk: Status changes do not trigger a re-render

The summary card relies on the existing vertical-tabs render cycle. CLI agent session status and Oz conversation status changes already drive re-renders for the focused-session row; verify the summary aggregation runs on the same notify path.

Mitigation: `build_vertical_tabs_summary_data` runs inside the existing render pass over `pane_group.visible_pane_ids()`. As long as the same `app.notify()` triggers fire (CLI agent session updates, conversation status updates), Summary mode picks them up. Add this to the manual validation pass: start a CLI agent in a Summary-mode tab and confirm the prefix icon transitions through running → idle.

### Risk: Mechanical test fallout from the `primary_labels` type change

Every existing test that constructs `VerticalTabsSummaryData` literal needs updating.

Mitigation: this is intentionally mechanical. Group the updates into one commit so reviewers can verify it's a pure type lift. A small `fn label(text: &str) -> VerticalTabsSummaryPrimaryLabel` test helper keeps the existing assertions compact.

## Follow-ups

- Snapshot or harness-based tests for `render_summary_tab_item` once the surrounding code grows enough to justify the harness — currently we'd need to mock `AppContext`, `Theme`, and `Appearance`.
- If product later wants per-line click targets (e.g. clicking a conversation title to jump to that pane), the per-line `VerticalTabsSummaryPrimaryLabel` is already the right place to carry a `PaneId`; thread that through then.
- Consolidate `summary_conversation_status_for_terminal` with `resolve_icon_with_status_variant` if both keep growing — for now they share a small helper but render through different paths (status pill vs. icon-with-status composite).
