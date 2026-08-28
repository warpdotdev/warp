#[cfg(feature = "completions_v2")]
mod js;
mod wsl_guest_listing;

use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use lazy_static::lazy_static;
use smol_str::SmolStr;
use typed_path::{TypedPath, TypedPathBuf};
use warp_completer::completer::{
    CommandExitStatus, CommandOutput, CompletionContext, EngineDirEntry, EngineFileType,
    GeneratorContext, PathCompletionContext, PathSeparators, TopLevelCommandCaseSensitivity,
};
use warp_completer::signatures::CommandRegistry;
use warp_core::features::FeatureFlag;
use warp_util::path::{EscapeChar, ShellFamily};
use warpui::{AppContext, SingletonEntity};

use crate::safe_warn;
use crate::terminal::model::session::{ExecuteCommandOptions, Session, SessionType};
use crate::util::AsciiDebug;
use crate::workflows::aliases::WorkflowAliases;

lazy_static! {
    pub static ref CURR_DIRECTORY_ENTRY: EngineDirEntry = EngineDirEntry {
        file_name: ".".to_owned(),
        file_type: EngineFileType::Directory,
    };
    pub static ref PARENT_DIRECTORY_ENTRY: EngineDirEntry = EngineDirEntry {
        file_name: "..".to_owned(),
        file_type: EngineFileType::Directory,
    };
    static ref EMPTY_COMMAND_REGISTRY: Arc<CommandRegistry> = Arc::new(CommandRegistry::empty());
}

#[derive(Clone)]
pub struct SessionContext {
    pub session: Arc<Session>,
    command_registry: Arc<CommandRegistry>,
    pub current_working_directory: TypedPathBuf,

    #[cfg(feature = "completions_v2")]
    js_ctx: Option<js::SessionJsExecutionContext>,

    /// Directory listings keyed by absolute path. Callers that must reflect a directory's
    /// current contents should use `refresh_directory_entries` to re-read from disk.
    cached_directory_entries: Arc<dashmap::DashMap<TypedPathBuf, Arc<Vec<EngineDirEntry>>>>,

    /// Snapshot of all Warp workflow aliases.
    workflow_aliases: HashMap<String, String>,
}

impl SessionContext {
    /// Lists `directory` fresh from disk and caches the results.
    pub(crate) async fn refresh_directory_entries(
        &self,
        directory: TypedPathBuf,
    ) -> Arc<Vec<EngineDirEntry>> {
        let result = Arc::new(
            self.list_directory_entries_internal(&directory.to_path())
                .await,
        );
        self.cached_directory_entries
            .insert(directory, result.clone());
        result
    }

    async fn list_directory_entries_internal(
        &self,
        directory: &TypedPath<'_>,
    ) -> Vec<EngineDirEntry> {
        match self.session.session_type() {
            SessionType::Local => {
                // The host cannot resolve an `IO_REPARSE_TAG_LX_SYMLINK` over `\\wsl$`
                // (APP-3993): it can't classify a symlink-to-directory correctly, and it can't
                // traverse *through* a symlinked directory to list its contents at all. So a WSL
                // session asks the guest for the listing directly, following symlinks (`-L`) so
                // both problems are avoided at the source, rather than patching up a host listing
                // afterwards. A slow or failing guest falls back to the plain host listing below
                // rather than emptying the completion list.
                #[cfg(windows)]
                if self.session.is_wsl()
                    && let Some(entries) = wsl_guest_listing::list_entries(self, directory).await
                {
                    return entries;
                }

                let dir = match self.session.maybe_convert_to_native_path(directory) {
                    Ok(dir) => dir,
                    Err(err) => {
                        log::warn!("Failed to convert path: {err:#}");
                        return Vec::new();
                    }
                };
                // We intentionally use the synchronous `std::fs::read_dir`,
                // despite this being an async function, because the overhead
                // of switching threads is very expensive relative to the
                // amount of work being done.  Converting the a `DirEntry`
                // to `EngineDirEntry` can usually be done without additional
                // syscalls (though one is necessary if the entry is a
                // symlink).
                //
                // It's possible that it would be better to use
                // `async_fs::read_dir` if the directory is on a network mount,
                // but I don't think it's worth optimizing for that case.
                let Some(read_dir) = std::fs::read_dir(dir.as_path()).ok() else {
                    return vec![];
                };

                read_dir
                    .filter_map(|res| res.and_then(EngineDirEntry::try_from).ok())
                    .collect::<Vec<_>>()
            }
            SessionType::WarpifiedRemote { .. } => {
                let env_vars = self
                    .session
                    .path()
                    .as_deref()
                    .map(|path| HashMap::from_iter([("PATH".to_string(), path.to_string())]));

                let Some(ls_command) = ls_script_for_dir(directory) else {
                    return vec![];
                };

                // The in-band command executor doesn't support executing from
                // from an arbitrary directory, so we need to cd into the
                // directory we want within the ls script.
                let command_output_result = self
                    .session
                    .execute_command(
                        &ls_command,
                        None,
                        env_vars,
                        ExecuteCommandOptions::default(),
                    )
                    .await;

                match command_output_result {
                    Ok(command_output) => match command_output.status {
                        CommandExitStatus::Success => {
                            parse_ls_script_output(command_output.output()).unwrap_or_else(|| {
                                log::warn!(
                                    "Executing `ls` on remote box returned malformed or truncated output: `{:?}`",
                                    AsciiDebug(command_output.output())
                                );
                                vec![]
                            })
                        }
                        CommandExitStatus::Failure => {
                            safe_warn!(
                                safe: ("Executing `ls` on remote box failed with non-zero status code."),
                                full: ("Executing `ls` on remote box failed with error: {}", &String::from_utf8_lossy(command_output.output()))
                            );
                            vec![]
                        }
                    },
                    Err(err) => {
                        log::warn!("Executing `ls` on remote box failed with error {err:?}");
                        vec![]
                    }
                }
            }
        }
    }
}

