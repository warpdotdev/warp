# PRODUCT: TUI Cloud Orchestration Children
Linear: [CODE-1822 — Orchestration](https://linear.app/warpdotdev/issue/CODE-1822/orchestration)
Depends on: [specs/code-1822-tui-local-children/TECH.md](../code-1822-tui-local-children/TECH.md)

## Summary
The Warp TUI can launch cloud child agents from an accepted orchestration request and represent each child as a retained, navigable cloud session. Before cloud session viewing is available, the child surface is read-only and shows the cloud run's status plus an actionable link to view the run in Oz.

## Figma
Figma: none provided.

## Goals
- Make remote `run_agents` requests complete successfully in the TUI instead of returning the current unsupported-mode failure.
- Give each cloud child a stable TUI session and orchestration tab from launch through its terminal lifecycle state.
- Reuse the same cloud launch configuration, startup-error meaning, and lifecycle status meaning as the GUI without changing existing GUI behavior.
- Keep the v0 surface intentionally read-only while establishing the stable session identity needed for future cloud session viewing.

## Non-goals
- Viewing or interacting with the cloud child's shared terminal session in the TUI.
- Rendering the cloud child's transcript in the read-only cloud session.
- Sending follow-up prompts or steering the cloud child from the read-only cloud session.
- Stopping, killing, deleting, or restarting a cloud child from the read-only cloud session.
- Automatically retrying an orchestrated child after GitHub authentication completes.
- Changing `StartAgentExecutor`, `RunAgentsExecutor`, their result types, or their existing timeout and blocked-launch semantics.
- Changing the existing GUI cloud-pane launch, retry, status, or session-attachment behavior.
- Adding support for local CLI-harness child agents.

## Behavior
### Launch and navigation
1. When the user accepts a valid remote `run_agents` request in the TUI, Warp starts each requested cloud child using the approved run-wide execution configuration and that child's name, title, and prompt.
2. Each cloud child gets a retained TUI session as soon as its launch is dispatched. The session is created before the server assigns a cloud run ID, allowing startup progress and actionable startup blockers to remain visible.
3. A newly created cloud child appears in the existing orchestration tab bar as soon as it is a navigable retained session. Creating it does not steal focus from the orchestrator or another selected child.
4. Selecting the cloud child's orchestration tab focuses the same retained session for that child's lifetime. Status updates never replace the session, change its identity, reset orchestration paging, or steal focus.
5. Multiple cloud children may launch concurrently. Each child displays and updates only its own name, status, startup issue, and run link.
6. Local and cloud children can coexist in the same orchestration tree and use the same existing ordering and navigation behavior.

### Cloud session layout
7. Before cloud session viewing is supported, a focused cloud child renders a read-only status view instead of a terminal transcript, prompt input, inline menus, normal footer, zero state, or shell content.
8. The cloud session's primary callout is centered horizontally and vertically within the content area available beneath the orchestration tab bar.
9. The cloud session displays:
   - A status glyph using the same semantic status treatment as other TUI orchestration surfaces.
   - A concise status or startup message.
   - The relevant actionable link when one exists.
10. The child's orchestration tab/header also displays the same current status. The header and callout never disagree about the child's state.
11. The cloud session adapts to light, dark, and custom terminal themes using semantic styles rather than fixed colors.
12. On narrow terminal widths, callout text and visible URLs wrap without overflowing or dropping URL content. Resizing recenters and reflows the cloud session without changing its state.

### Starting and successful launch
13. From session creation until a run ID or startup issue is received, the cloud session shows a running/attention glyph and a message equivalent to `Starting cloud run…`.
14. A successful server response associates the returned task ID and run ID with the existing child session and resolves that child as launched in the parent `run_agents` result.
15. Once the run ID is available, the cloud session displays `Click the link or hit Enter to view cloud run here:` followed by the run's Oz URL.
16. The Oz URL targets the current Warp channel's Oz web application and the assigned run ID. The link never waits for a shared terminal session to become available.
17. The visible URL is selectable and copyable as text.
18. Clicking the link opens it in the user's configured browser.
19. Pressing Enter while the read-only cloud child or its orchestration tab bar is focused opens its current primary link. For a launched child this is the Oz run URL; for an authentication-blocked child this is the authentication URL.
20. Opening a link does not change TUI session selection, orchestration tab focus, child status, or launch state.

### Ongoing lifecycle status
21. After launch, the cloud session and orchestration tab reflect the child's server lifecycle using the existing TUI status meanings:
   - Queued, started, restarted, and in-progress states display as in progress.
   - Blocked displays as blocked.
   - Succeeded and idle display as succeeded.
   - Failed and errored display as failed.
   - Cancelled displays as cancelled.
22. Lifecycle status continues updating while the cloud child session is focused or in the background.
23. A terminal lifecycle state does not remove the cloud child session. The Oz run link remains available after success, failure, error, or cancellation.
24. Duplicate, replayed, delayed, or out-of-order lifecycle notifications must not create duplicate sessions or associate one child's status with another child.
25. If a lifecycle state is unknown, the cloud session fails closed to an error state rather than presenting the run as successful.

### GitHub authentication required
26. If cloud startup fails because GitHub authentication is required before a server-side run is created, the existing child session is retained and displays a blocked status.
27. The blocked cloud session shows:
   - The server-provided or shared fallback explanation.
   - The actionable GitHub authentication URL.
   - Clear text that the orchestration request must be run again after authentication.
28. Clicking the authentication link or pressing Enter opens the authentication URL.
29. Matching the current GUI behavior for an orchestrated remote child, the original `run_agents` child outcome is reported as failed. Completing authentication does not retroactively change that outcome.
30. The TUI does not automatically retry the failed child after authentication. This preserves the GUI's observable orchestrated-child result contract, but intentionally does not copy the GUI cloud pane model's independent post-failure auto-retry. A later orchestration request creates a new launch attempt using normal duplicate-agent and session behavior.
31. An authentication-blocked child is not removed by terminal failed-launch cleanup, matching the existing blocked-child behavior.

### Other startup failures
32. A non-recoverable startup failure that occurs before a run ID is assigned is reported in the parent `run_agents` result with its meaningful shared error message.
33. Terminal pre-run failures use the existing failed-child cleanup behavior: the optimistic child session and conversation are removed so the tab bar does not retain a dead child with no cloud run.
34. Cleanup of one failed child does not remove, refocus, or alter successfully launched or blocked siblings.
35. If request preparation fails before dispatch—for example, a missing parent run ID or unresolved required skill—the child reports that preparation failure rather than timing out.

### Interaction boundaries
36. The cloud session accepts no prompt or shell input. Printable keys are never forwarded to a local PTY or remote run.
37. The cloud session does not expose stop, kill, delete, retry, or follow-up actions.
38. When orchestration tabs are available and the cloud session itself is focused, the bottom footer shows the same `Shift + ↑ sub-agents` hint as a regular TUI session. `Shift+Up` focuses the orchestration tab bar directly even though no prompt input is rendered.
39. While cloud orchestration tabs are focused, the footer shows the regular tab navigation hints for Tab/Left/Right and Shift+Left/Right, but omits `Shift+Down` because returning focus to the read-only cloud session has no user-visible effect.
40. Existing application exit behavior remains available; focusing a cloud session must not make terminal-control keybindings act on a nonexistent local process.
41. The reusable link interaction behaves consistently for the Oz run URL and GitHub authentication URL, including hover treatment, click target, Enter activation, selection, and copying.
