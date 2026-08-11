use std::fmt::Display;

use warp_errors::{ReportErrorLogMode, report_error};
use warp_multi_agent_api as api;

/// A citation listed in an AI response.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum AIAgentCitation {
    WarpDriveObject {
        uid: String,
    },
    WarpDocumentation {
        path: String,
    },
    WebPage {
        url: String,
    },
    /// A memory from an attached memory store. `content` is the raw memory
    /// text shown as a preview in the chip.
    AgentMemory {
        memory_store_id: String,
        memory_id: String,
        content: String,
    },
}

impl Display for AIAgentCitation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AIAgentCitation::WarpDriveObject { uid } => {
                write!(f, "Warp Drive Object: {uid}")
            }
            AIAgentCitation::WarpDocumentation { path } => {
                write!(f, "Warp Documentation: {path}")
            }
            AIAgentCitation::WebPage { url } => {
                write!(f, "Web Page: {url}")
            }
            AIAgentCitation::AgentMemory {
                memory_store_id,
                memory_id,
                ..
            } => {
                write!(f, "Agent Memory: {memory_store_id}/{memory_id}")
            }
        }
    }
}

/// Error type for Citation conversion errors
#[derive(Debug, thiserror::Error)]
#[error("Unknown citation type")]
pub struct UnknownCitationTypeError;

impl TryFrom<api::Citation> for AIAgentCitation {
    type Error = UnknownCitationTypeError;

    fn try_from(citation: api::Citation) -> Result<Self, Self::Error> {
        let doc_type = api::DocumentType::try_from(citation.document_type)
            .unwrap_or(api::DocumentType::Unknown);

        match doc_type {
            api::DocumentType::WarpDriveWorkflow
            | api::DocumentType::WarpDriveNotebook
            | api::DocumentType::WarpDriveEnvVar
            | api::DocumentType::Rule => Ok(AIAgentCitation::WarpDriveObject {
                uid: citation.document_id,
            }),
            api::DocumentType::WarpDocumentation => Ok(AIAgentCitation::WarpDocumentation {
                path: citation.document_id,
            }),
            api::DocumentType::WebPage => Ok(AIAgentCitation::WebPage {
                url: citation.document_id,
            }),
            api::DocumentType::Unknown => {
                // The LLM produced a citation `document_type` string outside the set the
                // server recognizes (see `ApiDocumentTypeFromDocumentType` server-side), so
                // it fell back to `UNKNOWN`. This is uncertain external (LLM) behavior that
                // may also mean we're missing a document type mapping here, so it's worth an
                // engineer's attention -- but it can recur across many messages/citations in
                // a single run, so throttle to once per run.
                //
                // `document_id` is opaque, LLM-derived content (it can carry a URL with query
                // params, a filesystem path, etc.), so it must not be logged/reported -- only
                // the bounded `document_type` enum value and non-content metadata go in `extra:`.
                report_error!(
                    "Citation has an unrecognized document type; dropping it",
                    extra: {
                        "document_type" => %citation.document_type,
                        "document_id_len" => %citation.document_id.len(),
                    },
                    ReportErrorLogMode::OncePerRun
                );
                Err(UnknownCitationTypeError)
            }
        }
    }
}
