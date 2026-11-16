# REMOTE-1458: Unified agent icon-with-status across Warp surfaces
## Summary
Warp shows an "agent icon" on several surfaces — vertical tabs, the pane header, the inline conversation list menu (the slide-out list of ACTIVE + PAST runs in `conversation_list/item.rs`), and the notifications mailbox. This spec defines the canonical agent-icon treatment and the cross-surface consistency contract so the same logical run always renders as the same icon no matter which surface it appears on. The Agent Management View (card list) is explicitly out of scope for this pass — its richer layout owns its own status treatment and we're not disrupting it yet. Reference issue: https://linear.app/warpdotdev/issue/REMOTE-1458.
## Problem
Across Warp's agent-run surfaces today, users frequently can't tell two fundamental things:
- **Whether a cloud run is Oz or a third-party harness** (Claude Code, Gemini CLI). Cloud runs with a non-oz harness render with the Oz cloud glyph during the entire pre-setup / setup phase because the client-side `CLIAgentSessionsModel` — the thing every surface consults for harness identity — is only populated once the harness CLI is detected in the shared session. Until then, even a Claude run that the user explicitly selected looks like an Oz run.
- **Whether a third-party agent run is local or in the cloud.** A local `claude` CLI session and a cloud-mode Claude run render with identical brand colors and glyphs; nothing distinguishes the two, so users can't tell whether "Claude" in their tab bar is running on their machine or in a remote sandbox.
Concretely, each surface contributes to the ambiguity in its own way:
- `app/src/workspace/view/vertical_tabs.rs` resolves the tab icon from `CLIAgentSessionsModel::session(terminal_view.id())`. Before the harness CLI starts, ambient runs fall through to the plain `OzAgent` variant regardless of which harness the user selected.
- `app/src/terminal/view/pane_impl.rs` renders a single plain `OzCloud` glyph for every ambient run via `render_ambient_agent_indicator` and an unbadged Oz/OzCloud glyph for conversation-bound terminals via `render_agent_indicator`. No brand circle, no status badge, no harness distinction, no local-vs-cloud distinction.
- `app/src/workspace/view/conversation_list/item.rs` (`render_item`) renders a plain `Icon::Cloud` glyph for every ambient row and a plain `render_status_element` for every local row — no brand color, no harness distinction. Rows representing Oz cloud, Claude cloud, and Gemini cloud runs all render as the same cloud outline.
- `app/src/ai/agent_management/notifications/item_rendering.rs` renders the correct brand color and glyph per CLI agent but hardcodes `is_ambient: false`, so a notification triggered by a cloud-mode Claude run looks identical to one triggered by a local Claude session.
Beyond the visual problem, the derivation logic itself is duplicated across surfaces and drifts: vertical tabs alone has two separate waterfalls (`resolve_icon_with_status_variant` and `TypedPane::summary_pane_kind`) that recompute the same fields, and neither routes through `TerminalView::selected_conversation_status_for_display`, which is where "surface InProgress during cloud setup" logic wants to live. Adding a fifth surface today means copy-pasting the waterfall a third time and re-reasoning about every edge case.
## Goals
- Render the same agent icon shape everywhere Warp represents an agent run: a brand-color circle containing the agent glyph, a status badge at the bottom-right (local runs) or a white cloud lobe with the status icon inside (ambient/cloud runs).
- Make the icon reflect the user's selected harness the moment a cloud run is committed — i.e. as soon as `AmbientAgentViewModel` transitions out of `NotAmbientAgent`, not only once the CLI command has started.
- Make the status component reflect the correct run state on every surface, including the cloud-setup pre-first-exchange phase (surface as InProgress).
- When a single logical run is represented on multiple surfaces at the same time (e.g. a cloud task that's both a visible pane and a card in the conversation list), every surface renders a pixel-identical icon.
- Keep non-agent surfaces (plain terminal pane, shell indicators, error indicators) visually unchanged.
## Non-goals
- Redesigning the agent-icon visual itself. The circle + badge + cloud-lobe shape is defined in the Figma reference at https://www.figma.com/design/chk9pwt35jTJhf9KnHmZyE/Components?node-id=6535-4010 and is taken as given here.
- Changing how CLIAgentSessionsModel is populated, how shared sessions are joined, or how the Oz / harness detection pipeline works.
- Showing agent icons on surfaces that don't currently render an agent identity at all beyond the ones listed here.
- Migrating status rendering away from `ConversationStatus`; we still use that type as the canonical "what state is this in" value.
- Changing per-harness brand colors, icon glyphs, or sizing constants beyond what's needed to reuse the vertical-tab sizing on other surfaces.
## User experience
The canonical agent icon is a brand-color circle containing the agent glyph, with either a bottom-right status badge (local runs) or a white cloud lobe containing the status icon (ambient/cloud runs). It is rendered uniformly on every surface enumerated below, at surface-appropriate sizes.
### Vertical tabs
- When the user has selected a third-party harness (Claude, Gemini) and the run is in any ambient state (Setup, Composing, WaitingForSession, AgentRunning, Failed, Cancelled, NeedsGithubAuth), the vertical tab icon shows that harness's brand-color circle with the cloud lobe — not the Oz cloud circle — even before the harness CLI has started running.
- When the `CLIAgentSessionsModel` later registers the actual CLI session, the tab continues to render the same agent brand (matching the selected harness), so there's no visible "flip" at harness-start.
- When the run's harness is Oz, the tab shows the existing Oz ambient circle (brand purple background, Oz glyph, cloud lobe, status).
- When the selected harness changes while composing (user opens the dropdown and picks a different harness), the icon updates immediately.
### Pane header
- The existing "agent indicator" slot on the pane-header title row (rendered via `render_ambient_agent_indicator` for cloud sessions and `render_agent_indicator` for conversation-bound terminals) switches to the same brand-color circle + status badge + cloud lobe treatment used in vertical tabs, at the same visual dimensions (16px icon / ~24px overall circle).
- For an ambient Claude run in setup phase, the pane header shows a Claude-orange circle with a white cloud lobe containing the InProgress spinner — matching what the vertical tab shows for the same run.
- For a local CLI agent session (e.g. `claude` running in a plain terminal), the pane header shows the CLI agent's brand circle with the status badge, no cloud lobe.
- For plain-terminal / shell / error cases (no agent), the existing indicators (`render_terminal_mode_indicator` and its shell / error variants) remain unchanged. The new circle treatment only replaces the agent-specific indicators.
### Inline conversation list menu
- In the rows rendered by `conversation_list/item.rs::render_item`, the current leading-slot icon (a plain `Icon::Cloud` for ambient rows, or `render_status_element` for local rows) is replaced by the same agent icon-with-status treatment — brand circle, optional cloud lobe, status badge inside the lobe (ambient) or at the bottom-right (local).
- For cloud Task rows (i.e. rows whose underlying `ConversationOrTask::Task` is an `AmbientAgentTask`), the harness is resolved from `agent_config_snapshot.harness` and the cloud lobe is rendered.
- For local Conversation rows (`ConversationOrTask::Conversation`), the icon renders as a local Oz circle (no cloud lobe). Local conversations don't carry a harness field; they're treated as Oz for this surface.
- The status inside the badge/lobe comes from `ConversationOrTask::status(app)` — which already maps task states and conversation states to `ConversationStatus`.
- Row sizing is tuned to match the existing `status_element_size` footprint (`font_size + STATUS_ELEMENT_PADDING * 2.`) so adopting the unified icon doesn't shift row heights. Visual proportions follow the vertical tabs sizing.
- **Out of scope**: the Agent Management View (`AgentManagementView::render_card` / `render_header_row` in `app/src/ai/agent_management/view.rs`) retains its current leading status icon. That surface has its own richer card layout with action buttons, session status labels, etc., and is intentionally not migrated in this pass.
### Notifications mailbox
- Notifications triggered by a cloud-mode agent run surface the cloud lobe in their avatar; notifications from local runs continue to render without the lobe.
- Concretely, when `AgentNotificationsModel::add_notification` fires, the emit path resolves whether the originating terminal view was in an ambient session (via `TerminalView::ambient_agent_view_model`) and threads that flag into the notification, which in turn reaches `render_agent_avatar` instead of today's hardcoded `false`.
- The avatar's brand color, glyph, and status icon keep matching the existing notification categories (`Complete`, `Request`, `Error`); only the cloud lobe is newly honored.
### Cross-surface consistency guarantee
- Given any single logical agent run (a cloud Task, a local conversation, a local CLI session), when it is visible simultaneously on two or more of the surfaces above, the resulting icon is pixel-identical (same brand color, same glyph, same status, same cloud-lobe presence). Achieving this is the centralization objective of this spec, and is validated by a shared test suite.
## Edge cases
1. **Harness selected but run not yet dispatched (Composing state)**: the vertical tab and pane header immediately reflect the selected third-party harness. The card surface is not applicable (no task yet).
2. **Harness changed mid-composing**: icons update immediately on every surface that reflects the composing view-model.
3. **Viewer joins an existing shared session whose harness is not yet fetched**: briefly renders as Oz until `enter_viewing_existing_session` resolves the task; once resolved, the icon updates to the correct harness. Acceptable — not new behavior, just inherited from REMOTE-1454.
4. **Harness CLI detected by `CLIAgentSessionsModel` does not match the view-model's selected harness (rare)**: the `CLIAgentSessionsModel` session wins on surfaces that consult it directly (vertical tabs and pane header), because it represents observed reality. The conversation-list card still derives from `agent_config_snapshot.harness` since that's its only input.
5. **Plain terminal, no agent at all**: all four surfaces render whatever they render today — the new circle treatment does not replace plain-terminal indicators.
6. **Cloud notification vs. local notification for the same agent kind (e.g. Claude)**: they now visually differ — the cloud one has the white lobe, the local one has the bottom-right status badge. This is intentional and consistent with vertical tabs.
7. **Completed ambient run viewed as a historical conversation (no live `AmbientAgentViewModel`)**: the card renders based on the `AmbientAgentTask` fields. If the task's `agent_config_snapshot.harness` is present, we show that harness with the cloud lobe; otherwise we fall back to Oz. The pane header for a read-only replay uses whatever `ambient_agent_view_model.selected_harness()` resolved to when the viewer joined.
## Success criteria
- Selecting Claude (or Gemini) in the harness dropdown and submitting a cloud run immediately updates the tab icon to the Claude (or Gemini) brand circle with a cloud lobe containing the InProgress status — no Oz-cloud intermediate state.
- The pane header for the same run shows the same Claude circle + cloud lobe + status as the tab.
- The conversation list card for the same run shows the same Claude circle + cloud lobe + status.
- When the run transitions to Success / Error / Blocked / Cancelled, the status icon inside the cloud lobe updates on every surface at once.
- Starting a local `claude` CLI session (no cloud) shows a Claude brand circle with a bottom-right status badge on the vertical tab, pane header, and notifications (when a notification fires). No cloud lobe on any surface.
- The legacy non-agent indicators (plain terminal, shell, error) are visually unchanged.
- All the existing validation cases from REMOTE-1454 (non-oz cloud setup UX) continue to pass unchanged.
## Validation
- Spawn a Claude cloud run and observe the tab icon, pane header indicator, and conversation list card; confirm all three render as a Claude-orange circle with a cloud lobe containing the spinner during setup, and that they all transition together to the harness-running state when the `claude` CLI begins executing.
- Spawn a Gemini cloud run and confirm the same using the Gemini brand color.
- Spawn an Oz cloud run and confirm the three surfaces show Oz purple + cloud lobe + status, matching each other.
- Start a local `claude` CLI session in a plain terminal and confirm the tab, pane header, and any notification emitted (e.g. on a blocked permission request) show a Claude circle + bottom-right status badge with no cloud lobe.
- Cancel / fail / block a cloud Claude run; confirm the status icon inside the cloud lobe updates on every surface.
- Open the inline conversation list menu after running several agents (Oz cloud, Claude cloud, Gemini cloud, local Claude, local Oz) and confirm each row's leading icon correctly identifies the agent and ambient-ness.
- Let a cloud-mode Claude run complete while the tab is not focused; open the notifications mailbox and confirm the notification's avatar is a Claude circle with a cloud lobe.
- Toggle `CloudMode` off; confirm no regression to existing non-cloud agent icons (local Oz, local CLI agents).
- Toggle `AgentHarness` off; confirm all cloud runs fall back to the Oz treatment and no Claude/Gemini icons appear on any surface.
