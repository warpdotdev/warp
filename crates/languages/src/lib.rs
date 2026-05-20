use std::{
    borrow::Cow,
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

use arborium::tree_sitter::{Language as ParserGrammar, Query};
use lazy_static::lazy_static;
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use warp_editor::content::text::IndentUnit;

#[derive(RustEmbed)]
#[folder = "grammars"]
struct Grammars;

lazy_static! {
    static ref LANGUAGE_REGISTRY: LanguageRegistry = LanguageRegistry::new();
}

pub const SUPPORTED_LANGUAGES: [&str; 33] = [
    "rust",
    "golang",
    "yaml",
    "python",
    "javascript",
    "jsx",
    "typescript",
    "tsx",
    "java",
    "cpp",
    "shell",
    "csharp",
    "html",
    "css",
    "c",
    "json",
    "jq",
    "hcl",
    "lua",
    "ruby",
    "php",
    "toml",
    "swift",
    "kotlin",
    "scala",
    "powershell",
    "elixir",
    "sql",
    "starlark",
    "objective-c",
    "xml",
    "vue",
    "dockerfile",
];

/// Registry that holds all of the supported languages.
pub struct LanguageRegistry {
    /// List of languages we support mapped from their display name. They are hold in Arc so they could be shared
    /// between different editors.
    languages: Mutex<HashMap<String, Arc<Language>>>,
}

impl LanguageRegistry {
    fn new() -> Self {
        Self {
            languages: Mutex::new(HashMap::new()),
        }
    }

    pub fn language_by_name(&self, name: &str) -> Option<Arc<Language>> {
        if !SUPPORTED_LANGUAGES.contains(&name) {
            return None;
        }

        let mut languages = self.languages.lock().expect("Mutex should not be poisoned");

        if let Some(lang) = languages.get(name) {
            return Some(lang.clone());
        }

        let language = Arc::new(load_language(name)?);
        languages.insert(name.to_string(), language.clone());
        Some(language)
    }
}

/// Normalizes common markdown language aliases to their internal names.
/// For example, "go" -> "golang", "bash" -> "shell", etc.
fn normalize_language_name(name: &str) -> &str {
    match name {
        "go" => "golang",
        "bash" | "sh" | "zsh" => "shell",
        "js" => "javascript",
        "ts" => "typescript",
        "py" => "python",
        "rb" => "ruby",
        "rs" => "rust",
        "cs" | "c#" => "csharp",
        "c++" => "cpp",
        "objc" | "objective_c" => "objective-c",
        "terraform" | "tf" => "hcl",
        "kt" => "kotlin",
        "docker" | "containerfile" => "dockerfile",
        other => other,
    }
}

pub fn language_by_name(name: &str) -> Option<Arc<Language>> {
    let normalized = normalize_language_name(name);
    LANGUAGE_REGISTRY.language_by_name(normalized)
}

/// Find the corresponding language entry by the filename.
pub fn language_by_filename(path: &Path) -> Option<Arc<Language>> {
    // First check for specific filenames that don't use extensions.
    if let Some(filename) = path.file_name().and_then(|name| name.to_str()) {
        match filename {
            // Bash config files
            ".bashrc" | ".bash_profile" => {
                return language_by_name("shell");
            }
            // ZSH config files
            ".zshrc" | ".zsh_profile" | ".zprofile" => {
                return language_by_name("shell");
            }
            // Bazel build files
            "BUILD" | "WORKSPACE" => {
                return language_by_name("starlark");
            }
            // Dockerfiles
            "Dockerfile" | "Containerfile" | "dockerfile" | "containerfile" => {
                return language_by_name("dockerfile");
            }
            _ => {
                // Also match Dockerfile variants like Dockerfile.dev, Dockerfile.prod
                if filename.starts_with("Dockerfile.") || filename.starts_with("Containerfile.") {
                    return language_by_name("dockerfile");
                }
            }
        }
    }

    let extension = path.extension()?.to_str()?;
    match extension {
        "rs" => language_by_name("rust"),
        "go" => language_by_name("golang"),
        "yml" | "yaml" => language_by_name("yaml"),
        "py" | "py3" | "pyw" | "pyi" => language_by_name("python"),
        "js" | "cjs" | "mjs" => language_by_name("javascript"),
        "jsx" => language_by_name("jsx"),
        "tsx" => language_by_name("tsx"),
        "ts" | "cts" | "mts" => language_by_name("typescript"),
        "java" | "groovy" | "gvy" | "gy" | "gsh" => language_by_name("java"),
        "cpp" | "cxx" | "cc" | "h" | "hh" | "hpp" | "hxx" | "H" | "h++" => language_by_name("cpp"),
        "sh" | "zsh" | "bash" | "command" => language_by_name("shell"),
        "cs" => language_by_name("csharp"),
        "html" | "htm" => language_by_name("html"),
        "css" => language_by_name("css"),
        "c" => language_by_name("c"),
        "json" => language_by_name("json"),
        "jq" => language_by_name("jq"),
        "tf" | "hcl" | "tfvars" => language_by_name("hcl"),
        "lua" => language_by_name("lua"),
        "rb" => language_by_name("ruby"),
        "php" | "phtml" => language_by_name("php"),
        "toml" => language_by_name("toml"),
        "swift" => language_by_name("swift"),
        "kt" | "kts" => language_by_name("kotlin"),
        "scala" | "sbt" | "sc" => language_by_name("scala"),
        "ps1" | "pwsh" => language_by_name("powershell"),
        "ex" | "exs" => language_by_name("elixir"),
        "sql" => language_by_name("sql"),
        "bzl" | "bazel" => language_by_name("starlark"),
        "m" | "mm" => language_by_name("objective-c"),
        "xml" => language_by_name("xml"),
        "vue" => language_by_name("vue"),
        "dockerfile" => language_by_name("dockerfile"),
        _ => None,
    }
}

/// Captures the language-specific parser grammar and queries for syntax features like highlighting and
/// bracket pairing. In the future, this will also be the entry point for LSP.
pub struct Language {
    /// Tree-sitter parser grammar.
    pub grammar: ParserGrammar,
    /// Query for syntax highlighting.
    pub highlight_query: Query,
    /// Query for auto indent.
    pub indents_query: Option<Query>,
    /// Unit for each indent action.
    pub indent_unit: IndentUnit,
    /// Comment prefix.
    pub comment_prefix: Option<String>,
    /// Language-specific bracket pairs.
    pub bracket_pairs: Vec<(char, char)>,
    /// Query for parsing symbols.
    pub symbols_query: Option<Query>,
    /// Display name for the language.
    pub display_name: String,
}

impl Language {
    /// Returns the display name of the language.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct LanguageConfig {
    display_name: String,
    indent_unit: IndentUnit,
    comment_prefix: Option<String>,
    #[serde(default)]
    brackets: Vec<BracketPair>,
}

#[derive(Serialize, Deserialize, Debug)]
struct BracketPair {
    start: String,
    end: String,
}

/// Map our internal language name to the canonical arborium language name.
fn to_arborium_name(lang: &str) -> &str {
    match lang {
        "golang" => "go",
        "shell" => "bash",
        "csharp" => "c-sharp",
        "jsx" => "javascript",
        "objective-c" => "objc",
        "sql" => "sql",
        other => other,
    }
}

/// Get the bundled highlight query from arborium for a given language.
fn get_arborium_highlight_query(lang: &str) -> Option<&str> {
    match lang {
        "rust" => Some(arborium::lang_rust::HIGHLIGHTS_QUERY),
        "golang" => Some(arborium::lang_go::HIGHLIGHTS_QUERY),
        "yaml" => Some(arborium::lang_yaml::HIGHLIGHTS_QUERY),
        "python" => Some(arborium::lang_python::HIGHLIGHTS_QUERY),
        "javascript" => Some(arborium::lang_javascript::HIGHLIGHTS_QUERY),
        "jsx" => Some(arborium::lang_javascript::HIGHLIGHTS_QUERY),
        "typescript" => Some(&arborium::lang_typescript::HIGHLIGHTS_QUERY),
        "tsx" => Some(&arborium::lang_tsx::HIGHLIGHTS_QUERY),
        "java" => Some(arborium::lang_java::HIGHLIGHTS_QUERY),
        "cpp" => Some(&arborium::lang_cpp::HIGHLIGHTS_QUERY),
        "shell" => Some(arborium::lang_bash::HIGHLIGHTS_QUERY),
        "csharp" => Some(arborium::lang_c_sharp::HIGHLIGHTS_QUERY),
        "html" => Some(arborium::lang_html::HIGHLIGHTS_QUERY),
        "css" => Some(arborium::lang_css::HIGHLIGHTS_QUERY),
        "c" => Some(arborium::lang_c::HIGHLIGHTS_QUERY),
        "json" => Some(arborium::lang_json::HIGHLIGHTS_QUERY),
        "jq" => Some(arborium::lang_jq::HIGHLIGHTS_QUERY),
        "hcl" => Some(arborium::lang_hcl::HIGHLIGHTS_QUERY),
        "lua" => Some(arborium::lang_lua::HIGHLIGHTS_QUERY),
        "ruby" => Some(arborium::lang_ruby::HIGHLIGHTS_QUERY),
        "php" => Some(arborium::lang_php::HIGHLIGHTS_QUERY),
        "toml" => Some(arborium::lang_toml::HIGHLIGHTS_QUERY),
        "swift" => Some(arborium::lang_swift::HIGHLIGHTS_QUERY),
        "kotlin" => Some(arborium::lang_kotlin::HIGHLIGHTS_QUERY),
        "scala" => Some(arborium::lang_scala::HIGHLIGHTS_QUERY),
        "powershell" => Some(arborium::lang_powershell::HIGHLIGHTS_QUERY),
        "elixir" => Some(arborium::lang_elixir::HIGHLIGHTS_QUERY),
        "sql" => Some(arborium::lang_sql::HIGHLIGHTS_QUERY),
        "starlark" => Some(arborium::lang_starlark::HIGHLIGHTS_QUERY),
        "objective-c" => Some(&arborium::lang_objc::HIGHLIGHTS_QUERY),
        "xml" => Some(arborium::lang_xml::HIGHLIGHTS_QUERY),
        "vue" => Some(&arborium::lang_vue::HIGHLIGHTS_QUERY),
        "dockerfile" => Some(arborium::lang_dockerfile::HIGHLIGHTS_QUERY),
        _ => None,
    }
}

fn load_language(lang: &str) -> Option<Language> {
    let arborium_name = to_arborium_name(lang);
    let grammar = arborium::get_language(arborium_name)?;

    let config_path = [lang, "config.yaml"].join("\\");
    let config = load_yaml(&config_path);
    let indent_unit = config.indent_unit;
    let comment_prefix = config.comment_prefix;
    let bracket_pairs = config
        .brackets
        .into_iter()
        .filter_map(|bracket_pair| {
            let start = bracket_pair.start.chars().next()?;
            let end = bracket_pair.end.chars().next()?;
            Some((start, end))
        })
        .collect();

    // Use arborium's bundled highlight query instead of loading from custom .scm files
    let highlight_query_str = get_arborium_highlight_query(lang)?;
    let highlight_query = Query::new(&grammar, highlight_query_str)
        .expect("arborium highlight query should be valid");

    let indents_query_path = [lang, "indents.scm"].join("\\");
    let indents_query = load_query(&indents_query_path, &grammar);

    let symbols_query_path = [lang, "identifiers.scm"].join("\\");
    let symbols_query = load_query(&symbols_query_path, &grammar);

    Some(Language {
        highlight_query,
        indents_query,
        grammar,
        indent_unit,
        comment_prefix,
        bracket_pairs,
        symbols_query,
        display_name: config.display_name,
    })
}

fn load_yaml(path: &str) -> LanguageConfig {
    match <Grammars as RustEmbed>::get(path) {
        Some(file) => {
            let config: LanguageConfig =
                serde_yaml::from_slice(&file.data).expect("Unable to deserialize the YAML content");
            config
        }
        None => {
            panic!("Couldn't initiate yaml config from {path}");
        }
    }
}

fn load_query(path: &str, grammar: &ParserGrammar) -> Option<Query> {
    let file = <Grammars as RustEmbed>::get(path)?;
    let query_content = match file.data {
        Cow::Borrowed(inner) => Cow::Borrowed(std::str::from_utf8(inner).unwrap()),
        Cow::Owned(inner) => Cow::Owned(String::from_utf8(inner).unwrap()),
    };

    Some(
        Query::new(grammar, &query_content)
            .unwrap_or_else(|err| panic!("TSQuery creation should work from {path}: {err}")),
    )
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