#[async_trait]
impl PathCompletionContext for SessionContext {
    fn home_directory(&self) -> Option<&str> {
        self.session.home_dir()
    }

    fn cdpath(&self) -> Option<&str> {
        self.session.cdpath()
    }

    fn pwd(&self) -> TypedPath<'_> {
        self.current_working_directory.to_path()
    }

    fn shell_family(&self) -> ShellFamily {
        self.session.shell_family()
    }

    async fn list_directory_entries(&self, directory: TypedPathBuf) -> Arc<Vec<EngineDirEntry>> {
        if let Some(entries) = self.cached_directory_entries.get(&directory) {
            return entries.clone();
        }

        let result = self
            .list_directory_entries_internal(&directory.to_path())
            .await;

        let result = Arc::new(result);
        self.cached_directory_entries
            .insert(directory, result.clone());
        result
    }

    fn path_separators(&self) -> PathSeparators {
        self.session.path_separators()
    }
}

#[async_trait]
impl GeneratorContext for SessionContext {
    async fn execute_command_at_pwd(
        &self,
        shell_command: &str,
        session_env_vars: Option<HashMap<String, String>>,
    ) -> Result<CommandOutput> {
        let mut env_vars = session_env_vars.unwrap_or_default();
        // We need to run the command with the PATH var set explicitly even if we have session env vars
        // because if the user opened Warp through a parent process that didn't have the PATH var set
        // (i.e. outside of a shell, for example opening the app via Finder),
        // the subshell won't inherit the PATH var, but we need the PATH var
        // to reference executables we might run as part of generators.
        if let Some(path) = self.session.path().as_deref() {
            env_vars.insert("PATH".to_string(), path.to_string());
        }

        let env_vars_option = if env_vars.is_empty() {
            None
        } else {
            Some(env_vars)
        };

        self.session
            .execute_command(
                shell_command,
                self.pwd().to_str(),
                env_vars_option,
                ExecuteCommandOptions {
                    run_command_in_same_shell_as_session: !FeatureFlag::RunGeneratorsWithCmdExe
                        .is_enabled(),
                },
            )
            .await
    }

    fn supports_parallel_execution(&self) -> bool {
        self.session.supports_parallel_command_execution()
    }
}

impl CompletionContext for SessionContext {
    fn generator_context(&self) -> Option<&dyn GeneratorContext> {
        Some(self)
    }

    fn path_completion_context(&self) -> Option<&dyn PathCompletionContext> {
        Some(self)
    }

    fn top_level_commands(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        Box::new(
            self.session
                .top_level_commands()
                .chain(self.workflow_aliases.keys().map(String::as_str)),
        )
    }

    fn command_case_sensitivity(&self) -> TopLevelCommandCaseSensitivity {
        self.session.command_case_sensitivity()
    }

    fn escape_char(&self) -> EscapeChar {
        self.session.shell_family().escape_char()
    }

