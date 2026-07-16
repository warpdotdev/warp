//! Background monitor that scans the running harness block for known runtime
//! failure substrings (e.g. invalid API key, exhausted credits) and reports
//! the first hit so the driver can fail the task fast instead of letting the
//! harness hang.
use std::sync::Arc;
use std::time::Duration;

use regex::escape;
use warpui::ModelSpawner;

use super::terminal::BlockOutputMatch;
use super::AgentDriver;
use crate::terminal::cli_agent_sessions::CLIAgentSessionStatus;
use crate::terminal::model::block::BlockId;
use crate::terminal::model::find::RegexDFAs;

const SCAN_INTERVALS: &[Duration] = &[
    Duration::from_secs(5),
    Duration::from_secs(5),
    Duration::from_secs(5),
    Duration::from_secs(5),
    Duration::from_secs(5),
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(15),
    Duration::from_secs(15),
    Duration::from_secs(15),
];

const STALL_POLL_INTERVAL: Duration = Duration::from_secs(10);

const STALL_CONFIRMATION_BUDGET: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub(crate) struct DetectedHarnessError {
    pub pattern: String,
    pub excerpt: String,
}

/// Build a combined case-insensitive DFA from the harness's static patterns.
pub(crate) fn build_dfas(patterns: &[&'static str]) -> Option<RegexDFAs> {
    if patterns.is_empty() {
        return None;
    }
    let escaped: Vec<String> = patterns.iter().map(|p| escape(p)).collect();
    let refs: Vec<&str> = escaped.iter().map(String::as_str).collect();
    match RegexDFAs::new_many(
        &refs, false, // enable_unicode_word_boundary
        false, // case_sensitive
    ) {
        Ok(dfas) => Some(dfas),
        Err(err) => {
            log::warn!("Failed to build harness output DFAs: {err}");
            None
        }
    }
}

/// Map a `matched_text` produced by the combined DFA back to the originating
/// needle. The DFA matched `(p1|p2|…)`, so the matched substring is exactly
/// one of `patterns` up to case — we lowercase-compare to identify it.
pub(crate) fn pattern_for_match(
    matched_text: &str,
    patterns: &[&'static str],
) -> Option<&'static str> {
    let matched_lower = matched_text.to_lowercase();
    patterns
        .iter()
        .copied()
        .find(|p| p.to_lowercase() == matched_lower)
}

fn outputs_stalled(before: Option<&str>, after: Option<&str>) -> bool {
    matches!((before, after), (Some(a), Some(b)) if a == b)
}

async fn find_match_once(
    block_id: &BlockId,
    dfas: &Arc<RegexDFAs>,
    foreground: &ModelSpawner<AgentDriver>,
) -> Option<BlockOutputMatch> {
    let block_id_for_tick = block_id.clone();
    let dfas_for_tick = Arc::clone(dfas);
    foreground
        .spawn(move |me, ctx| {
            me.terminal_driver
                .as_ref(ctx)
                .find_first_match_in_block_output(&block_id_for_tick, &dfas_for_tick, ctx)
        })
        .await
        .ok()
        .flatten()
}

async fn fetch_plaintext(
    block_id: &BlockId,
    foreground: &ModelSpawner<AgentDriver>,
) -> Option<String> {
    let block_id_for_tick = block_id.clone();
    foreground
        .spawn(move |me, ctx| {
            me.terminal_driver
                .as_ref(ctx)
                .block_output_plaintext(&block_id_for_tick, ctx)
        })
        .await
        .ok()
        .flatten()
}

/// Run the stall-confirmation loop after a pattern hit.
async fn confirm_stall(
    block_id: &BlockId,
    dfas: &Arc<RegexDFAs>,
    foreground: &ModelSpawner<AgentDriver>,
) -> (Option<BlockOutputMatch>, Duration) {
    let mut previous = fetch_plaintext(block_id, foreground).await;
    let mut elapsed = Duration::ZERO;
    while elapsed < STALL_CONFIRMATION_BUDGET {
        warpui::r#async::Timer::after(STALL_POLL_INTERVAL).await;
        elapsed += STALL_POLL_INTERVAL;
        let current = fetch_plaintext(block_id, foreground).await;
        if outputs_stalled(previous.as_deref(), current.as_deref()) {
            // Output settled. Re-run the DFA so we report the post-dwell
            // match in case the row positions shifted while we waited.
            return (find_match_once(block_id, dfas, foreground).await, elapsed);
        }
        previous = current;
    }
    (None, elapsed)
}

/// Watch the given block for harness output errors on the
/// [`SCAN_INTERVALS`] cadence.
///
/// On every pattern hit, run a stall-confirmation loop (up to
/// [`STALL_CONFIRMATION_BUDGET`]) and only resolve with
/// `Some(DetectedHarnessError)` when the harness output stabilizes with
/// the pattern still present. If the harness keeps producing output
/// (e.g. spinner frames during an automatic retry), the detection is
/// dropped and the scanner resumes normal polling.
///
/// Returns `None` when the schedule completes without a confirmed hit
/// (or when `patterns` is empty / DFA construction fails).
/// Cancellation-safe: dropping the future stops both loops.
pub(crate) async fn watch_block_for_errors(
    block_id: BlockId,
    patterns: &'static [&'static str],
    foreground: &ModelSpawner<AgentDriver>,
) -> Option<DetectedHarnessError> {
    if patterns.is_empty() {
        return None;
    }
    let dfas = Arc::new(build_dfas(patterns)?);

    // Total observation budget = sum of all scan intervals. Used as an
    // early-exit guard after stall confirmation in case a flaky harness
    // has burned most of the window into confirmation loops.
    let total_budget: Duration = SCAN_INTERVALS.iter().copied().sum();
    let mut elapsed = Duration::ZERO;

    for &interval in SCAN_INTERVALS {
        warpui::r#async::Timer::after(interval).await;
        elapsed += interval;

        if find_match_once(&block_id, &dfas, foreground)
            .await
            .is_none()
        {
            continue;
        }

        // Candidate match observed. Confirm via the stall loop so we
        // don't false-positive while the harness is mid-retry.
        let (confirmed, confirmation_elapsed) = confirm_stall(&block_id, &dfas, foreground).await;
        elapsed += confirmation_elapsed;

        let Some(hit) = confirmed else {
            if elapsed >= total_budget {
                break;
            }
            continue;
        };

        // The DFA matched one of our needles, so `pattern_for_match`
        // should always resolve. Fall back to the matched substring
        // verbatim if it somehow doesn't.
        let pattern = pattern_for_match(&hit.matched_text, patterns)
            .map(str::to_owned)
            .unwrap_or_else(|| hit.matched_text.clone());
        return Some(DetectedHarnessError {
            pattern,
            excerpt: cap_excerpt(&hit.excerpt),
        });
    }
    None
}

pub(crate) fn should_suppress_runtime_failure(status: Option<&CLIAgentSessionStatus>) -> bool {
    // On CLIAgentSessionStatus::Failed, we directly update the task status,
    // so we don't need to do runtime pattern matching to find the failure.
    matches!(
        status,
        Some(CLIAgentSessionStatus::Success) | Some(CLIAgentSessionStatus::Failed { .. })
    )
}

/// Cap excerpt length so we don't blow up status messages or logs with
/// terminal-width rows.
const EXCERPT_MAX_LEN: usize = 240;
fn cap_excerpt(excerpt: &str) -> String {
    if excerpt.chars().count() <= EXCERPT_MAX_LEN {
        return excerpt.to_owned();
    }
    let mut out: String = excerpt.chars().take(EXCERPT_MAX_LEN).collect();
    out.push('…');
    out
}

#[cfg(test)]
#[path = "harness_output_monitor_tests.rs"]
mod tests;
