use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
#[cfg(not(target_arch = "wasm32"))]
use command::r#async::Command;
use lsp_types::{
    ClientCapabilities, ClientInfo, DidChangeWatchedFilesClientCapabilities, GotoCapability,
    HoverClientCapabilities, InitializeParams, MarkupKind, PublishDiagnosticsClientCapabilities,
    TextDocumentClientCapabilities, TextDocumentSyncClientCapabilities, Uri,
    WindowClientCapabilities, WorkDoneProgressParams, WorkspaceClientCapabilities, WorkspaceFolder,
};

use crate::command_builder::CommandBuilder;
use crate::descriptor::{placeholder, LspServerDescriptor};
use crate::manager::ServerKey;
use crate::supported_servers::LSPServerType;

/// Result of resolving an LSP server command, including the command and init params.
#[cfg(not(target_arch = "wasm32"))]
pub struct ResolvedLspCommand {
    pub command: Command,
    pub params: InitializeParams,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageId {
    Rust,
    Go,
    Python,
    TypeScript,
    TypeScriptReact,
    JavaScript,
    JavaScriptReact,
    C,
    Cpp,
}

impl LanguageId {
    pub fn from_path(path: &Path) -> Option<Self> {
        let extn = path.extension()?;
        match extn.to_str()? {
            "rs" => Some(Self::Rust),
            "go" => Some(Self::Go),
            "py" => Some(Self::Python),
            "ts" => Some(Self::TypeScript),
            "tsx" => Some(Self::TypeScriptReact),
            "js" | "mjs" | "cjs" => Some(Self::JavaScript),
            "jsx" => Some(Self::JavaScriptReact),
            "c" | "C" => Some(Self::C),
            "cc" | "cpp" | "cxx" => Some(Self::Cpp),
            // NOTE: `.h` files are ambiguous (could be C or C++). We map them to Cpp
            // because clangd defaults to C++ for `.h` files anyway. When a
            // compile_commands.json is present, clangd will use the correct language
            // regardless of the languageId we send.
            "h" | "H" | "hh" | "hpp" | "hxx" => Some(Self::Cpp),
            _ => None,
        }
    }

    /// Returns the language identifier as used by LSP.
    /// See: https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocumentItem
    pub(crate) fn lsp_language_identifier(&self) -> &'static str {
        match self {
            LanguageId::Rust => "rust",
            LanguageId::Go => "go",
            LanguageId::Python => "python",
            LanguageId::TypeScript => "typescript",
            LanguageId::TypeScriptReact => "typescriptreact",
            LanguageId::JavaScript => "javascript",
            LanguageId::JavaScriptReact => "javascriptreact",
            LanguageId::C => "c",
            LanguageId::Cpp => "cpp",
        }
    }

    /// For now we assume a 1:1 language -> LSP server type. This might change in the future as we support more configurabilities.
    pub fn server_type(&self) -> LSPServerType {
        match self {
            LanguageId::Rust => LSPServerType::RustAnalyzer,
            LanguageId::Go => LSPServerType::GoPls,
            LanguageId::Python => LSPServerType::Pyright,
            LanguageId::TypeScript
            | LanguageId::TypeScriptReact
            | LanguageId::JavaScript
            | LanguageId::JavaScriptReact => LSPServerType::TypeScriptLanguageServer,
            LanguageId::C | LanguageId::Cpp => LSPServerType::Clangd,
        }
    }
}

/// Configuration for spawning an LSP server process.
#[derive(Clone)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub struct LspServerConfig {
    server_type: LSPServerType,
    initial_workspace: PathBuf,
    /// The local PATH variable set when starting the server. This is needed when the app is started
    /// without a shell based parent process.
    /// TODO(kevin): This might not be sufficient for all cases (e.g. user might remove LSP from PATH).
    path_env_var: Option<String>,
    client_name: String,
    /// Shared HTTP client used for LSP installation checks and downloads.
    client: Arc<http_client::Client>,
    /// Optional path relative to the LSP log namespace for server stderr output.
    log_relative_path: Option<PathBuf>,
}

impl fmt::Debug for LspServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LspServerConfig")
            .field("server_type", &self.server_type)
            .field("initial_workspace", &self.initial_workspace)
            .field("path_env_var", &self.path_env_var)
            .field("client_name", &self.client_name)
            .field("log_relative_path", &self.log_relative_path)
            .finish()
    }
}

impl LspServerConfig {
    pub fn new(
        server_type: LSPServerType,
        initial_workspace: PathBuf,
        path_env_var: Option<String>,
        client_name: String,
        client: Arc<http_client::Client>,
    ) -> Self {
        Self {
            server_type,
            initial_workspace,
            path_env_var,
            client_name,
            client,
            log_relative_path: None,
        }
    }

