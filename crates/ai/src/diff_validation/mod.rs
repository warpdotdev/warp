use itertools::{EitherOrBoth, Itertools};
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    fmt::{self, Display},
    ops::Range,
    path::PathBuf,
    sync::LazyLock,
};
use strsim::jaro_winkler;
lazy_static! {
    /// Regex to parse a line number from a string in the format "{number}|{line}"
    static ref LINE_NUMBER_PARSE: Regex = Regex::new(r"^(\d+)\|(.*)$").expect("Regex is valid");
}

use cfg_if::cfg_if;
use derivative::Derivative;
use serde_json::json;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub enum ParsedDiff {
    /// An edit to a file using
    StrReplaceEdit {
        file: Option<String>,
        search: Option<String>,
        replace: Option<String>,
    },
    /// An edit to a file based on the V4A diff format:
    /// https://cookbook.openai.com/examples/gpt4-1_prompting_guide#apply-patch.
    V4AEdit {
        file: Option<String>,
        move_to: Option<String>,
        hunks: Vec<V4AHunk>,
    },
}

impl ParsedDiff {
    pub fn file(&self) -> Option<&String> {
        match self {
            ParsedDiff::StrReplaceEdit { file, .. } => file.as_ref(),
            ParsedDiff::V4AEdit { file, .. } => file.as_ref(),
        }
    }
}

impl Display for ParsedDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParsedDiff::StrReplaceEdit {
                file,
                search,
                replace,
            } => {
                write!(
                    f,
                    "{}",
                    json!({ "file": file, "search": search, "replace": replace})
                )
            }
            ParsedDiff::V4AEdit {
                file,
                move_to,
                hunks,
            } => {
                write!(
                    f,
                    "{}",
                    json!({
                        "file": file,
                        "move_to": move_to,
                        "hunks": hunks
                    })
                )
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffType {
    Create {
        /// The delta representing the creation.
        /// A delta for a file creation has an empty replacement line range.
        delta: DiffDelta,
    },
    Update {
        deltas: Vec<DiffDelta>,
        /// If set, the file should be renamed to this path when applying the diff.
        /// This path should also be a non-existing filepath.
        rename: Option<PathBuf>,
    },
    Delete {
        /// The delta representing the deletion.
        /// A delta for a file deletion has a replacement line range
        /// that spans the entire file and an empty insertion.
        delta: DiffDelta,
    },
}

impl DiffType {
    pub fn creation(content: String) -> Self {
        DiffType::Create {
            delta: DiffDelta {
                replacement_line_range: 0..0,
                insertion: content,
            },
        }
    }

    pub fn deletion(num_lines: usize) -> Self {
        DiffType::Delete {
            delta: DiffDelta {
                replacement_line_range: 1..num_lines.saturating_add(1),
                insertion: String::new(),
            },
        }
    }

    pub fn update(deltas: Vec<DiffDelta>, rename_to: Option<String>) -> Self {
        DiffType::Update {
            deltas,
            rename: rename_to.map(Into::into),
        }
    }
}

#[derive(Debug, Clone, Derivative)]
#[derivative(Eq, PartialEq)]
pub struct AIRequestedCodeDiff {
    pub file_name: String,
    pub diff_type: DiffType,
    /// Types of failures to create the diff that we want to capture via telemetry.
    #[derivative(PartialEq = "ignore")]
    pub failures: Option<DiffMatchFailures>,
    /// Original file content read during diff matching.
    /// Populated for edits and deletes; empty for new file creation.
    #[derivative(PartialEq = "ignore")]
    pub original_content: String,
}

impl AIRequestedCodeDiff {
    /// Determines if the failures are severe enough to warrant some logging/remediation.
    pub fn warrants_failure(&self) -> bool {
        match &self.failures {
            // NOTE: Avoid `..` rest patterns here so that devs adding new fields for failure types
            // must make a choice how their new field affects retries.
            Some(DiffMatchFailures {
                fuzzy_match_failures,
                noop_deltas,
                missing_line_numbers: _,
            }) => {
                let update_deltas_empty = match &self.diff_type {
                    DiffType::Update { deltas, .. } => deltas.is_empty(),
                    DiffType::Create { .. } | DiffType::Delete { .. } => false,
                };

                *fuzzy_match_failures > 0 || (*noop_deltas > 0 && update_deltas_empty)
            }
            None => false,
        }
    }
}

/// Visual representation of a single diff hunk.
#[derive(Clone, PartialEq, Eq)]
pub struct DiffDelta {
    pub replacement_line_range: Range<usize>,
    pub insertion: String,
}

impl fmt::Debug for DiffDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if cfg!(debug_assertions) {
            write!(
                f,
                "DiffDelta {{\nreplacement_line_range: {:?},",
                &self.replacement_line_range
            )?;
            f.write_str("\n--insertion--\n")?;
            f.write_str(&self.insertion)?;
            f.write_str("\n}")
        } else {
            Ok(())
        }
    }
}

