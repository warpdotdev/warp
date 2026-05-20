//! This module implements terminal find functionality for the alt screen.
use std::ops::RangeInclusive;

use crate::{
    terminal::model::{
        alt_screen::AltScreen,
        find::{FindConfig, RegexDFAs},
        index::Point,
    },
    view_components::find::FindDirection,
};

use super::FindOptions;

/// Runs a find operation on the blocklist using the given `options` and returns an
/// `AltScreenFindRun` with the results.
///
/// If the given `options` does not contain a query, returns `None`.
pub(super) fn run_find_on_alt_screen(
    options: FindOptions,
    alt_screen: &AltScreen,
) -> AltScreenFindRun {
    let dfas = options.query.as_ref().and_then(|query| {
        RegexDFAs::new_with_config(
            query.as_str(),
            FindConfig {
                is_regex_enabled: options.is_regex_enabled,
                is_case_sensitive: options.is_case_sensitive,
            },
        )
        .ok()
    });

    let matches = dfas
        .as_ref()
        .map(|dfas| alt_screen.find(dfas))
        .unwrap_or_default();
    let focused_match_index = (!matches.is_empty()).then_some(0);

    AltScreenFindRun {
        dfas,
        matches,
        focused_match_index,
        options,
    }
}

#[derive(Debug)]
pub struct AltScreenFindRun {
    /// Compiled [`RegexDFAs`] for the find query.
    ///
    /// If the query in `options` is Some(), this is guaranteed to be `Some()`.
    dfas: Option<RegexDFAs>,

    /// Matches found in the alt screen.
    ///
    /// Each match is a range of character indices in the alt screen grid.
    ///
    /// Matches in this vector are ordered in order of decreasing recency, from "bottom" to "top".
    /// This ensures that iterating over matches occurs in the order that is expected in the UI.
    ///
    /// The match at index 0 is the first match to be focused after a fresh find run -  this is the
    /// match closest to the bottom of the alt screen grid.
    matches: Vec<RangeInclusive<Point>>,

    /// The index of the currently focused match in `matches` vector.
    focused_match_index: Option<usize>,

    /// The `FindOptions` used to configure the find run.
    options: FindOptions,
}

impl AltScreenFindRun {
    pub fn options(&self) -> &FindOptions {
        &self.options
    }

    pub fn focused_match_index(&self) -> Option<usize> {
        self.focused_match_index
    }

    pub fn focused_match_range(&self) -> Option<&RangeInclusive<Point>> {
        self.focused_match_index
            .and_then(|index| self.matches.get(index))
    }

    /// Returns list of all alt screen matches
    pub fn matches(&self) -> &[RangeInclusive<Point>] {
        &self.matches
    }

    /// Focuses the next match in `matches` based on the given `direction`.
    pub(super) fn focus_next_match(&mut self, direction: FindDirection) {
        if let Some(current_focused_match_index) = self.focused_match_index() {
            let index = current_focused_match_index;
            let next_match_index = match direction {
                FindDirection::Up => {
                    if index + 1 < self.matches.len() {
                        index + 1
                    } else {
                        0
                    }
                }
                FindDirection::Down => {
                    if index > 0 {
                        index - 1
                    } else {
                        self.matches.len() - 1
                    }
                }
            };
            self.focused_match_index = Some(next_match_index);
        } else if !self.matches.is_empty() {
            self.focused_match_index = Some(0);
        }
    }

    /// Reruns the find operation with the same options on the alt screen.
    pub(super) fn rerun(mut self, alt_screen: &AltScreen) -> Self {
        let Some(dfas) = self.dfas.as_ref() else {
            return self;
        };

        let new_matches = alt_screen.find(dfas);
        self.matches = new_matches;

        // If there are no more matches, reset the focused index.
        if self.matches.is_empty() {
            self.focused_match_index = None;
        } else if let Some(mut focused_match_index) = self.focused_match_index {
            // If there are matches and we had one focused before, bring it into range
            // if it isn't already.
            while focused_match_index >= self.matches.len() {
                focused_match_index = focused_match_index.saturating_sub(1);
            }
            self.focused_match_index = Some(focused_match_index);
        } else {
            // If there are matches but there wasn't an existing focused match,
            // focus the first match.
            self.focused_match_index = Some(0);
        }
        self
    }

    /// Returns a cleared version of this run (has no matches but the same options).
    pub(super) fn cleared(mut self) -> Self {
        let new_dfas =
            self.options.query.as_ref().and_then(|query| {
                match RegexDFAs::new_with_config(
                    query.as_str(),
                    FindConfig {
                        is_regex_enabled: self.options.is_regex_enabled,
                        is_case_sensitive: self.options.is_case_sensitive,
                    },
                ) {
                    Ok(dfas) => Some(dfas),
                    Err(e) => {
                        log::warn!(
                            "Failed to construct new RegexDFAs for cleared AltScreenFindRun: {e:?}"
                        );
                        None
                    }
                }
            });
        self.dfas = new_dfas;
        self.matches = vec![];
        self.focused_match_index = None;
        self
    }
}

#[cfg(test)]
#[path = "alt_screen_tests.rs"]
mod tests;
