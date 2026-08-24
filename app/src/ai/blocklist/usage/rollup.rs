//! Aggregates credit usage across an orchestrator and its locally-loaded
//! descendants for the agent-mode footer rollup feature (QUALITY-671).
//!
//! Pure function — no I/O, no GraphQL. Walks
//! [`BlocklistAIHistoryModel`] using the shared
//! [`descendant_conversation_ids_in_spawn_order`] helper, sums each loaded
//! conversation's `credits_spent`, and emits a per-agent breakdown for the
//! footer's "View details" list.

use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::orchestration_topology::descendant_conversation_ids_in_spawn_order;

/// Avatar identity for a row in the per-agent breakdown.
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

/// Aggregated credit usage for an orchestrator and its locally-loaded
/// descendants.
#[derive(Debug, Clone, PartialEq)]
pub struct OrchestrationCreditRollup {
    /// Sum of `credits_spent` across the orchestrator and every
    /// locally-loaded descendant.
    pub total_credits: f32,
    /// Sum of `usage_totals().total_cost_in_cents()` across the orchestrator
    /// and every locally-loaded descendant that has spent > 0 credits,
    /// mirroring `total_credits`. `None` when any such contributing
    /// conversation lacks a known dollar-cost baseline — a partial sum
    /// would misrepresent the true total, so the rollup omits the dollar
    /// figure entirely rather than showing an incomplete one. A
    /// zero-credit contributor (e.g. a freshly spawned child that hasn't
    /// reported usage yet) is skipped rather than treated as an unknown
    /// baseline, since it hasn't contributed anything to sum.
    pub total_cost_in_cents: Option<f32>,
    /// Sum of `usage_totals().charged_usage`'s total token count across the
    /// orchestrator and every locally-loaded descendant that has spent > 0
    /// credits. `None` under the same conditions as `total_cost_in_cents`.
    pub total_tokens: Option<u32>,
    /// One entry per agent that has spent > 0 credits, sorted by
    /// `credits_spent` descending. Ties are broken by spawn order (earlier
    /// spawn first; orchestrator always sorts before its descendants in a
    /// tie).
    pub per_agent: Vec<PerAgentCreditEntry>,
}

/// Folds one more conversation's optional dollar cost into a running total,
/// propagating `None` permanently once any contributor lacks a known
/// baseline (see `OrchestrationCreditRollup::total_cost_in_cents`).
fn accumulate_cost(total: &mut Option<f32>, cost: Option<f32>) {
    *total = match (*total, cost) {
        (Some(t), Some(c)) => Some(t + c),
        _ => None,
    };
}

/// Folds one more conversation's optional token count into a running total,
/// propagating `None` permanently once any contributor lacks a known count
/// (see `OrchestrationCreditRollup::total_tokens`).
fn accumulate_tokens(total: &mut Option<u32>, tokens: Option<u32>) {
    *total = match (*total, tokens) {
        (Some(t), Some(c)) => Some(t + c),
        _ => None,
    };
}

/// Computes the orchestration credit rollup for `parent_id`.
///
/// Returns `None` when:
/// * the orchestrator has no locally-loaded descendants, OR
/// * the orchestrator and every loaded descendant have spent zero credits.
///
/// Unloaded descendants (IDs in the topology index without a matching
/// `AIConversation` in `conversations_by_id`) are silently skipped — see
/// PRODUCT.md invariant 10.
pub fn compute_orchestration_rollup(
    parent_id: AIConversationId,
    history: &BlocklistAIHistoryModel,
) -> Option<OrchestrationCreditRollup> {
    // Descendants in spawn order so ties break naturally. The orchestrator
    // is prepended at index 0 so it sorts before its descendants at equal
    // credit totals.
    let descendant_ids = descendant_conversation_ids_in_spawn_order(history, parent_id);
    if descendant_ids.is_empty() {
        return None;
    }

    let mut total_credits: f32 = 0.0;
    let mut total_cost_in_cents: Option<f32> = Some(0.0);
    let mut total_tokens: Option<u32> = Some(0);
    let mut entries: Vec<(usize, PerAgentCreditEntry)> = Vec::new();

    if let Some(orchestrator) = history.conversation(&parent_id) {
        let credits = orchestrator.credits_spent();
        total_credits += credits;
        // Skip cost/token accumulation for a zero-credit contributor: it
        // hasn't reported any usage yet, so its `None` charge metadata
        // must not poison the running total for contributors that have.
        if credits > 0.0 {
            let orchestrator_usage_totals = orchestrator.usage_totals();
            accumulate_cost(
                &mut total_cost_in_cents,
                orchestrator_usage_totals.total_cost_in_cents(),
            );
            accumulate_tokens(
                &mut total_tokens,
                orchestrator_usage_totals
                    .charged_usage
                    .map(|usage| usage.total_tokens()),
            );
            entries.push((
                0,
                PerAgentCreditEntry {
                    conversation_id: parent_id,
                    display_name: orchestrator_display_name(orchestrator),
                    avatar: AgentAvatar::Orchestrator,
                    credits_spent: credits,
                },
            ));
        }
    }

    for (spawn_idx, descendant_id) in descendant_ids.iter().enumerate() {
        let Some(descendant) = history.conversation(descendant_id) else {
            // PRODUCT invariant 10: silently skip unloaded descendants.
            continue;
        };
        let credits = descendant.credits_spent();
        total_credits += credits;
        // See the matching comment in the orchestrator branch above.
        if credits > 0.0 {
            let descendant_usage_totals = descendant.usage_totals();
            accumulate_cost(
                &mut total_cost_in_cents,
                descendant_usage_totals.total_cost_in_cents(),
            );
            accumulate_tokens(
                &mut total_tokens,
                descendant_usage_totals
                    .charged_usage
                    .map(|usage| usage.total_tokens()),
            );
            entries.push((
                spawn_idx + 1,
                PerAgentCreditEntry {
                    conversation_id: *descendant_id,
                    display_name: child_display_name(descendant),
                    avatar: AgentAvatar::Child,
                    credits_spent: credits,
                },
            ));
        }
    }

    if entries.is_empty() {
        return None;
    }

    // Sort by credits descending; ties broken by spawn order ascending so
    // the earlier-spawned agent appears first.
    entries.sort_by(|a, b| {
        b.1.credits_spent
            .partial_cmp(&a.1.credits_spent)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });

    Some(OrchestrationCreditRollup {
        total_credits,
        total_cost_in_cents,
        total_tokens,
        per_agent: entries.into_iter().map(|(_, entry)| entry).collect(),
    })
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
