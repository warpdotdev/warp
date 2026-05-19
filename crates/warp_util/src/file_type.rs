//! File type detection utilities.
//!
//! This module provides utilities for determining whether a file is likely to be a text file
//! based on its filename and extension. It uses a hybrid approach combining MIME type detection
//! with explicit extension checking for edge cases.

use content_inspector::{inspect, ContentType};
use mime_guess::{self, mime};
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// File extensions for Markdown files.
const MARKDOWN_EXTENSIONS: &[&str] = &["md", "markdown"];

/// Names of files that are typically Markdown or plain text.
const MARKDOWN_FILE_NAMES: &[&str] = &["README", "CHANGELOG", "LICENSE"];

/// Checks if a buffer appears to contain binary content.
/// Returns true if the buffer appears to be binary, false if it appears to be text.
pub fn is_buffer_binary(buffer: &[u8]) -> bool {
    matches!(inspect(buffer), ContentType::BINARY)
}

/// Checks if a file's content appears to be binary by reading a small chunk.
/// Returns true if the file appears to be binary, false if it appears to be text.
/// Returns true if the file cannot be read.
pub fn is_file_content_binary(path: impl AsRef<Path>) -> bool {
    const CHUNK_SIZE: usize = 1024;

    let Ok(mut file) = File::open(path) else {
        return true;
    };

    let mut buffer = [0u8; CHUNK_SIZE];
    let Ok(n) = file.read(&mut buffer) else {
        return true;
    };

    is_buffer_binary(&buffer[..n])
}

/// Checks if a file is a binary file that should not be opened in Warp.
/// Note that we only check the file extension, not the file content.
/// Returns true for common binary file extensions like images, videos, executables, etc.
pub fn is_binary_file(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    match path.extension() {
        Some(ext) => {
            if let Some(ext) = ext.to_str() {
                matches!(
                    ext.to_lowercase().as_str(),
                    "jpg"
                        | "jpeg"
                        | "png"
                        | "gif"
                        | "bmp"
                        | "tiff"
                        | "tif"
                        | "webp"
                        | "ico"
                        | "pdf"
                        | "doc"
                        | "docx"
                        | "xls"
                        | "xlsx"
                        | "ppt"
                        | "pptx"
                        | "odt"
                        | "ods"
                        | "odp"
                        | "zip"
                        | "tar"
                        | "gz"
                        | "bz2"
                        | "xz"
                        | "7z"
                        | "rar"
                        | "dmg"
                        | "iso"
                        | "img"
                        | "exe"
                        | "msi"
                        | "deb"
                        | "rpm"
                        | "app"
                        | "pkg"
                        | "bin"
                        | "so"
                        | "dll"
                        | "dylib"
                        | "mp3"
                        | "mp4"
                        | "avi"
                        | "mov"
                        | "wmv"
                        | "flv"
                        | "mkv"
                        | "wav"
                        | "flac"
                        | "ogg"
                        | "woff"
                        | "woff2"
                        | "ttf"
                        | "otf"
                        | "eot"
                        | "db"
                        | "sqlite"
                        | "sqlite3"
                        | "pyc"
                        | "pyo"
                        | "class"
                        | "jar"
                )
            } else {
                false
            }
        }
        None => path
            .to_str()
            .map(|path| !is_text_file(path))
            .unwrap_or_default(),
    }
}

/// Guess whether or not `path` is a Markdown file:
/// * Does it have a Markdown extension?
/// * Is it an extension-less file that's commonly Markdown.
pub fn is_markdown_file(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    match path.extension() {
        Some(ext) => MARKDOWN_EXTENSIONS
            .iter()
            .any(|markdown_ext| ext.eq_ignore_ascii_case(markdown_ext)),
        None => path.file_name().is_some_and(|file_name| {
            MARKDOWN_FILE_NAMES
                .iter()
                .any(|markdown_name| file_name.eq_ignore_ascii_case(markdown_name))
        }),
    }
}

