#[cfg(feature = "completions_v2")]
mod js;

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

    cached_directory_entries: dashmap::DashMap<TypedPathBuf, Arc<Vec<EngineDirEntry>>>,

    /// Snapshot of all Warp workflow aliases.
    workflow_aliases: HashMap<String, String>,
}

impl SessionContext {
    async fn list_directory_entries_internal(
        &self,
        directory: &TypedPath<'_>,
    ) -> Vec<EngineDirEntry> {
        match self.session.session_type() {
            SessionType::Local => {
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

                if let Ok(command_output) = command_output_result {
                    let Ok(output_string) = command_output.to_string() else {
                        log::warn!(
                            "Executing `ls` on remote box returned unparsable bytes: `{:?}`",
                            AsciiDebug(command_output.output())
                        );
                        return vec![];
                    };

                    match command_output.status {
                        CommandExitStatus::Success => {
                            let mut entries = Vec::new();
                            let mut entries_iter = output_string.split('\0');
                            let dirs = entries_iter
                                .by_ref()
                                // We use two consecutive null characters to separate files and
                                // folders, so detect that here. Note that take_while consumes the
                                // first entry that returns false.
                                .take_while(|entry| !entry.is_empty())
                                .filter_map(|entry| {
                                    if entry == "." {
                                        return None;
                                    }

                                    Path::new(entry)
                                        .file_name()
                                        .and_then(|name| name.to_str())
                                        .map(|name| EngineDirEntry {
                                            file_name: name.to_owned(),
                                            file_type: EngineFileType::Directory,
                                        })
                                });
                            entries.extend(dirs);

                            let files = entries_iter.filter_map(|entry| {
                                Path::new(entry)
                                    .file_name()
                                    .and_then(|name| name.to_str())
                                    .map(|name| EngineDirEntry {
                                        file_name: name.to_owned(),
                                        file_type: EngineFileType::File,
                                    })
                            });
                            entries.extend(files);

                            entries
                        }
                        CommandExitStatus::Failure => {
                            safe_warn!(
                                safe: ("Executing `ls` on remote box failed with non-zero status code."),
                                full: ("Executing `ls` on remote box failed with error: {}", &String::from_utf8_lossy(command_output.output()))
                            );
                            vec![]
                        }
                    }
                } else {
                    log::warn!(
                        "Executing `ls` on remote box failed with error {command_output_result:?}"
                    );
                    vec![]
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
                    cached_directory_entries: Default::default(),
                    workflow_aliases,
                }
            } else {
                Self {
                    session: session.into(),
                    command_registry,
                    current_working_directory,
                    cached_directory_entries: Default::default(),
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
    let command = format!(
        r#"
cd {escaped_dir} && 
find . -maxdepth 1 -type d -print0 &&
printf '%b' '\0' &&
find . -maxdepth 1 -not -type d -print0
            "#
    )
    // Ensure all newlines are escaped, and that the command is a single line.
    // ls_script_for_dir should not contain newlines, as we need to run it as a
    // single line for TMUX control mode at this time.
    .replace("\n", " ");

    Some(command)
}

#[cfg(test)]
#[path = "test.rs"]
mod tests;
