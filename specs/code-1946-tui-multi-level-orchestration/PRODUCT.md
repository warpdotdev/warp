# PRODUCT: Multi-Level Orchestration in the Warp TUI

Linear: [CODE-1946 — Design proposal: multi-level orchestration UI in the Warp TUI](https://linear.app/warpdotdev/issue/CODE-1946/design-proposal-multi-level-orchestration-ui-in-the-warp-tui)
Baseline this amends: [specs/code-1822-tui-orchestration-tab-bar/PRODUCT.md](../code-1822-tui-orchestration-tab-bar/PRODUCT.md)
GUI reference: `app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs`, `app/src/ai/blocklist/orchestration_topology.rs`
Figma: none provided (the mockups below are exact cell-grid frames; the TUI renders to a
character grid, so text frames are the design medium).

Status: **approved.** The requester (Daniel Peng) approved the design, including both
formerly open questions (keyboard scope and group-kill semantics), in the Slack thread
on 2026-08-10. A `TECH.md` (with validation and test planning) follows in the
implementation PR, and the superseded clauses listed at the end are folded back into
the CODE-1822 baseline spec when implementation lands.

## Summary

The TUI already ships a working **single-level** orchestration UI: the `run_agents`
permission card, retained background sessions per child, and the `Agents:` tab bar. The
behavior underneath is already multi-level — grandchildren materialize, restore across
restarts, and are killed transitively — but the presentation is flat:
`TuiOrchestrationModel::snapshot()` lists every descendant of the tree root as a sibling
tab (`crates/warp_tui/src/orchestration_model.rs:194`), so depth is invisible. No
breadcrumbs, no drill-down, no subtree rollups, and an orchestrating child looks identical
to a leaf.

This spec makes the tab bar a **drill-down bar that mirrors the GUI's multi-level pill
bar**: the bar shows one level of the tree at a time (an anchor plus its direct children),
breadcrumb chips lead back up when drilled below the root, and a child that is itself an
orchestrator carries a subtree rollup badge. The anchoring rule is the GUI's rule
translated verbatim, so no new "descend" keystroke is needed — selection alone drives the
level shown.

## Current state (evidence)

Captured from a live orchestration run on HEAD `7d93fa46886`: screenshots, an MP4, and raw
`tmux` text frames are attached to the draft GitHub release
[`tui-orch-triage-evidence-2026-08-09`](https://github.com/warpdotdev/warp/releases/tag/tui-orch-triage-evidence-2026-08-09)
and linked from CODE-1946. The decisive frame: cloud child `gamma` spawned its own nested
child `delta`, which completed its task — but the bar still reads only:

```
   Agents:    orchestrator  |   ⊹ alpha     ⊛ beta     * gamma
  ✓ * gamma  ▸
  Nested orchestration test completed successfully: gamma started its own nested cloud child, delta, which answered
  5+5 = 10, and gamma reported that back.
```

`delta` never appears anywhere. (For a **remote** parent like gamma this is compounded by
QUALITY-1544 — the client never materializes a remote child's own children — but the flat
rendering applies equally to locally materialized grandchildren, proven by
`orchestration_model_tests.rs:863-922` where a child and grandchild surface as the flat
list `[child, grandchild]`.)

## Goals

- Make parentage legible in a single-row cell grid without a second row, a tree view, or a
  modal.
- Mirror the GUI's multi-level information architecture (anchored level, breadcrumbs,
  group rollups, canonical ordering) so the two front-ends give the same mental model.
- Keep every agent in the tree reachable in a bounded number of keystrokes.
- Change nothing visible at orchestration depth 1 beyond two small additive cues (an
  aggregated status glyph on the orchestrator tab, a rollup badge on orchestrating
  children).
- Stay legible and operable down to ~56 columns.

## Non-goals

1. **Hover detail cards.** The GUI's hover card has no terminal analogue and nothing
   replaces it: switching to an agent is one keystroke and shows strictly more.
2. **Per-agent 3-dot menus.** Their entries are pane/tab concepts the TUI does not have.
   "View in Oz" already exists: the cloud-run view binds `Enter` to open the run URL.
3. **Pinning.** Unchanged from the CODE-1822 baseline: no TUI pin affordance; existing
   GUI pin state continues to affect ordering for parity.
4. A second row, an expandable tree view, or an overlay. The bar stays exactly one row.
5. Any change to orchestration behavior, depth policy, or the server-side depth budget.
   This is a presentation and navigation change only.
6. Surfacing a numeric remaining-depth budget. The client cannot know it (the GUI
   documents the same constraint at `run_agents_card_view.rs:1541-1544`).
7. Fixing QUALITY-1544 (remote children's own children never materialize client-side).
   Behavior 40-42 defines how the bar behaves while that gap exists.

## Behavior

### Level scoping and the anchor

1. The bar renders exactly one level of the orchestration tree: breadcrumb chips (when
   applicable), the **anchor** conversation as the main tab, a divider, then the anchor's
   **direct** children in canonical pill order.
2. The anchor is derived from the selected conversation with the GUI's rule
   (`drill_down_anchor_id`, `orchestration_pill_bar.rs:737-748`), translated verbatim:
   - If the selected conversation has at least one navigable child, it is the anchor.
   - Otherwise its parent is the anchor, so a leaf always shows itself among its siblings.
   - At the tree root this reproduces today's behavior exactly.
3. There is no separate descend/ascend state or keystroke. Selecting a child that is
   itself an orchestrator re-anchors the bar to that child's level ("drilling in");
   selecting a breadcrumb chip re-anchors one or more levels up ("drilling out"). The bar
   always follows the selection and can never show a level the selection is not part of.
4. The anchor occupies the existing main-tab slot. Its label is the anchor's agent name,
   or `orchestrator` when the anchor is the tree root — at depth 1 the label is unchanged.
5. The anchor tab gains a leading status glyph showing the **aggregated status of its
   subtree** (`aggregated_orchestrator_status` semantics: an orchestrator whose own turn
   finished while a descendant still runs shows `●`, not a terminal glyph). This is the
   GUI's anchor-pill treatment and closes the "orchestrator tab shows nothing" gap.
6. A conversation is included only while it maps to a retained, focusable TUI session
   (CODE-1822 clause 4 carries over, applied per level).

### Breadcrumbs

7. When the anchor is not the tree root, breadcrumb chips render between the `Agents:`
   label and the anchor tab, mirroring the GUI's `breadcrumb_ids` rule exactly: one chip
   for the tree **root**, plus one chip for the anchor's direct **parent** when the anchor
   sits 2+ levels below the root. Never more than two chips, at any depth.
8. A breadcrumb chip is a real, selectable, clickable tab: a `‹` marker plus the
   ancestor's name, label capped at 12 display cells (children keep 20). Selecting it
   switches to that conversation, which re-anchors the bar per rule 2.
9. Breadcrumb chips are part of the fixed (non-paginating) prefix. They never paginate
   and are never hidden by child-region overflow; at narrow widths they degrade by
   shrinking (rules 47-50), never by disappearing, so ascent is always reachable.

### Group children (subtree rollups)

10. A child with at least one loaded descendant is a **group** child and renders a
    trailing badge `▸N`, where N is the loaded-descendant count of its subtree
    (`LoadedSubtreeRollup::descendant_count`). The badge never advertises nodes whose
    conversations are not loaded.
11. The badge's color follows the subtree's **aggregated status** with this explicit,
    total mapping **[amended 2026-08-10 per design review]**:
    - **yellow** (`attention_glyph_style`, terminal yellow) while any descendant is
      working or stuck: `InProgress`, `TransientError` (recovering counts as running),
      `WaitingForEvents` (alive and resumable per QUALITY-780), or `Blocked`;
    - **red** (`error_text_style`, terminal red) when the settled subtree contains a
      failure: aggregated `Error`;
    - **neutral_7** (`neutral_7_text_style`) when everything settled without one:
      aggregated `Success` or `Cancelled`.
    The child's leading glyph keeps today's semantics — own status while live,
    identity glyph once terminal — matching the GUI, where the avatar shows the child's
    own status and the trailing group badge shows the rollup.
12. A non-group child renders exactly as today: one glyph, one label, no badge.
13. The anchor never carries a `▸N` badge; its children are the rest of the row.
14. Reading order within a tab is `[glyph] [label] [▸N]`. The badge is its own click
    target: clicking it selects that child (same as clicking the tab body), which by rule
    3 drills into its level.

### Ordering

15. Each level orders its children with the same canonical pill order as the GUI
    (`child_conversations_in_pill_order`): pinned, then blocked, errored, active,
    done-by-recency, with spawn order breaking ties. The TUI maintains no separate policy
    (CODE-1822 clauses 9-12 carry over, scoped to the level).
16. A group child sorts by its **own** status, not its rollup. A grandchild's lifecycle
    change must never reorder the level the user is looking at; it may only restyle the
    parent's badge.

### Navigation and keybindings

17. `←` / `→` navigate **within the rendered row**: breadcrumb chips, anchor, then the
    visible level's children, wrapping across the row's ends. At depth 1 (no breadcrumbs)
    this is byte-for-byte today's behavior. **[Approved by the requester, 2026-08-10.]**
18. `Tab` / `Shift+Tab` keep today's **tree-wide** walk: the root followed by all
    descendants in pill order (the GUI's keyboard-cycling order,
    `adjacent_orchestration_child_conversation_id`). Landing on a conversation at another
    depth re-anchors the bar per rule 2, so `Tab` alone still reaches every agent.
    Implementation note — rules 17-18 **deliberately split an existing alias pair**. On
    master, `←`/`→` and `Tab`/`Shift+Tab` are registered as interchangeable triggers of
    the same two actions, `tui:orchestration_tabs:previous`/`:next`
    (`orchestration_tab_bar.rs:60-109`). Under this spec they become two distinct action
    pairs: a row-scoped previous/next (arrows) and a tree-scoped previous/next
    (Tab/Shift+Tab). Do not re-unify them; the divergence is the decision, not an
    oversight (decision 5).
19. `Shift+←` / `Shift+→` select the first / last **child of the current level**.
    Semantics unchanged, scope narrowed from "all descendants" to the visible level.
20. `↓` leaves the bar to send a message, `Shift+↓` leaves the bar, `Esc` returns to the
    tree root's session, and a click selects a tab — all unchanged from the baseline.
    Selecting a tab switches sessions immediately; there is no two-step selection.
21. Every agent in the tree remains reachable by keyboard alone: `Tab` walks the whole
    tree, and `←`/`→` plus breadcrumb selection covers level-local movement. No state of
    the bar can strand the selection.
22. The focused footer stays the existing two variants, plus one addition for group
    children (rule 27). Breadcrumbs need no footer copy — the chips themselves are the
    affordance.

### Kill semantics

23. `Ctrl+C` with the bar focused and a **leaf** child selected: unchanged — a single
    press kills that child and returns focus to the root session.
24. `Ctrl+C` with the bar focused and a **group** child selected kills the child **and
    its entire subtree**, deepest-first, so no descendant session is orphaned.
    **[Approved by the requester, 2026-08-10.]** This deliberately diverges from the GUI, whose
    per-pill Kill removes only the target node and leaves its descendants' hidden panes
    orphaned (deleting a parent drops its `children_by_parent` entry, making the subtree
    unreachable from the bar). In a TUI, unreachable retained sessions are pure leakage;
    the divergence is recorded in "Deliberate divergences from the GUI".
    **[Amended 2026-08-10, implementation review.]** Because selecting a navigable group
    child re-anchors the bar (rule 3), the selected group child occupies the anchor's
    main-tab slot; the same single-press subtree kill therefore also applies when the
    bar is focused and the selection **is** the drilled-in anchor (anchor ≠ root), and
    the anchor-selected footer names the blast radius (rule 27). The root tab is never a
    kill target, and a bar-focused `Ctrl+C` on a killable tab never falls through to the
    conversation-cancel/app-exit path.
25. The two-press armed-kill flow while *viewing* a child conversation without bar focus
    (first `Ctrl+C` arms, second within the window kills) carries over unchanged, with
    the same subtree semantics when the viewed conversation is a group child.
26. Killing a subtree tombstones every killed conversation so late events cannot
    resurrect any of them, and cancels in-flight execution per node (cloud task
    cancellation for remote children, controller cancellation for local ones) — the
    existing per-node kill path applied deepest-first.
27. The focused footer names the blast radius when a group child is selected:
    `Ctrl+C to kill sub-agent +N nested` (N = loaded descendant count). For a leaf the
    existing `Ctrl+C to kill sub-agent` copy is unchanged.

### The permission card

28. When `FeatureFlag::MultiLevelOrchestration` is enabled, the acceptance card adds one
    muted line directly under the agent identity line:
    `These agents may start their own child agents.` — the same gate as the GUI card
    (`run_agents_card_view.rs:1545-1559`), so both front-ends make the approver the same
    promise. **[Amended 2026-08-10 per design review: the `↳` glyph is dropped and a
    trailing period added. The GUI still renders the copy without the period; that
    divergence is flagged for a follow-up on the GUI side, not changed here.]**
29. The card does not surface a remaining-depth number (non-goal 6).
30. There is no nested-approval card treatment because there is no nested approval: a
    child conversation always auto-executes `run_agents`
    (`app/src/ai/blocklist/action_model/execute/run_agents.rs:428-451`); with the flag
    disabled the call fails closed with a Denied result. The disclosure only ever matters
    on the root card.

### Inter-agent messages

31. When a received message's sender is neither the current conversation's direct child
    nor its parent/orchestrator, the collapsed message header prefixes the sender with its
    parent's name: `● ⟡ researcher › crawler`. When the sender is a direct relation the
    header is unchanged. This is the only transcript surface where depth is worth cells.
32. Message headers keep their existing status glyph, identity glyph, and preview
    behavior in every other respect.

### Tree changes while drilled in

33. If the selected conversation (and therefore the anchor's level) is removed — killed
    from elsewhere, subtree torn down, session dropped — focus falls back to the tree
    root's session and the bar re-anchors to the root level. This is the existing
    kill-fallback behavior generalized to any depth.
34. If a level's membership changes while the user is drilled in (a sibling finishes, a
    new child materializes), the level re-orders per rule 15 and pagination follows the
    existing reveal/explicit-page rules (CODE-1822 clauses 39-42). Reordering never
    changes the selection by itself.
35. When the last child of an anchor disappears, the anchor becomes a leaf and the bar
    re-anchors to the anchor's parent per rule 2 (or disappears entirely when the root has
    no navigable children left — today's behavior).
36. A newly materialized grandchild appears in its parent's level and flips that parent
    to a group child (badge appears) without stealing focus or changing the selection.
37. Explicit overflow paging state is tracked per level (keyed by the anchor), so paging
    within a drilled-in level does not disturb the root level's page and vice versa.

### Restore

38. Restoring a multi-level tree restores the same presentation: the bar anchors on the
    restored selection per rule 2, group children show badges from their restored
    subtrees, and every restored descendant is reachable via rule 18's tree-wide walk.
39. Restored remote children whose status arrives asynchronously update their glyph and
    any ancestor badges as statuses land, with no user action.

### Remote children and QUALITY-1544

40. A child running remotely whose own children exist only server-side shows **no badge
    and no level**: the client has no conversations for them (QUALITY-1544). The bar must
    not invent placeholders for nodes it cannot navigate to.
41. The spec treats this as a data gap, not a UI bug: when QUALITY-1544 lands and remote
    grandchildren materialize client-side, rules 10-14 apply to them with no further UI
    work.
42. Rule 31's message-header prefix is the interim depth signal for such subtrees: a
    message from an invisible remote grandchild still names its parent when the client
    can resolve it, and falls back to the existing unknown-sender treatment when it
    cannot.

### Feature gating

43. The multi-level presentation (rules 1-22, 24-27, 31-42) ships behind
    `FeatureFlag::MultiLevelOrchestration` — the same flag as the GUI surfaces. With the
    flag disabled, the bar keeps today's flat root-anchored rendering, and children's
    `run_agents` calls continue to fail closed (rule 30), so depth > 1 cannot arise from
    new activity. The card disclosure (rule 28) uses the same gate.

### Discoverability

44. The `?` shortcuts overlay's existing `Orchestration` section (gated on
    `orchestration_available`) is unchanged in trigger and placement; its content is
    extended with the level keys only if implementation finds room without crowding:
    `Shift+↑ navigate to agents` remains the single required entry.
45. The `▸N` badge is the discoverability affordance for depth: it is the only element
    that signals "there is more underneath", and selecting the badge's tab reveals the
    level (rule 3).
46. No new slash command. `Shift+↑` remains the single entry point to the bar.

### Narrow terminals

47. The bar remains a single row at every width and never writes outside it. The
    baseline's "prioritize chrome" clause (CODE-1822 clause 44) is superseded by an
    explicit ladder. Chrome is shed before content, in this drop order:
    - **T0**, ≥ 96 cols: everything — `   Agents:   ` leading, breadcrumb labels at 12,
      anchor label at 20, child labels at 20, `▸N` badges.
    - **T1**, < 96: breadcrumb label cap 12 → 8; child label cap 20 → 16.
    - **T2**, < 84: the `Agents:` leading collapses to two cells of padding.
    - **T3**, < 72: breadcrumb chips collapse to marker-only (`‹`, still selectable);
      anchor label cap → 8.
    - **T4**, < 64: the anchor collapses to its glyph alone; child label cap → 12;
      badge `▸N` → `▸`.
    - **T5**, < 56: the badge is dropped; child label cap → 8.
48. Never dropped at any width: the divider, the anchor's glyph, one breadcrumb marker
    per rendered chip while drilled in, the selected child's glyph plus at least one
    label cell with ellipsis, and any applicable overflow arrow. Below that, the child
    region pages down to a single tab — the existing floor.
49. The tier boundaries are recommended defaults, not contract: implementation may tune
    them a few cells either way if render-to-lines tests show better packing, but the
    drop *order* (chrome before breadcrumbs before anchor before badges before labels) is
    normative.
50. Rationale: today's fixed prefix is 31 cells (`   Agents:   ` 13 + `orchestrator` tab
    14 + divider 4), leaving 29 cells for children at 60 columns — one child plus an
    arrow. T4 cuts the prefix to 9 cells and fits three.

