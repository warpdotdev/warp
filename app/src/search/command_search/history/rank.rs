use chrono::{DateTime, Local};
use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;

use crate::terminal::HistoryEntry;
use crate::terminal::model::session::SessionId;

// ----- Tunable constants -----
//
// Everything below shapes `rank()`'s formula: `score = adjusted_skim * f(priors)`, where
// `adjusted_skim` is the match-quality component (raw Skim score plus the corrections in
// `adjusted_skim()`) and `f(priors)` is a bounded multiplier built from recency, frequency,
// session, cwd, and exit status. Change a value, rebuild, and re-run
// `cargo test -p warp --lib search::command_search::history` to see the effect against the
// golden fixtures in `rank_tests.rs`.

/// Bottom of `f(priors)`'s range. Raise to score history higher relative to every other Command
/// Search source's own (unscaled) score, across the board; lower to score it lower.
const PRIOR_MULTIPLIER_BASELINE: f64 = 0.8;

/// Width added on top of [`PRIOR_MULTIPLIER_BASELINE`], so `f(priors)` ranges over `[0.8, 1.2]`.
/// Raise to let priors reorder history results more aggressively relative to raw match quality;
/// lower to let raw match quality dominate more. Too wide and a fresh weak match can outrank an
/// older strong one (see `rank_tests.rs`'s `older_exact_match_outranks_fresher_weak_match` and
/// `recency_breaks_ties_among_equal_quality_substring_matches`, which pin down the current width).
const PRIOR_MULTIPLIER_SWING: f64 = 0.4;

/// How much of `f(priors)` is driven by recency vs. the other priors below. Raise to make
/// freshness matter more.
const RECENCY_WEIGHT: f64 = 0.30;

/// How much of `f(priors)` is driven by how often a command has been run. Raise to make
/// frequently-used commands rank higher.
const FREQUENCY_WEIGHT: f64 = 0.08;

/// How much of `f(priors)` is driven by whether a command ran in the current session. Raise to
/// favor the current session's own history more.
const SESSION_WEIGHT: f64 = 0.05;

/// How much of `f(priors)` is driven by whether a command ran in the current working directory.
/// Raise to favor commands run from here more.
const CWD_WEIGHT: f64 = 0.02;

/// How much `f(priors)` is reduced for a command whose last run failed. Raise to penalize failed
/// commands more.
const EXIT_PENALTY_WEIGHT: f64 = 0.03;

/// Days for the recency term to decay by half. Lower makes recent commands matter more (and older
/// ones fade faster); raise for a longer memory.
const RECENCY_HALF_LIFE_DAYS: f64 = 3.0;

/// Execution count at which the frequency term saturates (maxes out). Lower means fewer repeats
/// are needed to count as "frequent"; raise to require more repeats.
const FREQUENCY_SATURATION_COUNT: f64 = 20.0;

/// Minimum adjusted-Skim score, per character of the query, for a match to be shown at all. Raise
/// to filter out more loose/scattered matches; lower to show more borderline ones. Legitimate
/// matches score in the high teens to twenties per character (see `rank_tests.rs`).
const RAW_SKIM_FLOOR_PER_CHAR: f64 = 8.0;

/// Per-character bonus for a run of contiguously-matched characters, folded into `adjusted_skim`.
/// Raise to favor tight, contiguous matches over scattered ones more strongly (fixes issue #1810,
/// where Skim's word-boundary bonus made a scattered match outscore a contiguous one).
const CONSECUTIVE_BONUS_PER_CHAR: f64 = 4.0;

/// Bonus added once to `adjusted_skim` when the query exactly matches the whole command. Raise to
/// make an exact match harder to displace by a fresher partial match; needed because SkimMatcherV2
/// scores a query identically whether it's the whole command or just a prefix of a longer one.
const EXACT_WHOLE_LINE_BONUS: f64 = 12.0;

/// Synthetic "days per list position" used to derive an age for entries with no timestamp, so a
/// commonly-typed command near the tail of an untracked history file still reads as recent
/// instead of decaying to zero relevance. Reuses [`RECENCY_HALF_LIFE_DAYS`] as the decay rate.
const FALLBACK_AGE_DAYS_PER_POSITION: f64 = 1.0;

/// Derived, not meant to be hand-tuned: theoretical lower bound of the weighted prior sum
/// (`RECENCY_WEIGHT * recency + ... - EXIT_PENALTY_WEIGHT * exit_pen`), reached when every
/// positive prior is absent and the command's last run failed. Used to rescale that sum into
/// `[0, 1]` before it becomes `f(priors)`'s swing; recomputes automatically if the weights above
/// change.
const PRIOR_SUM_MIN: f64 = -EXIT_PENALTY_WEIGHT;

