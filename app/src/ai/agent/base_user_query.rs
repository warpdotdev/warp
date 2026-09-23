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
//! Viewer-typed prompts and older relays leave `user_query_b64` unset; their attribution comes
//! from the viewer's presence profile. Invalid payloads get explicit unavailable attribution.
//! Both fall back to the request's `prompt` and `attachments` for content.

use std::fmt;

#[cfg(any(test, feature = "local_tty"))]
use base64::Engine as _;
#[cfg(any(test, feature = "local_tty"))]
use prost::Message as _;
#[cfg(any(test, feature = "local_tty"))]
use session_sharing_protocol::common::ProfileData;
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

    pub(crate) fn from_proto(query: api::request::input::UserQuery) -> Self {
        Self(Box::new(query))
    }

    /// The query to send for a prompt a shared-session viewer typed themselves. The relay
    /// leaves `user_query_b64` unset for those, so the sharer records the viewer from presence:
    /// a `WarpClient` origin and the viewer as author. The relay authenticated the viewer and
    /// the sharer only observes them, so the resolution is `CLIENT_SESSION` and no team is
    /// claimed. A viewer whose profile is unknown gets an explicit `ServerSynthesized` origin,
    /// so the query is never attributed to the sharer as if they had typed it.
    #[cfg(any(test, feature = "local_tty"))]
    pub(crate) fn for_viewer(profile: Option<&ProfileData>) -> Self {
        let Some(profile) = profile.filter(|profile| !profile.firebase_uid.is_empty()) else {
            return Self::unattributed("shared_session_author_unavailable");
        };
        Self::from_proto(api::request::input::UserQuery {
            origin: Some(warp_client_origin()),
            author: Some(api::QueryAuthor {
                principal: Some(api::query_author::Principal::User(api::WarpUser {
                    uid: profile.firebase_uid.clone(),
                    email: profile.email.clone().unwrap_or_default(),
                    team_uid: String::new(),
                })),
                resolution: api::IdentityResolution::ClientSession.into(),
            }),
            ..Default::default()
        })
    }

    /// A query whose author cannot be established, marked with a `ServerSynthesized` origin
    /// naming `reason` so warp-server neither treats it as fresh local input nor leaves it
    /// looking like the sharer's own.
    #[cfg(any(test, feature = "local_tty"))]
    pub(crate) fn unattributed(reason: &str) -> Self {
        Self::from_proto(api::request::input::UserQuery {
            origin: Some(api::UserQueryOrigin {
                variant: Some(api::user_query_origin::Variant::ServerSynthesized(
                    api::user_query_origin::ServerSynthesized {
                        reason: reason.to_string(),
                    },
                )),
            }),
            ..Default::default()
        })
    }

    /// Lifts a persisted or streamed `Message.UserQuery`'s attribution (origin, author, source
    /// message, and nothing else) so a restored or live input keeps who was behind it when it
    /// is sent again. `None` when the message carries no attribution: historical queries stay
    /// unattributed rather than being re-stamped as the current user's.
    pub(crate) fn from_message(query: &api::message::UserQuery) -> Option<Self> {
        if query.origin.is_none() && query.author.is_none() && query.source_message.is_none() {
            return None;
        }
        Some(Self::from_proto(api::request::input::UserQuery {
            origin: query.origin.clone(),
            author: query.author.clone(),
            source_message: query.source_message.clone(),
            ..Default::default()
        }))
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

/// The origin of a query a person typed in a Warp client.
pub(crate) fn warp_client_origin() -> api::UserQueryOrigin {
    api::UserQueryOrigin {
        variant: Some(api::user_query_origin::Variant::WarpClient(
            api::user_query_origin::WarpClient {},
        )),
    }
}

#[cfg(test)]
#[path = "base_user_query_tests.rs"]
mod tests;
