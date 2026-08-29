use anyhow::Result;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use prost::Message;
use session_sharing_protocol::common::Role;
use session_sharing_protocol::sharer::SessionSourceType;
use warp_multi_agent_api::ResponseEvent;
use warpui_core::id;
use warpui_core::keymap::ContextPredicate;

use crate::model::Point;

impl From<Point> for session_sharing_protocol::common::Point {
    fn from(val: Point) -> Self {
        session_sharing_protocol::common::Point {
            row: val.row,
            col: val.col,
        }
    }
}

impl From<session_sharing_protocol::common::Point> for Point {
    fn from(value: session_sharing_protocol::common::Point) -> Self {
        Self {
            row: value.row,
            col: value.col,
        }
    }
}

/// `SessionSourceType` paired with the orchestrator `task_id` that rides
/// on the `source_task_id` sidecar.
#[derive(Debug, Clone)]
pub struct SharedSessionSource {
    pub source_type: SessionSourceType,
    pub source_task_id: Option<String>,
}

impl SharedSessionSource {
    pub fn user(source_task_id: Option<String>) -> Self {
        Self {
            source_type: SessionSourceType::User,
            source_task_id,
        }
    }

    pub fn ambient_agent(task_id: Option<String>) -> Self {
        Self {
            source_type: SessionSourceType::AmbientAgent {
                task_id: task_id.clone(),
            },
            source_task_id: task_id,
        }
    }

    /// Sidecar first, then `AmbientAgent.task_id` for legacy producers.
    pub fn orchestrator_task_id(&self) -> Option<&str> {
        self.source_task_id.as_deref().or(match &self.source_type {
            SessionSourceType::AmbientAgent { task_id } => task_id.as_deref(),
            SessionSourceType::User => None,
        })
    }
}

impl Default for SharedSessionSource {
    fn default() -> Self {
        Self::user(None)
    }
}

/// The type of shared session a particular session is, if applicable.
#[derive(Debug, Clone)]
pub enum SharedSessionStatus {
    /// This session is not a shared session.
    /// When a sharer ends a session, the status
    /// changes back to [`SharedSessionStatus::NotShared`].
    NotShared,

    /// We're in the process of joining the session but have not
    /// established the connection with the server yet, or have not received all the events that occurred before the viewer joined yet.
    ViewPending,

    /// This session is a shared session that we are actively viewing.
    /// We have received all the scrollback and events for the shared session that occurred before the viewer joined, and are caught up and receiving events live.
    ActiveViewer { role: Role },

    /// We were viewing a shared session but it ended.
    FinishedViewer,

    /// We haven't yet attempted to share the session because it is not bootstrapped yet.
    /// The `source` encodes what kind of shared session will be created once
    /// the session finishes bootstrapping.
    SharePendingPreBootstrap { source: SharedSessionSource },

    /// The session is bootstrapped and we're in the process of
    /// sharing the session but have not yet established the
    /// connection with the server.
    SharePending,

    /// This session is actively being shared.
    ActiveSharer,
}

impl SharedSessionStatus {
    pub fn reader() -> Self {
        Self::ActiveViewer { role: Role::Reader }
    }

    pub fn executor() -> Self {
        Self::ActiveViewer {
            role: Role::Executor,
        }
    }

    pub fn is_view_pending(&self) -> bool {
        matches!(self, SharedSessionStatus::ViewPending)
    }

    pub fn is_active_viewer(&self) -> bool {
        matches!(self, SharedSessionStatus::ActiveViewer { .. })
    }

    pub fn is_finished_viewer(&self) -> bool {
        matches!(self, SharedSessionStatus::FinishedViewer)
    }

    pub fn is_viewer(&self) -> bool {
        self.is_view_pending() || self.is_active_viewer() || self.is_finished_viewer()
    }

    pub fn is_executor(&self) -> bool {
        matches!(self, SharedSessionStatus::ActiveViewer { role } if role.can_execute())
    }

    pub fn is_reader(&self) -> bool {
        matches!(
            self,
            SharedSessionStatus::ActiveViewer { role: Role::Reader }
        )
    }

    pub fn is_share_pending(&self) -> bool {
        matches!(
            self,
            SharedSessionStatus::SharePending
                | SharedSessionStatus::SharePendingPreBootstrap { .. }
        )
    }

    pub fn is_active_sharer(&self) -> bool {
        matches!(self, SharedSessionStatus::ActiveSharer)
    }

    pub fn is_sharer(&self) -> bool {
        self.is_share_pending() || self.is_active_sharer()
    }

    pub fn is_sharer_or_viewer(&self) -> bool {
        !matches!(self, Self::NotShared)
    }

    pub fn as_keymap_context(&self) -> &'static str {
        match self {
            Self::NotShared => "SharedSessionStatus_NotShared",
            Self::ViewPending => "SharedSessionStatus_ViewPending",
            Self::ActiveViewer { role: Role::Reader } => "SharedSessionStatus_Reader",
            Self::ActiveViewer {
                role: Role::Executor | Role::Full,
            } => "SharedSessionStatus_Executor",
            Self::FinishedViewer => "SharedSessionStatus_FinishedViewer",
            Self::SharePendingPreBootstrap { .. } => "SharedSessionStatus_SharePendingPreBootstrap",
            Self::SharePending => "SharedSessionStatus_SharePending",
            Self::ActiveSharer => "SharedSessionStatus_ActiveSharer",
        }
    }

    pub fn active_viewer_keymap_context() -> ContextPredicate {
        id!(Self::reader().as_keymap_context()) | id!(Self::executor().as_keymap_context())
    }
}

/// Decodes a serialized response event string by base64-decoding
/// and then decoding the protobuf payload into a ResponseEvent.
pub fn decode_agent_response_event(encoded: &str) -> Result<ResponseEvent> {
    let bytes = STANDARD_NO_PAD.decode(encoded)?;
    let event = ResponseEvent::decode(bytes.as_slice())?;
    Ok(event)
}

/// Encodes a ResponseEvent by protobuf-encoding it and base64-encoding the bytes.
pub fn encode_agent_response_event(event: &ResponseEvent) -> String {
    let bytes = event.encode_to_vec();
    STANDARD_NO_PAD.encode(bytes)
}
