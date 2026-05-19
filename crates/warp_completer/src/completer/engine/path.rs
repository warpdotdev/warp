use std::collections::HashSet;
use std::fmt::{Display, Formatter};
use std::fs::DirEntry;
use std::sync::Arc;

use async_trait::async_trait;
use itertools::{iproduct, Itertools};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use typed_path::{TypedPath, TypedPathBuf};
use warp_command_signatures::{IconType, PathSuggestionType};
use warp_util::path::{ShellFamily, HOME_DIR_ENV_VAR_PREFIX};

use crate::completer::context::PathSeparators;
use crate::completer::suggest::Priority;
use crate::completer::{
    context::PathCompletionContext,
    matchers::MatchStrategy,
    suggest::{MatchedSuggestion, Suggestion, SuggestionType},
};
use crate::parsers::ParsedToken;

/// TODO(CORE-3074): This only applies to Unix.
const ROOT_DIR_STR: &str = "/";

lazy_static! {
    pub static ref CURR_DIRECTORY_ENTRY: EngineDirEntry = EngineDirEntry {
        file_name: ".".to_owned(),
        file_type: EngineFileType::Directory,
    };
    pub static ref PARENT_DIRECTORY_ENTRY: EngineDirEntry = EngineDirEntry {
        file_name: "..".to_owned(),
        file_type: EngineFileType::Directory,
    };
}

/// A `DirEntry` for the completions engine that abstracts whether the contents
/// come from a remote or local filesystem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct EngineDirEntry {
    pub file_name: String,
    pub file_type: EngineFileType,
}

impl EngineDirEntry {
    pub fn is_dir(&self) -> bool {
        self.file_type == EngineFileType::Directory
    }

    pub fn file_name(&self) -> &str {
        self.file_name.as_str()
    }

    pub fn is_hidden(&self) -> bool {
        self.file_name.starts_with('.')
    }
}

impl TryFrom<DirEntry> for EngineDirEntry {
    type Error = std::io::Error;

    fn try_from(value: DirEntry) -> Result<Self, Self::Error> {
        let file_type = value.file_type()?;
        let is_dir = if file_type.is_dir() {
            true
        } else if file_type.is_symlink() {
            // If the file is a symlink, follow the symlink and check if the target is a directory.
            value
                .path()
                .metadata()
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false)
        } else {
            false
        };
        let file_type = if is_dir {
            EngineFileType::Directory
        } else {
            EngineFileType::File
        };
        Ok(Self {
            file_name: value.file_name().to_string_lossy().to_string(),
            file_type,
        })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum EngineFileType {
    Directory,
    File,
}

impl Display for EngineFileType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                EngineFileType::Directory => "Directory",
                EngineFileType::File => "File",
            }
        )
    }
}

impl From<EngineFileType> for PathSuggestionType {
    fn from(path_type: EngineFileType) -> Self {
        match path_type {
            EngineFileType::Directory => Self::Folder,
            EngineFileType::File => Self::File,
        }
    }
}

/// Returns the sorted directories relative to the provided path and filter.
///
/// Note we are returning a Vector instead of iterator here because Rust currently doesn't support
/// returning opaque types (impl) in traits. This should have minimum impact on the memory allocation
/// since we are already calling `sort_by` before collecting which allocates memory.
pub(crate) async fn sorted_directories_relative_to(
    path: &ParsedToken,
    matcher: MatchStrategy,
    ctx: &dyn PathCompletionContext,
) -> Vec<MatchedSuggestion> {
    list_directory_contents(path, matcher, ctx)
        .await
        .into_iter()
        .filter(|path_suggestion| {
            path_suggestion
                .suggestion
                .file_type
                .is_some_and(|file_type| file_type == EngineFileType::Directory)
        })
        .sorted_by(|suggestion_a, suggestion_b| {
            suggestion_a
                .suggestion
                .cmp_by_display(&suggestion_b.suggestion)
        })
        .collect()
}