#[cfg_attr(test, derive(PartialEq))]
pub struct SearchAndReplace {
    pub search: String,
    pub replace: String,
}

#[cfg(debug_assertions)]
impl core::fmt::Debug for SearchAndReplace {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SearchAndReplace {\n--search--\n")?;
        f.write_str(&self.search)?;
        f.write_str("\n--replace--\n")?;
        f.write_str(&self.replace)?;
        f.write_str("\n}")
    }
}

impl TryFrom<ParsedDiff> for SearchAndReplace {
    type Error = ();

    fn try_from(diff: ParsedDiff) -> Result<Self, Self::Error> {
        match diff {
            ParsedDiff::StrReplaceEdit {
                search: None,
                replace: None,
                ..
            } => Err(()),
            ParsedDiff::StrReplaceEdit {
                search, replace, ..
            } => Ok(SearchAndReplace {
                search: search.unwrap_or_default(),
                replace: remove_extra_line_num_prefix(replace.unwrap_or_default()),
            }),
            ParsedDiff::V4AEdit { .. } => {
                // V4AEdit is not supported for conversion to SearchAndReplace
                Err(())
            }
        }
    }
}

// See https://cookbook.openai.com/examples/gpt4-1_prompting_guide#apply-patch
// for the semantic meaning of these fields.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct V4AHunk {
    /// Context to quickly identify the location of the change (i.e. the `@@` statements).
    pub change_context: Vec<String>,
    /// The context lines right before the change.
    pub pre_context: String,
    /// The old content that is being replaced.
    /// Empty if lines are only being added.
    pub old: String,
    /// The new content that replaces the old content.
    /// Empty if lines are only being deleted.
    pub new: String,
    /// The context lines right after the change.
    pub post_context: String,
}

/// A customized version of [`str::lines`] which treats the empty string differenly.
///
/// Generally, calling `.lines()` (on UNIX) is equivalent to calling `.split("\n")`. However,
/// trailing empty lines are ignored by [`str::lines`] which produces some weird behavior.
fn lines(s: &str) -> impl Iterator<Item = &str> {
    match s {
        "" => "\n".lines(),
        _ => s.lines(),
    }
}

/// Returns `true` if `search` is a non-empty strict prefix of `file_line` after trimming
/// leading whitespace from both.
///
/// "Strict" means the trimmed `file_line` is strictly longer than the trimmed `search`;
/// equal-length matches are rejected so callers can rely on there being a non-empty unmatched
/// suffix.
fn is_strict_trimmed_prefix(search: &str, file_line: &str) -> bool {
    let trimmed_search = search.trim_start();
    let trimmed_file = file_line.trim_start();
    !trimmed_search.is_empty()
        && trimmed_file.len() > trimmed_search.len()
        && trimmed_file.starts_with(trimmed_search)
}

/// If `search_line` is a proper prefix of `file_line` (ignoring leading whitespace on both),
/// returns the unmatched suffix from `file_line`. Otherwise returns `None`.
fn unmatched_line_suffix<'a>(search_line: &str, file_line: &'a str) -> Option<&'a str> {
    if is_strict_trimmed_prefix(search_line, file_line) {
        let trimmed_search = search_line.trim_start();
        let trimmed_file = file_line.trim_start();
        Some(&trimmed_file[trimmed_search.len()..])
    } else {
        None
    }
}

/// Preserve the unmatched suffix of the final matched file line when the LLM emitted only a
/// partial final line.
///
/// When search and replace have the same line count, preserve the suffix to maintain the legacy
/// behavior for simple partial-line replacements. When line counts differ, only preserve it if the
/// replacement's final line still carries the same partial context as the search's final line.
fn append_unmatched_line_suffix(search: &str, file_line: &str, insertion: &mut String) {
    let Some(search_last_line) = lines(search).last() else {
        return;
    };
    let Some(suffix) = unmatched_line_suffix(search_last_line, file_line) else {
        return;
    };

    let search_line_count = lines(search).count();
    let insertion_line_count = lines(insertion).count();
    let Some(insertion_last_line) = lines(insertion).last() else {
        return;
    };

    if search_line_count != insertion_line_count
        && search_last_line.trim_start() != insertion_last_line.trim_start()
    {
        return;
    }

    let insertion_point = insertion.trim_end_matches('\n').len();
    insertion.insert_str(insertion_point, suffix);
}
/// We told the model not to include line numbers for the replacement content. However, it can
/// still happen. Try to remove them here.
/// https://github.com/warpdotdev/warp-server/blob/d9c1b6d1443290f2355979ae552d41af01a63bde/logic/ai/prompt/tools/suggest_diff.yaml#L34-L34
fn remove_extra_line_num_prefix(replace: String) -> String {
    static LINE_NUMBER_PATTERN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\d+\|").expect("line number regex must compile"));

    lines(&replace)
        .map(|line| LINE_NUMBER_PATTERN.replace(line, "").into_owned())
        .join("\n")
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize)]
pub struct DiffMatchFailures {
    /// Failures to perform a fuzzy match with content.
    pub fuzzy_match_failures: u8,
    /// The <search> and <replace> content was identical.
    pub noop_deltas: u8,
    /// Search blocks that are missing line numbers.
    pub missing_line_numbers: u8,
}

