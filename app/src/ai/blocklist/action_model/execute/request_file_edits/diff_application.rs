//! Module containing helper code to apply suggested diffs from an LLM
//! to a set of files on the user's filesystem.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;

use ai::diff_validation::{
    fuzzy_match_diffs, fuzzy_match_v4a_diffs, AIRequestedCodeDiff, DiffDelta, DiffMatchFailures,
    DiffType, ParsedDiff, SearchAndReplace, V4AHunk,
};
use itertools::Itertools;
use vec1::Vec1;
use warpui::r#async::executor::Background;

use super::telemetry::{
    DiffInvalidFileEvent, DiffMatchFailedEvent, MissingLineNumbersEvent,
    RequestFileEditsTelemetryEvent,
};
use crate::ai::agent::{AIIdentifiers, FileEdit};
use crate::ai::blocklist::SessionContext;
use crate::ai::paths::host_native_absolute_path;
use crate::auth::auth_state::AuthState;
use crate::{safe_debug, safe_warn, send_telemetry_on_executor};

/// Result of reading a file from disk or a remote server.
///
/// This is the common currency between the local (`std::fs`) and remote
/// (`RemoteServerClient`) file-reading paths so that all diff application
/// logic can be shared.
pub(crate) enum FileReadResult {
    /// The file was found and its full content is available.
    Found(String),
    /// The file does not exist.
    NotFound,
    /// The file could not be read for a reason other than "not found".
    ReadError(String),
}

impl From<std::io::Result<String>> for FileReadResult {
    fn from(result: std::io::Result<String>) -> Self {
        match result {
            Ok(content) => FileReadResult::Found(content),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => FileReadResult::NotFound,
            Err(err) => FileReadResult::ReadError(format!("{err:#}")),
        }
    }
}

/// Errors that can occur while applying a diff.
#[derive(Debug)]
pub(crate) enum DiffApplicationError {
    /// Some diffs could not be matched against the file content.
    UnmatchedDiffs {
        file: String,
        match_failures: DiffMatchFailures,
    },
    /// The file that a diff should be applied to did not exist.
    MissingFile {
        file: String,
    },
    /// The file could not be read (I/O error, permissions, remote connectivity, etc.).
    ReadFailed {
        file: String,
        // TODO(CODE-353): Display I/O errors to the user, since they may be able to fix them.
        #[expect(dead_code)]
        message: String,
    },
    /// A file that was supposed to be new already exists.
    AlreadyExists {
        file: String,
    },
    /// The diff contained multiple attempts to create the same file.
    MultipleFileCreation {
        file: String,
    },
    /// No diffs could be applied.
    EmptyDiff,
    MutatedDeletedFile {
        file: String,
    },
    MultipleFileRenames {
        file: String,
    },
    /// File read/write operations are not available on this remote session.
    /// This covers both connection-dropped and unsupported SSH session types.
    RemoteFileOperationsUnsupported,
}

impl DiffApplicationError {
    /// Format this error for inclusion in the agent conversation. The error should help the LLM
    /// retry and generate a valid diff.
    fn to_conversation_message(&self) -> String {
        match self {
            DiffApplicationError::UnmatchedDiffs {
                file,
                match_failures,
            } => {
                use std::fmt::Write;
                let mut message = String::new();
                if match_failures.fuzzy_match_failures > 0 {
                    let _ = write!(message, "Could not apply all diffs to {file}.");
                }

                if match_failures.noop_deltas > 0 {
                    if !message.is_empty() {
                        message.push(' ');
                    }
                    let _ = write!(message, "The changes to {file} were already made.");
                }
                message
            }
            DiffApplicationError::MissingFile { file } => {
                format!("{file} does not exist. Is the path correct?")
            }
            DiffApplicationError::AlreadyExists { file } => {
                format!("Could not create {file} because it already exists.")
            }
            DiffApplicationError::ReadFailed { file, .. } => {
                format!("Could not read {file}")
            }
            DiffApplicationError::MultipleFileCreation { file } => {
                format!("There can only be one attempt to create {file}.")
            }
            DiffApplicationError::MultipleFileRenames { file } => {
                format!("There can only be one attempt to rename {file}.")
            }
            DiffApplicationError::MutatedDeletedFile { file } => {
                format!("Could not mutate a deleted file {file}.")
            }
            DiffApplicationError::EmptyDiff => "No diffs could be applied.".to_string(),
            DiffApplicationError::RemoteFileOperationsUnsupported => {
                "The file read/edit tool is not available on this remote session. Try using a different tool.".to_string()
            }
        }
    }

