use fuzzy_match::match_indices_case_insensitive;

use crate::search::command_palette::agent_sessions::candidate::AgentSessionCandidate;

/// A candidate that was fuzzy matched against a search term, together with
/// everything the row needs to render its highlights.
#[derive(Debug, Clone)]
pub struct MatchedAgentSession {
    pub candidate: AgentSessionCandidate,
    pub match_result: AgentSessionMatchResult,
}

impl MatchedAgentSession {
    /// The score of the best-matching field, or `0` when the row was not
    /// matched at all (the empty query).
    pub fn score(&self) -> i64 {
        self.match_result.score
    }

    pub fn highlight_indices(&self) -> &AgentSessionHighlightIndices {
        &self.match_result.highlight_indices
    }
}

/// The outcome of matching one candidate.
#[derive(Debug, Clone)]
pub struct AgentSessionMatchResult {
    score: i64,
    highlight_indices: AgentSessionHighlightIndices,
}

impl AgentSessionMatchResult {
    /// The result for a row that was not matched against anything — what the
    /// empty query produces, where every candidate is shown as-is.
    pub fn no_match() -> Self {
        Self {
            score: 0,
            highlight_indices: AgentSessionHighlightIndices::default(),
        }
    }

    pub fn score(&self) -> i64 {
        self.score
    }
}

/// Matching indices for one candidate, per field.
///
/// These are **char** indices, which is what the text elements' highlight API
/// expects. (`fuzzy_match`'s own function-level doc says "byte"; its
/// implementation calls `fuzzy_indices`, which returns char indices, and the
/// field doc on `FuzzyMatchResult::matched_indices` says so too.)
#[derive(Debug, Clone, Default)]
pub struct AgentSessionHighlightIndices {
    task_indices: Vec<usize>,
    project_indices: Vec<usize>,
    cwd_indices: Vec<usize>,
}

impl AgentSessionHighlightIndices {
    pub fn task_indices(&self) -> &Vec<usize> {
        &self.task_indices
    }

    pub fn project_indices(&self) -> &Vec<usize> {
        &self.project_indices
    }

    pub fn cwd_indices(&self) -> &Vec<usize> {
        &self.cwd_indices
    }
}

/// Matches one candidate against `search_term`, or `None` if it does not match.
///
/// Each of the three fields a user might remember a session by — its task name,
/// its project, its directory — is matched independently and the best score
/// wins, so recalling any one of them finds the session.
///
/// An empty `search_term` matches everything with a score of `0`: the popup's
/// zero state is "every session you have", not "nothing".
pub fn match_agent_session(
    candidate: &AgentSessionCandidate,
    search_term: &str,
) -> Option<MatchedAgentSession> {
    if search_term.is_empty() {
        return Some(MatchedAgentSession {
            candidate: candidate.clone(),
            match_result: AgentSessionMatchResult::no_match(),
        });
    }

    let task_match = match_indices_case_insensitive(&candidate.task_name, search_term);
    let project_match = match_indices_case_insensitive(&candidate.project_name, search_term);
    let cwd_match = match_indices_case_insensitive(&candidate.cwd, search_term);

    if task_match.is_none() && project_match.is_none() && cwd_match.is_none() {
        return None;
    }

    let score = [
        task_match.as_ref(),
        project_match.as_ref(),
        cwd_match.as_ref(),
    ]
    .into_iter()
    .flatten()
    .map(|result| result.score)
    .max()
    .unwrap_or(0);

    Some(MatchedAgentSession {
        candidate: candidate.clone(),
        match_result: AgentSessionMatchResult {
            score,
            highlight_indices: AgentSessionHighlightIndices {
                task_indices: task_match.map(|r| r.matched_indices).unwrap_or_default(),
                project_indices: project_match.map(|r| r.matched_indices).unwrap_or_default(),
                cwd_indices: cwd_match.map(|r| r.matched_indices).unwrap_or_default(),
            },
        },
    })
}

/// Returns the candidates matching `search_term`, in the order they were given.
pub fn filter_agent_sessions<'a, 'b, I>(
    candidates: I,
    search_term: &'b str,
) -> impl Iterator<Item = MatchedAgentSession> + use<'a, 'b, I>
where
    I: IntoIterator<Item = &'a AgentSessionCandidate>,
{
    candidates
        .into_iter()
        .filter_map(move |candidate| match_agent_session(candidate, search_term))
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
