//! `rank()`'s formula is `score = adjusted_skim * f(priors)`: `adjusted_skim` is the match-quality
//! component (raw Skim score plus the corrections in `adjusted_skim()`) and `f(priors)` is a
//! bounded multiplier built from recency, session, and exit status.

use chrono::{DateTime, Local};
use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;

use crate::terminal::HistoryEntry;
use crate::terminal::model::session::SessionId;

/// Bottom of `f(priors)`'s range.
const PRIOR_MULTIPLIER_BASELINE: f64 = 0.8;

/// Width added on top of [`PRIOR_MULTIPLIER_BASELINE`], so `f(priors)` ranges over `[0.8, 1.2]`.
/// Bounded by `rank_tests.rs`'s `older_exact_match_outranks_fresher_weak_match` and
/// `recency_breaks_ties_among_equal_quality_substring_matches`, which pin down this width.
const PRIOR_MULTIPLIER_SWING: f64 = 0.4;

/// How much of `f(priors)` is driven by recency vs. the other priors below.
const RECENCY_WEIGHT: f64 = 0.10;

/// How much of `f(priors)` is driven by whether a command ran in the current session.
const SESSION_WEIGHT: f64 = 0.05;

/// How much `f(priors)` is reduced for a command whose last run failed.
const EXIT_PENALTY_WEIGHT: f64 = 0.03;

/// Days for the recency term to decay by half.
const RECENCY_HALF_LIFE_DAYS: f64 = 3.0;

/// Minimum adjusted-Skim score, per character of the query, for a match to be shown at all.
/// Legitimate matches score in the high teens to twenties per character (see `rank_tests.rs`).
const RAW_SKIM_FLOOR_PER_CHAR: f64 = 8.0;

/// Per-character bonus for a run of contiguously-matched characters, folded into `adjusted_skim`.
const CONSECUTIVE_BONUS_PER_CHAR: f64 = 4.0;

/// Bonus added once to `adjusted_skim` when the query exactly matches the whole command; needed
/// because SkimMatcherV2 scores a query identically whether it's the whole command or just a
/// prefix of a longer one.
const EXACT_WHOLE_LINE_BONUS: f64 = 12.0;

/// Recency assigned to entries with no timestamp (history-file rows with no matching sqlite
/// record), i.e. exactly between "as fresh as possible" (1.0) and "as stale as possible" (0.0):
/// there's no data to justify treating an untracked entry as either.
const MISSING_TIMESTAMP_RECENCY: f64 = 0.5;

/// Theoretical lower bound of the weighted prior sum, reached when every positive prior is absent
/// and the command's last run failed. Rescales that sum into `[0, 1]` before it becomes
/// `f(priors)`'s swing.
const PRIOR_SUM_MIN: f64 = -EXIT_PENALTY_WEIGHT;

/// Theoretical upper bound of the weighted prior sum, reached when every positive prior is fully
/// satisfied and the command's last run succeeded.
const PRIOR_SUM_MAX: f64 = RECENCY_WEIGHT + SESSION_WEIGHT;

/// The Skim-scale quality of a fuzzy match, before history priors are applied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MatchQuality {
    /// Sum of every AND-ed token's raw Skim score (`fzf`-style term summation for a multi-word
    /// query) plus the [`CONSECUTIVE_BONUS_PER_CHAR`] correction, on the same raw scale every
    /// other Command Search source's own Skim-based score lives on. [`rank`] multiplies this by a
    /// bounded prior multiplier (see [`PRIOR_MULTIPLIER_SWING`]) to get the final score, so this
    /// value dominates a candidate's cross-source position, though priors can still shift it
    /// within that multiplier's range.
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
    pub match_quality: MatchQuality,
    pub now: DateTime<Local>,
    pub current_session_id: SessionId,
    /// Whether the query is empty (the zero-state case, where `SearchMixer` still invokes
    /// history so it has something to show before the user types). Priors like session are only
    /// meaningful relative to an actual query; applying them here would reorder the zero state
    /// away from its established chronological order, so [`rank`] gives every blank query the
    /// same score instead of computing one from priors.
    pub is_blank_query: bool,
}

/// Combines a candidate's match quality with its history priors into a single sortable score, or
/// `None` if the match quality doesn't clear [`RAW_SKIM_FLOOR_PER_CHAR`].
pub(crate) fn rank(inputs: RankInputs<'_>) -> Option<OrderedFloat<f64>> {
    if inputs.is_blank_query {
        return Some(OrderedFloat(0.0));
    }

    if inputs.match_quality.adjusted_skim_per_char < RAW_SKIM_FLOOR_PER_CHAR {
        return None;
    }

    let recency = match inputs.entry.start_ts {
        Some(start_ts) => {
            let age_days = age_days(start_ts, inputs.now);
            (-std::f64::consts::LN_2 * age_days / RECENCY_HALF_LIFE_DAYS).exp()
        }
        None => MISSING_TIMESTAMP_RECENCY,
    };
    let session = f64::from(inputs.entry.session_id == Some(inputs.current_session_id));
    let exit_penalty = f64::from(
        inputs
            .entry
            .exit_code
            .is_some_and(|code| !code.was_successful()),
    );

    let prior_sum =
        RECENCY_WEIGHT * recency + SESSION_WEIGHT * session - EXIT_PENALTY_WEIGHT * exit_penalty;
    let normalized_priors =
        ((prior_sum - PRIOR_SUM_MIN) / (PRIOR_SUM_MAX - PRIOR_SUM_MIN)).clamp(0.0, 1.0);
    let prior_multiplier = PRIOR_MULTIPLIER_BASELINE + PRIOR_MULTIPLIER_SWING * normalized_priors;

    Some(OrderedFloat(
        inputs.match_quality.adjusted_skim * prior_multiplier,
    ))
}

/// Age, in days, of a command with a known `start_ts`, used for the recency term.
fn age_days(start_ts: DateTime<Local>, now: DateTime<Local>) -> f64 {
    let seconds_per_day = chrono::TimeDelta::days(1).num_seconds() as f64;
    ((now - start_ts).num_seconds() as f64 / seconds_per_day).max(0.0)
}

#[cfg(test)]
#[path = "rank_tests.rs"]
mod tests;