    /// Format a list of errors for inclusion in the agent conversation.
    pub fn error_for_conversation(errors: &Vec1<DiffApplicationError>) -> String {
        if errors.len() == 1 {
            errors.first().to_conversation_message()
        } else {
            errors
                .iter()
                .format_with("\n", |err, f| {
                    f(&format_args!("* {}", err.to_conversation_message()))
                })
                .to_string()
        }
    }
}

/// Given a list of suggested edits from the server API, parse it into applicable diffs to be shown
/// to the user as a series of code diffs.
///
/// * Search-and-replace diffs are matched to existing files on disk
/// * Created files are expected to not already exist
///
/// Errors are reported as telemetry, and also returned for display.
pub(crate) async fn apply_edits<F, Fut>(
    edits: Vec<FileEdit>,
    session_context: &SessionContext,
    ai_identifiers: &AIIdentifiers,
    background_executor: Arc<Background>,
    auth_state: Arc<AuthState>,
    passive_diff: bool,
    read_file: F,
) -> Result<Vec<AIRequestedCodeDiff>, Vec1<DiffApplicationError>>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = FileReadResult>,
{
    let result = apply_edits_internal(edits, session_context, &read_file).await;

    // Send telemetry for all diff application errors.

    // Count of attempts to edit a file that doesn't exist or create a file that already exists.
    let mut invalid_file_count = 0;

    for error in result.errors.iter() {
        match error {
            DiffApplicationError::UnmatchedDiffs { match_failures, .. } => {
                send_telemetry_on_executor!(
                    auth_state,
                    RequestFileEditsTelemetryEvent::DiffMatchFailed(DiffMatchFailedEvent {
                        identifiers: ai_identifiers.clone(),
                        failures: *match_failures,
                        passive_diff,
                    }),
                    background_executor
                );
            }
            DiffApplicationError::MissingFile { .. }
            | DiffApplicationError::ReadFailed { .. }
            | DiffApplicationError::AlreadyExists { .. }
            | DiffApplicationError::MultipleFileCreation { .. }
            | DiffApplicationError::MutatedDeletedFile { .. }
            | DiffApplicationError::MultipleFileRenames { .. }
            | DiffApplicationError::RemoteFileOperationsUnsupported => {
                invalid_file_count += 1;
            }
            DiffApplicationError::EmptyDiff => {}
        }
    }

    if invalid_file_count > 0 {
        send_telemetry_on_executor!(
            auth_state,
            RequestFileEditsTelemetryEvent::DiffInvalidFile(DiffInvalidFileEvent {
                count: invalid_file_count,
                identifiers: ai_identifiers.clone(),
                passive_diff,
            }),
            background_executor
        );
    }

    // Send telemetry for any warnings, which don't necessarily prevent diff application.

    let total_missing_line_numbers: u8 = result
        .warnings
        .iter()
        .map(|warning| match warning {
            DiffWarning::MissingLineNumbers { count, .. } => *count,
        })
        .sum();

    if total_missing_line_numbers > 0 {
        send_telemetry_on_executor!(
            auth_state,
            RequestFileEditsTelemetryEvent::MissingLineNumbers(MissingLineNumbersEvent {
                identifiers: ai_identifiers.clone(),
                count: total_missing_line_numbers,
                passive_diff,
            }),
            background_executor
        );
    }

    match Vec1::try_from_vec(result.errors) {
        Ok(errors) => Err(errors),
        Err(vec1::Size0Error) => Ok(result.diffs),
    }
}

