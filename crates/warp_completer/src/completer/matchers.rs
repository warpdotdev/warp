use serde::{Deserialize, Serialize};

use fuzzy_match::{match_indices_case_insensitive, FuzzyMatchResult};

/// Determine if `from` starts with `partial` in a case insensitive manner.
/// Returns None if `partial` does not start with `from`, otherwise specifying
/// whether the match is exact or a prefix, and whether it is case sensitive.
fn match_type_for_case_insensitive(partial: &str, from: &str) -> Option<Match> {
    if partial.len() > from.len() {
        return None;
    }
    let mut starts_with = true;
    let mut is_case_sensitive = true;
    for (a, b) in from.chars().zip(partial.chars()) {
        if a == b {
            continue;
        } else if a.eq_ignore_ascii_case(&b) {
            is_case_sensitive = false;
        } else {
            starts_with = false;
            break;
        }
    }
    let same_length = partial.len() == from.len();
    starts_with.then_some(match same_length {
        true => Match::Exact { is_case_sensitive },
        false => Match::Prefix { is_case_sensitive },
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchStrategy {
    /// Yields only case-sensitive matches, otherwise yields None.
    CaseSensitive,
    /// Yields both case-sensitive and case-insensitive matches, otherwise yields None.
    CaseInsensitive,
    /// Yields both case-sensitive and case-insensitive matches. If there is no
    /// exact/prefix match result, it will try to find a fuzzy (i.e. approximate)
    /// match result.
    Fuzzy,
}

impl MatchStrategy {
    /// Given the matcher variant, return a MatchType if partial matches from.
    /// Note that this function will return the most specific match type (irrespective
    /// of the matcher). For example, a fuzzy matcher will return an Exact match
    /// for partial="git" and from="git" even though a Prefix match and Fuzzy match
    /// is also technically correct.
    pub fn get_match_type(&self, partial: &str, from: &str) -> Option<Match> {
        use Match::*;

        match self {
            MatchStrategy::CaseSensitive => {
                if from == partial {
                    Some(Exact {
                        is_case_sensitive: true,
                    })
                } else if from.starts_with(partial) {
                    Some(Prefix {
                        is_case_sensitive: true,
                    })
                } else {
                    None
                }
            }
            MatchStrategy::CaseInsensitive => match_type_for_case_insensitive(partial, from),
            MatchStrategy::Fuzzy => {
                let case_insensitive_match = match_type_for_case_insensitive(partial, from);
                if case_insensitive_match.is_some() {
                    return case_insensitive_match;
                }

                match_indices_case_insensitive(from, partial)
                    .map(|match_result| Fuzzy { match_result })
            }
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum Match {
    Prefix { is_case_sensitive: bool },
    Exact { is_case_sensitive: bool },
    Fuzzy { match_result: FuzzyMatchResult },
}

/// How precisely a search pattern matches its result.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum MatchType {
    Prefix {
        is_case_sensitive: bool,
    },
    Exact {
        is_case_sensitive: bool,
    },
    Fuzzy,
    /// The `Other` variant is used when we have matches that aren't related to the
    /// search pattern, for example, workflow enum suggestions
    Other,
}

impl From<Match> for MatchType {
    fn from(match_type: Match) -> Self {
        match match_type {
            Match::Prefix { is_case_sensitive } => MatchType::Prefix { is_case_sensitive },
            Match::Exact { is_case_sensitive } => MatchType::Exact { is_case_sensitive },
            Match::Fuzzy { .. } => MatchType::Fuzzy,
        }
    }
}

#[cfg(test)]
#[path = "matchers_tests.rs"]
mod tests;
