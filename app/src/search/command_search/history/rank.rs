use chrono::{DateTime, Local};
use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;

use crate::terminal::HistoryEntry;
use crate::terminal::model::session::SessionId;

const MATCH_WEIGHT: f64 = 0.55;
const RECENCY_WEIGHT: f64 = 0.30;
const FREQUENCY_WEIGHT: f64 = 0.08;
const SESSION_WEIGHT: f64 = 0.05;
const CWD_WEIGHT: f64 = 0.02;
const EXIT_PENALTY_WEIGHT: f64 = 0.03;

const MATCH_EXACT_WEIGHT: f64 = 0.45;
const MATCH_SKIM_WEIGHT: f64 = 0.35;
const MATCH_CONSECUTIVE_WEIGHT: f64 = 0.15;
const MATCH_TIGHTNESS_WEIGHT: f64 = 0.05;

const EXACT_WHOLE_LINE: f64 = 1.0;
const EXACT_SUBSTRING: f64 = 0.85;
const EXACT_PREFIX: f64 = 0.55;

/// Divisor in the `skim / (skim + SKIM_SOFT_CAP)` curve used to normalize Skim's raw,
/// query-length-scaled score into 0..1. Chosen so the per-character raw scores seen in the
/// concrete fzf-comparison examples from APP-5650 (roughly 20-23) land mid-curve, leaving room
/// for camelCase/boundary-bonus-heavy matches to approach 1.0 without a hard clamp.
const SKIM_SOFT_CAP: f64 = 30.0;

/// Half-life, in days, of the recency term. Also applied to the position-based age fallback used
/// for entries with no timestamp (see `age_days`).
const RECENCY_HALF_LIFE_DAYS: f64 = 3.0;

/// Synthetic "days per list position" used to derive an age for entries with no timestamp, so a
/// commonly-typed command near the tail of an untracked history file still reads as recent
/// instead of decaying to zero relevance. Reuses `RECENCY_HALF_LIFE_DAYS` as the decay rate, so a
/// commonly-typed command a few positions back still retains a majority of the recency term.
const FALLBACK_AGE_DAYS_PER_POSITION: f64 = 1.0;

/// Count of executions beyond which the frequency term stops increasing. `ln(1 + 20)` is the
/// normalizer, so exactly 20 executions maps to a frequency term of 1.0.
const FREQUENCY_SATURATION_COUNT: f64 = 20.0;

/// Minimum combined match quality (see [`MatchQuality::combined`]) a candidate must clear to be
/// shown at all. Skim's DP will happily align a handful of scattered characters anywhere in a
/// long string; this filters out alignments too loose to be a meaningful result rather than
/// merely down-ranking them.
const MATCH_SCORE_FLOOR: f64 = 0.12;

/// Width, in match-quality units, of each band in [`pack_sort_key`]'s ordering gate.
const MATCH_BAND_GRANULARITY: f64 = 0.25;

/// Score contribution per match-quality band in [`pack_sort_key`]. Must exceed the maximum
/// possible span of `final_score` (bounded to +/-1 there) so bands never bleed into each other.
const MATCH_BAND_SLOT: f64 = 3.0;

/// Score contribution for a whole-line exact match in [`pack_sort_key`]. Must exceed the maximum
/// possible total band contribution (5 bands * `MATCH_BAND_SLOT`) so an exact match always
/// outranks a non-exact one regardless of any other term.
const EXACT_LINE_SLOT: f64 = 100.0;

/// Scale applied to the raw age (in days) used as the final tiebreaker in [`pack_sort_key`].
/// Small enough that it never perturbs a genuine `final_score` difference, but still gives a
/// deterministic, newer-wins resolution for otherwise-identical candidates.
const AGE_TIE_BREAK_SCALE: f64 = 1e-9;

/// The normalized quality of a fuzzy match, before history priors are applied. Each field is in
/// `0..=1`; see [`Self::combined`] for how they are weighted together.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MatchQuality {
    /// Whether the query is the whole command, a substring of it, or just a prefix-aligned fuzzy
    /// match. See `EXACT_WHOLE_LINE`, `EXACT_SUBSTRING`, `EXACT_PREFIX`.
    exact: f64,
    /// Skim's raw score, normalized by query length and soft-capped to 0..1.
    skim: f64,
    /// Longest run of contiguously-matched characters, as a fraction of the query length.
    consecutive: f64,
    /// How tightly the matched characters are clustered together in the command text.
    tightness: f64,
}

impl MatchQuality {
    fn combined(self) -> f64 {
        MATCH_EXACT_WEIGHT * self.exact
            + MATCH_SKIM_WEIGHT * self.skim
            + MATCH_CONSECUTIVE_WEIGHT * self.consecutive
            + MATCH_TIGHTNESS_WEIGHT * self.tightness
    }
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