/// Determines if a file is likely to be a text file based on its filename.
///
/// This function uses a hybrid approach:
/// 1. First attempts MIME type detection via `mime_guess` for common cases
/// 2. Falls back to explicit extension checking for development-specific files
/// 3. Handles special cases like files without extensions that are commonly text
///
/// # Arguments
/// * `filename` - The filename or path to check
///
/// # Returns
/// `true` if the file is likely to be a text file, `false` otherwise
fn is_text_file(filename: &str) -> bool {
    // Use mime_guess for initial detection
    let mime = mime_guess::from_path(filename).first_or_octet_stream();

    // Check if it's explicitly a text MIME type
    if mime.type_() == mime::TEXT {
        return true;
    }

    // Check for common application types that are actually text
    if mime.type_() == mime::APPLICATION {
        let subtype = mime.subtype().as_str();
        if matches!(
            subtype,
            "json"
                | "xml"
                | "javascript"
                | "yaml"
                | "toml"
                | "x-yaml"
                | "x-toml"
                | "x-javascript"
                | "x-sh"
                | "x-shellscript"
                | "x-httpd-php"
                | "x-ruby"
                | "x-python"
                | "x-perl"
                | "sql"
        ) {
            return true;
        }
    }

    // Get the file extension for fallback checking
    let extension = Path::new(filename)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Explicit extension checking for development files that might not be caught by MIME
    if is_development_text_extension(&extension) {
        return true;
    }

    // Handle files without extensions that are commonly text
    if extension.is_empty() {
        return is_extensionless_text_file(filename);
    }

    false
}

/// Checks if a file extension corresponds to a development-related text file.
fn is_development_text_extension(extension: &str) -> bool {
    matches!(
        extension,
        // Programming languages not always caught by MIME
        "rs" | "go" | "py" | "py3" | "pyw" | "pyi" | "js" | "mjs" | "cjs" |
        "ts" | "tsx" | "jsx" | "java" | "c" | "cc" | "cpp" | "cxx" |
        "h" | "hh" | "hpp" | "hxx" | "cs" | "php" | "phtml" | "rb" | "swift" |
        "kt" | "kts" | "scala" | "sh" | "bash" | "zsh" | "fish" |
        "ps1" | "bat" | "cmd" | "asm" | "s" | "vb" | "pl" | "r" |
        "m" | "mm" | "dart" | "lua" | "vim" | "el" | "clj" | "cljs" |
        "hs" | "lhs" | "ml" | "mli" | "fs" | "fsi" | "fsx" | "ex" |
        "exs" | "erl" | "hrl" | "elm" | "nim" | "cr" | "zig" | "v" |
        "jl" | "rkt" | "scm" | "lisp" | "cl" | "coffee" | "purs" |
        "reason" | "re" | "res" | "resi" |
        // Web technologies
        "html" | "htm" | "css" | "scss" | "sass" | "less" | "vue" |
        "svelte" | "astro" | "blade" | "twig" | "mustache" | "hbs" |
        "handlebars" | "ejs" | "pug" | "jade" | "erb" | "haml" |
        // Configuration and data formats
        "toml" | "yaml" | "yml" | "json" | "jsonc" | "json5" | "jq" |
        "xml" | "ini" | "cfg" | "conf" | "config" | "properties" |
        "env" | "dotenv" | "editorconfig" | "gitignore" | "gitattributes" |
        // Documentation
        "md" | "markdown" | "mdown" | "mkd" | "rst" | "txt" |
        "rtf" | "tex" | "latex" | "adoc" | "asciidoc" | "org" |
        "pod" | "rdoc" | "textile" | "wiki" | "mediawiki" |
        // Build and project files
        "cmake" | "gradle" | "sbt" | "ant" | "maven" | "pom" |
        "build" | "mk" | "mak" | "ninja" | "bazel" | "bzl" |
        "dockerfile" | "containerfile" |
        // Package manager files
        "lock" | "sum" | "mod" |
        // Development tools config
        "prettierrc" | "eslintrc" | "stylelintrc" | "babelrc" |
        "postcssrc" | "browserslistrc" | "npmrc" | "yarnrc" |
        "nvmrc" | "rvmrc" | "gemfile" | "podfile" | "cartfile" |
        // Log and temporary files
        "log" | "diff" | "patch" | "bak" | "tmp" | "temp" |
        // Other common text formats
        "csv" | "tsv" | "sql" | "graphql" | "gql" | "proto" |
        "thrift" | "avro" | "schema" | "xsd" | "dtd" | "rng" |
        "rnc" | "wsdl" | "wadl"
    )
}

