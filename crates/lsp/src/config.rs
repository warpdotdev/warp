use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use lsp_types::Uri;

use crate::supported_servers::LSPServerType;

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
    pub fn from_path(_path: &Path) -> Option<Self> {
        None
    }

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

#[derive(Clone)]
pub struct LspServerConfig {
    server_type: LSPServerType,
    initial_workspace: PathBuf,
    path_env_var: Option<String>,
    client_name: String,
    #[allow(dead_code)]
    client: Arc<http_client::Client>,
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

    pub fn with_log_relative_path(mut self, log_relative_path: PathBuf) -> Self {
        self.log_relative_path = Some(log_relative_path);
        self
    }

    pub fn log_relative_path(&self) -> Option<&PathBuf> {
        self.log_relative_path.as_ref()
    }

    pub fn initial_workspace(&self) -> &Path {
        &self.initial_workspace
    }

    pub(crate) fn server_name(&self) -> String {
        self.server_type.binary_name().to_string()
    }

    pub(crate) fn server_type(&self) -> LSPServerType {
        self.server_type
    }
}

pub(crate) fn path_to_lsp_uri(path: &Path) -> Result<Uri> {
    if !path.is_absolute() {
        return Err(anyhow::anyhow!("Path must be absolute: {}", path.display()));
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let url = url::Url::from_file_path(path).map_err(|()| {
            anyhow::anyhow!("Failed to convert path to file URI: {}", path.display())
        })?;
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
    let scheme = uri.scheme().map(|s| s.as_str());
    if scheme != Some("file") {
        return Err(anyhow::anyhow!("Invalid file URI: {}", uri.as_str()));
    }

    let decoded_path = uri
        .path()
        .as_estr()
        .decode()
        .into_string()
        .map_err(|e| anyhow::anyhow!("Invalid UTF-8 in URI path: {e}"))?;

    let mut path_str: &str = decoded_path.as_ref();
    if cfg!(windows) {
        path_str = path_str.strip_prefix('/').unwrap_or(path_str);
        return Ok(PathBuf::from(path_str.replace('/', "\\")));
    }

    Ok(PathBuf::from(path_str))
}
