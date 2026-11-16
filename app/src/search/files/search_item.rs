use fuzzy_match::FuzzyMatchResult;

/// Basic file search result structure that can be used across different UI components.
/// This is the common data format returned by the FileSearchModel.
#[derive(Debug, Clone)]
pub struct FileSearchResult {
    pub path: String,
    pub project_directory: String,
    pub is_directory: bool,
}

/// Extended file search item that includes match results for UI rendering.
/// UI components can convert FileSearchResult + FuzzyMatchResult into this.
#[derive(Debug, Clone)]
pub struct FileSearchItem {
    pub path: String,
    pub match_result: FuzzyMatchResult,
    pub is_directory: bool,
}

impl FileSearchItem {
    /// Create a FileSearchItem from a FileSearchResult and match result
    pub fn from_result(result: FileSearchResult, match_result: FuzzyMatchResult) -> Self {
        Self {
            path: result.path,
            match_result,
            is_directory: result.is_directory,
        }
    }

    /// Create a FileSearchItem with no match highlighting (for zero state)
    pub fn from_result_no_match(result: FileSearchResult) -> Self {
        Self {
            path: result.path,
            match_result: FuzzyMatchResult::no_match(),
            is_directory: result.is_directory,
        }
    }
}
