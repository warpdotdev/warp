use std::fmt::{Display, Formatter};
use std::io;
use std::path::{Path, PathBuf};

use chrono::Local;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationFileExport {
    path: PathBuf,
    overwrote_existing: bool,
}

impl ConversationFileExport {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn overwrote_existing(&self) -> bool {
        self.overwrote_existing
    }
}

#[derive(Debug)]
pub struct ConversationFileExportError {
    path: PathBuf,
    source: io::Error,
}

impl ConversationFileExportError {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn user_message(&self) -> String {
        match self.source.kind() {
            io::ErrorKind::PermissionDenied => format!(
                "Permission denied writing to {}. Check file permissions.",
                self.path.display()
            ),
            io::ErrorKind::NotFound => format!(
                "Directory not found: {}",
                self.path
                    .parent()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default()
            ),
            io::ErrorKind::AlreadyExists => {
                format!("File {} already exists", self.path.display())
            }
            _ => format!(
                "Failed to export to {}: {}",
                self.path.display(),
                self.source
            ),
        }
    }
}

impl Display for ConversationFileExportError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "failed to write conversation to {}: {}",
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for ConversationFileExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

pub fn export_conversation_markdown(
    current_directory: Option<&str>,
    filename_arg: Option<&str>,
    conversation_title: Option<&str>,
    markdown: &str,
) -> Result<ConversationFileExport, ConversationFileExportError> {
    let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let filename = conversation_export_filename_at(filename_arg, conversation_title, &timestamp);
    let current_directory = current_directory
        .map(PathBuf::from)
        .or_else(|| {
            log::debug!("No active conversation CWD, falling back to std::env::current_dir()");
            std::env::current_dir().ok()
        })
        .unwrap_or_else(|| {
            log::warn!("Failed to determine current directory, using '.'");
            PathBuf::from(".")
        });
    let path = current_directory.join(filename);
    let overwrote_existing = path.exists();

    std::fs::write(&path, markdown).map_err(|source| ConversationFileExportError {
        path: path.clone(),
        source,
    })?;

    Ok(ConversationFileExport {
        path,
        overwrote_existing,
    })
}

fn conversation_export_filename_at(
    filename_arg: Option<&str>,
    conversation_title: Option<&str>,
    timestamp: &str,
) -> String {
    let filename = filename_arg
        .map(str::trim)
        .filter(|filename| !filename.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let title = conversation_title
                .unwrap_or("conversation")
                .chars()
                .map(|character| {
                    if character.is_whitespace() {
                        '_'
                    } else if character.is_alphanumeric() || character == '_' || character == '-' {
                        character
                    } else {
                        '_'
                    }
                })
                .collect::<String>();
            format!("{timestamp}-{title}.md")
        });

    if filename.ends_with(".md") {
        filename
    } else {
        format!("{filename}.md")
    }
}

#[cfg(test)]
#[path = "conversation_export_tests.rs"]
mod tests;