    fn aliases(&self) -> Box<dyn Iterator<Item = (&str, &str)> + '_> {
        let session_aliases = self
            .session
            .aliases()
            .iter()
            .map(|(alias, command)| (alias.as_str(), command.as_str()));
        let workflow_aliases = self
            .workflow_aliases
            .iter()
            .map(|(alias, command)| (alias.as_str(), command.as_str()));
        Box::new(workflow_aliases.chain(session_aliases))
    }

    fn alias_command(&self, alias: &str) -> Option<&str> {
        self.workflow_aliases
            .get(alias)
            .or_else(|| self.session.aliases().get(alias))
            .map(Deref::deref)
    }

    fn abbreviations(&self) -> Option<&HashMap<SmolStr, String>> {
        Some(self.session.abbreviations())
    }

    fn functions(&self) -> Option<&HashSet<SmolStr>> {
        Some(self.session.functions())
    }

    fn builtins(&self) -> Option<&HashSet<SmolStr>> {
        Some(self.session.builtins())
    }

    fn command_registry(&self) -> &CommandRegistry {
        &self.command_registry
    }

    fn environment_variable_names(&self) -> Option<&HashSet<SmolStr>> {
        Some(self.session.environment_variable_names())
    }

    fn shell_supports_autocd(&self) -> Option<bool> {
        Some(self.session.shell().supports_autocd())
    }

    #[cfg(feature = "completions_v2")]
    fn js_context(&self) -> Option<&dyn warp_completer::completer::JsExecutionContext> {
        self.js_ctx
            .as_ref()
            .map(|ctx| -> &dyn warp_completer::completer::JsExecutionContext { ctx })
    }

    fn shell_family(&self) -> Option<ShellFamily> {
        Some(self.session.shell_family())
    }
}

impl SessionContext {
    pub fn new(
        session: impl Into<Arc<Session>>,
        command_registry: Arc<CommandRegistry>,
        current_working_directory: TypedPathBuf,
        #[allow(unused_variables)] ctx: &AppContext,
    ) -> Self {
        let workflow_aliases = if FeatureFlag::WorkflowAliases.is_enabled() {
            WorkflowAliases::as_ref(ctx).autocomplete_data(ctx)
        } else {
            Default::default()
        };

        cfg_if::cfg_if! {
            if #[cfg(feature = "completions_v2")] {
                use crate::plugin::{PluginHost, service::CallJsFunctionService};

                let js_function_caller = PluginHost::handle(ctx)
                    .as_ref(ctx)
                    .plugin_service_caller::<CallJsFunctionService>();
                Self {
                    session: session.into(),
                    command_registry,
                    current_working_directory,
                    js_ctx: js_function_caller.map(js::SessionJsExecutionContext::new),
                    cached_directory_entries: Arc::new(Default::default()),
                    workflow_aliases,
                }
            } else {
                Self {
                    session: session.into(),
                    command_registry,
                    current_working_directory,
                    cached_directory_entries: Arc::new(Default::default()),
                    workflow_aliases,
                }
            }
        }
    }
}

/// `CompletionContext` implementation for "global" completions, that provide completions on all
/// commands in the `command_registry` rather than providing session-specific completions.
///
/// This `CompletionContext` is not coupled to a specific session and thus does not provide path or
/// generator execution, which wouldn't have clear semantics without being coupled to a session.
#[derive(Clone)]
pub struct SessionAgnosticContext {
    command_registry: Arc<CommandRegistry>,
}

impl SessionAgnosticContext {
    pub fn new(command_registry: Arc<CommandRegistry>) -> Self {
        Self { command_registry }
    }
}

impl CompletionContext for SessionAgnosticContext {
    fn top_level_commands(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        Box::new(self.command_registry.registered_commands())
    }

    fn command_registry(&self) -> &CommandRegistry {
        &self.command_registry
    }

    fn environment_variable_names(&self) -> Option<&HashSet<SmolStr>> {
        None
    }

    fn shell_supports_autocd(&self) -> Option<bool> {
        None
    }

    fn path_completion_context(&self) -> Option<&dyn PathCompletionContext> {
        None
    }

    fn generator_context(&self) -> Option<&dyn GeneratorContext> {
        None
    }
}

/// Empty `CompletionContext` used in places without a live shell session
/// (i.e. shared session viewers without a real terminal instance).
#[derive(Clone)]
pub struct EmptyCompletionContext;
impl EmptyCompletionContext {
    pub fn new() -> Self {
        Self
    }
}
impl CompletionContext for EmptyCompletionContext {
    fn top_level_commands(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        Box::new(std::iter::empty())
    }