/// Fix two common issues with responses from the models that request code actions:
///
/// Omitting line numbers from the search section of a diff.
/// Using the wrong line number, often off-by-one.
///
/// Also returns the number of fuzzy matches that failed for telemetry.
///
/// Method
/// For each search block of n lines:
/// - Scan through n-sized windows of the requested file.
/// - Check the window for similarity to the search block.
///   - This currently uses Jaro-Winkler.
/// - If the window is similar, store it as a potential match.
/// - After all potential matches are found:
///   - If line numbers were included in the search block, return the potential match closest to the expected line numbers.
///   - Otherwise, return the potential match with the highest similarity.
pub fn fuzzy_match_diffs(
    file_name: &str,
    diffs: &[SearchAndReplace],
    file_content: impl Into<String>,
) -> AIRequestedCodeDiff {
    let file_content = file_content.into();
    let (deltas, failures) = fuzzy_match_file_diffs(diffs, &file_content);

    // Only surface failures when they are meaningful to the caller.
    // In particular, noop diffs are only considered failures if they result in no applied deltas.
    let update_deltas_empty = deltas.is_empty();
    let failures = if failures.fuzzy_match_failures > 0
        || failures.missing_line_numbers > 0
        || (failures.noop_deltas > 0 && update_deltas_empty)
    {
        Some(failures)
    } else {
        None
    };

    AIRequestedCodeDiff {
        file_name: file_name.into(),
        diff_type: DiffType::Update {
            deltas,
            rename: None,
        },
        failures,
        original_content: file_content,
    }
}

/// Match V4A format diffs against file content using context-based matching with fallback strategies.
/// First attempts exact matching, then falls back to indentation-agnostic matching,
/// and finally to JaroWinkler fuzzy matching.
pub fn fuzzy_match_v4a_diffs(
    file_name: &str,
    diffs: &[V4AHunk],
    rename_to: Option<String>,
    file_content: impl Into<String>,
) -> AIRequestedCodeDiff {
    let file_content = file_content.into();
    let mut deltas = Vec::new();
    let mut failures = DiffMatchFailures::default();

    let file_lines: Vec<&str> = file_content.lines().collect();

    for diff in diffs {
        // Check for no-op diffs
        if diff.old == diff.new {
            log::info!("Ignoring V4A diff with identical old and new content.");
            failures.noop_deltas += 1;
            continue;
        }

        // Find the location of the edit using context
        let match_range = find_v4a_match(diff, &file_lines);

        match match_range {
            Some(range) => {
                // Check if the replacement is identical to what's already there
                let matched_content = file_lines[range.start - 1..range.end - 1].join("\n");
                if diff.new == matched_content {
                    log::info!(
                        "Ignoring V4A diff where new content is identical to matched file content"
                    );
                    failures.noop_deltas += 1;
                    continue;
                }

                deltas.push(DiffDelta {
                    replacement_line_range: range.start..range.end,
                    insertion: diff.new.clone(),
                });
            }
            None => {
                log::warn!("Failed to find matching location for V4A diff");
                failures.fuzzy_match_failures += 1;
            }
        }
    }

    // Sort by start line and remove overlapping deltas. When the LLM produces
    // multiple hunks targeting the same region (e.g. a large deletion whose
    // matched range subsumes a nearby single-line edit), the overlapping delta
    // must be dropped — applying both would produce an invalid edit range in
    // the editor buffer (see WARP-CLIENT-DEV-NYY).
    deltas.sort_by_key(|d| d.replacement_line_range.start);
    deltas = deduplicate_overlapping_deltas(deltas);

    let update_deltas_empty = deltas.is_empty();
    let failures = if failures.fuzzy_match_failures > 0
        || failures.missing_line_numbers > 0
        || (failures.noop_deltas > 0 && update_deltas_empty)
    {
        Some(failures)
    } else {
        None
    };

    AIRequestedCodeDiff {
        file_name: file_name.into(),
        diff_type: DiffType::update(deltas, rename_to),
        failures,
        original_content: file_content,
    }
}