/// Derived, not meant to be hand-tuned: theoretical upper bound of the weighted prior sum, reached
/// when every positive prior is fully satisfied and the command's last run succeeded.
const PRIOR_SUM_MAX: f64 = RECENCY_WEIGHT + FREQUENCY_WEIGHT + SESSION_WEIGHT + CWD_WEIGHT;

// ----- End tunable constants -----

/// The Skim-scale quality of a fuzzy match, before history priors are applied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MatchQuality {
    /// Sum of every AND-ed token's raw Skim score (`fzf`-style term summation for a multi-word
    /// query) plus the [`CONSECUTIVE_BONUS_PER_CHAR`] correction, on the same raw scale every
    /// other Command Search source's own Skim-based score lives on. [`rank`] multiplies this by
    /// the prior multiplier to get the final score, so history's cross-source position is set by
    /// this value, not by priors.
    adjusted_skim: f64,
    /// `adjusted_skim` normalized by the query's character count. Used only to gate out junk
    /// matches via [`RAW_SKIM_FLOOR_PER_CHAR`]; the final score uses `adjusted_skim` directly so
    /// query length doesn't otherwise affect history's scale relative to other sources.
    adjusted_skim_per_char: f64,
}

/// Splits `query` on whitespace for fzf-style space-AND matching. An empty (or all-whitespace)
/// query yields a single empty token, preserving the existing zero-state behavior of matching
/// every candidate.
pub(crate) fn tokenize_query(query: &str) -> Vec<&str> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        vec![trimmed]
    } else {
        trimmed.split_whitespace().collect()
    }
}

/// Matches every term in `tokens` against `command` as an independent fuzzy subsequence and ANDs
/// the results together, returning `None` if any term fails to match anywhere in `command`.
///
/// On a match, returns a [`FuzzyMatchResult`] whose `matched_indices` is the union of every
/// term's matched indices (for highlighting) alongside the resulting [`MatchQuality`].
pub(crate) fn match_history_command(
    command: &str,
    tokens: &[&str],
) -> Option<(FuzzyMatchResult, MatchQuality)> {
    let mut token_matches = Vec::with_capacity(tokens.len());
    for token in tokens {
        token_matches.push(fuzzy_match::match_indices_case_insensitive(command, token)?);
    }

    let mut merged_indices: Vec<usize> = token_matches
        .iter()
        .flat_map(|token_match| token_match.matched_indices.iter().copied())
        .collect();
    merged_indices.sort_unstable();
    merged_indices.dedup();

    let query_char_count: usize = tokens.iter().map(|token| token.chars().count()).sum();
    let raw_score_total: i64 = token_matches
        .iter()
        .map(|token_match| token_match.score)
        .sum();
    let adjusted_skim = adjusted_skim(command, tokens, &token_matches);
    let adjusted_skim_per_char = if query_char_count == 0 {
        0.0
    } else {
        adjusted_skim / query_char_count as f64
    };

    Some((
        FuzzyMatchResult {
            score: raw_score_total,
            matched_indices: merged_indices,
        },
        MatchQuality {
            adjusted_skim,
            adjusted_skim_per_char,
        },
    ))
}

/// Sums every token's raw Skim score (fzf-style term summation for a multi-word, AND-ed query)
/// plus [`CONSECUTIVE_BONUS_PER_CHAR`] for each of that token's contiguously-matched characters
/// beyond the first, plus [`EXACT_WHOLE_LINE_BONUS`] if `tokens` (rejoined) exactly equals
/// `command`.
fn adjusted_skim(command: &str, tokens: &[&str], token_matches: &[FuzzyMatchResult]) -> f64 {
    let per_token_total: f64 = token_matches
        .iter()
        .map(|token_match| {
            let longest_run = longest_consecutive_run(&token_match.matched_indices);
            token_match.score as f64
                + longest_run.saturating_sub(1) as f64 * CONSECUTIVE_BONUS_PER_CHAR
        })
        .sum();

    let query = tokens.join(" ");
    let exact_bonus = if !query.is_empty() && command.eq_ignore_ascii_case(&query) {
        EXACT_WHOLE_LINE_BONUS
    } else {
        0.0
    };

    per_token_total + exact_bonus
}

/// Longest run of consecutive (i.e. `idx, idx+1, idx+2, ...`) indices in `indices`, which is
/// assumed sorted ascending (true of every `FuzzyMatchResult` produced by `fuzzy_match`).
fn longest_consecutive_run(indices: &[usize]) -> usize {
    let mut longest = 0;
    let mut current = 0;
    let mut previous = None;
    for &index in indices {
        current = if previous == index.checked_sub(1) {
            current + 1
        } else {
            1
        };
        longest = longest.max(current);
        previous = Some(index);
    }
    longest
}

