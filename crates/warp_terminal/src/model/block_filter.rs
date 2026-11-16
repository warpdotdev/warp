use regex_automata::hybrid::BuildError;

use super::find::{FindConfig, RegexDFAs};
pub type ContextLines = u16;
pub const DEFAULT_CONTEXT_LINES_VALUE: ContextLines = 0;
#[derive(Clone, Debug, PartialEq)]
pub struct BlockFilterQuery {
    pub query: String,
    /// The number of context lines to include above/below each matched line.
    pub num_context_lines: ContextLines,
    pub regex_enabled: bool,
    pub case_sensitivity_enabled: bool,
    pub invert_filter_enabled: bool,
    /// Only active queries will be applied to a block. Inactive queries will not
    /// be applied, but are used to store the previous filter state on a block.
    pub is_active: bool,
}

impl BlockFilterQuery {
    pub fn construct_dfas(&self) -> Result<RegexDFAs, Box<BuildError>> {
        RegexDFAs::new_with_config(
            self.query.as_str(),
            FindConfig {
                is_regex_enabled: self.regex_enabled,
                is_case_sensitive: self.case_sensitivity_enabled,
            },
        )
    }

    /// Returns true if this block filter query will apply an active filter to
    /// the block. If false, this query will set the block into a non-filtered
    /// state.
    pub fn is_active_and_nonempty(&self) -> bool {
        !self.query.is_empty() && self.is_active
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn new_for_test(query: String) -> Self {
        Self {
            query,
            num_context_lines: 0,
            regex_enabled: false,
            case_sensitivity_enabled: false,
            invert_filter_enabled: false,
            is_active: true,
        }
    }
}