/// Given a list of `DiffDelta`s sorted by `replacement_line_range.start`,
/// drop any delta whose range overlaps with the preceding accepted delta.
///
/// "Overlaps" means `B.start < A.end` (strictly inside or partial overlap).
/// Adjacent ranges (`A.end == B.start`) are kept.
fn deduplicate_overlapping_deltas(sorted_deltas: Vec<DiffDelta>) -> Vec<DiffDelta> {
    let mut result: Vec<DiffDelta> = Vec::with_capacity(sorted_deltas.len());

    for delta in sorted_deltas {
        let dominated = result.last().is_some_and(|prev| {
            delta.replacement_line_range.start < prev.replacement_line_range.end
        });
        if dominated {
            log::warn!(
                "Dropping V4A delta with overlapping range {:?} \
                 (subsumed by preceding delta with range {:?})",
                delta.replacement_line_range,
                result.last().unwrap().replacement_line_range,
            );
            continue;
        }
        result.push(delta);
    }

    result
}

fn fuzzy_match_file_diffs(
    diffs: &[SearchAndReplace],
    file_content: &str,
) -> (Vec<DiffDelta>, DiffMatchFailures) {
    let mut deltas = Vec::new();
    let mut failures = DiffMatchFailures::default();

    let target_lines: Vec<&str> = lines(file_content).collect();

    for diff in diffs {
        #[cfg(debug_assertions)]
        log::debug!("{diff:#?}");

        let (mut line_range, search) = parse_line_numbers(&diff.search);

        // Missing line numbers are not necessarily fatal, due to fuzzy matching, but we still
        // want to track them.
        if line_range.is_none() && !search.is_empty() {
            failures.missing_line_numbers += 1;
        }

        if search == diff.replace {
            log::info!("Ignoring diff with identical <search> and <replace>.");
            failures.noop_deltas += 1;
            continue;
        }

        // Find similar sections in the file content using the matching strategies.
        let fuzzy_match_line_numbers = if line_range == Some(0..0) {
            // An empty line range indicates prepending to the file.
            line_range
        } else {
            line_range = line_range.filter(|range| {
                log::debug!("Parsed line range: {range:?}");
                range.start > 0
                    && range.start <= target_lines.len()
                    && range.end > 0
                    // Because the end is both 1-indexed and exclusive, the last valid end is 1 past
                    // the last line number.
                    && range.end <= target_lines.len() + 1
                    && range.end >= range.start
            });

            // First, search for an exact match, then fall back to ignoring whitespace if needed.
            let mut matched = match_diff(
                &search,
                line_range.clone(),
                &target_lines,
                SECTION_MATCH_THRESHOLD,
                MakeExactMatch,
            )
            .or_else(|| {
                // If there's no exact match, try ignoring whitespace.
                match_diff(
                    &search,
                    line_range.clone(),
                    &target_lines,
                    SECTION_MATCH_THRESHOLD,
                    MakeIndentationAgnosticMatch,
                )
            });

            // Prefix-tail rescue: only attempt when we have a line-number hint to
            // disambiguate.  Without a hint, short prefix searches like `fn main() {`
            // could match many windows in the file and silently pick the wrong one.
            if matched.is_none() && line_range.is_some() {
                matched = match_diff(
                    &search,
                    line_range.clone(),
                    &target_lines,
                    // Binary scorer: match is exact (1.0) or not at all.
                    1.0,
                    MakePrefixTailMatch,
                );
            }

            if matched.is_none() {
                matched = match_diff(
                    &search,
                    line_range.clone(),
                    &target_lines,
                    SECTION_MATCH_THRESHOLD,
                    MakeJaroWinklerMatch,
                );
            }

            matched
        };

        log::debug!("fuzzy match result: {fuzzy_match_line_numbers:?}");

        match fuzzy_match_line_numbers {
            Some(range) => {
                #[cfg(debug_assertions)]
                {
                    log::debug!("Matched content in file:");
                    for line_num in range.clone() {
                        log::debug!("{}|{}", line_num, target_lines[line_num - 1]);
                    }
                }

                // Check if the text to be replaced is identical to the replacement block.
                // This may happen if the search block was based on stale file information, and the
                // replacement block matches the file content but _not_ the search block.
                if range != (0..0)
                    && diff
                        .replace
                        .lines()
                        .zip_longest(&target_lines[range.start - 1..range.end - 1])
                        .all(|pair| match pair {
                            EitherOrBoth::Both(replace, original) => replace == *original,
                            EitherOrBoth::Left(_) | EitherOrBoth::Right(_) => false,
                        })
                {
                    log::info!("Ignoring diff with <replace> identical to the file contents");
                    failures.noop_deltas += 1;
                    continue;
                }

                // Create deltas based on the matches found.
                //
                // Some LLMs emit a search block whose last line is a prefix of the
                // actual file line (e.g. search ends with "let x" while the file has
                // "let x = 2;"). Because the matcher operates on whole-line windows,
                // the delta would replace the entire line and drop the unmatched
                // suffix. Detect this and preserve the suffix in the insertion.
                let mut insertion = diff.replace.clone();
                if range.end >= 2 {
                    append_unmatched_line_suffix(
                        &search,
                        target_lines[range.end - 2],
                        &mut insertion,
                    );
                }
                deltas.push(DiffDelta {
                    replacement_line_range: range.start..range.end,
                    insertion,
                });
            }
            None => {
                failures.fuzzy_match_failures += 1;
            }
        }
    }

    (deltas, failures)
}

