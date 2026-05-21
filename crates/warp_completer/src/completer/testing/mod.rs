//! This module contains test-only APIs and utils for testing the completions engine.
#[cfg(feature = "v2")]
mod v2;

use std::{
    collections::{HashMap, HashSet},
    ops::Deref,
    path::PathBuf,
    sync::Arc,
};

use async_trait::async_trait;
use smol_str::SmolStr;
use typed_path::{TypedPath, TypedPathBuf};
use warp_command_signatures::IconType;
use warp_core::command::ExitCode;
use warp_util::path::{EscapeChar, ShellFamily, TEST_SESSION_HOME_DIR};

use crate::{
    completer::{
        CommandOutput, CompletionContext, Description, EngineDirEntry, EngineFileType,
        GeneratorContext, PathCompletionContext, Suggestion, TopLevelCommandCaseSensitivity,
    },
    signatures::{
        testing::{TEST_ALIAS_COMMAND, TEST_GENERATOR_1_COMMAND, TEST_GENERATOR_2_COMMAND},
        CommandRegistry,
    },
};

use super::{CommandExitStatus, MatchedSuggestion, PathSeparators};

impl EngineDirEntry {
    pub fn test_file(file_name: &str) -> Self {
        EngineDirEntry {
            file_name: file_name.to_owned(),
            file_type: EngineFileType::File,
        }
    }

    pub fn test_dir(file_name: &str) -> Self {
        EngineDirEntry {
            file_name: file_name.to_owned(),
            file_type: EngineFileType::Directory,
        }
    }
}

impl Description {
    pub fn into_token_name(self) -> String {
        self.token.item
    }
}

impl Suggestion {
    pub fn with_icon_override(mut self, icon_type: IconType) -> Self {
        self.override_icon = Some(icon_type);
        self
    }

    pub fn with_file_type(mut self, file_type: EngineFileType) -> Self {
        self.file_type = Some(file_type);
        self
    }
}

impl MatchedSuggestion {
    pub fn with_file_type(mut self, file_type: EngineFileType) -> Self {
        self.suggestion = self.suggestion.with_file_type(file_type);
        self
    }

    pub fn with_replacement(mut self, replacement: impl Into<SmolStr>) -> Self {
        self.suggestion.replacement = replacement.into();
        self
    }
}

/// A mock `GeneratorContext` implementation that allows callers to specify commands that may be
/// run (as part of completions generator/alias execution) and their outputs.
#[derive(Default)]
pub struct MockGeneratorContext {
    expected_commands_to_output: HashMap<String, String>,
}

impl MockGeneratorContext {
    pub fn new() -> Self {
        Self {
            expected_commands_to_output: HashMap::new(),
        }
    }

    /// Creates a generator context that expects generator/alias commands used by the test command
    /// signature (`test_signature()`).
    pub fn for_test_signature() -> Self {
        Self::new()
            .with_expected_command(TEST_ALIAS_COMMAND, "alias")
            .with_expected_command(TEST_GENERATOR_1_COMMAND, "1")
            .with_expected_command(TEST_GENERATOR_2_COMMAND, "2")
    }

    pub fn with_expected_command(
        mut self,
        command: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        self.expected_commands_to_output
            .insert(command.into(), output.into());
        self
    }
}

#[async_trait]
impl GeneratorContext for MockGeneratorContext {
    async fn execute_command_at_pwd(
        &self,
        shell_command: &str,
        _session_env_vars: Option<HashMap<String, String>>,
    ) -> anyhow::Result<CommandOutput> {
        Ok(CommandOutput {
            stdout: self
                .expected_commands_to_output
                .get(shell_command)
                .expect(
                    "Generator command expectation should have been set on TestGeneratorContext.",
                )
                .clone()
                .into_bytes(),
            stderr: Vec::new(),
            status: CommandExitStatus::Success,
            exit_code: Some(ExitCode::from(0)),
        })
    }

    fn supports_parallel_execution(&self) -> bool {
        false
    }
}

/// A mock `PathCompletionContext` implementation that allows callers to specify a fake directory
/// structure as pairs of directories and their immediate child entries.
#[derive(Debug, Clone)]
pub struct MockPathCompletionContext {
    home_directory: Option<String>,
    cdpath: Option<String>,
    pwd: TypedPathBuf,
    directory_to_entries: HashMap<PathBuf, Vec<EngineDirEntry>>,
}