### Key routing and focus priority

This section records how `←`/`→` are routed today, verified on master, because this
design reuses those keys for level navigation (rule 17). The short version: key
dispatch is focus-scoped, this design adds no new keybindings and no new keymap
contexts, and the input box always wins the arrows while it is focused.

51. Dispatch mechanism: a keystroke is offered to the **focused view first, then its
    ancestors**, and a binding fires only where its context predicate matches. The
    responder chain is the focused view plus its ancestor views only
    (`crates/warpui_core/src/core/app.rs:2066-2074` `get_responder_chain`,
    `app.rs:1498-1518` `view_ancestors`); each view in the chain contributes its own
    keymap context (`app.rs:2008-2033`); matching walks the chain deepest-first and the
    first match wins (`app.rs:2178-2214` `dispatch_keystroke`, root-first chain iterated
    with `.rev()`). A view outside the focused chain never sees the key.
52. When the **input box is focused, it takes the arrows**: `←`/`→` are
    `tui:input:move_left`/`move_right` (cursor movement, including across wrapped
    multi-line input; `crates/warp_tui/src/editor_interaction.rs:197-210`) and
    `Shift+←`/`Shift+→` are `tui:input:select_left`/`select_right`
    (`editor_interaction.rs:253-266`), all registered against the `TuiInputView` context
    (`crates/warp_tui/src/keybindings.rs:93-98`). The tab bar's bindings cannot fire
    then, for two independent reasons:
    - they require the `TuiOrchestrationTabBarFocused` context flag
      (`orchestration_tab_bar.rs:59`), and
    - that flag is inserted into the session view's keymap context only while the bar
      holds focus **and** the agent composer owns the input target
      (`terminal_session_view.rs:5022-5031`) — so a blocking card or full-screen
      terminal surface also suppresses the bar's bindings automatically.