#[derive(Debug, PartialEq)]
pub struct Match {
    pub start_line: usize,
    pub end_line: usize,
    pub similarity: f64,
}

/// Find similar sections in the target file using Jaro-Winkler similarity
/// These lines are 1-indexed to match the line numbers in the search string.
pub fn find_similar_sections(
    search_text: &str,
    target_lines: &[&str],
    threshold: f64,
) -> Vec<Match> {
    let search_len = search_text.lines().count();
    if search_len == 0 {
        return Vec::new();
    }
    // Slide through the target file looking for matches
    target_lines
        .windows(search_len)
        .enumerate()
        .filter_map(|(i, target_window)| {
            let similarity = section_similarity(search_text, target_window);
            (similarity >= threshold).then_some(Match {
                start_line: i + 1,
                similarity,
                end_line: i + search_len + 1,
            })
        })
        .collect()
}

/// Scores `search_window_lines`-length windows using a provided scoring function.
///
/// Returned matches are 1-indexed, and sorted by similarity.
/// If `expected_range` is provided, the scoring also considers how close matches are to the expected range.
fn score_matches<T: Scorer>(
    target_lines: &[&str],
    search_window_lines: usize,
    threshold: f64,
    expected_range: Option<Range<usize>>,
    scorer: &T,
) -> Vec<Match> {
    if search_window_lines == 0 || search_window_lines > target_lines.len() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    let mut max_similarity = 0.;
    #[cfg(debug_assertions)]
    let mut most_similar_range = None;
    for (i, window) in target_lines.windows(search_window_lines).enumerate() {
        let similarity = scorer.score(window);
        if similarity > max_similarity {
            max_similarity = similarity;
            #[cfg(debug_assertions)]
            {
                most_similar_range = Some(i..i + search_window_lines);
            }
        }
        if similarity >= threshold {
            matches.push(Match {
                // Matches are 1-indexed.
                start_line: i + 1,
                end_line: i + search_window_lines + 1,
                similarity,
            });
        }
    }

    if matches.is_empty() {
        log::debug!("No matches meeting the threshold for scorer {scorer}");
        #[cfg(debug_assertions)]
        if let Some(range) = most_similar_range {
            log::debug!(
                "Closest match with score {}:\n{}",
                max_similarity,
                &target_lines[range].join("\n")
            );
        }
    }

    // Sort by similarity and, optionally, line range.
    matches.sort_by(move |a, b| {
        let by_similarity = a
            .similarity
            .partial_cmp(&b.similarity)
            .unwrap_or(Ordering::Equal)
            .reverse();
        if let Some(Range { start, .. }) = expected_range {
            by_similarity.then_with(|| {
                let a_distance = a.start_line.abs_diff(start);
                let b_distance = b.start_line.abs_diff(start);
                a_distance.cmp(&b_distance)
            })
        } else {
            by_similarity
        }
    });

    matches
}

/// A `Scorer` scores a target window by how closely it matches some search text. Higher scores indicate closer matches.
trait Scorer: fmt::Display {
    fn score(&self, target_lines: &[&str]) -> f64;
}

/// `MakeScorer` is a factory trait for [`Scorer`]s. It works around lifetime issues with scorers that reference the search text.
trait MakeScorer: fmt::Display {
    type ScorerInstance<'a>: Scorer;
    fn for_search<'a>(&self, search_text: &'a str) -> Self::ScorerInstance<'a>;
}

/// A [`Scorer`] that returns 1 if the target window is an exact match to the search block, 0 otherwise.
struct ExactMatch<'a> {
    search_lines: Vec<&'a str>,
}

impl<'a> ExactMatch<'a> {
    fn new(search_text: &'a str) -> Self {
        let search_lines = lines(search_text).collect();
        Self { search_lines }
    }
}

impl Scorer for ExactMatch<'_> {
    fn score(&self, target_lines: &[&str]) -> f64 {
        if target_lines == self.search_lines {
            1.0
        } else {
            0.0
        }
    }
}

impl fmt::Display for ExactMatch<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{MakeExactMatch}")
    }
}

#[derive(Clone)]
struct MakeExactMatch;

impl fmt::Display for MakeExactMatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Exact")
    }
}

impl MakeScorer for MakeExactMatch {
    type ScorerInstance<'a> = ExactMatch<'a>;

    fn for_search<'a>(&self, search_text: &'a str) -> Self::ScorerInstance<'a> {
        ExactMatch::new(search_text)
    }
}

/// A [`Scorer`] that returns 1 if the target window is an exact match to the search block
/// after trimming leading whitespace from all lines, 0 otherwise.
struct IndentationAgnosticMatch<'a> {
    search_lines: Vec<&'a str>,
}