/// Inputs to [`rank`] for a single history candidate that has already cleared the fuzzy-match
/// gate.
pub(crate) struct RankInputs<'a> {
    pub entry: &'a HistoryEntry,
    /// Number of times this command has been executed, per `History::command_execution_count`.
    pub frequency: u32,
    pub match_quality: MatchQuality,
    pub now: DateTime<Local>,
    pub current_session_id: SessionId,
    pub current_cwd: Option<&'a str>,
    /// Number of other candidates newer than this one in the full (chronologically-ordered)
    /// history list. Used as an age proxy for entries with no timestamp; see `age_days`.
    pub newer_candidate_count: usize,
    /// Whether the query is empty (the zero-state case, where `SearchMixer` still invokes
    /// history so it has something to show before the user types). Priors like frequency and cwd
    /// are only meaningful relative to an actual query; applying them here would reorder the
    /// zero state away from its established chronological order, so [`rank`] gives every blank
    /// query the same score instead of computing one from priors.
    pub is_blank_query: bool,
}

/// Combines a candidate's match quality with its history priors into a single sortable score, or
/// `None` if the match quality doesn't clear [`RAW_SKIM_FLOOR_PER_CHAR`].
///
/// The result is `adjusted_skim * f(priors)`, staying on the same raw Skim scale every other
/// Command Search source's own score lives on: `f(priors)` is a narrow `[0.8, 1.2]` multiplier
/// (see [`PRIOR_MULTIPLIER_SWING`]), so priors can only ever reorder candidates whose match
/// quality is already comparable, never let a fresh weak match outrank an older strong one, or
/// let history's position relative to other sources depend on how many priors it happens to
/// satisfy. Higher is better, consistent with `SearchItem::score`.
pub(crate) fn rank(inputs: RankInputs<'_>) -> Option<OrderedFloat<f64>> {
    if inputs.is_blank_query {
        // Every blank-query candidate ties at the same score, so the mixer's stable sort leaves
        // `History::commands_shared()`'s chronological order intact, exactly as it did before
        // this ranking existed (Skim scores every candidate 0 for an empty pattern too).
        return Some(OrderedFloat(0.0));
    }

    if inputs.match_quality.adjusted_skim_per_char < RAW_SKIM_FLOOR_PER_CHAR {
        return None;
    }

    let age_days = age_days(inputs.entry, inputs.now, inputs.newer_candidate_count);
    let recency = (-std::f64::consts::LN_2 * age_days / RECENCY_HALF_LIFE_DAYS).exp();
    let frequency =
        ((inputs.frequency as f64 + 1.0).ln() / (FREQUENCY_SATURATION_COUNT + 1.0).ln()).min(1.0);
    let session = f64::from(inputs.entry.session_id == Some(inputs.current_session_id));
    let cwd = f64::from(
        matches!((inputs.entry.pwd.as_deref(), inputs.current_cwd), (Some(a), Some(b)) if a == b),
    );
    let exit_penalty = f64::from(
        inputs
            .entry
            .exit_code
            .is_some_and(|code| !code.was_successful()),
    );

    let prior_sum = RECENCY_WEIGHT * recency
        + FREQUENCY_WEIGHT * frequency
        + SESSION_WEIGHT * session
        + CWD_WEIGHT * cwd
        - EXIT_PENALTY_WEIGHT * exit_penalty;
    let normalized_priors =
        ((prior_sum - PRIOR_SUM_MIN) / (PRIOR_SUM_MAX - PRIOR_SUM_MIN)).clamp(0.0, 1.0);
    let prior_multiplier = PRIOR_MULTIPLIER_BASELINE + PRIOR_MULTIPLIER_SWING * normalized_priors;

    Some(OrderedFloat(
        inputs.match_quality.adjusted_skim * prior_multiplier,
    ))
}

/// Age, in days, used for the recency term. Falls back to a synthetic age based on how many
/// newer candidates exist for entries with no timestamp (history-file rows with no matching
/// sqlite record), so they decay gracefully instead of reading as infinitely old.
fn age_days(entry: &HistoryEntry, now: DateTime<Local>, newer_candidate_count: usize) -> f64 {
    match entry.start_ts {
        Some(start_ts) => (now - start_ts).num_seconds() as f64 / 86_400.0,
        None => newer_candidate_count as f64 * FALLBACK_AGE_DAYS_PER_POSITION,
    }
    .max(0.0)
}

#[cfg(test)]
#[path = "rank_tests.rs"]
mod tests;