    /// Sets the relative log path for this server's stderr output.
    pub fn with_log_relative_path(mut self, log_relative_path: PathBuf) -> Self {
        self.log_relative_path = Some(log_relative_path);
        self
    }
    /// Returns the relative log path if configured.
    pub fn log_relative_path(&self) -> Option<&PathBuf> {
        self.log_relative_path.as_ref()
    }

    /// Returns the initial workspace path.
    pub fn initial_workspace(&self) -> &Path {
        &self.initial_workspace
    }

    pub(crate) fn server_name(&self) -> String {
        self.server_type.binary_name().to_string()
    }

    pub(crate) fn languages(&self) -> Vec<LanguageId> {
        self.server_type.languages()
    }

    /// Creates the command and init params for the LSP server.
    ///
    /// PATH takes precedence over custom installations. If the binary is available
    /// and working on PATH, we use that. Otherwise, we fall back to our custom installation.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) async fn command_and_params(self) -> Result<ResolvedLspCommand> {
        // PATH takes precedence - only use custom installation if not working on PATH
        let executor = crate::CommandBuilder::new(self.path_env_var.clone());
        let is_working_on_path = self
            .server_type
            .is_working_on_path(&executor, self.client.clone())
            .await;
        let custom_binary_config = if is_working_on_path {
            // Binary works on PATH, don't use custom installation
            None
        } else {
            // Not working on PATH, check for custom installation
            self.server_type
                .find_installed_binary_config(executor.path_env_var())
                .await
        };

        // Bail early with a clear error instead of attempting to spawn a
        // binary that doesn't exist (which would fail with a confusing
        // "No such file or directory" OS error).
        if !is_working_on_path && custom_binary_config.is_none() {
            anyhow::bail!(
                "{} is not installed. Binary was not found on PATH and no custom installation exists",
                self.server_type.binary_name()
            );
        }

        let mut command = self
            .server_type
            .create_command(custom_binary_config.clone(), &executor);

        // Set the working directory to the workspace root. This is required for
        // LSP servers like rust-analyzer to properly discover the project structure.
        command.current_dir(&self.initial_workspace);

        log::info!(
            "LSP {} starting with custom_binary_config: {:?}",
            self.server_type.binary_name(),
            custom_binary_config
        );

        let params = default_init_params(&self.initial_workspace, self.client_name)?;

        Ok(ResolvedLspCommand { command, params })
    }

    pub(crate) fn server_type(&self) -> LSPServerType {
        self.server_type
    }

    pub(crate) fn key(&self) -> ServerKey {
        ServerKey::BuiltIn(self.server_type)
    }
}

/// Spawn-time configuration for a user-defined custom LSP server, paralleling
/// `LspServerConfig` (which carries the built-in `LSPServerType`). Held by
/// `LspServerModel` via `LspServerConfigKind`; the model dispatches to the
/// right inner config's `command_and_params()` at spawn time.
#[derive(Clone)]
pub struct CustomLspServerConfig {
    descriptor: LspServerDescriptor,
    initial_workspace: PathBuf,
    /// Resolves the `{{workspace_slug}}` placeholder.
    workspace_slug: String,
    /// Per-server cache directory owned by Warp, used to resolve `{{cache_dir}}`.
    cache_dir: PathBuf,
    /// Same `PATH` semantics as `LspServerConfig::path_env_var` — propagated to
    /// the spawned process so the user's `command` resolves correctly when Warp
    /// is launched without a shell parent.
    path_env_var: Option<String>,
    client_name: String,
    /// Optional path relative to the LSP log namespace for server stderr output.
    log_relative_path: Option<PathBuf>,
}

impl fmt::Debug for CustomLspServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustomLspServerConfig")
            .field("descriptor.name", &self.descriptor.name)
            .field("initial_workspace", &self.initial_workspace)
            .field("workspace_slug", &self.workspace_slug)
            .field("cache_dir", &self.cache_dir)
            .field("path_env_var", &self.path_env_var)
            .field("client_name", &self.client_name)
            .field("log_relative_path", &self.log_relative_path)
            .finish()
    }
}

impl CustomLspServerConfig {
    pub fn new(
        descriptor: LspServerDescriptor,
        initial_workspace: PathBuf,
        workspace_slug: String,
        cache_dir: PathBuf,
        path_env_var: Option<String>,
        client_name: String,
    ) -> Self {
        Self {
            descriptor,
            initial_workspace,
            workspace_slug,
            cache_dir,
            path_env_var,
            client_name,
            log_relative_path: None,
        }
    }

    /// Sets the relative log path for this server's stderr output.
    pub fn with_log_relative_path(mut self, log_relative_path: PathBuf) -> Self {
        self.log_relative_path = Some(log_relative_path);
        self
    }

