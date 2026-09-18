//! The `Request.Input.UserQuery` a shared-session agent prompt starts from.
//!
//! warp-server injects follow-ups (Slack replies, GitHub comments, automations, ...) into a
//! shared session as an `AgentPromptRequest` whose `user_query_b64` carries the serialized
//! `warp.multi_agent.v1.Request.Input.UserQuery`. That proto is the authoritative query; the
//! request's `prompt` and `attachments` only duplicate its text and files for older sharers.
//!
//! The sharer decodes it here and keeps it as the *base* of the `AIAgentInput::UserQuery` it
//! builds: the fields this client models (text, mode, intended agent) are seeded from it by
//! [`BaseUserQuery::seed_input_fields`], and `api::convert_to` writes them back over the base
//! when the request is sent. Every other field, including ones this client does not model
//! (origin, author, source message, ...), travels through untouched.
//!
//! Older relays and viewer-typed prompts leave `user_query_b64` unset, and a payload that does
//! not decode is ignored; both fall back to the request's `prompt` and `attachments`.

use std::fmt;

#[cfg(any(test, feature = "local_tty"))]
use base64::Engine as _;
#[cfg(any(test, feature = "local_tty"))]
use prost::Message as _;
#[cfg(any(test, feature = "local_tty"))]
use warp_errors::report_error;
use warp_multi_agent_api as api;
use warp_multi_agent_api::AgentType;

use super::api::convert_user_query_mode;
use super::{UserQueryMode, extract_user_query_mode};

/// Boxed for size only: the proto is ~1 KiB inline, which would bloat every `AIAgentInput` and
/// `QueuedQueryKind` variant (clippy `large_enum_variant`). Nothing shares it across threads.
#[derive(Clone, PartialEq)]
pub struct BaseUserQuery(Box<api::request::input::UserQuery>);

impl BaseUserQuery {
    /// Decodes the standard-Base64 protobuf carried on `AgentPromptRequest::user_query_b64`.
    ///
    /// Returns `None` (after reporting the failure) when the payload is not valid Base64 or not
    /// a valid `Request.Input.UserQuery`; a partially decoded query is never returned. A payload
    /// that does not decode means warp-server and this client disagree on the encoding, which
    /// is a bug on one side, so it is reported rather than only logged.
    #[cfg(any(test, feature = "local_tty"))]
    pub(crate) fn decode_b64(encoded: &str) -> Option<Self> {
        let bytes = match base64::engine::general_purpose::STANDARD.decode(encoded) {
            Ok(bytes) => bytes,
            Err(err) => {
                report_error!(
                    anyhow::Error::new(err)
                        .context("Ignoring shared-session user query: payload is not base64")
                );
                return None;
            }
        };
        match api::request::input::UserQuery::decode(bytes.as_slice()) {
            Ok(query) => Some(Self::from_proto(query)),
            Err(err) => {
                report_error!(
                    anyhow::Error::new(err)
                        .context("Ignoring shared-session user query: payload is not a UserQuery")
                );
                None
            }
        }
    }

    #[cfg(any(test, feature = "local_tty"))]
    pub(crate) fn from_proto(query: api::request::input::UserQuery) -> Self {
        Self(Box::new(query))
    }

    /// The query text, or `None` when the server left it empty and only duplicated it into
    /// `AgentPromptRequest::prompt`.
    pub(crate) fn query(&self) -> Option<&str> {
        (!self.0.query.is_empty()).then_some(self.0.query.as_str())
    }

    /// The mode the server set, or `None` when it left the field unset.
    pub(crate) fn user_query_mode(&self) -> Option<UserQueryMode> {
        self.0
            .mode
            .as_ref()
            .map(|mode| convert_user_query_mode(Some(mode)))
    }

    /// The agent the server named, or `None` when it left the field unspecified.
    pub(crate) fn intended_agent(&self) -> Option<AgentType> {
        AgentType::try_from(self.0.intended_agent)
            .ok()
            .filter(|agent| *agent != AgentType::Unknown)
    }

    /// The text, mode, and intended agent an `AIAgentInput::UserQuery` built on this base
    /// should carry.
    ///
    /// The server's fields win where it set them; the client's values fill in what it left
    /// unset. Text the server sent is normalized like typed text (a `/plan` or `/orchestrate`
    /// prefix is stripped into the mode), unless the server classified the mode differently,
    /// in which case the text is kept verbatim so the prefix is neither lost nor doubled when
    /// the mode is rendered back in front of it.
    pub(crate) fn seed_input_fields(
        &self,
        client_query: String,
        client_mode: UserQueryMode,
        client_agent: Option<AgentType>,
    ) -> (String, UserQueryMode, Option<AgentType>) {
        let base_mode = self.user_query_mode();
        let (query, mode) = match self.query() {
            None => (client_query, base_mode.unwrap_or(client_mode)),
            Some(text) => {
                let (stripped, text_mode) = extract_user_query_mode(text.to_owned());
                match base_mode {
                    None => (stripped, text_mode),
                    Some(mode) if mode == text_mode => (stripped, mode),
                    Some(mode) => (text.to_owned(), mode),
                }
            }
        };
        (query, mode, self.intended_agent().or(client_agent))
    }

    /// The decoded query, cloned so the outgoing request's user query can be written over it.
    pub(crate) fn to_proto(&self) -> api::request::input::UserQuery {
        (*self.0).clone()
    }
}

impl fmt::Debug for BaseUserQuery {
    /// Prints shape only: the query text, attachments, and attribution carry user content and
    /// identities that must not reach logs.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BaseUserQuery")
            .field("query_len", &self.0.query.len())
            .field("attachment_count", &self.0.referenced_attachments.len())
            .field("has_mode", &self.0.mode.is_some())
            .field("has_origin", &self.0.origin.is_some())
            .field("has_author", &self.0.author.is_some())
            .field("has_source_message", &self.0.source_message.is_some())
            .finish()
    }
}

#[cfg(test)]
#[path = "base_user_query_tests.rs"]
mod tests;
