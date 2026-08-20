//! Aggregates usage across an orchestrator and its locally-loaded
//! descendants for the agent-mode footer rollup feature (QUALITY-671,
//! QUALITY-1703).
//!
//! Pure function — no I/O, no GraphQL. Walks
//! [`BlocklistAIHistoryModel`] using the shared
//! [`descendant_conversation_ids_in_spawn_order`] helper, sums each loaded
//! conversation's credits and tool-call stats, and emits per-agent
//! breakdowns for the footer's "View details" lists.

use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::orchestration_topology::descendant_conversation_ids_in_spawn_order;

/// Avatar identity for a row in a per-agent breakdown.
///
/// The actual rendering still requires a theme (which the rollup, being a
/// pure function, cannot consult), so this enum only carries the structural
/// information needed to choose a renderer at render time. The child variant
/// reuses the orchestration pill bar's deterministic per-name color +
/// uppercase initial via the existing avatar helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentAvatar {
    /// The orchestrator itself. Rendered with the Oz glyph on `ansi_fg_cyan`.
    Orchestrator,
    /// A descendant agent. Rendered with the same deterministic-color +
    /// initial-letter treatment as the orchestration pill bar.
    Child,
}

/// One row in the per-agent credit breakdown list.
#[derive(Debug, Clone, PartialEq)]
pub struct PerAgentCreditEntry {
    pub conversation_id: AIConversationId,
    pub display_name: String,
    pub avatar: AgentAvatar,
    pub credits_spent: f32,
}

/// One row in the per-agent diffs-applied breakdown list.
#[derive(Debug, Clone, PartialEq)]
pub struct PerAgentDiffEntry {
    pub conversation_id: AIConversationId,
    pub display_name: String,
    pub avatar: AgentAvatar,
    pub lines_added: i32,
    pub lines_removed: i32,
}

/// Aggregated usage for an orchestrator and its locally-loaded descendants.
#[derive(Debug, Clone, PartialEq)]
pub struct OrchestrationUsageRollup {
    /// Sum of `credits_spent` across the orchestrator and every
    /// locally-loaded descendant.
    pub total_credits: f32,
    /// One entry per agent that has spent > 0 credits, sorted by
    /// `credits_spent` descending. Ties are broken by spawn order (earlier
    /// spawn first; orchestrator always sorts before its descendants in a
    /// tie).
    pub credits_per_agent: Vec<PerAgentCreditEntry>,
    /// Sum of files changed (via `apply_file_diff_stats`) across the
    /// orchestrator and every locally-loaded descendant.
    pub total_files_changed: i32,
    /// Sum of lines added across the orchestrator and every locally-loaded
    /// descendant.
    pub total_lines_added: i32,
    /// Sum of lines removed across the orchestrator and every
    /// locally-loaded descendant.
    pub total_lines_removed: i32,
    /// One entry per agent that has applied a non-empty diff (lines added
    /// or removed > 0), sorted by total lines changed (added + removed)
    /// descending. Ties are broken by spawn order (earlier spawn first).
    pub diffs_per_agent: Vec<PerAgentDiffEntry>,
    /// Sum of commands executed across the orchestrator and every
    /// locally-loaded descendant.
    pub total_commands_executed: i32,
}

/// Accumulates rollup totals and per-agent entries while walking the
/// orchestrator + descendant conversations. Kept as a single struct (rather
/// than a handful of loose `&mut` parameters) so `accumulate` stays under
/// the usual argument-count lint while still updating every total and
/// breakdown list in one pass per conversation.
#[derive(Default)]
struct RollupAccumulator {
    total_credits: f32,
    total_files_changed: i32,
    total_lines_added: i32,
    total_lines_removed: i32,
    total_commands_executed: i32,
    credit_entries: Vec<(usize, PerAgentCreditEntry)>,
    diff_entries: Vec<(usize, PerAgentDiffEntry)>,
}

impl RollupAccumulator {
    /// Folds one conversation's credits and tool-usage stats into the
    /// running totals, pushing a per-agent entry into each breakdown list
    /// the conversation actually contributes to.
    fn accumulate(
        &mut self,
        spawn_idx: usize,
        conversation_id: AIConversationId,
        avatar: AgentAvatar,
        display_name: String,
        conversation: &AIConversation,
    ) {
        let credits = conversation.credits_spent();
        self.total_credits += credits;
        if credits > 0.0 {
            self.credit_entries.push((
                spawn_idx,
                PerAgentCreditEntry {
                    conversation_id,
                    display_name: display_name.clone(),
                    avatar: avatar.clone(),
                    credits_spent: credits,
                },
            ));
        }

        let tool_usage = conversation.tool_usage_metadata();
        let diff_stats = &tool_usage.apply_file_diff_stats;
        self.total_files_changed += diff_stats.files_changed;
        self.total_lines_added += diff_stats.lines_added;
        self.total_lines_removed += diff_stats.lines_removed;
        self.total_commands_executed += tool_usage.run_command_stats.commands_executed;

        if diff_stats.lines_added > 0 || diff_stats.lines_removed > 0 {
            self.diff_entries.push((
                spawn_idx,
                PerAgentDiffEntry {
                    conversation_id,
                    display_name,
                    avatar,
                    lines_added: diff_stats.lines_added,
                    lines_removed: diff_stats.lines_removed,
                },
            ));
        }
    }