    /// Returns the relative log path if configured.
    pub fn log_relative_path(&self) -> Option<&PathBuf> {
        self.log_relative_path.as_ref()
    }

    /// Returns the initial workspace path.
    pub fn initial_workspace(&self) -> &Path {
        &self.initial_workspace
    }

    /// Returns the descriptor this config wraps.
    pub fn descriptor(&self) -> &LspServerDescriptor {
        &self.descriptor
    }

    pub(crate) fn server_name(&self) -> String {
        self.descriptor.name.clone()
    }

    pub(crate) fn key(&self) -> ServerKey {
        ServerKey::Custom(self.descriptor.name.clone())
    }

    /// Builds the spawn `Command` and `InitializeParams` for this custom
    /// server. Runs placeholder substitution (`placeholder::expand`) over the
    /// command path, args, and env-var values; runs `placeholder::expand_json`
    /// over `initialization_options`. Sets `current_dir` to `initial_workspace`.
    ///
    /// No `PATH` fallback / install machinery: there is no install flow for
    /// custom servers, so if `descriptor.command` does not resolve, the
    /// launch fails through the existing `LspState::Failed` path. The
    /// descriptor's `name` is attached to any build-time error and included
    /// in the log message so failures are attributable when multiple custom
    /// servers exist.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) async fn command_and_params(self) -> Result<ResolvedLspCommand> {
        let placeholder_ctx = placeholder::LspPlaceholderContext::new(
            self.initial_workspace.clone(),
            self.workspace_slug,
            self.cache_dir,
        );

        log::info!(
            "Custom LSP \"{}\" placeholder context: workspace_root={}, workspace_slug={}, cache_dir={}",
            self.descriptor.name,
            placeholder_ctx.workspace_root.display(),
            placeholder_ctx.workspace_slug,
            placeholder_ctx.cache_dir.display(),
        );

        let resolved_command = placeholder::expand(&self.descriptor.command, &placeholder_ctx);
        let resolved_args: Vec<String> = self
            .descriptor
            .args
            .iter()
            .map(|arg| placeholder::expand(arg, &placeholder_ctx))
            .collect();

        let executor = CommandBuilder::new(self.path_env_var);
        let mut command = executor.command(&resolved_command);
        command.args(&resolved_args);
        command.current_dir(&self.initial_workspace);
        for (key, value) in &self.descriptor.env {
            command.env(key, placeholder::expand(value, &placeholder_ctx));
        }

        let mut params = default_init_params(&self.initial_workspace, self.client_name)
            .with_context(|| {
                format!(
                    "building init params for custom LSP \"{}\"",
                    self.descriptor.name
                )
            })?;
        if let Some(init_options) = &self.descriptor.initialization_options {
            params.initialization_options =
                Some(placeholder::expand_json(init_options, &placeholder_ctx));
        }

        log::info!(
            "Custom LSP \"{}\" starting with command: {} {:?}",
            self.descriptor.name,
            resolved_command,
            resolved_args
        );

        Ok(ResolvedLspCommand { command, params })
    }
}

/// Spawn-time configuration for an LSP server. Warp supports two kinds:
///
/// - `BuiltIn` — the frozen built-in servers. Deprecated path: no new
///   languages are added here, and eventually customs will absorb its
///   role entirely.
/// - `Custom` — user-defined via `[[editor.language_servers]]` in
///   `settings.toml`. The forward path for all new language support.
///
/// `Custom` is boxed because `CustomLspServerConfig` is ~3x the size of
/// `LspServerConfig`; without the box every `LspServerConfigKind` value
/// would carry the worst-case footprint even for built-ins.
#[derive(Clone, Debug)]
pub enum LspServerConfigKind {
    BuiltIn(LspServerConfig),
    Custom(Box<CustomLspServerConfig>),
}

impl LspServerConfigKind {
    /// Identifier for the server, used in log messages and footer rendering.
    pub fn server_name(&self) -> String {
        match self {
            Self::BuiltIn(c) => c.server_name(),
            Self::Custom(c) => c.server_name(),
        }
    }

    /// Uniform identity for this server across both kinds. Used by the
    /// manager for duplicate detection and emitted in
    /// `LspManagerModelEvent::ServerRemoved`.
    pub fn key(&self) -> ServerKey {
        match self {
            Self::BuiltIn(c) => c.key(),
            Self::Custom(c) => c.key(),
        }
    }

    pub fn initial_workspace(&self) -> &Path {
        match self {
            Self::BuiltIn(c) => c.initial_workspace(),
            Self::Custom(c) => c.initial_workspace(),
        }
    }

    pub fn log_relative_path(&self) -> Option<&PathBuf> {
        match self {
            Self::BuiltIn(c) => c.log_relative_path(),
            Self::Custom(c) => c.log_relative_path(),
        }
    }