impl MockPathCompletionContext {
    pub fn new(pwd: TypedPathBuf) -> Self {
        Self {
            home_directory: TEST_SESSION_HOME_DIR.clone(),
            cdpath: None,
            pwd,
            directory_to_entries: HashMap::new(),
        }
    }

    pub fn with_home_directory(mut self, home_directory: String) -> Self {
        self.home_directory = Some(home_directory);
        self
    }

    pub fn with_cdpath(mut self, cdpath: String) -> Self {
        self.cdpath = Some(cdpath);
        self
    }

    /// The given entries are mocked as children of the context's `pwd`, such that `entries`
    /// is returned if the completions engine calls
    /// `path_ctx.list_directory_entries(path_ctx.pwd())`.
    pub fn with_entries_in_pwd(
        mut self,
        entries: impl IntoIterator<Item = EngineDirEntry>,
    ) -> Self {
        let Ok(pwd) = PathBuf::try_from(self.pwd.clone()) else {
            log::warn!(
                "Failed to convert TypedPath to OS-native path. Not populating entries for pwd"
            );
            return self;
        };

        self.directory_to_entries
            .insert(pwd, entries.into_iter().collect());
        self
    }

    /// The given entries are mocked as children of the given `directory_path`, such that `entries`
    /// is returned if the completions engine calls
    /// `path_ctx.list_directory_entries(directory_path.as_path())`.
    pub fn with_entries(
        mut self,
        directory_path: TypedPathBuf,
        entries: impl IntoIterator<Item = EngineDirEntry>,
    ) -> Self {
        let Ok(directory_path) = PathBuf::try_from(directory_path) else {
            log::warn!(
                "Failed to convert TypedPath to OS-native path. Not populating entries for directory"
            );
            return self;
        };

        self.directory_to_entries
            .insert(directory_path.to_path_buf(), entries.into_iter().collect());
        self
    }
}

impl Default for MockPathCompletionContext {
    fn default() -> Self {
        #[cfg(unix)]
        let pwd = "/home/";
        #[cfg(windows)]
        let pwd = r"C:\Users\";
        Self::new(TypedPathBuf::from(pwd))
    }
}

#[async_trait]
impl PathCompletionContext for MockPathCompletionContext {
    async fn list_directory_entries(&self, directory: TypedPathBuf) -> Arc<Vec<EngineDirEntry>> {
        let Ok(directory) = PathBuf::try_from(directory) else {
            log::warn!(
                "Failed to convert TypedPath to OS-native path, returning empty directory entries"
            );
            return Arc::new(Vec::new());
        };
        self.directory_to_entries
            .get(&directory)
            .cloned()
            .unwrap_or_default()
            .into()
    }

    fn shell_family(&self) -> ShellFamily {
        ShellFamily::Posix
    }

    fn home_directory(&self) -> Option<&str> {
        self.home_directory.as_deref()
    }

    fn cdpath(&self) -> Option<&str> {
        self.cdpath.as_deref()
    }

    fn pwd(&self) -> TypedPath<'_> {
        self.pwd.to_path()
    }

    fn path_separators(&self) -> PathSeparators {
        PathSeparators::for_unix()
    }
}

/// A fake `CompletionContext` implementation for use in testing.
pub struct FakeCompletionContext {
    top_level_commands: Vec<SmolStr>,
    aliases: Option<HashMap<SmolStr, String>>,
    abbreviations: Option<HashMap<SmolStr, String>>,
    functions: Option<HashSet<SmolStr>>,
    builtins: Option<HashSet<SmolStr>>,
    supports_autocd: Option<bool>,
    environment_variable_names: Option<HashSet<SmolStr>>,
    path_completion_context: Option<MockPathCompletionContext>,
    generator_context: Option<MockGeneratorContext>,
    command_case_sensitivity: TopLevelCommandCaseSensitivity,
    escape_char: EscapeChar,
    shell_family: Option<ShellFamily>,

    command_registry: CommandRegistry,

    #[cfg(feature = "v2")]
    js_ctx: v2::FakeJsExecutionContext,
}

impl FakeCompletionContext {
    pub fn new(command_registry: CommandRegistry) -> Self {
        Self {
            command_registry,
            supports_autocd: None,
            environment_variable_names: None,
            aliases: None,
            abbreviations: None,
            functions: None,
            builtins: None,
            top_level_commands: Vec::default(),
            path_completion_context: None,
            generator_context: None,
            command_case_sensitivity: TopLevelCommandCaseSensitivity::CaseInsensitive,
            escape_char: EscapeChar::Backslash,
            shell_family: None,

            #[cfg(feature = "v2")]
            js_ctx: v2::FakeJsExecutionContext {},
        }
    }

