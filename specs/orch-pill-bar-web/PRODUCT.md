# Orchestration Pill Bar in Shared Session Web Viewer

## Summary
When viewing a shared session that used orchestration, the session viewer (web or native) displays a pill bar showing the orchestrator and all child agents. Each pill shows the agent's name and lifecycle status. Clicking a pill switches the view to that agent's conversation in-place, giving viewers full visibility into multi-agent sessions. Gated behind `FeatureFlag::OrchestrationViewerPillBar`.

## Problem
Today, when a user views an orchestrated session on the web, only the orchestrator's conversation is visible. The existence of child agents, their names, statuses, and conversation transcripts are invisible to the viewer. This makes it impossible to understand what a multi-agent session did or to inspect individual agent work.

## Figma
Figma: none provided. Visual treatment matches the native client's orchestration pill bar. On web, pane-management actions are unavailable since the web viewer is single-pane.

## Behavior

### Pill bar visibility
1. The pill bar renders when the viewed shared session contains an orchestrated conversation — one where the orchestrator spawned at least one child agent.
2. The pill bar is positioned above the agent view content, below the pane header — matching the native client's placement.
3. The pill bar is hidden when the session has no orchestration (single-agent session with no children).
4. When the viewer joins a live session before any children have been spawned, the pill bar appears once the first child is detected. A brief delay is acceptable.
5. When viewing a completed session, the pill bar renders immediately with all children and their final statuses.

### Pill rendering
6. The leftmost pill represents the orchestrator. It displays the Oz icon and the hardcoded label "Orchestrator", matching the native client's behavior.
7. Child agent pills appear to the right of the orchestrator, in spawn order.
8. Each child pill displays:
   a. A colored avatar disc with the first letter of the agent name, uppercased.
   b. The agent's display name, truncated with ellipsis if it exceeds ~110px.
   c. A status badge overlaid at the bottom-right of the avatar disc.
9. Avatar disc colors are deterministic: same agent name always produces the same color, drawn from the same fixed 6-color palette used in the native client. A viewer and the sharer see the same color for the same agent.
10. The currently-selected pill (whose conversation is displayed) has a visually distinct highlighted background. All other pills use a muted background.

### Status badges
11. Each child pill shows a status badge reflecting the agent's lifecycle state:
    - **In Progress**: animated indicator (spinner or pulse).
    - **Succeeded**: checkmark.
    - **Failed / Errored**: error icon.
    - **Blocked**: pause icon.
    - **Cancelled**: cancelled icon.
12. The orchestrator pill does not display a status badge.
13. Status badges update as the sharer's session progresses (live sessions). A brief delay (a few seconds) between a child completing and its badge updating is acceptable. For completed sessions, badges show the final state immediately.

### Pill click interaction
14. Clicking a non-selected pill switches the agent view in-place to display that agent's conversation transcript. The clicked pill becomes the selected pill. The pill bar remains visible.
15. Clicking the currently-selected pill is a no-op.
16. Clicking the orchestrator pill while viewing a child conversation switches back to the orchestrator's conversation. This is the primary "back" navigation.
17. On web, there is no 3-dot overflow menu on pills. Pane-management actions (open in new pane, open in new tab, focus pane) are not available since the web viewer is single-pane and read-only. On native, the overflow menu and pane-management actions behave the same as for local orchestration.

### Hover details card
18. Hovering over a pill for 300ms displays a details card below the hovered pill. Moving the mouse away dismisses the card after 80ms.
19. The details card displays (when available):
    a. Agent avatar and full name (not truncated).
    b. Working directory path.
    c. Task description or prompt summary.
    d. Harness type chip (e.g. "Claude Code", "Codex") — omitted for the default Oz harness.
    e. PR branch name and number.
    f. Status label for child agents.
20. Fields with no data are omitted from the card rather than shown empty.
21. The card repositions to avoid overflowing the viewport.

### Conversation display
22. When the view switches to a child agent's conversation, the agent view renders that child's full conversation transcript (messages, tool calls, tool results, agent output) in read-only mode.
23. If the child's transcript has not been loaded yet, the agent view shows the default zero state (empty conversation view) while the transcript is fetched. The pill bar remains visible and interactive during loading. **Open question:** a dedicated loading indicator for transcript fetching would be better UX; this is deferred to a design follow-up that could improve loading states across all shared session views.
24. The pill bar continues to show all agents from the orchestration while viewing any conversation in the group — including while viewing a child.
25. Switching between conversations preserves scroll position per conversation. The existing pane swap mechanism handles this automatically.
26. Once a child's session has been joined, the conversation state is retained. Switching away and back displays the retained state immediately without re-joining.

### Live session behavior
27. New children that spawn while the viewer is connected appear as new pills appended to the right of existing child pills. A brief delay before newly spawned children appear is acceptable.
28. When viewing the orchestrator conversation, inline orchestration cards (run_agents confirmation, lifecycle status) render as they appear in the orchestrator's transcript (these flow through the existing session WebSocket).
29. Clicking a child pill joins that child's session. For a live child, new messages stream in after the initial replay completes. For a terminal child, the full transcript is replayed and the session completes.
30. If the sharer's session ends while the viewer is watching, the pill bar remains visible with the last known statuses.

### Completed session behavior
31. When a viewer opens a completed orchestrated session, the full orchestration structure (orchestrator + all children with final statuses) is available immediately.
32. The viewer can click through all child conversations to review their work.

### Horizontal overflow
33. When there are more child pills than fit in the available width, overflow pills are clipped at the pane boundary, matching the native pill bar's current behavior.

### Edge cases
34. If the orchestrator called `run_agents` but no children have spawned yet (e.g. waiting for user approval), the pill bar is hidden. It appears once the first child is detected, matching the native pill bar's behavior.
35. If a child agent's name is missing or empty, the pill displays a placeholder initial and a generic label like "Agent".
36. If the viewer loses network connectivity, the pill bar retains its last known state. On reconnection, it reconciles with current session state by re-fetching the children list.
37. If a child transcript fetch fails (network error, auth failure), the agent view shows an error state with a retry affordance. The pill bar remains functional.
38. A session with multiple `run_agents` batches shows all children from all batches in a single flat pill list, ordered by spawn time.

### Non-goals
39. Nested orchestration (a child that is itself an orchestrator with grandchildren) is out of scope. Only single-level parent→children is supported.
40. The pill bar is not interactive for execution control — the viewer cannot cancel, restart, or message agents.
41. Pinning pills or reordering them is not supported.
42. The pill bar works in both the web (WASM) viewer and the native client when viewing shared sessions. On the web viewer, pane-management actions (open in new pane/tab) are not available since the web viewer is single-pane. On the native client, pane-management actions are available as they are for local orchestration.