    /// Builds the spawn `Command` and `InitializeParams`. Dispatches to the
    /// inner config's `command_and_params()`; below this call the LSP
    /// runtime treats both kinds identically.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) async fn command_and_params(self) -> Result<ResolvedLspCommand> {
        match self {
            Self::BuiltIn(c) => c.command_and_params().await,
            Self::Custom(c) => (*c).command_and_params().await,
        }
    }
}

pub(crate) fn path_to_lsp_uri(path: &Path) -> Result<Uri> {
    if !path.is_absolute() {
        return Err(anyhow::anyhow!("Path must be absolute: {}", path.display()));
    }

    // url::Url::from_file_path handles percent-encoding internally but is not
    // available on WASM. LSP is not supported on WASM either, so the fallback
    // is a simple string concatenation.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let url = url::Url::from_file_path(path).map_err(|()| {
            anyhow::anyhow!("Failed to convert path to file URI: {}", path.display())
        })?;

        // The url crate doesn't encode brackets, but LSP requires them to be
        // percent-encoded (e.g. Next.js [slug].tsx routes).
        let uri_str = url.as_str().replace('[', "%5B").replace(']', "%5D");

        uri_str.parse::<Uri>().map_err(anyhow::Error::from)
    }

    #[cfg(target_arch = "wasm32")]
    {
        let path_str = path.to_string_lossy();
        let uri_string = format!("file://{path_str}");
        uri_string.parse::<Uri>().map_err(anyhow::Error::from)
    }
}

pub(crate) fn lsp_uri_to_path(uri: &Uri) -> Result<PathBuf> {
    // Validate this is a file URI
    let scheme = uri.scheme().map(|s| s.as_str());
    if scheme != Some("file") {
        return Err(anyhow::anyhow!("Invalid file URI: {}", uri.as_str()));
    }

    // Decode percent-encoded characters (e.g., %40 -> @)
    // This is necessary because LSP servers return URL-encoded paths
    let decoded_path = uri
        .path()
        .as_estr()
        .decode()
        .into_string()
        .map_err(|e| anyhow::anyhow!("Invalid UTF-8 in URI path: {e}"))?;

    let mut path_str: &str = decoded_path.as_ref();

    // Windows URIs are formatted like: file:///C:/path/to/file
    // The path component is `/C:/path/to/file`, strip the leading slash.
    if cfg!(windows) {
        path_str = path_str.strip_prefix('/').unwrap_or(path_str);
        return Ok(PathBuf::from(path_str.replace('/', "\\")));
    }

    Ok(PathBuf::from(path_str))
}

fn path_to_workspace_folder(path: &Path) -> Result<WorkspaceFolder> {
    path_to_lsp_uri(path).map(|url| WorkspaceFolder {
        uri: url,
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
    })
}

fn default_client_capabilities() -> ClientCapabilities {
    ClientCapabilities {
        workspace: Some(WorkspaceClientCapabilities {
            did_change_watched_files: Option::from(DidChangeWatchedFilesClientCapabilities {
                dynamic_registration: Some(true),
                relative_pattern_support: Some(true),
            }),
            ..Default::default()
        }),
        window: Some(WindowClientCapabilities {
            work_done_progress: Some(true),
            ..Default::default()
        }),
        text_document: Some(TextDocumentClientCapabilities {
            synchronization: Some(TextDocumentSyncClientCapabilities {
                dynamic_registration: Some(true),
                will_save: Some(false),
                will_save_wait_until: Some(false),
                did_save: Some(true),
            }),
            definition: Some(GotoCapability {
                dynamic_registration: Some(false),
                link_support: Some(true),
            }),
            hover: Some(HoverClientCapabilities {
                dynamic_registration: Some(false),
                // Request Markdown content from the LSP for hover responses.
                // This enables proper syntax highlighting in hover tooltips.
                content_format: Some(vec![MarkupKind::Markdown, MarkupKind::PlainText]),
            }),
            publish_diagnostics: Some(PublishDiagnosticsClientCapabilities {
                version_support: Some(true),
                related_information: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn default_init_params(workspace_uri: &Path, client_name: String) -> Result<InitializeParams> {
    let workspace_folder = path_to_workspace_folder(workspace_uri)?;

    Ok(InitializeParams {
        process_id: Some(std::process::id()),
        capabilities: default_client_capabilities(),
        workspace_folders: Some(vec![workspace_folder]),
        client_info: Some(ClientInfo {
            name: client_name,
            version: option_env!("GIT_RELEASE_TAG").map(|s| s.to_string()),
        }),
        locale: None,
        work_done_progress_params: WorkDoneProgressParams::default(),
        ..Default::default()
    })
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