    /// Sets the "top-level" commands available for root command completion. Note that if the
    /// context includes aliases and/or abbreviations, the aliases and abbreviations must also be
    /// included in this list. The given list defines all strings that are eligible to be suggested
    /// in the root command position.
    pub fn with_top_level_commands(
        mut self,
        top_level_commands: impl IntoIterator<Item = impl Into<SmolStr>>,
    ) -> Self {
        self.top_level_commands = top_level_commands.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_case_sensitivity(mut self) -> Self {
        self.command_case_sensitivity = TopLevelCommandCaseSensitivity::CaseSensitive;
        self
    }

    pub fn with_supports_autocd(mut self, supports_autocd: bool) -> Self {
        self.supports_autocd = Some(supports_autocd);
        self
    }

    pub fn with_environment_variable_names(
        mut self,
        environment_variable_names: HashSet<SmolStr>,
    ) -> Self {
        self.environment_variable_names = Some(environment_variable_names);
        self
    }

    pub fn with_aliases(mut self, aliases: HashMap<SmolStr, String>) -> Self {
        self.aliases = Some(aliases);
        self
    }

    pub fn with_abbreviations(mut self, abbreviations: HashMap<SmolStr, String>) -> Self {
        self.abbreviations = Some(abbreviations);
        self
    }

    pub fn with_functions(mut self, functions: HashSet<SmolStr>) -> Self {
        self.functions = Some(functions);
        self
    }

    pub fn with_builtins(mut self, builtins: HashSet<SmolStr>) -> Self {
        self.builtins = Some(builtins);
        self
    }

    pub fn with_path_completion_context(mut self, path_ctx: MockPathCompletionContext) -> Self {
        self.path_completion_context = Some(path_ctx);
        self
    }

    pub fn with_generator_context(mut self, generator_ctx: MockGeneratorContext) -> Self {
        self.generator_context = Some(generator_ctx);
        self
    }

    pub fn with_shell_family(mut self, shell_family: ShellFamily) -> Self {
        self.shell_family = Some(shell_family);
        self
    }
}

impl CompletionContext for FakeCompletionContext {
    fn path_completion_context(&self) -> Option<&dyn PathCompletionContext> {
        self.path_completion_context
            .as_ref()
            .map(|context| context as &dyn PathCompletionContext)
    }

    fn generator_context(&self) -> Option<&dyn GeneratorContext> {
        self.generator_context
            .as_ref()
            .map(|context| context as &dyn GeneratorContext)
    }

    #[cfg(feature = "v2")]
    fn js_context(&self) -> Option<&dyn crate::completer::context::JsExecutionContext> {
        Some(&self.js_ctx)
    }

    fn top_level_commands(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        Box::new(
            self.top_level_commands
                .iter()
                .map(|command| command.as_str()),
        )
    }

    fn command_case_sensitivity(&self) -> TopLevelCommandCaseSensitivity {
        self.command_case_sensitivity
    }

    fn escape_char(&self) -> EscapeChar {
        self.escape_char
    }

    fn aliases(&self) -> Box<dyn Iterator<Item = (&str, &str)> + '_> {
        Box::new(
            self.aliases
                .as_ref()
                .into_iter()
                .flat_map(|aliases| aliases.iter())
                .map(|(alias, command)| (alias.as_str(), command.as_str())),
        )
    }

    fn alias_command(&self, alias: &str) -> Option<&str> {
        self.aliases
            .as_ref()
            .and_then(|aliases| aliases.get(alias))
            .map(Deref::deref)
    }

    fn abbreviations(&self) -> Option<&HashMap<SmolStr, String>> {
        self.abbreviations.as_ref()
    }

    fn functions(&self) -> Option<&HashSet<SmolStr>> {
        self.functions.as_ref()
    }

    fn builtins(&self) -> Option<&HashSet<SmolStr>> {
        self.builtins.as_ref()
    }

    fn environment_variable_names(&self) -> Option<&HashSet<SmolStr>> {
        self.environment_variable_names.as_ref()
    }

    fn shell_supports_autocd(&self) -> Option<bool> {
        self.supports_autocd
    }

    fn command_registry(&self) -> &CommandRegistry {
        &self.command_registry
    }

    fn shell_family(&self) -> Option<ShellFamily> {
        self.shell_family
    }
}