impl<'a> IndentationAgnosticMatch<'a> {
    fn new(search_text: &'a str) -> Self {
        let search_lines = lines(search_text).map(|line| line.trim_start()).collect();
        Self { search_lines }
    }
}

impl Scorer for IndentationAgnosticMatch<'_> {
    fn score(&self, target_lines: &[&str]) -> f64 {
        debug_assert_eq!(
            self.search_lines.len(),
            target_lines.len(),
            "Incorrect target window length"
        );

        // Trim leading whitespace from all lines in the target window.
        // We don't need to accumulate into an intermediate vector because the target window is
        // already the same length as the search block.
        if target_lines
            .iter()
            .map(|line| line.trim_start())
            .zip(self.search_lines.iter())
            .all(|(a, b)| a == *b)
        {
            1.0
        } else {
            0.0
        }
    }
}

impl fmt::Display for IndentationAgnosticMatch<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{MakeIndentationAgnosticMatch}")
    }
}

#[derive(Clone)]
struct MakeIndentationAgnosticMatch;

impl fmt::Display for MakeIndentationAgnosticMatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Indentation Agnostic")
    }
}

impl MakeScorer for MakeIndentationAgnosticMatch {
    type ScorerInstance<'a> = IndentationAgnosticMatch<'a>;

    fn for_search<'a>(&self, search_text: &'a str) -> Self::ScorerInstance<'a> {
        IndentationAgnosticMatch::new(search_text)
    }
}

/// A [`Scorer`] that returns 1.0 when the target window equals the search block on every line
/// except the last (after `trim_start`), and the final search line (after `trim_start`) is a
/// strict prefix of the final target line (after `trim_start`). Returns 0.0 otherwise.
///
/// This tier exists to rescue diffs where the LLM truncated the trailing content of the final
/// search line (e.g. `if foo() {` when the file actually has `if foo() && bar() {`). Jaro-Winkler
/// often scores such pairs just under the 0.9 threshold for long lines; this scorer handles the
/// case deterministically when a line-number hint is available to disambiguate.
struct PrefixTailMatch<'a> {
    /// Lines with leading whitespace trimmed.
    search_lines: Vec<&'a str>,
}

impl<'a> PrefixTailMatch<'a> {
    fn new(search_text: &'a str) -> Self {
        let search_lines = lines(search_text).map(|line| line.trim_start()).collect();
        Self { search_lines }
    }
}

impl Scorer for PrefixTailMatch<'_> {
    fn score(&self, target_lines: &[&str]) -> f64 {
        if target_lines.len() != self.search_lines.len() || self.search_lines.is_empty() {
            return 0.0;
        }

        let last_idx = self.search_lines.len() - 1;

        // All non-final lines must match exactly after trimming leading whitespace.
        let prefix_lines_exact = self.search_lines[..last_idx]
            .iter()
            .zip(&target_lines[..last_idx])
            .all(|(s, t)| *s == t.trim_start());
        if !prefix_lines_exact {
            return 0.0;
        }

        // Final line: trimmed search must be a non-empty strict prefix of the trimmed target.
        // Strict prefix (target longer) avoids redundant matches that Exact/IndentAgnostic already
        // handle, and prevents empty-prefix false positives.
        if is_strict_trimmed_prefix(self.search_lines[last_idx], target_lines[last_idx]) {
            1.0
        } else {
            0.0
        }
    }
}

impl fmt::Display for PrefixTailMatch<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{MakePrefixTailMatch}")
    }
}

#[derive(Clone)]
struct MakePrefixTailMatch;

impl fmt::Display for MakePrefixTailMatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Prefix Tail")
    }
}

impl MakeScorer for MakePrefixTailMatch {
    type ScorerInstance<'a> = PrefixTailMatch<'a>;

    fn for_search<'a>(&self, search_text: &'a str) -> Self::ScorerInstance<'a> {
        PrefixTailMatch::new(search_text)
    }
}

/// Check if an array of lines matches a search string.
/// This currently uses fuzzy matching.
fn section_similarity(search_text: &str, target_lines: &[&str]) -> f64 {
    let window_text = target_lines.join("\n");
    jaro_winkler(search_text, &window_text)
}

