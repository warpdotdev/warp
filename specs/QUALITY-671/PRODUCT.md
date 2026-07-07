# Enhance credit usage details in the orchestrator tab

Linear: https://linear.app/warpdotdev/issue/QUALITY-671

## Summary
The expanded credit usage footer in agent mode is per-conversation. When the conversation is an orchestrator, the footer hides the real cost of the work it dispatched — credits incurred by child agents are never surfaced on the parent. This feature changes the orchestrator's "Credits spent (total)" row to reflect the *orchestration total* (orchestrator + all locally-known descendants) and adds a click-to-expand per-agent breakdown beneath it.

## Figma
- Frame (overview, collapsed + expanded side-by-side): https://www.figma.com/design/AsF5uAM6L5tUmc11vm9YSi/Agent-orchestration?node-id=4646-33383
- Expanded "Hide details" state: https://www.figma.com/design/AsF5uAM6L5tUmc11vm9YSi/Agent-orchestration?node-id=4636-32699

## Goals
- Show the true end-to-end credit cost of an orchestration run in the parent's expanded usage footer at a glance.
- Let the user drill into per-agent credit attribution without leaving the footer.
- Reuse existing agent identity (avatar, display name) so the breakdown reads consistently with the orchestration pill bar.

## Non-goals
- Server-side billing or pricing changes. The feature is purely a presentation of usage data the server already returns per conversation.
- Rolling up any metric other than credits in v1. Tool calls, files changed, lines +/-, commands, models, context window, and last-response timing all stay self-only. Rollup of other metrics is a possible follow-up.
- Rolling up usage from descendants whose conversation state is not loaded locally (e.g. a remote child running on a worker the user has never opened on this client).

## Behavior

1. On any conversation that has no descendant child agents loaded locally, or whose loaded descendants have all spent zero credits, the expanded credit usage footer renders exactly as it does today. No new UI is added.

2. When the conversation rendering the footer is an orchestrator with at least one locally-loaded descendant that has spent credits, the "USAGE SUMMARY" section's "Credits spent (total)" row is modified as follows:
   a. The numeric value (e.g. "33 credits") becomes the orchestration total — the sum of credits spent by the orchestrator plus all of its locally-known descendants (children, grandchildren, etc., transitively).
   b. A "View details" link with a chevron-down icon is rendered immediately to the right of the value, on the same row.
   c. Clicking "View details" replaces the link with "Hide details" and a chevron-up icon, and reveals a per-agent breakdown list directly below the row (see invariant 5).
   d. Clicking "Hide details" collapses the per-agent list and restores "View details".

3. The "Credits spent (last response)" row is unchanged. It always reflects only the orchestrator's own most-recent-block credits, never a rollup.

4. All other rows in the expanded footer ("Tool calls", "Models", "Context window used" in USAGE SUMMARY, plus the entire TOOL CALL SUMMARY and LAST RESPONSE TIME sections) continue to reflect only the orchestrator's own values. They are not rolled up in v1. Credits-only rollup is the locked v1 scope; broader rollup is a possible follow-up.

5. The per-agent breakdown list, when "View details" is active:
   a. Contains one row per agent contributing to the rollup. The orchestrator is listed alongside its descendants — there is no separate "self" row.
   b. Rows are sorted by credits spent, descending. Ties are broken by spawn order (earlier spawn first).
   c. Each row displays: the agent's avatar disc (orchestrator uses the orchestrator avatar; children use the existing per-name color + initial avatar from the orchestration pill bar), the agent's display name (e.g. "Orchestrator", "DesignBot"), and the credit value formatted by `format_credits`.
   d. Only agents that have spent > 0 credits are included. Just-spawned or idle agents are omitted (they pop in as soon as they consume credits).
   e. When ≤ 5 rows are eligible, all rows are shown.
   f. When > 5 rows are eligible, the first 5 are shown followed by a "Show N more" link where N = total_eligible − 5. Clicking the link reveals all remaining rows and removes the link from the list (the link does not become "Show fewer").
   g. The per-agent list does not have its own toggles, sorting, or hover affordances beyond the row content. No row is clickable in v1.

6. Local UI state that resets when the footer is collapsed (chevron at the top of the footer) and reopened:
   a. The "View details" toggle resets to its default closed state. The default for the freshly-opened footer is "View details" (per-agent list hidden).
   b. The "Show N more" expansion (when applicable) resets — the list is again truncated to the first 5 rows with the "Show N more" link.

7. While the footer is open, the rollup total, the per-agent list contents (rows, ordering, values), and the "Show N more" count update live as child agents stream new tokens or finish responses. No user action is needed.

8. When a new descendant child first spends a credit while the user is looking at the expanded footer, its row appears in the per-agent list at the position dictated by its credit value (descending sort).

9. When a descendant child is removed/pruned from the local client, its row disappears on the next render, and the rollup total decreases accordingly.

10. Descendants whose conversation state is not loaded locally do not contribute to the rollup and do not appear in the per-agent list. The rollup is a best-effort sum across locally-known agents. In practice this gap is small (server-side usage updates stream to the client), so the v1 surface does not warn the user that some agents may be missing. If real-world discrepancies prove confusing, a server-side rollup query is a follow-up.

11. The collapsed footer pill (the small button with the credit number + chevron) shows the orchestration total when a rollup applies (per invariant 2). When the rollup does not apply (no eligible descendants), the pill shows the orchestrator's own credit number exactly as it does today.
   a. The "+N" delta annotation on the pill (current behavior: show the most-recent-response credit count when total ≠ last response) continues to use the orchestrator's own most-recent-block credits. With the rollup active, this delta represents "the credits the orchestrator's last response added to the orchestration total" — still meaningful at a glance.
   b. The existing "hide the button entirely when there's no usage data" rule is evaluated against the rollup total when applicable, so the pill appears as soon as any contributing agent has spent a credit (not only when the orchestrator itself has).

12. The "View details" / "Hide details" link and the per-agent list visually match the existing footer's typography, spacing, and color treatment. Avatar discs use the same component used in the orchestration pill bar's pill avatars (per-name deterministic color + uppercase initial; orchestrator uses `Icon::Oz` on `ansi_fg_cyan`).

13. The feature is self-gating: there is no dedicated feature flag. The rollup activates whenever the orchestrator has at least one locally-loaded descendant with non-zero credits; otherwise the row renders exactly as today. The underlying ability to create child agents is gated by `FeatureFlag::OrchestrationV2`, and the expanded footer surface itself is gated by `FeatureFlag::AgentView`, so the rollup is effectively reachable only when both are on — no additional flag is needed.

14. The footer remains keyboard accessible. The "View details" link is reachable via the normal focus order and activatable with Enter/Space. The "Show N more" link is reachable and activatable the same way. Screen reader semantics for the per-agent list mirror existing list semantics in the footer (no new ARIA invention).

15. The rollup is read-only. No billing, telemetry, or persistence changes — it is a view on data already in `AIConversation.conversation_usage_metadata` for the orchestrator and its locally-known descendants.

16. Forked conversations: a fork descended from the orchestrator is treated as a regular descendant. Its post-fork usage contributes to the rollup if its metadata is loaded; otherwise it is ignored like any other unloaded descendant.

17. Settings-mode usage view (the per-conversation history surface, not the agent-mode footer) is unchanged. The rollup applies only to `DisplayMode::Footer`.
