# TECH: Multi-Level Orchestration in the Warp TUI

Implements [PRODUCT.md](./PRODUCT.md) (approved 2026-08-10). Linear:
[CODE-1946](https://linear.app/warpdotdev/issue/CODE-1946/design-proposal-multi-level-orchestration-ui-in-the-warp-tui).
GUI reference: `app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs`,
`app/src/ai/blocklist/orchestration_topology.rs`.

Everything below ships behind `FeatureFlag::MultiLevelOrchestration` (the GUI's flag,
already in `DOGFOOD_FLAGS`). With the flag disabled, the bar keeps the historical flat
root-anchored projection, the ladder does not attach, and no new keybinding names change
behavior — `Tab`'s tree walk over a flat tree is identical to the old row walk.

## Architecture

The change keeps the existing layering: semantic topology and selection stay in
`TuiOrchestrationModel`; presentation stays in `orchestration_tab_bar.rs` + the generic
`tab_bar.rs`; the session/cloud views wire events, focus, kill, and footers.

### Semantic layer — `crates/warp_tui/src/orchestration_model.rs`

- `TuiOrchestrationSnapshot` now describes one **level**: `anchor_conversation_id`,
  `anchor_label` (`orchestrator` at the root), `anchor_status`
  (`aggregated_orchestrator_status`, `Some` only under the flag), `breadcrumbs`
  (root + optional parent, never more than two), and level-scoped `children`, each with an
  optional `subtree_rollup: LoadedSubtreeRollup` (spec rules 1-16).
- `drill_down_anchor_id` translates the GUI rule verbatim, with the TUI's navigability
  filter: a conversation with at least one child that maps to a retained session anchors
  its own level; a leaf anchors its parent's level. No descend/ascend state exists — the
  anchor is derived from the selection on every snapshot (rules 2-3, 33-36).
- Ordering reuses `child_conversations_in_pill_order` per level (flag on) or
  `descendant_conversations_in_pill_order` flat (flag off); rollups and aggregation reuse
  `loaded_subtree_rollup` / `aggregated_orchestrator_status`. All four helpers are the
  GUI's shared topology code, newly re-exported through `app/src/tui_export.rs` — the TUI
  maintains no separate ordering or aggregation policy (rules 15-16, non-goal 5).
- Explicit overflow paging is a `HashMap<anchor, TuiTabBarPagingState>` so paging one
  level never disturbs another; `focus_conversation_session` drops only the target
  level's entry to resume reveal, and dead anchors are pruned on topology changes
  (rule 37).
- `adjacent_tree_conversation` implements the tree-wide `Tab` walk: root followed by all
  session-backed descendants in pill order, wrapping (rule 18).
- `kill_child_agent_subtree` = `kill_descendant_agents` (deepest-first, existing) + the
  per-node kill, gated so flag-off keeps single-node kill (rules 23-26).

### Generic tab bar — `crates/warp_tui/src/tab_bar.rs`

- `TuiTab` gains a trailing adornment (the `▸N` badge; shares the tab's click target,
  rule 14) and a per-tab label cap overriding the config-wide maximum.
- `TuiTabBarConfig` gains `breadcrumb_tabs` (fixed, non-paginating prefix rendered with a
  one-cell gap before the main tab; accounted in `fixed_prefix_width`, rules 7-9) and
  `narrow_variants: Vec<TuiTabBarNarrowVariant>` — width-bounded alternative
  presentations of the same tab keys.
- Rendering generalizes the precomposed page switch: page transitions are computed per
  width segment (one segment per narrow variant plus the base config) and merged into a
  single `TuiSizeConstraintSwitch`, so pagination keeps working inside every ladder tier.
- Row navigation (`←`/`→`) covers breadcrumbs + anchor + level children with wrap
  (rule 17); `secondary_edge_target` stays level-scoped (rule 19); `tree_root_key`
  (first breadcrumb, else main tab) backs the Escape binding's return-to-root (rule 20).

### Presentation — `crates/warp_tui/src/orchestration_tab_bar.rs`

- `orchestration_tab_bar_config` builds the base (T0) config from the snapshot — anchor
  glyph from the aggregated status, `▸N` badges colored by the explicit design mapping
  in `rollup_badge_style` (yellow for `InProgress`/`TransientError`/`WaitingForEvents`/
  `Blocked`, red for `Error`, `neutral_7` for `Success`/`Cancelled` — rule 11 as amended
  2026-08-10), breadcrumb chips (`‹` marker, label cap 12) — and, only when the snapshot
  is multi-level, attaches the degradation ladder T1-T5 as narrow variants
  (rules 5, 10-14).
- The ladder (rules 47-50): <96 breadcrumb cap 8 / child cap 16; <84 leading collapses to
  two cells; <72 marker-only breadcrumbs, anchor cap 8; <64 glyph-only anchor, child cap
  12, badge `▸`; <56 badge dropped, child cap 8. Boundaries are constants
  (`NARROW_TIERS`) — tunable defaults; the drop order is the contract.
- Keybindings split the old alias pair (decision 5): `left`/`right` keep
  `tui:orchestration_tabs:previous`/`next` (row-scoped); `shift-tab`/`tab` move to new
  `tui:orchestration_tabs:tree_previous`/`tree_next` actions resolved through
  `adjacent_tree_conversation`. Same context predicate
  (`TuiOrchestrationTabBarFocused`), no new keymap contexts (rules 51-56).
- Footers: the child-selected variants take the selected child's loaded-descendant count
  and render `Ctrl+C to kill sub-agent +N nested` for groups (rule 27).

### Views — `terminal_session_view.rs`, `cloud_run_view.rs`

- The bar-focused single-press kill and its footer key off `bar_focused_kill_target`:
  a selected level child, or the drilled-in anchor itself when it occupies the main-tab
  slot (anchor ≠ root, per rule 24's amendment — a selected navigable group child
  re-anchors, so the anchor slot is how a group child is selected). The root tab is
  never a kill target, and a bar-focused `Ctrl+C` on a killable tab cannot fall through
  to the conversation-cancel/app-exit path. The unfocused-bar two-press armed kill
  keeps its selected ≠ root predicate; both paths share `kill_child_agent_subtree`
  (rules 23-25).
- Row navigation cannot dead-end on sessionless targets (rules 17, 21): the snapshot
  filters breadcrumb chips to session-backed conversations and marks a sessionless
  anchor non-navigable; the tab bar skips non-selectable tabs in `navigation_target`
  and ignores clicks on them.
- `PageChanged` passes the current anchor as the paging level; `Escape` targets
  `tree_root_key`; pill-bar telemetry reports the anchor as `source_conversation_id`.

### Card and transcript

- `orchestration_block/render.rs`: the acceptance card adds the muted
  `These agents may start their own child agents.` line under the identity line, same
  gate as the GUI card (rules 28-30; copy amended 2026-08-10 per design review — the
  GUI still omits the trailing period, flagged as a follow-up divergence).
- `agent_message.rs`: a sender that is neither the current conversation's direct child
  nor its parent/orchestrator gets its parent's name prefixed
  (`researcher › crawler`); an unnamed parent falls back to `orchestrator` only when it
  is the tree root (`Agent` otherwise), and unresolvable lineage falls back to the
  existing treatment (rules 31-32, 42).

## Validation

Unit tests (render-to-lines per `tui-testing`), all green via
`cargo nextest run -p warp_tui` (981) and `-p warpui_core` (313), plus `./script/format`
and `cargo clippy -- -D warnings`:

- `orchestration_model_tests.rs` — anchored root level with rollup counts; re-anchoring
  when a group child is selected (breadcrumb = root); leaf anchors its parent's level;
  breadcrumbs cap at root + parent at depth 3; per-level explicit paging isolation;
  tree-wide adjacency wrap; subtree kill removing nested sessions and conversations while
  the parent survives; flag-off flat projection unchanged.
- `orchestration_tab_bar_tests.rs` — cell-exact T0 row
  (`   Agents:    ‹ orchestrator   ● researcher  |   ● crawler ▸2`), T2 (collapsed
  leading, `‹ orche...`), T4 (`‹   ●  |`, marker badge), ladder attaching only under the
  multi-level snapshot, and the flag-off historical row.
- `tab_bar_tests.rs` — row navigation including breadcrumbs, breadcrumb prefix width
  accounting, trailing badge width + rendering, `tree_root_key`, narrow-variant
  switching below width bounds.
- `terminal_session_view_tests.rs` — split binding names stay scoped to the tab-focused
  context; group-child footer names the blast radius; existing Escape/kill flows updated
  to the new APIs.
- `orchestration_block_tests.rs` — disclosure renders only with the flag enabled.
- `agent_message_tests.rs` — grandchild header prefix; direct relations stay unprefixed.

Manual verification follows `tui-verify-change`: a real orchestration run with cloud
children deep enough for nesting, capturing the anchored root level (badge + anchor
glyph), a drilled-in level with breadcrumbs, and the narrow-width ladder.

## Known limitations

- QUALITY-1544 (remote children's own children never materialize client-side) is out of
  scope; the bar renders only what the client knows (rules 40-42). Message-header
  prefixes are the interim depth signal for those subtrees.
- Pinning, hover cards, and per-agent menus remain non-goals from the CODE-1822 baseline.
