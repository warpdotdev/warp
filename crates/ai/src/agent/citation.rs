use std::fmt::Display;
use std::hash::{Hash, Hasher};

use warp_multi_agent_api as api;

/// A citation listed in an AI response.
#[derive(Debug, Clone)]
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
    /// text shown as a preview in the chip; `Hash`/`Eq` use only the IDs.
    AgentMemory {
        memory_store_id: String,
        memory_id: String,
        content: String,
    },
}

impl PartialEq for AIAgentCitation {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::WarpDriveObject { uid: a }, Self::WarpDriveObject { uid: b }) => a == b,
            (Self::WarpDocumentation { path: a }, Self::WarpDocumentation { path: b }) => a == b,
            (Self::WebPage { url: a }, Self::WebPage { url: b }) => a == b,
            (
                Self::AgentMemory {
                    memory_store_id: s1,
                    memory_id: i1,
                    ..
                },
                Self::AgentMemory {
                    memory_store_id: s2,
                    memory_id: i2,
                    ..
                },
            ) => s1 == s2 && i1 == i2,
            _ => false,
        }
    }
}

impl Eq for AIAgentCitation {}

impl Hash for AIAgentCitation {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::WarpDriveObject { uid } => {
                0u8.hash(state);
                uid.hash(state);
            }
            Self::WarpDocumentation { path } => {
                1u8.hash(state);
                path.hash(state);
            }
            Self::WebPage { url } => {
                2u8.hash(state);
                url.hash(state);
            }
            Self::AgentMemory {
                memory_store_id,
                memory_id,
                ..
            } => {
                3u8.hash(state);
                memory_store_id.hash(state);
                memory_id.hash(state);
            }
        }
    }
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
            api::DocumentType::Unknown => Err(UnknownCitationTypeError),
        }
    }
}
