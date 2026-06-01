# Session Sharing for Orchestrated Agent Sessions
## Summary
When a shared agent session has spawned child agents, parent-scoped session views should show the existing orchestration pill bar so viewers can inspect the orchestrator and its direct children. Direct child session links remain child-scoped and do not open the parent orchestration view.
## Problem
Remote cloud agents are already viewed through shared-session viewers, and local orchestrated sessions already use a pill bar to switch between parent and child conversations. The missing product definition is where that existing pill bar should appear across session-sharing entrypoints and local/remote topologies.
## Goals
1. A viewer opening a parent/orchestrator session can inspect the orchestrator and all direct child agents from the existing orchestration pill bar.
2. Sharing a parent session makes the direct child sessions accessible from that parent session.
3. Opening a direct child session link stays scoped to that child session.
4. The parent-scoped pill bar appears consistently in native Warp and the web/WASM shared-session viewer, with platform-specific affordances following the existing pill bar behavior.
5. The behavior is consistent across local-local, local-remote, remote-remote, and remote-local orchestration topologies from the viewer’s perspective.
## Figma
Figma: none provided. Use the existing orchestration pill bar behavior and visual treatment. This spec does not redefine the pill bar’s layout, status badges, hover cards, ordering, truncation, or other interaction details.
## Behavior
### Terms and scope
1. An orchestrator session is an agent session whose active agent spawned one or more direct child agents.
2. A child session is the session for one direct child agent spawned by an orchestrator.
3. A parent session link targets the orchestrator session. A child session link targets one child session.
4. “Local” means the agent is running on the user’s current client when the user owns the session. “Remote” means the agent is running in a cloud or driver process and is viewed through a shared-session viewer. In remote-local flows, the child is local to the remote driver process, but still remote from the user’s client.
5. Only one level of orchestration is supported: one orchestrator plus its direct child agents. Child sessions are treated as leaf sessions for this feature.
### Sharing rules
6. All remote agent sessions are automatically shared, whether the remote agent is an orchestrator or a child.
7. Sessions that are local to the user’s client are not shared until the user explicitly shares them through an existing share entrypoint, such as the share modal, pane/header action, context menu, or copy-sharing-link action.
8. When a parent/orchestrator session becomes shared, its direct child sessions are also accessible to viewers of that parent session so the parent-scoped pill bar can display and open them.
9. Parent sharing applies to children that already exist when sharing starts and to direct children spawned while the parent share remains active.
10. A viewer who can access a parent session link is allowed to access the direct child sessions exposed through that parent view. Opening or copying a direct child link from that context is acceptable, but the direct child link remains child-scoped.
11. Sharing or opening a direct child session does not implicitly share or navigate to the parent or sibling child sessions.
12. Direct child links may still show human-readable names for agents referenced in that child’s transcript. Agent names are considered orchestration metadata that may be mutually visible between parent and child sessions when those sessions are otherwise accessible.
### Viewer entrypoints and pill bar presence
13. When a user starts or opens a remote orchestrator from the `/cloud-agent` flow in native Warp, the resulting shared-session viewer is parent-scoped and shows the existing orchestration pill bar when the orchestrator has direct children.
14. When a user opens an orchestrator session from the Oz web UI, the web viewer is parent-scoped and shows the existing orchestration pill bar when the orchestrator has direct children.
15. When a user opens a `warp://shared_session/...` or web shared-session link for an orchestrator, native Warp or the web viewer opens a parent-scoped view and shows the existing orchestration pill bar when the orchestrator has direct children.
16. When a user explicitly shares a local orchestrator session, the resulting parent session link opens a parent-scoped view and shows the existing orchestration pill bar when the orchestrator has direct children.
17. When a user opens a direct child session link in native Warp or on the web, Warp opens the child-scoped shared-session view. The child-scoped view does not show the parent/orchestrator pill, sibling child pills, or parent orchestration navigation.
18. “Open in desktop” or equivalent handoff actions preserve the link target. A parent link remains parent-scoped after handoff; a child link remains child-scoped after handoff.
19. Shared-session viewers that are created internally so a parent pill can display a remote child do not create additional visible browser tabs, windows, or user-facing links by themselves.
20. If the viewer joins a parent-scoped session before any child has been spawned, the pill bar appears after the first direct child becomes known. A short delay is acceptable.
21. If a parent-scoped viewer is open while additional direct children spawn, the pill bar updates to include those children.
22. On web/WASM, the pill bar uses the existing web-compatible subset of pill bar behavior. Native-only pane-management actions are not required on web.
23. Viewer pill selection is local to that viewer. Switching pills in a shared-session viewer should not force the sharer or other viewers to switch conversations. Any initial selected-conversation sync should follow existing shared-session behavior.
### Conversation body behavior
24. When viewing the orchestrator pill, the viewer sees the orchestrator session transcript, including orchestration cards, child launch requests, lifecycle updates, and other parent-session activity included by the share.
25. When viewing a child from a parent-scoped view, or when opening a child-scoped direct link, the viewer sees that child’s session transcript according to normal shared-session behavior.
26. Within orchestrator and child conversation bodies, user-visible references to agents by internal ID resolve to the correct human-readable agent name wherever the existing pill bar has enough metadata to do so. This includes send-message-to-agent blocks, received-message-from-agent blocks, and lifecycle/status blocks. If a name cannot be resolved, use the same fallback behavior as the existing pill bar or conversation renderer.
27. Since child sessions are leaf sessions for this feature, any orchestration-looking artifacts inside a child transcript render as ordinary transcript content and do not create a second-level pill bar or parent/child navigation.
28. Completed child sessions remain reachable from the parent pill bar and show their final available transcript and status.
29. If a child session exists but is not ready to join yet, selecting it from the parent pill bar shows the existing loading or pending behavior rather than stale parent content.
30. If a child session cannot be loaded because of a network, permission, or session creation failure, the child view shows an unavailable or error state using existing shared-session error patterns. The parent view and other direct child sessions remain usable.
31. If the parent or a child session ends while a viewer is inspecting the orchestration, ended-session behavior follows existing shared-session behavior for the active session.
### Topology-specific behavior
32. Local-local: when a local orchestrator starts local children, nothing is shared until the user shares the parent or a child. Sharing the parent makes the local children accessible from the parent link and visible in the parent-scoped pill bar. Sharing a child directly opens only that child.
33. Local-remote: when a local orchestrator starts a remote child, the local user can inspect the child from the local orchestration UI. The remote child session is automatically shared. If the user shares the local parent, the parent link shows the parent and the remote child in the parent-scoped pill bar.
34. Remote-remote: when a remote orchestrator starts remote children, the parent and children are automatically shared. Opening the parent link shows the parent-scoped pill bar. Opening a child link shows only that child.
35. Remote-local: when a remote orchestrator starts a child that is local to the remote driver process, both sessions are still remote from the user’s client and are automatically shared. Opening the parent link shows the parent-scoped pill bar. Opening a child link shows only that child.
36. Mixed child modes: if one orchestrator has both local-to-parent children and remote children, the parent-scoped pill bar presents the direct children together using the existing pill bar behavior. The viewer should not need to understand where each child is executing to navigate the orchestration.
### Permissions, privacy, and roles
37. A viewer who can open the parent link can inspect all direct children exposed through that parent link without needing separate child links.
38. A parent link must not reveal sessions outside the parent’s direct child set.
39. A child link must not provide navigation to the parent or sibling child sessions.
40. Human-readable agent names may appear in parent and child conversation bodies when the transcript references those agents.
41. A viewer’s role in a child session reached through the parent link must not exceed the viewer’s effective role for the parent share.
42. If a viewer has an interactive/executor role, existing shared-session controls apply only to the currently active session. The pill bar itself is navigation, not an agent-control surface.
43. Request-access, role-change, participant presence, reconnect, and ended-session behavior should follow existing shared-session behavior for the active session.
44. Copying a session sharing link should preserve the current scope. A parent share action copies a parent-scoped link; a child share action copies a child-scoped link.
### Loading and reconciliation
45. Parent-scoped viewers reconcile the direct child list while the parent session is live so newly spawned children appear and lifecycle status changes are reflected through the existing pill bar.
46. If network connectivity is lost, the pill bar keeps the last known direct children and statuses. On reconnection, the viewer reconciles with the current direct child list and statuses.
47. If a direct child finishes before a viewer joins the parent session, the child still appears in the parent pill bar when the parent share has permission to expose it.
48. If a direct child session was never successfully created or shared, the child can still appear as an errored or unavailable child if the parent transcript or orchestration metadata indicates the child launch failed.
49. If parent and child state disagree temporarily, the UI should prefer a stable, non-destructive presentation: keep known direct child pills visible, update statuses when confirmed, and avoid dropping a child from the list solely because a refresh is delayed.
## Non-goals
1. This spec does not add support for more than one level of orchestration.
2. A direct child link does not provide a breadcrumb or “back to parent orchestration” experience.
3. The parent pill bar is not a bulk control surface for cancelling, restarting, messaging, or otherwise managing child agents.
4. This spec does not change the visual design or detailed interaction model of the existing orchestration pill bar.
5. This spec does not introduce a combined export, fork, or replay artifact for an entire orchestration group.