53. When the **bar is focused** (`Shift+↑` from the input's first visual row with no
    active selection — CODE-1822 clauses 13-16 — or a prior bar interaction), the
    session view holds real focus (`set_orchestration_tab_focus` calls
    `ctx.focus_self()`), the input view is **not in the responder chain at all**, and
    the active set is exactly: `←`/`→`/`Tab`/`Shift+Tab`/`Shift+←`/`Shift+→`
    (`orchestration_tab_bar.rs:60-109`), `↓`/`Shift+↓` to leave the bar and `Esc` to
    return to the root (`terminal_session_view.rs:918-945`), and the fixed `Ctrl+C`
    (`orchestration_tab_bar.rs:52-57`). So the bar is keyboard-reachable only via
    `Shift+↑` (mouse clicks on tabs work without bar focus, per CODE-1822 clause 28).
54. Complete list of the TUI's other `←`/`→` consumers on master, each scoped to its own
    focused or blocking surface and therefore unable to contend with the bar or the
    input: the run_agents card's configuration pages (`orchestration_block.rs:77-98`,
    `TuiOrchestrationBlockConfiguring` context only — the acceptance card binds no
    arrows), the handoff card's configuration pages (`handoff/block.rs:88-103`), the
    ask-question card (`tui_ask_question_view.rs:60-83`, active-blocker context), the
    statusline-config reorder mode (`statusline_config_view.rs:57-72`), and the
    attachment bar when explicitly focused via `Tab` (`attachment_bar/view.rs:75-98`).
    The option selector binds only `↑`/`↓` (`option_selector.rs:44-63`). On the
    cloud-run child surface, `Enter` opens the run URL and `Shift+↑` focuses the bar
    (`cloud_run_view.rs:68-94`); no arrows. `←` on an **empty, unfocused-bar** agent
    input additionally opens the conversation switcher (`input/view.rs:949-955`) — an
    input-focused behavior this spec does not touch.
