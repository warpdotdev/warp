use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::anyhow;
use arborium::tree_sitter::{Parser, Query, QueryCursor, Tree};
use futures::channel::oneshot;
use ignore::gitignore::Gitignore;
use itertools::Itertools;
use rayon::prelude::*;
use repo_metadata::RepositoryUpdate;
use repo_metadata::entry::{BudgetExceededBehavior, IgnoredPathStrategy, is_file_parsable};
use streaming_iterator::StreamingIterator;
use syntax_tree::TextSlice;
use warp_errors::report_error;
use warp_util::standardized_path::StandardizedPath;

use crate::index::file_outline::{FileOutline, Outline, Symbol};
use crate::index::{Entry, FileId, FileMetadata, THREADPOOL};

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use crate::index::matches_gitignores;
    }
}
const MAX_OUTLINE_TOTAL_BYTES: usize = 256 * 1024 * 1024;
const MAX_SYMBOL_COMMENT_BYTES: usize = 512;
const MAX_SYMBOL_COMMENT_LINES: usize = 8;

/// Given a repo path, try to build its outline. An outline is a list of all its files and the symbols
/// of interest from each file.
pub async fn build_outline(
    path: &Path,
    max_num_files_limit: Option<usize>,
) -> anyhow::Result<Outline> {
    const MAX_DEPTH: usize = 200;
    let mut gitignores = vec![];

    // Add global gitignore, if it exists
    let (global_gitignore, _) = Gitignore::global();
    if !global_gitignore.is_empty() {
        gitignores.push(Arc::new(global_gitignore));
    }

    let gitignore_path = path.join(".gitignore");
    if gitignore_path.exists() {
        let (gitignore, _) = Gitignore::new(gitignore_path);
        gitignores.push(Arc::new(gitignore));
    }

    // First traverse the repo path to retrieve all files we want to parse.
    let mut files = Vec::new();
    let mut remaining_file_quotas = max_num_files_limit;
    let entry = Entry::build_tree(
        path,
        &mut files,
        &mut gitignores,
        remaining_file_quotas.as_mut(),
        MAX_DEPTH,
        0,
        &IgnoredPathStrategy::Exclude, // override_ignore_for_files
        BudgetExceededBehavior::StopAndLazyLoad,
    )
    .await?;
    files.sort_by(|left, right| left.path.as_str().cmp(right.path.as_str()));

    let (sender, receiver) = oneshot::channel();

    let Some(pool) = THREADPOOL.as_ref() else {
        return Err(anyhow!("No threadpool exists for outline generation."));
    };

    pool.spawn(move || {
        let result = pool.install(|| {
            files
                .par_iter()
                .map(|metadata| {
                    let outline = parse_file_outline(&metadata.path.to_local_path_lossy())
                        .ok()
                        .unwrap_or_default();
                    (metadata.file_id, outline)
                })
                .collect::<Vec<_>>()
        });

        if sender.send(result).is_err() {
            report_error!(
                anyhow!("Could not send result of outline generation to background thread"),
                warp_errors::ReportErrorLogMode::OncePerRun
            );
        }
    });

    let (file_id_to_outline, retained_outline_bytes) =
        retain_file_outlines(receiver.await?, MAX_OUTLINE_TOTAL_BYTES);

    Ok(Outline {
        root: entry,
        file_id_to_outline,
        retained_outline_bytes,
        remaining_file_quota: remaining_file_quotas,
        gitignores,
    })
}

fn retained_file_outline_bytes(outline: &FileOutline) -> usize {
    std::mem::size_of::<FileId>().saturating_add(outline.retained_bytes())
}

fn retain_file_outlines(
    outlines: Vec<(FileId, FileOutline)>,
    max_retained_bytes: usize,
) -> (HashMap<FileId, FileOutline>, usize) {
    let mut retained_outlines = HashMap::new();
    let mut retained_bytes = 0usize;

    for (file_id, outline) in outlines {
        let outline_bytes = retained_file_outline_bytes(&outline);
        let next_retained_bytes = retained_bytes.saturating_add(outline_bytes);
        if next_retained_bytes > max_retained_bytes {
            break;
        }

        retained_outlines.insert(file_id, outline);
        retained_bytes = next_retained_bytes;
    }

    (retained_outlines, retained_bytes)
}

