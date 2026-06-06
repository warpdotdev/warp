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
