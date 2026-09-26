use std::time::SystemTime;

use sha2::{Digest, Sha256};
pub mod auth;
pub mod client;
pub mod codebase_index_proto;
pub mod host_id;
pub mod host_response;
pub mod manager;
pub mod protocol;
pub mod repo_metadata_proto;
pub mod setup;
#[cfg(not(target_family = "wasm"))]
pub mod ssh;
pub mod transport;

pub use host_id::HostId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpectedFileRevision {
    Present {
        content_digest: [u8; 32],
        last_modified: Option<SystemTime>,
    },
    Missing,
    Uneditable,
}

impl ExpectedFileRevision {
    pub fn from_content(content: impl AsRef<[u8]>) -> Self {
        Self::Present {
            content_digest: Sha256::digest(content).into(),
            last_modified: None,
        }
    }

    pub fn to_proto(self) -> proto::ExpectedFileRevision {
        let state = match self {
            Self::Present { content_digest, .. } => Some(
                proto::expected_file_revision::State::ContentSha256(content_digest.to_vec()),
            ),
            Self::Missing => Some(proto::expected_file_revision::State::Missing(true)),
            Self::Uneditable => Some(proto::expected_file_revision::State::Uneditable(true)),
        };
        proto::ExpectedFileRevision {
            state,
            last_modified_epoch_millis: None,
        }
    }
}

#[allow(clippy::large_enum_variant)]
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/remote_server.rs"));

    // ── ClientMessage constructors ──────────────────────────────────
    //
    // These helpers wrap inner message types in the appropriate
    // HostScopedRequest / SessionScopedRequest / Notification envelope
    // so call sites don't need triple-nested struct literals.

    impl ClientMessage {
        /// Build a `ClientMessage` carrying a host-scoped request.
        pub fn host_scoped(request_id: String, inner: host_scoped_request::Message) -> Self {
            Self {
                request_id,
                message: Some(client_message::Message::HostScoped(HostScopedRequest {
                    message: Some(inner),
                })),
            }
        }

        /// Build a `ClientMessage` carrying a session-scoped request.
        pub fn session_scoped(request_id: String, inner: session_scoped_request::Message) -> Self {
            Self {
                request_id,
                message: Some(client_message::Message::SessionScoped(
                    SessionScopedRequest {
                        message: Some(inner),
                    },
                )),
            }
        }

        /// Build a `ClientMessage` carrying a notification (fire-and-forget).
        pub fn notification(inner: notification::Message) -> Self {
            Self {
                request_id: String::new(),
                message: Some(client_message::Message::Notification(Notification {
                    message: Some(inner),
                })),
            }
        }
    }
}