impl Outline {
    /// Update this outline in-place with a set of changed files. This is asynchronous because it
    /// requires re-parsing modified files.
    pub async fn update(&mut self, outline_update: RepositoryUpdate) {
        let RepositoryUpdate {
            added,
            modified,
            deleted,
            moved,
            ..
        } = outline_update;

        let mut files_metadata = vec![];
        let mut files_metadata_to_remove = vec![];

        for target_file in deleted
            .into_iter()
            .chain(moved.values().cloned())
            .filter(|target_file| !target_file.is_ignored)
        {
            if let Some(metadata) = self.root.remove(&target_file.path) {
                files_metadata_to_remove.push(metadata);
                if let Some(remaining_file_quota) = self.remaining_file_quota.as_mut() {
                    *remaining_file_quota = remaining_file_quota.saturating_add(1);
                }
            }
        }

        let mut target_files_to_parse = added
            .into_iter()
            .chain(modified)
            .chain(moved.keys().cloned())
            .filter(|target_file| !target_file.is_ignored)
            .collect_vec();
        target_files_to_parse.sort_by(|left, right| left.path.cmp(&right.path));
        target_files_to_parse.dedup_by(|left, right| left.path == right.path);
        let mut new_files_reserved = 0usize;
        for target_file in target_files_to_parse {
            let existing_metadata =
                self.root
                    .find_mut(&target_file.path)
                    .and_then(|entry| match entry {
                        Entry::File(metadata) => Some(metadata.clone()),
                        Entry::Directory(_) => None,
                    });
            if let Some(metadata) = existing_metadata {
                files_metadata.push((metadata, true));
            } else if self
                .remaining_file_quota
                .is_none_or(|remaining| remaining > new_files_reserved)
            {
                files_metadata.push((FileMetadata::new(target_file.path, false), false));
                new_files_reserved += 1;
            }
        }
        for metadata in &files_metadata_to_remove {
            if let Some(outline) = self.file_id_to_outline.remove(&metadata.file_id) {
                self.retained_outline_bytes = self
                    .retained_outline_bytes
                    .saturating_sub(retained_file_outline_bytes(&outline));
            }
        }

        if let Some(updated_outlines) = parse_symbols_for_files(files_metadata).await {
            let mut new_file_budget_exhausted = false;
            for (metadata, existing, outline) in updated_outlines {
                if existing
                    && let Some(previous_outline) =
                        self.file_id_to_outline.remove(&metadata.file_id)
                {
                    self.retained_outline_bytes = self
                        .retained_outline_bytes
                        .saturating_sub(retained_file_outline_bytes(&previous_outline));
                }
                let outline_bytes = retained_file_outline_bytes(&outline);
                let next_retained_bytes = self.retained_outline_bytes.saturating_add(outline_bytes);
                if (!existing && new_file_budget_exhausted)
                    || next_retained_bytes > MAX_OUTLINE_TOTAL_BYTES
                {
                    if !existing {
                        new_file_budget_exhausted = true;
                    }
                    continue;
                }

                let file_id = if existing {
                    metadata.file_id
                } else {
                    let Some(inserted_file_id) = self
                        .find_or_insert_path_to_file_tree(&metadata.path.to_local_path_lossy())
                        .map(|metadata| metadata.file_id)
                    else {
                        continue;
                    };
                    if let Some(remaining_file_quota) = self.remaining_file_quota.as_mut() {
                        *remaining_file_quota = remaining_file_quota.saturating_sub(1);
                    }
                    inserted_file_id
                };
                self.file_id_to_outline.insert(file_id, outline);
                self.retained_outline_bytes = next_retained_bytes;
            }
        }
    }

    /// Returns the `FileMetadata` for the file corresponding to the given target path.
    ///
    /// If the target path corresponds to a directory, returns `None`.
    fn find_or_insert_path_to_file_tree(&mut self, target_path: &Path) -> Option<&FileMetadata> {
        match &mut self.root {
            Entry::Directory(directory) => {
                let dir_local = directory.path.to_local_path_lossy();
                if target_path.strip_prefix(&dir_local).is_err() {
                    // Target is not descendant of the repo.
                    return None;
                }

                // Get all the ancestors between the target path and the directory, including the
                // target path itself.
                let ancestors_between_target_and_directory = std::iter::once(target_path)
                    .chain(
                        target_path
                            .ancestors()
                            .take_while(|ancestor| *ancestor != dir_local.as_path()),
                    )
                    .collect_vec();

                // Iterate over the ancestors in reverse order, starting from the ancestor that is
                // the child of `directory`. We get or insert the entry corresponding to each of
                // those target ancestors, and continue the iteration if that entry is a directory.
                // At the end of the iteration we'll have reached the target path.
                let mut current_parent = directory;
                for ancestor in ancestors_between_target_and_directory.iter().rev() {
                    if matches_gitignores(
                        ancestor,
                        ancestor.is_dir(),
                        &self.gitignores,
                        false, /* check_ancestors */
                    ) || ancestor.ends_with(".git")
                    {
                        // Short-circuit if an ancestor is ignored.
                        return None;
                    }
                    match current_parent.find_or_insert_child(ancestor) {
                        Some(Entry::File(file_metadata)) => {
                            // If this entry is a file, we've reached the target path -- files can't
                            // have children!
                            return Some(&*file_metadata);
                        }
                        Some(Entry::Directory(directory)) => {
                            current_parent = directory;
                        }
                        None => return None,
                    }
                }
                None
            }
            Entry::File(_) => {
                report_error!("File tree root shouldn't be a file node");
                None
            }
        }
    }
}