55. Focus is always visually disambiguated, so the owner of the arrows is never
    ambiguous: while the bar is focused the selected tab uses the focused magenta
    selection treatment (CODE-1822 clause 7) and the footer switches to the bar's
    footer variants (rules 22, 27); while the input is focused the input shows its
    cursor and the standard footer; a blocking card owns the footer and hints while
    active.
56. Invariant: rule 17 changes only what the already-bar-scoped `←`/`→` bindings
    traverse. It introduces no new keybindings, no new keymap contexts, and no change
    to any other surface's bindings, so no existing `←`/`→` consumer changes behavior.

## Mockups

Exact cell-grid frames derived from the tab bar's real accounting (`tab_bar.rs`: tabs are
`pad(1) + [glyph + space] + label + [space + badge] + pad(1)`, divider is `pad(1) + | +
pad(2)`, child gap 3, leading `   Agents:   ` 13 cells). Glyphs are the shipped
vocabulary: status `●` (running/waiting) `■` (blocked) `×` (error), identity glyphs
`⊹ ⟡ ✶ ◊ ⊛ * ✠`, breadcrumb marker `‹`, rollup badge `▸N`, overflow arrows `← →`.

Example tree: root orchestrator → researcher (→ crawler (→ fetch-a, fetch-b), indexer,
ranker), implementer, reviewer.

