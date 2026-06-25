#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

use itertools::Itertools;
use serde::{Deserialize, Serialize};
use strum_macros::EnumIter;

use crate::LanguageId;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
pub struct CustomBinaryConfig {
    pub binary_path: PathBuf,
    pub prepend_args: Vec<String>,
}

/// Legacy server identifiers kept only so older local SQLite rows can deserialize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, EnumIter)]
pub enum LSPServerType {
    RustAnalyzer,
    GoPls,
    Pyright,
    TypeScriptLanguageServer,
    Clangd,
}

impl LSPServerType {
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn find_installed_binary_config(
        &self,
        _path_env_var: Option<&str>,
    ) -> Option<CustomBinaryConfig> {
        None
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn is_working_on_path(
        &self,
        _executor: &(),
        _client: std::sync::Arc<http_client::Client>,
    ) -> bool {
        false
    }

    pub fn binary_name(&self) -> &'static str {
        match self {
            LSPServerType::RustAnalyzer => "rust-analyzer",
            LSPServerType::GoPls => "gopls",
            LSPServerType::Pyright => "pyright-langserver",
            LSPServerType::TypeScriptLanguageServer => "typescript-language-server",
            LSPServerType::Clangd => "clangd",
        }
    }

    pub fn languages(&self) -> Vec<LanguageId> {
        Vec::new()
    }

    pub fn language_name(&self) -> String {
        let languages = match self {
            LSPServerType::RustAnalyzer => vec![LanguageId::Rust],
            LSPServerType::GoPls => vec![LanguageId::Go],
            LSPServerType::Pyright => vec![LanguageId::Python],
            LSPServerType::TypeScriptLanguageServer => {
                vec![LanguageId::TypeScript, LanguageId::JavaScript]
            }
            LSPServerType::Clangd => vec![LanguageId::C, LanguageId::Cpp],
        };

        languages
            .iter()
            .map(|lang| {
                let id = lang.lsp_language_identifier();
                let mut chars = id.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .join("/")
    }

    pub fn all() -> impl Iterator<Item = LSPServerType> {
        std::iter::empty()
    }
}
