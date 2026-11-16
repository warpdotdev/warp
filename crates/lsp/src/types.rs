use std::path::PathBuf;

use lsp_types::{
    FileChangeType, FileEvent, Location as LspLocation, LocationLink, Position as LspPosition,
    Range as LspRange,
};

use crate::config::{lsp_uri_to_path, path_to_lsp_uri};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLocation {
    pub path: PathBuf,
    pub location: Location,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

impl Location {
    pub fn into_lsp(self) -> LspPosition {
        LspPosition {
            line: self.line as u32,
            character: self.column as u32,
        }
    }
}

impl From<LspLocation> for Location {
    fn from(location: LspLocation) -> Self {
        Self {
            line: location.range.start.line as usize,
            column: location.range.start.character as usize,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Range {
    pub start: Location,
    pub end: Location,
}

impl Range {
    fn into_lsp(self) -> LspRange {
        LspRange {
            start: self.start.into_lsp(),
            end: self.end.into_lsp(),
        }
    }
}

impl From<LspRange> for Range {
    fn from(range: LspRange) -> Self {
        Self {
            start: Location {
                line: range.start.line as usize,
                column: range.start.character as usize,
            },
            end: Location {
                line: range.end.line as usize,
                column: range.end.character as usize,
            },
        }
    }
}

#[derive(Debug)]
pub struct LspDefinitionLocation {
    origin: Option<LspRange>,
    target: LspLocation,
}

impl From<LspLocation> for LspDefinitionLocation {
    fn from(location: LspLocation) -> Self {
        Self {
            origin: None,
            target: location,
        }
    }
}

impl From<LocationLink> for LspDefinitionLocation {
    fn from(location_link: LocationLink) -> Self {
        Self {
            origin: location_link.origin_selection_range,
            target: LspLocation {
                uri: location_link.target_uri,
                // Use target_selection_range (the exact identifier location) instead of
                // target_range (the full declaration range including comments/attributes)
                // to jump directly to the definition name rather than the start of the
                // declaration.
                range: location_link.target_selection_range,
            },
        }
    }
}

#[derive(Debug)]
pub struct DefinitionLocation {
    pub origin: Option<Range>,
    pub target: FileLocation,
}

impl TryFrom<LspDefinitionLocation> for DefinitionLocation {
    type Error = anyhow::Error;

    fn try_from(location: LspDefinitionLocation) -> anyhow::Result<Self> {
        let path = lsp_uri_to_path(&location.target.uri)?;

        Ok(Self {
            origin: location.origin.map(Into::into),
            target: FileLocation {
                path,
                location: location.target.into(),
            },
        })
    }
}

/// A reference location returned from the LSP textDocument/references request.
#[derive(Debug, Clone)]
pub struct ReferenceLocation {
    pub file_path: PathBuf,
    pub range: Range,
}

impl TryFrom<LspLocation> for ReferenceLocation {
    type Error = anyhow::Error;

    fn try_from(location: LspLocation) -> anyhow::Result<Self> {
        let path = lsp_uri_to_path(&location.uri)?;

        Ok(Self {
            file_path: path,
            range: location.range.into(),
        })
    }
}

/// Edit returned by the LSP.
pub struct TextEdit {
    pub range: Range,
    pub text: String,
}

impl From<lsp_types::TextEdit> for TextEdit {
    fn from(edit: lsp_types::TextEdit) -> Self {
        Self {
            range: edit.range.into(),
            text: edit.new_text,
        }
    }
}

/// Document version that should be tracked by the LSP.
pub struct DocumentVersion(i32);

impl DocumentVersion {
    pub fn as_i32(&self) -> i32 {
        self.0
    }
}

impl From<usize> for DocumentVersion {
    fn from(version: usize) -> Self {
        Self(version as i32)
    }
}

#[derive(Debug)]
pub struct TextDocumentContentChangeEvent {
    pub range: Option<Range>,
    pub text: String,
}

impl TextDocumentContentChangeEvent {
    pub fn into_lsp(self) -> lsp_types::TextDocumentContentChangeEvent {
        lsp_types::TextDocumentContentChangeEvent {
            range: self.range.map(|range| range.into_lsp()),
            range_length: None,
            text: self.text,
        }
    }
}

/// Result from a hover request.
#[derive(Debug, Clone)]
pub struct HoverResult {
    /// The hover contents (documentation, type info, etc.)
    pub contents: HoverContents,
    /// The range of the symbol being hovered over, if provided by the server.
    pub range: Option<Range>,
}

/// The kind of markup content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkupKind {
    PlainText,
    Markdown,
}

#[derive(Debug, Clone)]
pub struct HoverContentSection {
    pub value: String,
    pub kind: MarkupKind,
}

/// The contents of a hover response.
#[derive(Debug, Clone)]
pub struct HoverContents {
    pub sections: Vec<HoverContentSection>,
}

impl HoverContents {
    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }
}

impl From<lsp_types::Hover> for HoverResult {
    fn from(hover: lsp_types::Hover) -> Self {
        let contents = match hover.contents {
            lsp_types::HoverContents::Scalar(value) => vec![HoverContentSection {
                value: marked_string_to_string(value),
                kind: MarkupKind::Markdown,
            }],
            lsp_types::HoverContents::Array(values) => values
                .into_iter()
                .map(|value| HoverContentSection {
                    value: marked_string_to_string(value),
                    kind: MarkupKind::Markdown,
                })
                .collect(),
            lsp_types::HoverContents::Markup(content) => {
                let kind = match content.kind {
                    lsp_types::MarkupKind::PlainText => MarkupKind::PlainText,
                    lsp_types::MarkupKind::Markdown => MarkupKind::Markdown,
                };
                vec![HoverContentSection {
                    value: content.value,
                    kind,
                }]
            }
        };

        Self {
            contents: HoverContents { sections: contents },
            range: hover.range.map(Into::into),
        }
    }
}

fn marked_string_to_string(marked: lsp_types::MarkedString) -> String {
    match marked {
        lsp_types::MarkedString::String(s) => s,
        lsp_types::MarkedString::LanguageString(ls) => {
            format!("```{}\n{}\n```", ls.language, ls.value)
        }
    }
}

/// A file change event that can be forwarded to the language server using
/// `workspace/didChangeWatchedFiles`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchedFileChangeEvent {
    pub path: PathBuf,
    pub typ: FileChangeType,
}

impl WatchedFileChangeEvent {
    pub fn into_lsp(self) -> anyhow::Result<FileEvent> {
        Ok(FileEvent {
            uri: path_to_lsp_uri(&self.path)?,
            typ: self.typ,
        })
    }
}