/// Warnings are issues that don't necessarily prevent diff application, but indicate an unexpected
/// response from the LLM.
///
/// For example, we expect the search string in a diff to include line numbers, but can rely on
/// fuzzy matching if they're missing.
#[derive(Debug, Clone)]
pub enum DiffWarning {
    /// Search blocks that are missing line numbers.
    MissingLineNumbers { count: u8 },
}

#[derive(Default)]
struct DiffResult {
    /// All successfully-applied diffs, grouped by file.
    diffs: Vec<AIRequestedCodeDiff>,
    /// All errors that occurred while applying diffs.
    errors: Vec<DiffApplicationError>,
    /// All warnings that occurred while applying diffs.
    warnings: Vec<DiffWarning>,
}

/// You generally want to use `apply_edits`, however, if you don't want to report telemetry or be as
/// strict, this is available.  For example, we use this when debug importing conversations.
async fn apply_edits_internal<F, Fut>(
    edits: Vec<FileEdit>,
    session_context: &SessionContext,
    read_file: &F,
) -> DiffResult
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = FileReadResult>,
{
    let mut search_replace_deltas: HashMap<String, Vec<SearchAndReplace>> = HashMap::new();
    let mut v4a_deltas: HashMap<String, Vec<V4AHunk>> = HashMap::new();
    let mut new_files: HashMap<String, String> = HashMap::new();
    let mut deleted_files: HashSet<String> = HashSet::new();
    let mut file_renames: HashMap<String, String> = HashMap::new();
    let mut result = DiffResult::default();

    for edit in edits {
        match edit {
            FileEdit::Edit(diff) => {
                let Some(file_path) = diff.file().cloned() else {
                    continue;
                };

                match diff {
                    ParsedDiff::StrReplaceEdit { .. } => {
                        let deltas = search_replace_deltas.entry(file_path).or_default();
                        if let Ok(d) = diff.try_into() {
                            deltas.push(d);
                        }
                    }
                    ParsedDiff::V4AEdit { hunks, move_to, .. } => {
                        v4a_deltas
                            .entry(file_path.clone())
                            .or_default()
                            .extend(hunks);

                        if let Some(move_to) = move_to {
                            if file_renames.contains_key(&file_path) {
                                result
                                    .errors
                                    .push(DiffApplicationError::MultipleFileRenames {
                                        file: file_path.clone(),
                                    });
                                continue;
                            }
                            file_renames.insert(file_path, move_to);
                        }
                    }
                };
            }
            FileEdit::Create { file, content } => {
                let Some(file_path) = file else { continue };

                match new_files.entry(file_path) {
                    Entry::Occupied(entry) => {
                        result
                            .errors
                            .push(DiffApplicationError::MultipleFileCreation {
                                file: entry.key().clone(),
                            });
                        continue;
                    }
                    Entry::Vacant(entry) => {
                        let Some(content) = content else {
                            continue;
                        };
                        entry.insert(content);
                    }
                }
            }
            FileEdit::Delete { file } => {
                // Deleting the same file multiple times is valid, I guess...
                let Some(file_path) = file else { continue };
                deleted_files.insert(file_path);
            }
        }
    }

    let search_replace_files: HashSet<String> = search_replace_deltas.keys().cloned().collect();
    let v4a_files: HashSet<String> = v4a_deltas.keys().cloned().collect();
    let new_file_paths: HashSet<String> = new_files.keys().cloned().collect();
    let deleted_file_paths: HashSet<String> = deleted_files.iter().cloned().collect();

    for (file_path, deltas) in search_replace_deltas {
        // If a file is also being explicitly created/deleted, skip applying edits to avoid
        // producing redundant errors (e.g. MissingFile alongside MultipleFileCreation).
        if new_file_paths.contains(&file_path) || deleted_file_paths.contains(&file_path) {
            continue;
        }

        apply_search_replace(file_path, deltas, session_context, read_file, &mut result).await;
    }

    for (file_path, deltas) in v4a_deltas {
        if new_file_paths.contains(&file_path) || deleted_file_paths.contains(&file_path) {
            continue;
        }

        let rename_to = file_renames.get(&file_path).cloned();
        apply_v4a_update(
            file_path,
            deltas,
            rename_to,
            session_context,
            read_file,
            &mut result,
        )
        .await;
    }

    for (file, content) in new_files {
        if search_replace_files.contains(&file)
            || v4a_files.contains(&file)
            || file_renames.contains_key(&file)
        {
            result
                .errors
                .push(DiffApplicationError::MultipleFileCreation { file });
        } else {
            apply_create_file(file, content, session_context, read_file, &mut result).await;
        }
    }

    for file in deleted_files {
        if new_file_paths.contains(&file)
            || search_replace_files.contains(&file)
            || v4a_files.contains(&file)
            || file_renames.contains_key(&file)
        {
            result
                .errors
                .push(DiffApplicationError::MutatedDeletedFile { file });
        } else {
            apply_delete_file(file, session_context, read_file, &mut result).await;
        }
    }

    result
}