const SECTION_MATCH_THRESHOLD: f64 = 0.9;
cfg_if! {
    if #[cfg(any(test, feature = "test-util"))] {
        mod test_util {
            use super::*;

            #[derive(Clone, Copy, Debug, Eq, PartialEq)]
            pub enum SearchReplaceMatchStrategy {
                Exact,
                IndentationAgnostic,
                JaroWinkler,
            }

            impl fmt::Display for SearchReplaceMatchStrategy {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    match self {
                        SearchReplaceMatchStrategy::Exact => MakeExactMatch.fmt(f),
                        SearchReplaceMatchStrategy::IndentationAgnostic => {
                            MakeIndentationAgnosticMatch.fmt(f)
                        }
                        SearchReplaceMatchStrategy::JaroWinkler => MakeJaroWinklerMatch.fmt(f),
                    }
                }
            }

            pub fn match_search_replace_block(
                search: &str,
                line_range: Option<Range<usize>>,
                file_content: &[&str],
                strategy: SearchReplaceMatchStrategy,
            ) -> Option<Range<usize>> {
                match strategy {
                    SearchReplaceMatchStrategy::Exact => match_diff(
                        search,
                        line_range,
                        file_content,
                        SECTION_MATCH_THRESHOLD,
                        MakeExactMatch,
                    ),
                    SearchReplaceMatchStrategy::IndentationAgnostic => match_diff(
                        search,
                        line_range,
                        file_content,
                        SECTION_MATCH_THRESHOLD,
                        MakeIndentationAgnosticMatch,
                    ),
                    SearchReplaceMatchStrategy::JaroWinkler => match_diff(
                        search,
                        line_range,
                        file_content,
                        SECTION_MATCH_THRESHOLD,
                        MakeJaroWinklerMatch,
                    ),
                }
            }
        }

        pub use test_util::{match_search_replace_block, SearchReplaceMatchStrategy};
    }
}

/// Given a search block and a scoring function, find the most likely matching range of lines from target file contents.
///
/// The result will be 1-indexed to match how we represent line numbers. An empty search string
/// corresponds to the line range 0..0, which is how we represent prepending to the file.
fn match_diff<S: MakeScorer>(
    search: &str,
    line_range: Option<Range<usize>>,
    file_content: &[&str],
    threshold: f64,
    factory: S,
) -> Option<Range<usize>> {
    let search_length = lines(search).count();
    let scorer = factory.for_search(search);

    // If we could parse a line range, check if it's approximately correct.
    if let Some(Range { start, end }) = &line_range {
        let search_start = start.saturating_sub(2);
        let search_end = (end + 2).min(file_content.len());
        if search_start <= search_end {
            let local_lines = &file_content[search_start..search_end];
            let local_matches = score_matches(
                local_lines,
                search_length,
                threshold,
                line_range
                    .clone()
                    .map(|range| range.start - search_start..range.end - search_start),
                &scorer,
            );
            if let Some(local_match) = local_matches.first() {
                let local_start = local_match.start_line + search_start;
                let local_end = local_match.end_line + search_start;
                log::debug!(
                    "Line numbers approximately correct. Parsed: {start}-{end} Matched {local_start}-{local_end} with {factory}",
                );
                return Some(local_start..local_end);
            }
        }
    }

    // Otherwise, search through the entire file content.
    let matches = score_matches(
        file_content,
        search_length,
        threshold,
        line_range.clone(),
        &scorer,
    );
    if let Some(m) = matches.first() {
        match line_range {
            Some(Range { start, end }) => {
                log::debug!(
                    "Mismatched line numbers fixed by matching. Parsed: {start}-{end} Matched {}-{} with {factory}",
                    m.start_line,
                    m.end_line
                );
            }
            None => {
                log::debug!("Missing line numbers fixed by matching with {factory}");
            }
        }
        return Some(m.start_line..m.end_line);
    }
    None
}

/// Given a search string, try to parse the line number range from it.
/// If the string is empty, return 0..0, because that's how we instruct the LLM to add something to the
/// start of the file.
///
/// These line numbers are 1-indexed, to match the line numbers in the search string.
/// An empty string returns the range 0..0, because that's how we instruct the LLM to add something to the
/// start of the file.
pub fn parse_line_numbers(search: &str) -> (Option<Range<usize>>, String) {
    let parsed: Vec<_> = search.lines().map(parse_line_number).collect();
    if parsed.is_empty() {
        (Some(0..0), search.to_string())
    } else {
        let starting_index = parsed.first().expect("We checked there is a line").0;
        let ending_index = parsed.last().expect("We checked there is a line").0;
        match (starting_index, ending_index) {
            (Some(start), Some(end)) => (
                Some(start..end + 1),
                parsed
                    .iter()
                    .map(|(_, line)| *line)
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => (None, search.to_string()),
        }
    }
}

fn parse_line_number(search: &str) -> (Option<usize>, &str) {
    if let Some((line_number, line)) = LINE_NUMBER_PARSE
        .captures(search)
        .and_then(|m| try_tuple2(m.get(1), m.get(2)))
        .and_then(|(a, b)| Some((a.as_str().parse::<usize>().ok()?, b.as_str())))
    {
        (Some(line_number), line)
    } else {
        (None, search)
    }
}

/// A [`Scorer`] that uses Jaro-Winkler similarity to compare target lines with search text.
///
/// Removes indentation from each line and then concatenates them with newline characters and
/// compares the full text.
struct JaroWinklerScorer {
    search_text: String,
}

impl JaroWinklerScorer {
    fn new(search_text: &str) -> Self {
        let search_text = lines(search_text)
            .map(str::trim_start)
            .collect_vec()
            .join("\n");
        Self { search_text }
    }
}

impl Scorer for JaroWinklerScorer {
    fn score(&self, target_lines: &[&str]) -> f64 {
        let target_text = target_lines
            .iter()
            .map(|line| line.trim_start())
            .collect_vec()
            .join("\n");
        jaro_winkler(&self.search_text, &target_text)
    }
}

impl fmt::Display for JaroWinklerScorer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{MakeJaroWinklerMatch}")
    }
}