    /// Finalizes the accumulated totals into a rollup, sorting each
    /// breakdown list independently. Returns `None` when nothing eligible
    /// was rolled up (every metric is zero across the orchestrator and its
    /// loaded descendants) — see PRODUCT.md invariant 10 (QUALITY-671).
    fn into_rollup(mut self) -> Option<OrchestrationUsageRollup> {
        let has_any_rollup_data = self.total_credits > 0.0
            || self.total_files_changed > 0
            || self.total_lines_added > 0
            || self.total_lines_removed > 0
            || self.total_commands_executed > 0;
        if !has_any_rollup_data {
            return None;
        }

        // Sort by credits descending; ties broken by spawn order ascending
        // so the earlier-spawned agent appears first.
        self.credit_entries.sort_by(|a, b| {
            b.1.credits_spent
                .partial_cmp(&a.1.credits_spent)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        // Sort by total lines changed descending, same tie-break as credits.
        self.diff_entries.sort_by(|a, b| {
            let a_lines = a.1.lines_added + a.1.lines_removed;
            let b_lines = b.1.lines_added + b.1.lines_removed;
            b_lines.cmp(&a_lines).then(a.0.cmp(&b.0))
        });

        Some(OrchestrationUsageRollup {
            total_credits: self.total_credits,
            credits_per_agent: self.credit_entries.into_iter().map(|(_, e)| e).collect(),
            total_files_changed: self.total_files_changed,
            total_lines_added: self.total_lines_added,
            total_lines_removed: self.total_lines_removed,
            diffs_per_agent: self.diff_entries.into_iter().map(|(_, e)| e).collect(),
            total_commands_executed: self.total_commands_executed,
        })
    }
}

/// Computes the orchestration usage rollup for `parent_id`.
///
/// Returns `None` when:
/// * the orchestrator has no locally-loaded descendants, OR
/// * the orchestrator and every loaded descendant have zero credits, zero
///   files changed, zero lines added/removed, and zero commands executed.
///
/// Unloaded descendants (IDs in the topology index without a matching
/// `AIConversation` in `conversations_by_id`) are silently skipped — see
/// PRODUCT.md invariant 10.
///
/// Each metric's per-agent breakdown (`credits_per_agent`,
/// `diffs_per_agent`) only ever includes agents that contributed to *that*
/// metric, independent of whether the rollup as a whole applies for a
/// different reason. This keeps each metric's own gating (e.g. whether its
/// "View details" toggle renders) unaffected by the others.
pub fn compute_orchestration_rollup(
    parent_id: AIConversationId,
    history: &BlocklistAIHistoryModel,
) -> Option<OrchestrationUsageRollup> {
    // Descendants in spawn order so ties break naturally. The orchestrator
    // is accumulated at index 0 so it sorts before its descendants at equal
    // totals.
    let descendant_ids = descendant_conversation_ids_in_spawn_order(history, parent_id);
    if descendant_ids.is_empty() {
        return None;
    }

    let mut accumulator = RollupAccumulator::default();

    if let Some(orchestrator) = history.conversation(&parent_id) {
        accumulator.accumulate(
            0,
            parent_id,
            AgentAvatar::Orchestrator,
            orchestrator_display_name(orchestrator),
            orchestrator,
        );
    }

    for (spawn_idx, descendant_id) in descendant_ids.iter().enumerate() {
        let Some(descendant) = history.conversation(descendant_id) else {
            // PRODUCT invariant 10: silently skip unloaded descendants.
            continue;
        };
        accumulator.accumulate(
            spawn_idx + 1,
            *descendant_id,
            AgentAvatar::Child,
            child_display_name(descendant),
            descendant,
        );
    }

    accumulator.into_rollup()
}

/// Display name for the orchestrator row. Prefers the explicitly assigned
/// `agent_name`, falls back to "Orchestrator" so the row is always
/// meaningful.
fn orchestrator_display_name(orchestrator: &AIConversation) -> String {
    orchestrator
        .agent_name()
        .filter(|n| !n.is_empty())
        .map(|n| n.to_string())
        .unwrap_or_else(|| "Orchestrator".to_string())
}

/// Display name for a child row. Mirrors the orchestration pill bar's
/// fallback (`"Agent"`) so the breakdown stays consistent with the pill
/// labels when an agent hasn't been named yet.
fn child_display_name(child: &AIConversation) -> String {
    child
        .agent_name()
        .filter(|n| !n.is_empty())
        .map(|n| n.to_string())
        .unwrap_or_else(|| "Agent".to_string())
}

#[cfg(test)]
#[path = "rollup_tests.rs"]
mod tests;