/// Converts a file-creation request into a diff.
async fn apply_create_file<F, Fut>(
    file_path: String,
    content: String,
    session_context: &SessionContext,
    read_file: &F,
    result: &mut DiffResult,
) where
    F: Fn(String) -> Fut,
    Fut: Future<Output = FileReadResult>,
{
    let absolute_path = host_native_absolute_path(
        &file_path,
        session_context.shell(),
        session_context.current_working_directory(),
    );

    match read_file(absolute_path.clone()).await {
        FileReadResult::Found(_) => {
            safe_warn!(
                safe: ("Agent Code tried to create a file that already exists"),
                full: ("Agent Code tried to create a file that already exists: {absolute_path:?}")
            );
            result
                .errors
                .push(DiffApplicationError::AlreadyExists { file: file_path });
        }
        FileReadResult::NotFound => {
            result.diffs.push(AIRequestedCodeDiff {
                file_name: file_path,
                diff_type: DiffType::creation(content),
                failures: None,
                original_content: String::new(),
            });
        }
        FileReadResult::ReadError(err) => {
            safe_warn!(
                safe: ("Unable to check if file exists for Agent Code: {err}"),
                full: ("Unable to check if file exists for Agent Code: {absolute_path:?} {err}")
            );
            result.errors.push(DiffApplicationError::ReadFailed {
                file: file_path,
                message: err,
            });
        }
    }
}

async fn apply_delete_file<F, Fut>(
    file_path: String,
    session_context: &SessionContext,
    read_file: &F,
    result: &mut DiffResult,
) where
    F: Fn(String) -> Fut,
    Fut: Future<Output = FileReadResult>,
{
    let absolute_path = host_native_absolute_path(
        &file_path,
        session_context.shell(),
        session_context.current_working_directory(),
    );

    match read_file(absolute_path.clone()).await {
        FileReadResult::Found(file_content) => {
            let num_lines = file_content.lines().count();
            result.diffs.push(AIRequestedCodeDiff {
                file_name: file_path,
                diff_type: DiffType::deletion(num_lines),
                failures: None,
                original_content: file_content,
            })
        }
        FileReadResult::NotFound => {
            result
                .errors
                .push(DiffApplicationError::MissingFile { file: file_path });
        }
        FileReadResult::ReadError(err) => {
            safe_warn!(
                safe: ("Unable to read file for Agent Code: {err}"),
                full: ("Unable to read file {absolute_path:?} for Agent Code: {err}")
            );
            result.errors.push(DiffApplicationError::ReadFailed {
                file: file_path,
                message: err,
            });
        }
    }
}