/// Like `sorted_directories_relative_to`, but iterates `$CDPATH` in shell
/// order (empty/`.` entry = pwd at that position; pwd appended as fallback if
/// no such entry) so completions surface in the order `cd` would resolve them.
pub(crate) async fn sorted_cd_directories(
    path: &ParsedToken,
    matcher: MatchStrategy,
    ctx: &dyn PathCompletionContext,
) -> Vec<MatchedSuggestion> {
    if !is_cdpath_eligible_token(path.as_str()) {
        return sorted_directories_relative_to(path, matcher, ctx).await;
    }

    let Some(cdpath) = ctx.cdpath() else {
        return sorted_directories_relative_to(path, matcher, ctx).await;
    };

    let mut results: Vec<MatchedSuggestion> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut pwd_searched = false;

    let push_unique = |suggestions: Vec<MatchedSuggestion>,
                       results: &mut Vec<MatchedSuggestion>,
                       seen: &mut HashSet<String>| {
        for s in suggestions {
            if seen.insert(s.suggestion.display.to_string()) {
                results.push(s);
            }
        }
    };

    for entry in cdpath.split(':') {
        if entry.is_empty() || entry == "." {
            if pwd_searched {
                continue;
            }
            let pwd = sorted_directories_relative_to(path, matcher, ctx).await;
            push_unique(pwd, &mut results, &mut seen);
            pwd_searched = true;
        } else {
            let override_ctx = CdpathOverrideContext {
                inner: ctx,
                cdpath_pwd: resolve_cdpath_entry(entry, ctx),
            };
            let extra = sorted_directories_relative_to(path, matcher, &override_ctx).await;
            push_unique(extra, &mut results, &mut seen);
        }
    }

    // Shell falls back to pwd when no `$CDPATH` entry matches. If no `.`/empty
    // entry positioned pwd already, append pwd matches now.
    if !pwd_searched {
        let pwd = sorted_directories_relative_to(path, matcher, ctx).await;
        push_unique(pwd, &mut results, &mut seen);
    }

    results
}

/// Tilde-expand a `$CDPATH` entry against the shell's home dir, then resolve
/// relative entries against the shell's pwd so `cd` matches shell behavior.
fn resolve_cdpath_entry(entry: &str, ctx: &dyn PathCompletionContext) -> TypedPathBuf {
    let expanded = if entry == "~" {
        ctx.home_directory().unwrap_or_default().to_owned()
    } else if let Some(rest) = entry.strip_prefix("~/") {
        format!("{}/{}", ctx.home_directory().unwrap_or_default(), rest)
    } else {
        entry.to_owned()
    };

    let resolved = TypedPathBuf::from(expanded.as_str());
    if resolved.is_absolute() {
        resolved
    } else {
        ctx.pwd().join(expanded)
    }
}

fn is_cdpath_eligible_token(token: &str) -> bool {
    !(token.starts_with('/')
        || token.starts_with('~')
        || token.starts_with("./")
        || token.starts_with("../")
        || token == "."
        || token == "..")
}

/// Wraps a `PathCompletionContext` and overrides only `pwd()` so we can reuse
/// the existing engine to list directories under a `$CDPATH` entry.
struct CdpathOverrideContext<'a> {
    inner: &'a dyn PathCompletionContext,
    cdpath_pwd: TypedPathBuf,
}

#[async_trait]
impl<'a> PathCompletionContext for CdpathOverrideContext<'a> {
    async fn list_directory_entries(&self, directory: TypedPathBuf) -> Arc<Vec<EngineDirEntry>> {
        self.inner.list_directory_entries(directory).await
    }

    fn home_directory(&self) -> Option<&str> {
        self.inner.home_directory()
    }

    fn cdpath(&self) -> Option<&str> {
        // Avoid recursing — the outer call already iterates entries.
        None
    }

    fn shell_family(&self) -> ShellFamily {
        self.inner.shell_family()
    }

    fn pwd(&self) -> TypedPath<'_> {
        self.cdpath_pwd.to_path()
    }

    fn path_separators(&self) -> PathSeparators {
        self.inner.path_separators()
    }
}

pub async fn sorted_paths_relative_to(
    path: &ParsedToken,
    matcher: MatchStrategy,
    ctx: &dyn PathCompletionContext,
) -> Vec<MatchedSuggestion> {
    list_directory_contents(path, matcher, ctx)
        .await
        .into_iter()
        .sorted_by(|suggestion_a, suggestion_b| {
            suggestion_a
                .suggestion
                .cmp_by_display(&suggestion_b.suggestion)
        })
        .collect()
}