#[derive(Clone)]
struct MakeJaroWinklerMatch;

impl fmt::Display for MakeJaroWinklerMatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Jaro-Winkler")
    }
}

impl MakeScorer for MakeJaroWinklerMatch {
    type ScorerInstance<'a> = JaroWinklerScorer;

    fn for_search<'a>(&self, search_text: &'a str) -> Self::ScorerInstance<'a> {
        JaroWinklerScorer::new(search_text)
    }
}

fn try_tuple2<A, B>(a: Option<A>, b: Option<B>) -> Option<(A, B)> {
    match (a, b) {
        (Some(a), Some(b)) => Some((a, b)),
        _ => None,
    }
}

/// Finds the line range in the file that matches the V4A edit's context.
/// Returns 1-indexed line range.
///
/// First attempts exact matching, then falls back to indentation-agnostic matching,
/// and finally to JaroWinkler fuzzy matching.
fn find_v4a_match(edit: &V4AHunk, file_lines: &[&str]) -> Option<Range<usize>> {
    let pre_context_lines: Vec<&str> = edit.pre_context.lines().collect();
    let old_lines: Vec<&str> = edit.old.lines().collect();
    let post_context_lines: Vec<&str> = edit.post_context.lines().collect();

    // If we have change_context (class/function markers), use them to narrow the search start
    let search_start = if !edit.change_context.is_empty() {
        find_change_context_start(&edit.change_context, file_lines)?
    } else {
        0
    };

    let search_lines = &file_lines[search_start..];

    // Now search for the pattern: pre_context + old + post_context
    let pattern_length = pre_context_lines.len() + old_lines.len() + post_context_lines.len();
    if pattern_length == 0 {
        return Some((search_start + 1)..(search_start + 1));
    }
    if pattern_length > search_lines.len() {
        return None;
    }

    // Combine all three sections into a single search text for the scorers
    let combined_search = [
        pre_context_lines.as_slice(),
        old_lines.as_slice(),
        post_context_lines.as_slice(),
    ]
    .concat()
    .join("\n");

    // Try exact match first
    if let Some(range) = match_diff(
        &combined_search,
        None, // No expected line range
        search_lines,
        1.0, // Exact match requires perfect score
        MakeExactMatch,
    ) {
        return calculate_old_range(search_start, range, &pre_context_lines, &old_lines);
    }

    // Try indentation-agnostic match
    if let Some(range) = match_diff(
        &combined_search,
        None,
        search_lines,
        1.0,
        MakeIndentationAgnosticMatch,
    ) {
        log::debug!("V4A match found using indentation-agnostic matching");
        return calculate_old_range(search_start, range, &pre_context_lines, &old_lines);
    }

    // Try JaroWinkler fuzzy match as last resort
    if let Some(range) = match_diff(
        &combined_search,
        None,
        search_lines,
        SECTION_MATCH_THRESHOLD,
        MakeJaroWinklerMatch,
    ) {
        log::debug!("V4A match found using JaroWinkler fuzzy matching");
        return calculate_old_range(search_start, range, &pre_context_lines, &old_lines);
    }

    None
}

/// Calculate the line range for the old content (or insertion point if old is empty).
/// Returns 1-indexed line range.
fn calculate_old_range(
    search_start: usize,
    matched_range: Range<usize>,
    pre_context_lines: &[&str],
    old_lines: &[&str],
) -> Option<Range<usize>> {
    // matched_range.start is 1-indexed and points to the start of the combined match
    // We need to skip past the pre_context to get to where the old content is
    let old_start = search_start + matched_range.start - 1 + pre_context_lines.len();
    let old_end = old_start + old_lines.len();

    // Return 1-indexed range
    Some((old_start + 1)..(old_end + 1))
}

/// Finds the starting line for searching based on change context markers.
/// Change context entries are class/function signatures that help narrow where to start searching.
/// Returns 0-indexed line number.
fn find_change_context_start(change_context: &[String], file_lines: &[&str]) -> Option<usize> {
    let mut current_pos = 0;

    // Find nested scopes by looking for each subsequent marker.
    for marker in change_context {
        if marker.is_empty() {
            continue;
        }

        let relative_match = file_lines[current_pos..]
            .iter()
            .position(|line| line.trim_start().starts_with(marker.trim()))?;
        current_pos = current_pos + relative_match + 1;
    }

    // Return the position after the last change context marker.
    Some(current_pos)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