### Root level, depth ≥ 2 present — 100 cols (T0)

Identical to today except the `●` on `orchestrator` (rule 5) and the `▸3` on `researcher`
(rule 10):

```
1234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890
   Agents:    ● orchestrator  |   ● researcher ▸3    ⟡ implementer    ■ reviewer
```

### Drilled into `researcher` — 100 cols

Selecting `researcher` (a group child) re-anchored the bar to its level. One breadcrumb
chip, because the parent is the root. `crawler` is itself a group (`▸2`):

```
   Agents:    ‹ orchestrator   ● researcher  |   ● crawler ▸2    ◊ indexer    ● ranker
```

### Drilled into `crawler`, depth 2 — 100 cols

Two breadcrumb chips — root then parent — never more (rule 7):

```
   Agents:    ‹ orchestrator   ‹ researcher   ● crawler  |   ● fetch-a    ⊛ fetch-b
```

### Tree-wide `Tab` landing on a grandchild

The bar re-anchors so the user sees the grandchild among its siblings, not a flat list.
This is the moment today's flat row is worst and the design earns the most:

```
before Tab (selection = orchestrator, anchor = root)
   Agents:    ● orchestrator  |   ● researcher ▸3    ⟡ implementer    ■ reviewer

after Tab lands on `crawler` (a child of researcher; crawler has children → it anchors)
   Agents:    ‹ orchestrator   ● researcher  |   ● crawler ▸2    ◊ indexer    ● ranker
                                                 ~~~~~~~~~~~~~ selected
```