/// Lists all directory contents within the directory identified by the parent directory of
/// `relative_to`.
/// If `relative_to` is `foo/bar/`, directory contents beanth `bar/` will be returned.
/// If `relative_to` is `foo/bar/a`, directory contents relative to `/bar` are returned, while
/// ensuring they match the trailing `a`.
/// `relative_to` can contain backslash escaped tildes so we can distinguish between tildes that
/// should be expanded into the home directory and a literal tilde.
/// NOTE: The resulting suggestion replacements are shell-escaped; display values are unescaped.
async fn list_directory_contents(
    relative_to: &ParsedToken,
    matcher: MatchStrategy,
    ctx: &dyn PathCompletionContext,
) -> Vec<MatchedSuggestion> {
    let home_dir = ctx.home_directory();

    let path_separators = ctx.path_separators();
    let split_path = SplitPath::new(
        ctx.pwd(),
        relative_to.as_str(),
        home_dir,
        path_separators.all,
    );

    let dir_entries = ctx
        .list_directory_entries(split_path.directory_absolute_path.clone())
        .await;

    let root_dir_entry =
        (split_path.directory_absolute_path.to_str() == Some(ROOT_DIR_STR)).then(|| {
            EngineDirEntry {
                file_name: ROOT_DIR_STR.to_owned(),
                file_type: EngineFileType::Directory,
            }
        });

    dir_entries
        .iter()
        .chain(root_dir_entry.iter())
        .chain([&*CURR_DIRECTORY_ENTRY, &*PARENT_DIRECTORY_ENTRY])
        .filter_map(move |entry| {
            let mut file_name = entry.file_name().to_string();

            let match_type = matcher.get_match_type(&split_path.file_name, file_name.as_str())?;

            let path = if entry.file_name() == ROOT_DIR_STR {
                ROOT_DIR_STR.to_owned()
            } else {
                if entry.is_dir() {
                    file_name.push(path_separators.main);
                }
                // We use `shell_escape()` instead of `escape()` on the relative path name to allow
                // home directory expansion if needed.
                format!(
                    "{}{}",
                    if split_path.directory_relative_path_name.is_empty() {
                        "".to_owned()
                    } else {
                        // `directory_relative_path_name` may have escaped tildes which we use to
                        // distinguish between a tilde representing the home directory and a literal
                        // tilde. `shell_escape()` will doubly escape an escaped tilde which is
                        // incorrect so we correct that behavior here.
                        ctx.shell_family()
                            .shell_escape(split_path.directory_relative_path_name.as_str())
                            .replace(r"\\\~", r"\~")
                    },
                    // Home directory expansion is never needed on file names, so we use the
                    // standard `escape()`.
                    ctx.shell_family().escape(file_name.as_str())
                )
            };

            (!entry.is_hidden() || split_path.file_name.starts_with('.')).then(|| {
                let mut suggestion = Suggestion::new(
                    file_name.as_str(),
                    path,
                    Some(entry.file_type.to_string()),
                    SuggestionType::Argument,
                    Priority::default(),
                );
                suggestion.file_type = Some(entry.file_type);
                suggestion.override_icon = Some(match entry.file_type {
                    EngineFileType::File => IconType::File,
                    EngineFileType::Directory => IconType::Folder,
                });
                MatchedSuggestion {
                    suggestion,
                    match_type,
                }
            })
        })
        .collect_vec()
}

/// A path split into the parent path (the entire piece before the last separator) and the
/// file_name (the piece after the last separator).
#[derive(Debug, PartialEq, Eq)]
struct SplitPath {
    /// The absolute path to the directory containing the file named `file_name`.
    directory_absolute_path: TypedPathBuf,

    /// The path to the directory containing the file named `file_name`, relative to the current
    /// working directory.  This is may contain unexpanded `~` or `$HOME`.
    directory_relative_path_name: String,

    /// The name of the `file`.
    file_name: String,
}

impl SplitPath {
    /// Returns a `SplitPath` based on the given path values.
    ///
    /// `current_directory` is the directory to which `relative_path` is relative.
    /// `relative_path` may contain '~' or '$HOME'. If `relative_path` begins with one of those
    /// strings, we expand that part of the path to the given `home_directory` value, if it is
    /// `Some()`. Note that `relative_path` comes directly from a user-specified path token. This
    /// may contain escaped tildes (for example if the user is completing on a path that contains
    /// literal tildes), which need to be unescaped before using the path to generate path
    /// suggestions.
    fn new(
        current_directory: TypedPath,
        relative_path: &str,
        home_directory: Option<&str>,
        path_separators: &[char],
    ) -> Self {
        let (directory_relative_path_name, file_name) = match relative_path.rfind(path_separators) {
            Some(pos) => relative_path.split_at(pos + 1),
            None => ("", relative_path),
        };

        let directory_absolute_path = if directory_relative_path_name.is_empty() {
            current_directory.to_path_buf()
        } else if let Some(rest) = iproduct!([HOME_DIR_ENV_VAR_PREFIX, "~"], path_separators)
            .find_map(|(prefix, sep)| {
                directory_relative_path_name.strip_prefix(&format!("{prefix}{sep}"))
            })
        {
            let mut home_directory = TypedPathBuf::from(home_directory.unwrap_or_default());
            home_directory.push(rest.replace(r"\~", "~"));
            home_directory
        } else {
            current_directory.join(directory_relative_path_name.replace(r"\~", "~"))
        };

        // Unescape escaped tildes in the filename.
        let file_name = file_name.replace(r"\~", "~");

        SplitPath {
            directory_absolute_path,
            directory_relative_path_name: directory_relative_path_name.to_owned(),
            file_name,
        }
    }
}

#[cfg(test)]
#[path = "path_tests.rs"]
mod tests;