/// Applies a set of search-and-replace diffs to a file.
async fn apply_search_replace<F, Fut>(
    file_path: String,
    deltas: Vec<SearchAndReplace>,
    session_context: &SessionContext,
    read_file: &F,
    result: &mut DiffResult,
) where
    F: Fn(String) -> Fut,
    Fut: Future<Output = FileReadResult>,
{
    let absolute_path = host_native_absolute_path(
        &file_path,
        session_context.shell(),
        session_context.current_working_directory(),
    );

    match read_file(absolute_path.clone()).await {
        FileReadResult::NotFound => {
            match deltas.into_iter().exactly_one() {
                Ok(SearchAndReplace { search, replace }) => {
                    if search.is_empty() {
                        result.diffs.push(AIRequestedCodeDiff {
                            file_name: file_path,
                            diff_type: DiffType::creation(replace),
                            failures: None,
                            original_content: String::new(),
                        })
                    } else {
                        safe_warn!(
                            safe: ("Suggested non-empty diff on non-existent file"),
                            full: ("Suggested non-empty diff on non-existent file: {absolute_path:?}")
                        );
                        // A non-empty search block on a non-existent file indicates that the
                        // LLM likely got the path wrong, and is not trying to create a new file.
                        result
                            .errors
                            .push(DiffApplicationError::MissingFile { file: file_path });
                    }
                }
                Err(err) => {
                    safe_warn!(
                        safe: ("Suggested {} diffs on non-existent file", err.len()),
                        full: ("Suggested {} diffs on non-existent file: {absolute_path:?}", err.len())
                    );
                    // Multiple diffs on a non-existent file indicate that the LLM likely got
                    // the path wrong, and is not trying to create a new file.
                    result
                        .errors
                        .push(DiffApplicationError::MissingFile { file: file_path });
                }
            }
        }
        FileReadResult::ReadError(err) => {
            safe_warn!(
                safe: ("Unable to read file for Agent Code: {err}"),
                full: ("Unable to read file {absolute_path:?} for Agent Code: {err}")
            );
            result.errors.push(DiffApplicationError::ReadFailed {
                file: file_path,
                message: err,
            });
        }
        FileReadResult::Found(file_content) => {
            safe_debug!(
                safe: ("Matching diffs"),
                full: ("Matching diffs for: {file_path:?}")
            );
            let fuzzy_match_diffs = fuzzy_match_diffs(&file_path, &deltas, file_content);

            // Add warnings from the failure info - the `DiffMatchFailures` type includes both
            // fatal and non-fatal errors.
            if let Some(failures) = fuzzy_match_diffs.failures.as_ref() {
                if failures.missing_line_numbers > 0 {
                    result.warnings.push(DiffWarning::MissingLineNumbers {
                        count: failures.missing_line_numbers,
                    });
                }
            }

            if fuzzy_match_diffs.warrants_failure() {
                if let Some(failures) = fuzzy_match_diffs.failures.as_ref() {
                    safe_warn!(
                        safe: ("Failure(s) applying diff: {failures:?}"),
                        full: ("Failure(s) applying diff for {absolute_path:?}: {failures:?}")
                    );
                    result.errors.push(DiffApplicationError::UnmatchedDiffs {
                        file: file_path.clone(),
                        match_failures: *failures,
                    });
                }
            }
            result.diffs.push(fuzzy_match_diffs);
        }
    }
}

