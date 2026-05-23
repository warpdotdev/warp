use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::anyhow;
use arborium::tree_sitter::{Parser, Query, QueryCursor, Tree};
use futures::channel::oneshot;
use ignore::gitignore::Gitignore;
use itertools::Itertools;
use rayon::prelude::*;
use repo_metadata::entry::{is_file_parsable, IgnoredPathStrategy};
use repo_metadata::RepositoryUpdate;
use streaming_iterator::StreamingIterator;
use syntax_tree::TextSlice;
use warp_util::standardized_path::StandardizedPath;

use crate::index::file_outline::{FileOutline, Outline, Symbol};
use crate::index::{Entry, FileId, FileMetadata, THREADPOOL};

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use crate::index::matches_gitignores;
    }
}

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
        gitignores.push(global_gitignore);
    }

    let gitignore_path = path.join(".gitignore");
    if gitignore_path.exists() {
        let (gitignore, _) = Gitignore::new(gitignore_path);
        gitignores.push(gitignore);
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
    )?;

    let (sender, receiver) = oneshot::channel();

    let Some(pool) = THREADPOOL.as_ref() else {
        return Err(anyhow!("No threadpool exists for outline generation."));
    };

    pool.spawn(move || {
        // Parse each file in parallel. Note that we have to fold and then reduce given the parallelization.
        let result = pool.install(|| {
            files
                .par_iter()
                .map(|metadata| {
                    let outline = parse_file_outline(&metadata.path.to_local_path_lossy())
                        .ok()
                        .unwrap_or_default();

                    (metadata.file_id, outline)
                })
                .collect::<HashMap<_, _>>()
        });

        if let Err(e) = sender.send(result) {
            log::error!("Could not send result of outline generation to background thread. {e:?}")
        }
    });

    let file_id_to_outline = receiver.await?;

    Ok(Outline {
        root: entry,
        file_id_to_outline,
        gitignores,
    })
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

        // Extract paths from TargetFile for removal, filtering out gitignored files
        for target_file in deleted
            .into_iter()
            .chain(moved.values().cloned())
            .filter(|target_file| !target_file.is_ignored)
        {
            if let Some(metadata) = self.root.remove(&target_file.path) {
                files_metadata_to_remove.push(metadata);
            }
        }

        // Extract paths from TargetFile for addition, filtering out gitignored files
        for target_file in added
            .into_iter()
            .chain(modified.into_iter())
            .chain(moved.keys().cloned())
            .filter(|target_file| !target_file.is_ignored)
        {
            if let Some(file_metadata) = self.find_or_insert_path_to_file_tree(&target_file.path) {
                files_metadata.push(file_metadata.clone());
            }
        }

        for metadata in &files_metadata_to_remove {
            self.file_id_to_outline.remove(&metadata.file_id);
        }

        if let Some(updated_outlines) = parse_symbols_for_files(files_metadata).await {
            self.file_id_to_outline.extend(updated_outlines);
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
                log::error!("File tree root shouldn't be a file node");
                None
            }
        }
    }
}

