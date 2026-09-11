use std::fmt;
use std::sync::Arc;

use base64::Engine;
use prost::Message;
use session_sharing_protocol::common::ProfileData;
use warp_multi_agent_api as api;

/// Unverified query metadata echoed independently of the session participant used for routing.
#[derive(Clone, PartialEq)]
pub struct UserQueryAttribution(Arc<api::UserQueryAttribution>);

impl fmt::Debug for UserQueryAttribution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserQueryAttribution")
            .finish_non_exhaustive()
    }
}

impl UserQueryAttribution {
    /// Marks a freshly submitted local query for the server to resolve its authenticated author.
    pub(crate) fn fresh_local() -> Self {
        Self(Arc::new(api::UserQueryAttribution {
            origin: Some(api::UserQueryOrigin {
                variant: Some(api::user_query_origin::Variant::WarpClient(
                    api::user_query_origin::WarpClient {},
                )),
            }),
            author: None,
            source_message: None,
        }))
    }

    pub(crate) fn from_message(query: &api::message::UserQuery) -> Option<Self> {
        if query.origin.is_none() && query.author.is_none() && query.source_message.is_none() {
            return None;
        }
        Some(Self(Arc::new(api::UserQueryAttribution {
            origin: query.origin.clone(),
            author: query.author.clone(),
            source_message: query.source_message.clone(),
        })))
    }

    pub(crate) fn from_shared_session(
        encoded: Option<&str>,
        requester: Option<&ProfileData>,
    ) -> Self {
        if let Some(encoded) = encoded {
            return match base64::engine::general_purpose::STANDARD.decode(encoded) {
                Ok(bytes) => match api::UserQueryAttribution::decode(bytes.as_slice()) {
                    Ok(attribution)
                        if attribution.origin.is_some()
                            || attribution.author.is_some()
                            || attribution.source_message.is_some() =>
                    {
                        Self(Arc::new(attribution))
                    }
                    Ok(_) => Self::unavailable("attribution_unavailable"),
                    Err(_) => {
                        log::warn!("Invalid protobuf in shared-session query attribution");
                        Self::unavailable("attribution_unavailable")
                    }
                },
                Err(_) => {
                    log::warn!("Invalid Base64 in shared-session query attribution");
                    Self::unavailable("attribution_unavailable")
                }
            };
        }
        let Some(requester) = requester.filter(|profile| !profile.firebase_uid.is_empty()) else {
            return Self::unavailable("shared_session_author_unavailable");
        };
        Self(Arc::new(api::UserQueryAttribution {
            origin: Some(api::UserQueryOrigin {
                variant: Some(api::user_query_origin::Variant::WarpClient(
                    api::user_query_origin::WarpClient {},
                )),
            }),
            author: Some(api::QueryAuthor {
                principal: Some(api::query_author::Principal::User(api::WarpUser {
                    uid: requester.firebase_uid.clone(),
                    email: requester.email.clone().unwrap_or_default(),
                    team_uid: String::new(),
                })),
                resolution: api::IdentityResolution::ClientSession.into(),
            }),
            source_message: None,
        }))
    }

    fn unavailable(reason: &str) -> Self {
        // Keep an explicit origin so missing forwarded metadata never attributes the query to its host.
        Self(Arc::new(api::UserQueryAttribution {
            origin: Some(api::UserQueryOrigin {
                variant: Some(api::user_query_origin::Variant::ServerSynthesized(
                    api::user_query_origin::ServerSynthesized {
                        reason: reason.into(),
                    },
                )),
            }),
            author: None,
            source_message: None,
        }))
    }

    pub(crate) fn envelope(&self) -> api::UserQueryAttribution {
        self.0.as_ref().clone()
    }

    pub(crate) fn request_fields(&self) -> api::request::input::UserQuery {
        api::request::input::UserQuery {
            origin: self.0.origin.clone(),
            author: self.0.author.clone(),
            source_message: self.0.source_message.clone(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
#[path = "query_attribution_tests.rs"]
mod tests;