/// Parse file symbols in parallel. This uses the [shared Rayon file-parsing pool](THREADPOOL),
/// but is `async` because it MUST NOT be called from the main thread.
async fn parse_symbols_for_files(
    mut files: Vec<(FileMetadata, bool)>,
) -> Option<Vec<(FileMetadata, bool, FileOutline)>> {
    let pool = THREADPOOL.as_ref()?;
    files.sort_by(|(left, _), (right, _)| left.path.as_str().cmp(right.path.as_str()));

    let (tx, rx) = oneshot::channel();

    pool.install(move || {
        rayon::spawn(move || {
            let result = files
                .par_iter()
                .map(|(metadata, existing)| {
                    let outline = parse_file_outline(&metadata.path.to_local_path_lossy())
                        .ok()
                        .unwrap_or_default();
                    (metadata.clone(), *existing, outline)
                })
                .collect::<Vec<_>>();
            let _ = tx.send(result);
        });
    });

    rx.await.ok()
}

/// Given the path of a file, try to construct its outline.
fn parse_file_outline(path: &Path) -> anyhow::Result<FileOutline> {
    if !is_file_parsable(path)? {
        return Err(anyhow!("File exceeds max file size limit for parsing"));
    }
    let standardized_path = StandardizedPath::try_from_local(path)?;
    let Some(language) = languages::language_by_filename(&standardized_path) else {
        return Err(anyhow!("Language unsupported for file {:?}", path));
    };
    let content = fs::read_to_string(path)?;

    let mut parser = Parser::new();
    parser.set_language(&language.grammar)?;
    let Some(tree) = parser.parse(&content, None) else {
        return Err(anyhow!("Couldn't parse AST"));
    };
    let symbols = language.symbols_query.as_ref().map(|query| {
        get_symbols(query, &tree, &content)
            .into_iter()
            .map(|(fn_name, type_prefix, comments, line_number)| Symbol {
                name: fn_name.to_owned(),
                type_prefix: type_prefix.map(String::from),
                comment: retain_comment(comments),
                line_number,
            })
            .collect_vec()
    });

    drop(tree);
    drop(parser);

    // Release extra unused memory from malloc to the system.  For some
    // reason, the memory obtained by the allocator is often not released
    // back to the OS after we're done with it, resulting in high memory
    // usage (from the perspective of the OS, though not from the perspective
    // of the allocator).
    //
    // See: https://github.com/tree-sitter/tree-sitter/issues/3129
    #[cfg(all(
        any(target_os = "linux", target_os = "freebsd"),
        target_env = "gnu",
        not(feature = "jemalloc")
    ))]
    unsafe {
        nix::libc::malloc_trim(0);
    }

    Ok(FileOutline { symbols })
}
fn retain_comment(comments: Vec<&str>) -> Option<Vec<String>> {
    let mut retained = Vec::new();
    let mut remaining_bytes = MAX_SYMBOL_COMMENT_BYTES;
    for line in comments
        .into_iter()
        .flat_map(str::lines)
        .take(MAX_SYMBOL_COMMENT_LINES)
    {
        if remaining_bytes == 0 {
            break;
        }
        let mut end = line.len().min(remaining_bytes);
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        if end == 0 {
            break;
        }
        retained.push(line[..end].to_owned());
        remaining_bytes -= end;
        if end < line.len() {
            break;
        }
    }
    (!retained.is_empty()).then_some(retained)
}

/// Given the content of a file, return all the symbols of interest.
fn get_symbols<'a>(
    query: &'a Query,
    tree: &Tree,
    file_content: &'a String,
) -> Vec<(&'a str, Option<&'a str>, Vec<&'a str>, usize)> {
    struct PendingComment<'a> {
        lines: Vec<&'a str>,
        last_line_number: usize,
    }
    let mut cursor = QueryCursor::new();
    let capture_names = query.capture_names();
    let mut captures = cursor.captures(query, tree.root_node(), TextSlice(file_content.as_bytes()));

    let mut symbols = vec![];
    let mut comment: Option<PendingComment> = None;
    while let Some(matches) = captures.next() {
        for cap in matches.0.captures {
            let capture_name = capture_names.get(cap.index as usize);
            let matched_content =
                &file_content[cap.node.byte_range().start..cap.node.byte_range().end];
            let line_number = cap.node.range().start_point.row;
            let end_point = cap.node.range().end_point;
            let end_line_number = if end_point.column == 0 && end_point.row > line_number {
                end_point.row - 1
            } else {
                end_point.row
            };
            match capture_name {
                Some(name) if *name == "comment" => match comment.as_mut() {
                    Some(pending_comment)
                        if pending_comment.last_line_number + 1 == line_number =>
                    {
                        pending_comment.lines.push(matched_content.trim());
                        pending_comment.last_line_number = end_line_number;
                    }
                    _ => {
                        comment = Some(PendingComment {
                            lines: vec![matched_content.trim()],
                            last_line_number: end_line_number,
                        })
                    }
                },
                _ => {
                    let comments = match comment.take() {
                        Some(pending_comment)
                            if pending_comment.last_line_number + 1 == line_number =>
                        {
                            pending_comment.lines
                        }
                        _ => vec![],
                    };
                    let type_prefix = capture_name.and_then(|s| s.split(".").nth(1));
                    symbols.push((matched_content, type_prefix, comments, line_number + 1));
                    // Convert to 1-indexed
                }
            }
        }
    }

    symbols
}

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;