    fn command_registry(&self) -> &CommandRegistry {
        &EMPTY_COMMAND_REGISTRY
    }

    fn environment_variable_names(&self) -> Option<&HashSet<SmolStr>> {
        None
    }

    fn shell_supports_autocd(&self) -> Option<bool> {
        None
    }

    fn path_completion_context(&self) -> Option<&dyn PathCompletionContext> {
        None
    }

    fn generator_context(&self) -> Option<&dyn GeneratorContext> {
        None
    }
}

/// List files and directories in a directory; used for completions on a remote machine.
/// This uses `find` instead of `ls` due to the challenges of parsing `ls` output for
/// unusual file names (e.g.: ones including newlines).
/// We intentionally ignore '.' and '..' here as we add those suggestions manually.
fn ls_script_for_dir(directory: &TypedPath) -> Option<String> {
    // We need to cd into the directory we want completions for
    let Some(dir_str) = directory.to_str() else {
        log::warn!("Non-unicode character found in path: `{directory:?}`");
        return None;
    };
    let escaped_dir = warp_util::path::ShellFamily::Posix.shell_escape(dir_str);

    // Get all directories with -print0, which makes all items end in `\0` (null character)
    // Get all files with -print0, which makes all items end in `\0`
    // Separate the two lists with `\0`
    // Ex: `a\0b\0\c\0\0d.txt\0e.txt\0f.txt\0`
    // Then do the same for anything that is not a directory, and call it a 'File'.
    //
    // Follow symlinks when classifying entries, so a symlink to a directory completes as a
    // directory (like a standard terminal).
    let command = format!(
        r#"
cd {escaped_dir} && 
find -L . -maxdepth 1 -type d -print0 &&
printf '%b' '\0' &&
find -L . -maxdepth 1 -not -type d -print0
            "#
    )
    // Ensure all newlines are escaped, and that the command is a single line, since some
    // in-band executors run commands a single line at a time.
    .replace("\n", " ");

    Some(command)
}

/// Parses the null-delimited output of the script `ls_script_for_dir` builds into directory and
/// file entries, or `None` if the output doesn't match that script's guaranteed wire format --
/// a dirs list, a `\0` separator, and a files list, with every entry (including the separator
/// and, when the files list is non-empty, its own entries) `\0`-terminated.
fn parse_ls_script_output(output: &[u8]) -> Option<Vec<EngineDirEntry>> {
    let mut entries = Vec::new();
    let mut segments = output.split(|&byte| byte == b'\0');

    let dirs = segments
        .by_ref()
        // We use two consecutive null characters to separate files and folders, so detect that
        // here. Note that take_while consumes the first entry that returns false -- the
        // dirs/files separator itself.
        .take_while(|segment| !segment.is_empty())
        .filter_map(|segment| dir_entry_from_segment(segment, EngineFileType::Directory));
    entries.extend(dirs);

    // What's left after the separator is the files list followed by its own trailing `\0`,
    // which `find -print0` guarantees on every complete pass, empty or not (an empty pass
    // contributes no bytes of its own, but the separator's `\0` is still the final byte). If
    // nothing is left at all, the separator was never reached: the output was truncated before
    // the second `find` pass ran, or was empty to begin with. If the last remaining segment
    // isn't empty, the files list itself was cut off mid-entry. Either way, this isn't a real,
    // complete listing.
    let remaining: Vec<&[u8]> = segments.collect();
    let (last, files) = remaining.split_last()?;
    if !last.is_empty() {
        return None;
    }

    entries.extend(
        files
            .iter()
            .filter_map(|segment| dir_entry_from_segment(segment, EngineFileType::File)),
    );

    Some(entries)
}

/// Converts one `\0`-delimited `find -print0` segment into an entry, or `None` if it's the `.`
/// entry `find` itself emits, or the segment isn't valid UTF-8.
fn dir_entry_from_segment(segment: &[u8], file_type: EngineFileType) -> Option<EngineDirEntry> {
    let path_str = std::str::from_utf8(segment).ok()?;
    if path_str == "." {
        return None;
    }
    let file_name = Path::new(path_str).file_name()?.to_str()?.to_owned();
    Some(EngineDirEntry {
        file_name,
        file_type,
    })
}

#[cfg(test)]
#[path = "test.rs"]
mod tests;
