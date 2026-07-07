# Local-to-Cloud Handoff — Product Spec
Linear: [REMOTE-1486](https://linear.app/warpdotdev/issue/REMOTE-1486)
## Summary
Let a user mid-conversation in a local Oz agent send that conversation to a fresh cloud agent. The cloud agent picks up the conversation history, the workspace state (uncommitted git diffs and modified files), and the user's optional new prompt. The local agent stays usable and is unaffected.
## Problem
Today there's no first-class way to delegate "what I'm working on right now" to the cloud. Users have to retell the cloud agent the context, copy/paste plans, and manually push uncommitted changes. The existing cloud-to-cloud handoff (REMOTE-1290) covers continuation across cloud sandboxes, but the symmetric local→cloud transition is missing.
## Goals
- A user in a local Oz agent conversation can hand off to a cloud agent without leaving their flow — clicking the "Hand off to cloud" chip (or running `/move-to-cloud`) opens a split cloud-mode pane next to the local agent, where the user types a follow-up prompt and submits.
- The cloud agent receives the local conversation's history (forked into a fresh cloud conversation) and the workspace's uncommitted state (git diffs + modified files the agent has touched).
- The new pane's env selector defaults to whichever env contains the most touched repos.
- Handoff does not interrupt the local conversation — the user can keep typing into it.
- The cloud agent runs an Oz harness in V0; the design leaves room for third-party harnesses as a follow-up.
## Non-goals
- Third-party harnesses (Claude Code, Gemini, etc.). Extending `/move-to-cloud` to dispatch on the active conversation's harness is a follow-up that reuses most of the plumbing.
- A symmetric cloud→local handoff. That's REMOTE-1290's existing rehydration target plus future work.
- Multi-conversation / batch handoff. One conversation at a time.
- Bidirectional sync after handoff. The cloud agent operates on a forked copy (different `conversation_id`); local edits after the handoff don't propagate to the cloud, and vice-versa.
- A CLI surface in V0. The chip and slash command are UI-only entry points.
- A redesign of the env selector or environment management UI.
- Capturing system state outside the workspace (caches, daemons, env vars, MCP server state) — same scope as cloud→cloud handoff.
## Figma
None provided.
## Behavior
### Entry points
1. A "Hand off to cloud" chip is added to the agent input footer's right slot whenever `FeatureFlag::OzHandoff && FeatureFlag::HandoffLocalCloud` are both enabled, in agent-view panes only, and not for session viewers (handoff is host-initiated). The chip uses the existing `bundled/svg/upload-cloud-01.svg` icon (cloud-with-upward-arrow); design may swap this for a bespoke icon as a follow-up.
2. A slash command `/move-to-cloud [optional prompt]` is registered under the same flag gates plus the existing `AGENT_VIEW | ACTIVE_CONVERSATION | AI_ENABLED` availability rules. The name is harness-agnostic so the same command can dispatch to non-Oz harnesses as a follow-up.
3. Both entry points dispatch `WorkspaceAction::OpenLocalToCloudHandoffPane`, which splits a fresh cloud-mode pane to the right of the active pane. The slash command pre-fills the new pane's prompt with whatever followed the command; the chip leaves it empty. The local pane stays in place and remains fully active throughout.
4. Per-conversation eligibility is enforced by the click handler, not chip visibility. If the active conversation has a synced `server_conversation_token` and is non-empty, the new pane is seeded with handoff context (forked + snapshot uploaded on submit). Otherwise the new pane opens as an ordinary fresh cloud-mode pane (no fork, no snapshot) — the user clearly wanted a cloud-mode pane regardless.
### Handoff pane
5. The handoff pane is a regular cloud-mode pane (entered via `AgentViewEntryOrigin::CloudAgent`). It uses the existing cloud-mode input footer — model selector, env selector chip, prompt editor, voice/file inputs, send button — with no handoff-specific buttons. The pane intentionally does **not** opt into the new `CloudModeInputV2` UI even when that flag is on; V2 is for fresh cloud-mode runs only.
6. There is no dedicated handoff banner UI in V0. Touched-repo derivation runs silently in the background (§9) and the env selector's default updates when derivation completes (§7). Per-repo `✓ / ⚠` overlap status is intentionally not surfaced; submission errors surface through the model's submission state but have no banner-style row.
7. The pane's env selector layers a repo-aware default on top of the existing recency-based default: each env is scored by the number of touched repos it contains; highest score wins, ties broken by most-recently-used. When no env contains any touched repo, the existing default applies (`CloudAgentSettings.last_selected_environment_id` → most-recently-used → no-env). The user can override at any time by clicking the env selector chip.
8. The send button follows the regular cloud-mode rules (prompt non-empty) plus a guard until touched-repo derivation completes. Closing the pane abandons the handoff with no side effects on the local conversation.
### Touched-repo derivation
9. The handoff pane opens immediately — it does not wait for any I/O. Touched-repo derivation runs asynchronously off the main thread; the pane chrome stays interactive throughout. Derivation:
    - Walks the most recent action results in the conversation (capped at `MAX_TOOL_CALLS_TO_SCAN = 500`): file paths from edit/read/grep/glob actions plus the `cwd` of every shell command.
    - For each path, walks up to the nearest `.git` directory. The set of distinct git roots is the touched-repo list.
    - For each git root, runs `git remote get-url origin` (best-effort) to parse a `<owner>/<repo>` for env-overlap matching. Branch and HEAD metadata are gathered later by the snapshot pipeline.
    - Modified files outside any `.git` are tracked separately as orphan files and uploaded as raw file contents during snapshotting.
10. If the conversation has no touched repos, the handoff still proceeds; the cloud agent starts with a clean workspace.
### Submitting
11. When the user submits the prompt, the client (off the main thread):
    1. Builds the snapshot from each touched repo (git diff including binary patches, untracked files, branch / HEAD metadata) and each orphan file.
    2. Calls `POST /agent/handoff/upload-snapshot` to mint an `initial_snapshot_token` and presigned upload URLs scoped to `handoff/{initial_snapshot_token}/`.
    3. Uploads the artifacts in parallel to GCS.
    4. Calls `POST /agent/runs` (`SpawnAgentRequest`) with `fork_from_conversation_id` + `initial_snapshot_token` set. The server forks the source conversation, creates the new task, and binds the initial snapshot token to the new run's queued execution; the cloud sandbox reads the snapshot files directly from `handoff/{initial_snapshot_token}/`.
    5. The pane transitions into the live cloud-mode session through the same `AmbientAgentViewModel::spawn_agent_with_request` streaming path used for fresh cloud-mode runs (`WaitingForSession` → `SessionStarted`).
12. Per-file upload failures are best-effort: each retries on transient errors with bounded backoff; failures past retry are logged but do not block the handoff. If every blob fails, the cloud agent is created without rehydration content and the failure is reported via `report_error!` for on-call. Failures of `upload-snapshot` or task-creation themselves are fatal: no cloud agent is created and the failure surfaces inline via `HandoffSubmissionState::Failed` so the user can retry.
13. While the handoff is in flight the send button is disabled ("Starting…"). Closing the pane abandons the handoff (in-flight uploads abort). The local conversation is unaffected throughout — the user may keep typing, run other commands, etc.
### Post-handoff state
14. The local conversation continues normally with no "this was handed off" annotation in V0 — the handoff pane being open next to it is the discoverability surface.
15. The new cloud agent has a *new* `conversation_id` (different from the local conversation's `server_conversation_token`); the local and cloud conversations diverge at the handoff point. It inherits the local conversation's task and message history up to the handoff, receives a `<system-message>`-wrapped rehydration prompt instructing it to apply the snapshot patches before answering, then handles the user's optional follow-up prompt. The new agent appears in the cloud agent management view and supports the standard reopen flows.
### Pre-SessionStarted visualization in the handoff pane
16. While the cloud agent's session is being established, the handoff pane shows the user's submitted prompt as a queued user-query indicator (REMOTE-1454's visual treatment, no Send-now / dismiss buttons), the warping "Setting up environment" indicator, and the collapsible "Running setup commands…" summary — the standard cloud-mode setup affordances.
17. When the cloud agent's first turn arrives, the queued-prompt indicator is removed and the pane behaves like any live cloud-mode pane. If the run fails, is cancelled, or requires GitHub auth before the session connects, the queued-prompt indicator is torn down and the existing failure / cancel / auth UI is shown.
### Edge cases and error states
18. If a touched repo's local clone is unreadable, missing, or has a corrupt git state, that repo is captured as a `gather_failed` entry in the snapshot manifest and the rest of the snapshot proceeds. The rehydration prompt tells the cloud agent to fail the apply for that repo and report it.
19. Modified files outside any `.git` are uploaded as raw file contents (the `kind: file` declaration form in the existing snapshot pipeline) and listed in the manifest with their original paths.
20. If the user has no environments, the pane still works with no env selected; the cloud agent runs against the platform default image.
21. If the user is at cloud agent capacity, the cloud agent is created in a queued state — the same behavior as `oz agent run-cloud`. The handoff pane shows the existing "queued / waiting for capacity" UI.
22. If the local pane closes mid-handoff, in-flight uploads abort and no cloud agent is created. The user is not warned in V0 — handoffs are short and rarely interrupted.
23. The handoff is per-conversation; running it twice produces two independent cloud agents, each forked from the same point.
### Permissions and authorization
24. Handoff requires the user to be logged in and have permission to create cloud agent runs in their workspace — same permissions as `oz agent run-cloud`. The chip is hidden from session viewers.
25. The selected environment must be readable by the user; the dropdown only lists envs the user already has view access to (same scoping as the existing cloud-agent setup `EnvironmentSelector`).
## Open questions
- The chip uses an existing icon for V0; design may swap it for a bespoke handoff icon later.