/// Checks if a file without an extension is commonly a text file.
fn is_extensionless_text_file(filename: &str) -> bool {
    let basename = Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(filename)
        .to_lowercase();

    matches!(basename.as_str(),
        // Common files without extensions
        "readme" | "license" | "licence" | "changelog" | "changes" |
        "authors" | "contributors" | "copying" | "install" |
        "news" | "todo" | "fixme" | "bugs" | "issues" | "release" |
        "history" | "version" | "notice" | "disclaimer" |
        // Build files
        "makefile" | "dockerfile" | "containerfile" | "rakefile" |
        "gemfile" | "podfile" | "cartfile" | "brewfile" |
        // Config files
        "procfile" | "profile" | "bashrc" | "zshrc" | "vimrc" |
        "tmux.conf" | "gitconfig" | "hgrc" |
        // Package files
        "cargo.toml" | "package.json" | "composer.json" |
        "pubspec.yaml" | "pyproject.toml"
    ) ||
    // Handle dot-prefixed config files
    basename.starts_with('.') && matches!(basename.as_str(),
        ".gitignore" | ".gitattributes" | ".editorconfig" |
        ".prettierrc" | ".eslintrc" | ".stylelintrc" | ".babelrc" |
        ".postcssrc" | ".browserslistrc" | ".npmrc" | ".yarnrc" |
        ".nvmrc" | ".rvmrc" | ".env" | ".envrc" | ".profile" |
        ".bashrc" | ".zshrc" | ".vimrc" | ".tmux.conf"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_common_text_files() {
        // Programming languages
        assert!(is_text_file("main.rs"));
        assert!(is_text_file("script.py"));
        assert!(is_text_file("app.js"));
        assert!(is_text_file("component.tsx"));
        assert!(is_text_file("Main.java"));
        assert!(is_text_file("header.h"));
        assert!(is_text_file("script.sh"));

        // Web files
        assert!(is_text_file("index.html"));
        assert!(is_text_file("styles.css"));
        assert!(is_text_file("component.vue"));

        // Configuration files
        assert!(is_text_file("config.json"));
        assert!(is_text_file("settings.yaml"));
        assert!(is_text_file("Cargo.toml"));
        assert!(is_text_file(".gitignore"));
        assert!(is_text_file(".env"));

        // Documentation
        assert!(is_text_file("README.md"));
        assert!(is_text_file("docs.txt"));
        assert!(is_text_file("manual.rst"));

        // Build files
        assert!(is_text_file("Dockerfile"));
        assert!(is_text_file("Makefile"));
        assert!(is_text_file("build.gradle"));

        // Files without extensions
        assert!(is_text_file("README"));
        assert!(is_text_file("LICENSE"));
        assert!(is_text_file("Dockerfile"));
    }

    #[test]
    fn test_binary_files() {
        // Images
        assert!(!is_text_file("image.png"));
        assert!(!is_text_file("photo.jpg"));
        assert!(!is_text_file("icon.ico"));
        // Note: SVG might be detected as text by MIME, which is correct

        // Executables
        assert!(!is_text_file("program.exe"));
        assert!(!is_text_file("app.dmg"));

        // Archives
        assert!(!is_text_file("archive.zip"));
        assert!(!is_text_file("package.tar.gz"));
        assert!(!is_text_file("data.7z"));

        // Media files
        assert!(!is_text_file("video.mp4"));
        assert!(!is_text_file("audio.mp3"));
        assert!(!is_text_file("sound.wav"));

        // Document formats (binary)
        assert!(!is_text_file("document.pdf"));
        assert!(!is_text_file("spreadsheet.xlsx"));
        assert!(!is_text_file("presentation.pptx"));
    }

    #[test]
    fn test_edge_cases() {
        // Empty filename
        assert!(!is_text_file(""));

        // Files with multiple extensions
        assert!(is_text_file("backup.tar.gz.txt"));
        assert!(is_text_file("config.local.json"));

        // Mixed case
        assert!(is_text_file("Component.TSX"));
        assert!(is_text_file("README.MD"));

        // Path separators
        assert!(is_text_file("/path/to/file.rs"));
        assert!(is_text_file("..\\windows\\path\\file.py"));

        // Unusual but valid text files
        assert!(is_text_file("script.fish"));
        assert!(is_text_file("data.graphql"));
        assert!(is_text_file("schema.proto"));
    }

    #[test]
    fn test_development_extensions() {
        // Test some specific development file types
        assert!(is_development_text_extension("rs"));
        assert!(is_development_text_extension("py"));
        assert!(is_development_text_extension("dockerfile"));
        assert!(is_development_text_extension("yaml"));

        assert!(!is_development_text_extension("png"));
        assert!(!is_development_text_extension("exe"));
        assert!(!is_development_text_extension("zip"));
    }

    #[test]
    fn test_extensionless_files() {
        assert!(is_extensionless_text_file("README"));
        assert!(is_extensionless_text_file("LICENSE"));
        assert!(is_extensionless_text_file("Dockerfile"));
        assert!(is_extensionless_text_file(".gitignore"));
        assert!(is_extensionless_text_file(".env"));

        assert!(!is_extensionless_text_file("binary"));
        assert!(!is_extensionless_text_file("unknown"));
        assert!(!is_extensionless_text_file("data"));
    }
}