async fn apply_v4a_update<F, Fut>(
    file_path: String,
    deltas: Vec<V4AHunk>,
    rename_to: Option<String>,
    session_context: &SessionContext,
    read_file: &F,
    result: &mut DiffResult,
) where
    F: Fn(String) -> Fut,
    Fut: Future<Output = FileReadResult>,
{
    let absolute_path = host_native_absolute_path(
        &file_path,
        session_context.shell(),
        session_context.current_working_directory(),
    );

    let file_content = match read_file(absolute_path.clone()).await {
        FileReadResult::NotFound => {
            safe_warn!(
                safe: ("V4A edits requested on non-existent file"),
                full: ("V4A edits requested on non-existent file: {absolute_path:?}")
            );
            result
                .errors
                .push(DiffApplicationError::MissingFile { file: file_path });
            return;
        }
        FileReadResult::ReadError(err) => {
            safe_warn!(
                safe: ("Unable to read file for Agent Code: {err}"),
                full: ("Unable to read file {absolute_path:?} for Agent Code: {err}")
            );
            result.errors.push(DiffApplicationError::ReadFailed {
                file: file_path,
                message: err,
            });
            return;
        }
        FileReadResult::Found(content) => content,
    };

    safe_debug!(
        safe: ("Matching V4A diffs"),
        full: ("Matching V4A diffs for: {file_path:?}")
    );

    // Check if we're renaming to an existing file.
    let rename_target_content = if let Some(target) = &rename_to {
        let target_absolute = host_native_absolute_path(
            target,
            session_context.shell(),
            session_context.current_working_directory(),
        );
        match read_file(target_absolute.clone()).await {
            FileReadResult::Found(content) => Some(content),
            FileReadResult::NotFound => None,
            FileReadResult::ReadError(err) => {
                safe_warn!(
                    safe: ("Unable to read rename target file: {err}"),
                    full: ("Unable to read rename target file {target_absolute:?}: {err}")
                );
                result.errors.push(DiffApplicationError::ReadFailed {
                    file: target.clone(),
                    message: err,
                });
                return;
            }
        }
    } else {
        None
    };

    if let Some(target_content) = rename_target_content {
        // Renaming A to B where B already exists:
        // 1. Create deletion for A.
        // 2. Replace all of B with A.
        // 3. Create update for B that applies the original diff to A.
        let rename_target = rename_to.unwrap();

        // First, match the V4A diffs against the source file (without rename)
        let source_diffs = fuzzy_match_v4a_diffs(&file_path, &deltas, None, file_content.clone());
        if source_diffs.warrants_failure() {
            if let Some(failures) = source_diffs.failures.as_ref() {
                safe_warn!(
                    safe: ("Failure(s) applying V4A diff: {failures:?}"),
                    full: ("Failure(s) applying V4A diff for {absolute_path:?}: {failures:?}")
                );
                result.errors.push(DiffApplicationError::UnmatchedDiffs {
                    file: file_path.clone(),
                    match_failures: *failures,
                });
            }
            return;
        }

        let target_num_lines = target_content.lines().count();
        let source_num_lines = file_content.lines().count();

        // Reuse the copy that fuzzy_match_v4a_diffs already
        // made for its original_content field, so we only
        // allocate once instead of cloning file_content again.
        let deletion_original_content = source_diffs.original_content;

        // Replace all of B's content with A's content.
        // Moves file_content — no clone needed.
        let mut new_deltas = Vec::new();
        let replacement_range = if target_num_lines == 0 {
            0..0
        } else {
            1..(target_num_lines + 1)
        };
        new_deltas.push(DiffDelta {
            replacement_line_range: replacement_range,
            insertion: file_content,
        });

        // Apply the original diff to A.
        if let DiffType::Update {
            deltas: source_deltas,
            ..
        } = source_diffs.diff_type
        {
            new_deltas.extend(source_deltas);
        }

        // Create deletion diff for source file A
        result.diffs.push(AIRequestedCodeDiff {
            file_name: file_path.clone(),
            diff_type: DiffType::deletion(source_num_lines),
            failures: None,
            original_content: deletion_original_content,
        });

        result.diffs.push(AIRequestedCodeDiff {
            file_name: rename_target,
            diff_type: DiffType::update(new_deltas, None),
            failures: None,
            original_content: target_content,
        });
    } else {
        // Normal case: no rename or rename to non-existent file
        let diffs = fuzzy_match_v4a_diffs(&file_path, &deltas, rename_to, file_content);
        if diffs.warrants_failure() {
            if let Some(failures) = diffs.failures.as_ref() {
                safe_warn!(
                    safe: ("Failure(s) applying V4A diff: {failures:?}"),
                    full: ("Failure(s) applying V4A diff for {absolute_path:?}: {failures:?}")
                );
                result.errors.push(DiffApplicationError::UnmatchedDiffs {
                    file: file_path.clone(),
                    match_failures: *failures,
                });
            }
        }
        result.diffs.push(diffs);
    }
}

#[cfg(test)]
#[path = "diff_application_tests.rs"]
mod tests;