### Focused footers

[Amended 2026-08-10, implementation review: the first variant is split — the root tab
is never a kill target, while a drilled-in anchor (a selected group child, per rule 24's
amendment) gets the group-kill footer.]

```
root tab selected:
Tab or ← → to navigate  Shift + ← → to go to start/end  ↓ to send a message

drilled-in anchor selected (anchor ≠ root):
Tab or ← → to navigate  Shift + ← → to go to start/end  ↓ to send a message  Ctrl+C to kill sub-agent +2 nested

leaf child selected:
Tab or ← → to navigate  Shift + ← → to go to start/end  ↓ to send a message  Ctrl+C to kill sub-agent

group child selected (rule 27):
Tab or ← → to navigate  Shift + ← → to go to start/end  ↓ to send a message  Ctrl+C to kill sub-agent +2 nested
```

### 80 cols (T1 + T2), drilled into `researcher`

`Agents:` collapsed to two cells, breadcrumb label capped at 8, child labels at 16:

```
12345678901234567890123456789012345678901234567890123456789012345678901234567890
   ‹ orche...   ● researcher  |   ● crawler ▸2    ◊ indexer    ● ranker
```

### 60 cols (T4), drilled into `researcher`

Breadcrumb is marker-only, the anchor is its glyph, badge shrinks to `▸`. The divider
still separates the anchor from its level, so the structure survives:

```
123456789012345678901234567890123456789012345678901234567890
   ‹   ●  |   ● crawler ▸    ◊ indexer    ● ranker
```

### 60 cols (T4), root anchored

Today this width fits one child; the shed chrome fits three:

```
   ●  |   ● researcher ▸    ⟡ implementer    ■ reviewer
```

### 60 cols with overflow — six children at the level

```
   ‹   ●  |   ● crawler ▸    ◊ indexer    ● ranker    →
```

### Permission card with the multi-level disclosure (rule 28)

```
■ Can I start additional agents for this task?
   Agents (3):
   ⊹ researcher  •  ⟡ implementer  •  ✶ reviewer
   These agents may start their own child agents.

   Location: Local  •  Harness: Warp  •  Model: Default model

Enter to accept  Ctrl + E to edit  Ctrl + C to reject
```

### Message from a non-direct sender (rule 31)

```
● ⟡ researcher › crawler ▸ Fetched 214 pages; two hosts rate-limited, retrying with backoff.
```

## Decisions

Each decision states the chosen option and the rejected alternatives with the reason.

1. **Drill-down over flat-with-depth-markers.** Chosen: drill-down (the GUI's model).
   Rejected: a flat row with parent-prefixed labels (`researcher › crawler`) — at a
   20-cell label cap the prefix consumes the name, the row grows unboundedly with tree
   size, and it still cannot express "this child has a subtree" without a second marker.
   Rejected: a two-segment bar (breadcrumb region + flat descendants) — pays breadcrumb
   cost without bounding the row. The requester's directive is GUI parity; drill-down is
   the GUI's architecture.
2. **Selection-derived anchoring; no descend/ascend keystrokes or state.** Chosen because
   it is the GUI's exact mechanic (`drill_down_anchor_id`) and it dissolves two problems:
   no new keybinding (the `Enter`-to-descend alternative collides with the cloud-run
   view's `Enter` = open Oz URL binding and would need context surgery), and no stale
   "descended" state to invalidate when the tree changes. Rejected: explicit
   descend/ascend keys with sticky drill state — more state, more keys, no capability
   gained.
3. **At most two breadcrumb chips (root + parent).** Chosen: mirror the GUI's
   `breadcrumb_ids` exactly. Rejected: full ancestor chains — unbounded width in the
   scarcest dimension for information the parent chip already implies, and a gratuitous
   divergence from the GUI.
4. **Rollup in the trailing badge, own status in the leading glyph.** Chosen: mirrors the
   GUI (avatar = own status, group badge = subtree rollup) and leaves leaf tabs
   pixel-identical to today. Rejected: aggregated status in the leading glyph — overloads
   one cell with two meanings and makes a group child's glyph inconsistent with its
   sort position (rule 16).
5. **`Tab` stays tree-wide while `←`/`→` become level-scoped.** Chosen, and **approved
   by the requester (2026-08-10)**: preserves total reachability and existing muscle
   memory while giving levels cheap local navigation, and rules 51-56 establish it
   conflicts with no existing keybinding and never contends with the input box. This
   splits today's alias pair — both key sets currently trigger the same
   `tui:orchestration_tabs:previous`/`:next` actions — into two distinct action pairs
   (see the implementation note under rule 18). Rejected: all keys tree-wide — skipping
   a finished subtree of N nodes costs N presses. Rejected: all keys level-scoped —
   strands reachability behind repeated drill-ins and breaks today's `Tab` behavior.
6. **Subtree kill on `Ctrl+C` for group children.** Chosen, and **approved by the
   requester (2026-08-10)**: the alternative — GUI-parity single-node kill — orphans
   grandchild sessions that the TUI can then never display or reclaim. Rejected: an
   extra confirmation press for group kills — inconsistent with the established
   single-press bar-focused kill, and the footer already names the blast radius
   (rule 27).
7. **No `?`-overlay expansion or slash command.** The badge plus footers carry
   discoverability (rules 44-46). Rejected: `/agents` command — adds a command for a
   surface that is already one keystroke away and invisible exactly when the command
   would do nothing.

## Deliberate divergences from the GUI

- **Kill cascades to the subtree** (rule 24); the GUI's per-pill Kill removes only the
  target node. The GUI's behavior orphans hidden panes; in the TUI orphaned retained
  sessions are unreachable and leak. If the GUI later adopts subtree kill, the two
  front-ends reconverge.
- **No pinning, hover cards, or per-agent menus** (non-goals 1-3), carried over from the
  CODE-1822 baseline.
- Everything else — anchoring, breadcrumbs, rollups, ordering, the card disclosure, the
  aggregation semantics — is the GUI model translated to cells.

## Assumptions

Recorded choices made without an explicit requester answer; all five stood unchallenged
through spec approval on 2026-08-10.

1. Gating on `FeatureFlag::MultiLevelOrchestration`, identical to the GUI surfaces.
2. The two depth-1 additive cues (anchor status glyph, `▸N` badge) are acceptable visual
   changes to the existing bar.
3. Degradation tier boundaries (rule 47) are tunable defaults; only the drop order is
   normative.
4. QUALITY-1544 is out of scope and not a blocker: the bar renders what the client knows
   (rules 40-42).
5. Message-header depth prefixes (rule 31) are in the first cut; they are cheap and are
   the only depth signal for remote subtrees until QUALITY-1544 lands. Drop to a
   follow-up if implementation needs to shed scope.

## Baseline amendments (CODE-1822 PRODUCT.md)

On approval, the following clauses of the baseline are superseded; all other clauses
carry over unchanged, scoped to the rendered level where they concern the child region:

- Clauses 3, 5 (bar contents / orchestrator label) → rules 1, 4-5, 7-9.
- Clauses 9-12 (ordering) → rules 15-16 (same policy, applied per level).
- Clauses 19-27 (keyboard navigation) → rules 17-21.
- Clauses 33 (fixed leading orchestrator) → rules 1, 7-9 (anchor plus breadcrumbs form
  the fixed prefix).
- Clause 39 (page state shared tree-wide) → rule 37 (page state per level).
- Clause 44 (narrow-width priorities) → rules 47-50.
- Non-goal "no Ctrl+C kill" was already superseded by the shipped kill path; rules 23-27
  define its multi-level form.