/// Parse file symbols in parallel. This uses the [shared Rayon file-parsing pool](THREADPOOL),
/// but is `async` because it MUST NOT be called from the main thread.
async fn parse_symbols_for_files(files: Vec<FileMetadata>) -> Option<HashMap<FileId, FileOutline>> {
    let pool = THREADPOOL.as_ref()?;

    let (tx, rx) = oneshot::channel();

    pool.install(move || {
        rayon::spawn(move || {
            // Parse each file in parallel. Note that we have to fold and then reduce given the parallelization.
            let result = files
                .par_iter()
                .map(|metadata| {
                    let outline = parse_file_outline(&metadata.path.to_local_path_lossy())
                        .ok()
                        .unwrap_or_default();

                    (metadata.file_id, outline)
                })
                .collect::<HashMap<_, _>>();
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
                comment: if comments.is_empty() {
                    None
                } else {
                    Some(comments.into_iter().map(String::from).collect())
                },
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
            match capture_name {
                Some(name) if *name == "comment" => match comment.as_mut() {
                    Some(pending_comment)
                        if pending_comment.last_line_number + 1 == line_number =>
                    {
                        pending_comment.lines.push(matched_content.trim());
                        pending_comment.last_line_number = line_number;
                    }
                    _ => {
                        comment = Some(PendingComment {
                            lines: vec![matched_content.trim()],
                            last_line_number: line_number,
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
mod tests {
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;

    fn create_test_file(dir: &TempDir, filename: &str, content: &str) -> PathBuf {
        let file_path = dir.path().join(filename);
        let mut file = File::create(&file_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file_path
    }

    #[test]
    fn test_parse_comments() {
        let temp_dir = TempDir::new().unwrap();
        let content = r#"
/// This is a struct for NewFunc
struct NewFunc {
    a: str,
}

// Hello
// World
fn first_function() {
    println!("First");
}

impl NewFunc {
    fn second_function() {
        println!("Second");
    }
}
"#;
        let file_path = create_test_file(&temp_dir, "multiple.rs", content);

        let outline = parse_file_outline(&file_path).unwrap();
        let symbols = outline.symbols.unwrap();
        assert_eq!(symbols[0].name, "NewFunc");
        assert_eq!(symbols[0].type_prefix, Some("struct".to_owned()));
        assert_eq!(
            symbols[0].comment,
            Some(vec!["/// This is a struct for NewFunc".to_owned()])
        );
        assert_eq!(symbols[0].line_number, 3); // struct NewFunc is on line 3
        assert_eq!(symbols[1].name, "first_function");
        assert_eq!(symbols[1].type_prefix, Some("fn".to_owned()));
        assert_eq!(symbols[1].line_number, 9); // first_function is on line 9
        assert_eq!(symbols[2].name, "second_function");
        assert_eq!(symbols[2].type_prefix, Some("fn".to_owned()));
        assert_eq!(symbols[2].line_number, 14); // second_function is on line 14
    }

    #[test]
    fn test_parse_multiple_languages() {
        let temp_dir = TempDir::new().unwrap();
        let content = r#"
struct NewFunc {
    a: str,
}

fn first_function() {
    println!("First");
}

impl NewFunc {
    fn second_function() {
        println!("Second");
    }
}
"#;
        let file_path = create_test_file(&temp_dir, "multiple.rs", content);

        let outline = parse_file_outline(&file_path).unwrap();
        let symbols = outline.symbols.unwrap();
        assert_eq!(symbols.len(), 3);
        assert_eq!(symbols[0].name, "NewFunc");
        assert_eq!(symbols[0].type_prefix, Some("struct".to_owned()));
        assert_eq!(symbols[1].name, "first_function");
        assert_eq!(symbols[1].type_prefix, Some("fn".to_owned()));
        assert_eq!(symbols[2].name, "second_function");
        assert_eq!(symbols[2].type_prefix, Some("fn".to_owned()));

        // Test parsing Python code with multiple symbol definitions
        // This verifies parsing of:
        // - Regular function definitions (def keyword)
        // - Class definitions (class keyword)
        // - Method definitions within a class (def keyword)
        let python_content = r#"
def first_function():
    print("First")

class TestClass:
    def __init__(self):
        pass

    def class_method(self):
        print("Method")

def second_function():
    print("Second")
"#;
        let file_path = create_test_file(&temp_dir, "multiple.py", python_content);
        let outline = parse_file_outline(&file_path).unwrap();
        let symbols = outline.symbols.unwrap();
        assert_eq!(symbols.len(), 5);
        assert_eq!(symbols[0].name, "first_function");
        assert_eq!(symbols[0].type_prefix, Some("def".to_owned()));
        assert_eq!(symbols[1].name, "TestClass");
        assert_eq!(symbols[1].type_prefix, Some("class".to_owned()));
        assert_eq!(symbols[2].name, "__init__");
        assert_eq!(symbols[2].type_prefix, Some("def".to_owned()));
        assert_eq!(symbols[3].name, "class_method");
        assert_eq!(symbols[3].type_prefix, Some("def".to_owned()));
        assert_eq!(symbols[4].name, "second_function");
        assert_eq!(symbols[4].type_prefix, Some("def".to_owned()));

        // Test parsing JavaScript code with multiple symbol definitions
        // This verifies parsing of:
        // - Function declarations
        // - Class declarations
        // - Method definitions
        // - Arrow functions assigned to variables
        let js_content = r#"
function regularFunction() {
    console.log('Regular function');
}

class TestClass {
    constructor() {
        this.value = 42;
    }

    classMethod() {
        return this.value;
    }
}
"#;
        let file_path = create_test_file(&temp_dir, "multiple.js", js_content);
        let outline = parse_file_outline(&file_path).unwrap();
        let symbols = outline.symbols.unwrap();
        assert_eq!(symbols.len(), 4);
        assert_eq!(symbols[0].name, "regularFunction");
        assert_eq!(symbols[0].type_prefix, Some("function".to_owned()));
        assert_eq!(symbols[1].name, "TestClass");
        assert_eq!(symbols[1].type_prefix, Some("class".to_owned()));
        assert_eq!(symbols[2].name, "constructor");
        assert_eq!(symbols[2].type_prefix, None);
        assert_eq!(symbols[3].name, "classMethod");
        assert_eq!(symbols[3].type_prefix, None);

        // Test parsing Go code with multiple symbol definitions
        // This verifies parsing of:
        // - Function definitions (func keyword)
        // - Type definitions (struct, interface)
        // - Method definitions (func with receiver)
        let go_content = r#"
package main

func mainFunction() {
    fmt.Println("Main function")
}

type TestStruct struct {
    field string
}

func (t *TestStruct) structMethod() string {
    return t.field
}

type TestInterface interface {
    InterfaceMethod() string
}

func helperFunction() {
    fmt.Println("Helper function")
}
"#;
        let file_path = create_test_file(&temp_dir, "multiple.go", go_content);
        let outline = parse_file_outline(&file_path).unwrap();
        let symbols = outline.symbols.unwrap();
        assert_eq!(symbols.len(), 5);
        assert_eq!(symbols[0].name, "mainFunction");
        assert_eq!(symbols[0].type_prefix, Some("func".to_owned()));
        assert_eq!(symbols[1].name, "TestStruct");
        assert_eq!(symbols[1].type_prefix, Some("type".to_owned()));
        assert_eq!(symbols[2].name, "structMethod");
        assert_eq!(symbols[2].type_prefix, Some("func".to_owned()));
        assert_eq!(symbols[3].name, "TestInterface");
        assert_eq!(symbols[3].type_prefix, Some("type".to_owned()));
        assert_eq!(symbols[4].name, "helperFunction");
        assert_eq!(symbols[4].type_prefix, Some("func".to_owned()));
    }
}