    let match_quality = MatchQuality {
        exact: exact_component(command, tokens, &token_matches),
        skim: skim_component(raw_score_total, query_char_count),
        consecutive: consecutive_component(&token_matches, query_char_count),
        tightness: tightness_component(&merged_indices),
    };

    Some((
        FuzzyMatchResult {
            score: raw_score_total,
            matched_indices: merged_indices,
        },
        match_quality,
    ))
}

fn exact_component(command: &str, tokens: &[&str], token_matches: &[FuzzyMatchResult]) -> f64 {
    let query = tokens.join(" ");
    if !query.is_empty() {
        let command_lower = command.to_lowercase();
        let query_lower = query.to_lowercase();
        if command_lower == query_lower {
            return EXACT_WHOLE_LINE;
        }
        if command_lower.contains(&query_lower) {
            return EXACT_SUBSTRING;
        }
    }
    // A match that isn't a literal substring anywhere still deserves partial credit for aligning
    // with the very start of the command, which is what most shell commands key off of.
    let first_token_starts_at_zero = token_matches
        .first()
        .and_then(|token_match| token_match.matched_indices.first())
        == Some(&0);
    if first_token_starts_at_zero {
        EXACT_PREFIX
    } else {
        0.0
    }
}

fn skim_component(raw_score_total: i64, query_char_count: usize) -> f64 {
    if query_char_count == 0 {
        return 0.0;
    }
    let normalized = (raw_score_total as f64 / query_char_count as f64).max(0.0);
    normalized / (normalized + SKIM_SOFT_CAP)
}

fn consecutive_component(token_matches: &[FuzzyMatchResult], query_char_count: usize) -> f64 {
    if query_char_count == 0 {
        return 0.0;
    }
    let total_longest_run: usize = token_matches
        .iter()
        .map(|token_match| longest_consecutive_run(&token_match.matched_indices))
        .sum();
    (total_longest_run as f64 / query_char_count as f64).min(1.0)
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

fn tightness_component(merged_indices: &[usize]) -> f64 {
    match (merged_indices.first(), merged_indices.last()) {
        (Some(&first), Some(&last)) => {
            let span = (last - first + 1) as f64;
            merged_indices.len() as f64 / span
        }
        _ => 0.0,
    }
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
/// `None` if the match quality doesn't clear `MATCH_SCORE_FLOOR`.
///
/// The result packs a `(exact_line, match_band, final_score, age)` ordering tuple into one
/// `f64`: `exact_line` and `match_band` gate the ordering so that history priors can only ever
/// break ties *within* a match-quality tier, never let a fresher weak match outrank an older
/// exact one. Higher is better, consistent with `SearchItem::score`.
pub(crate) fn rank(inputs: RankInputs<'_>) -> Option<OrderedFloat<f64>> {
    if inputs.is_blank_query {
        // Every blank-query candidate ties at the same score, so the mixer's stable sort leaves
        // `History::commands_shared()`'s chronological order intact, exactly as it did before
        // this ranking existed (Skim scores every candidate 0 for an empty pattern too).
        return Some(OrderedFloat(0.0));
    }

    let match_value = inputs.match_quality.combined();
    if match_value < MATCH_SCORE_FLOOR {
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

    let final_score = MATCH_WEIGHT * match_value
        + RECENCY_WEIGHT * recency
        + FREQUENCY_WEIGHT * frequency
        + SESSION_WEIGHT * session
        + CWD_WEIGHT * cwd
        - EXIT_PENALTY_WEIGHT * exit_penalty;

    Some(pack_sort_key(
        inputs.match_quality.exact,
        match_value,
        final_score,
        age_days,
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

fn pack_sort_key(
    exact: f64,
    match_value: f64,
    final_score: f64,
    age_days: f64,
) -> OrderedFloat<f64> {
    let exact_line = f64::from(exact >= EXACT_WHOLE_LINE);
    let match_band = (match_value / MATCH_BAND_GRANULARITY).floor();
    // `final_score`'s weights sum to 1.0 (plus a small negative exit penalty), so it's already
    // within a band-sized slot; clamp defensively so a future weight change can't overflow it.
    let final_clamped = final_score.clamp(-1.0, 1.0);
    let age_tie_break = -age_days * AGE_TIE_BREAK_SCALE;

    OrderedFloat(
        exact_line * EXACT_LINE_SLOT + match_band * MATCH_BAND_SLOT + final_clamped + age_tie_break,
    )
}

#[cfg(test)]
#[path = "rank_tests.rs"]
mod tests;
