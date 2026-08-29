use byte_unit::Byte;
use instant::Duration;
use serde::{Deserialize, Serialize};
use session_sharing_protocol::common::{Role, Scrollback, ScrollbackBlock, SessionId};
use warpui::{AppContext, WindowId};

use super::TerminalModel;
use super::model::block::SerializedBlock;
use super::model::terminal_model::BlockIndex;
use crate::channel::{Channel, ChannelState};
use crate::editor::{InteractionState, ReplicaId};
use crate::features::FeatureFlag;

pub mod ai_agent;
pub mod manager;
pub mod network;
pub mod participant_avatar_view;
pub mod permissions_manager;
pub mod presence_manager;
pub mod render_util;
pub mod replay_agent_conversations;
pub mod role_change_modal;
mod selections;
pub mod settings;
pub mod share_modal;
pub(super) mod shared_handlers;
pub mod sharer;
pub mod viewer;

#[cfg(test)]
pub use tests::MAX_BYTES_SHAREABLE;
pub use warp_terminal::shared_session::{SharedSessionSource, SharedSessionStatus};

/// The toast copy when copying a shared session link.
pub const COPY_LINK_TEXT: &str = "Sharing link copied";

/// Throttle period for selection updates. We throttle instead of debounce because we want
/// to send selections even when it updates fast, so it appears live.
/// Our throttle implementation throttles on the trailing edge (does not drop messages at the end, so the
/// most up to date will always be sent after some delay)
const SELECTION_THROTTLE_PERIOD: Duration = Duration::from_millis(20);

/// Whether or not a local session is also being shared.
/// Since a shared session creator is also the creator of a local session,
/// we make use of the local_tty::TerminalManager for shared session creators.
/// Otherwise, there would be a lot of overlap between a shared session creator
/// and a regular, purely local session.
#[derive(Debug, Clone, Default)]
pub enum IsSharedSessionCreator {
    /// This session should be shared automatically once bootstrapped.
    Yes { source: SharedSessionSource },
    #[default]
    No,
}

/// The scrollback options when starting a shared session.
/// Note: currently, these options only encode the point at which
/// scrollback _starts_. We do not yet support more
/// selective scrollback (e.g. a closed range).
/// The active block is included for the prompt when it is scrollback-eligible.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedSessionScrollbackType {
    /// Do not include any scrollback in this shared session.
    /// The active block can still be sent as part of scrollback for the prompt.
    /// TODO(suraj): consider renaming this to "from active block" or encapsulating
    /// this with the `FromBlock` variant with the block_index equal to the
    /// active block index.
    None,

    /// Include scrollback starting at `block_index`.
    FromBlock { block_index: BlockIndex },

    /// The entire blocklist should be part of the scrollback.
    All,
}

impl SharedSessionScrollbackType {
    /// Returns the set of scrollback that adheres to the scrollback type.
    /// Note that some blocks might not actually be included in the scrollback
    /// even if they were specified as part of the scrollback type.
    /// For example, if the [`Self::All]` variant is used, restored blocks
    /// _won't_ be included in scrollback, and neither will hidden active blocks.
    fn to_scrollback(self, model: &TerminalModel) -> Scrollback {
        let first_block_index = self.first_block_index(model);
        let blocks = model
            .block_list()
            .blocks()
            .iter()
            .skip(first_block_index.into())
            .filter(|block| {
                block.is_scrollback_block_for_shared_session(model.block_list().transcript_scope())
            })
            .filter_map(|block| {
                let serialized_block: SerializedBlock = block.into();
                let bytes = serde_json::to_vec(&serialized_block);
                bytes.ok().map(|raw| ScrollbackBlock { raw })
            })
            .collect();

        let is_alt_screen_active = model.is_alt_screen_active();

        Scrollback {
            blocks,
            is_alt_screen_active,
        }
    }

    /// Returns the first block index that will be used for scrollback.
    pub fn first_block_index(self, model: &TerminalModel) -> BlockIndex {
        match self {
            Self::None => model.block_list().active_block_index(),
            Self::FromBlock { block_index } => model
                .block_list()
                .blocks()
                .iter()
                .skip(block_index.into())
                .find(|block| {
                    block.is_scrollback_block_for_shared_session(
                        model.block_list().transcript_scope(),
                    )
                })
                .map_or(model.block_list().active_block_index(), |block| {
                    block.index()
                }),
            Self::All => Self::FromBlock {
                block_index: BlockIndex::zero(),
            }
            .first_block_index(model),
        }
    }
}

#[cfg(not(test))]
pub fn max_session_size(window_id: WindowId, app: &AppContext) -> Byte {
    use warpui::SingletonEntity;

    use crate::workspaces::user_workspaces::UserWorkspaces;
    UserWorkspaces::as_ref(app)
        .team_for_window(window_id)
        .and_then(|team| team.billing_metadata.tier.session_sharing_policy)
        .map(|policy| Byte::from_u64(policy.max_session_size))
        .unwrap_or(Byte::from_u64_with_unit(100, byte_unit::Unit::MB).unwrap())
}

#[cfg(test)]
pub fn max_session_size(_window_id: WindowId, _app: &AppContext) -> Byte {
    Byte::from_u64(MAX_BYTES_SHAREABLE as u64)
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub enum SharedSessionActionSource {
    /// From right-click menu in blocklist
    /// * `block_index`: provided with selected block, none when no blocks selected
    BlocklistContextMenu {
        block_index: Option<BlockIndex>,
    },
    Tab,
    PaneHeader,
    /// Includes keybindings.
    CommandPalette,
    OnboardingBlock,
    Closed {
        is_confirm_close_session: bool,
    },
    InactivityModal,
    /// The user did not initiate this action themselves.
    NonUser,
    /// The object-specific sharing dialog.
    SharingDialog,
    /// From the session sharing context menu items.
    RightClickMenu,
    /// From the agent/CLI footer chip.
    FooterChip,
}

/// Returns the native intent URL to join a shared session.
/// This should be used when opening the session from within Warp.
pub fn join_native_intent(session_id: &SessionId) -> String {
    format!(
        "{}://shared_session/{}",
        ChannelState::url_scheme(),
        session_id
    )
}

/// Returns the link to join a shared session.
pub fn join_link(session_id: &SessionId) -> String {
    // For non-bundled builds against the staging server, use the native app intent
    // because the staging web URL won't resolve to a local build.
    let use_web_url = !ChannelState::uses_staging_server() || cfg!(feature = "release_bundle");

    let mut link = if use_web_url {
        format!("{}/session/{}", ChannelState::server_root_url(), session_id,)
    } else {
        join_native_intent(session_id)
    };

    // If this is a preview build, route the sharing link to the preview server.
    if matches!(ChannelState::channel(), Channel::Preview) {
        link.push_str("?preview=true");
    }

    link
}

/// Returns the full session sharing URL given a path.
pub fn connect_endpoint(path: String) -> Option<String> {
    let base = ChannelState::session_sharing_server_url()?;
    if FeatureFlag::SessionSharingAcls.is_enabled() {
        let version = ChannelState::app_version().unwrap_or("v0.00.000");
        if path.contains("?") {
            return Some(format!("{base}{path}&version={version}"));
        } else {
            return Some(format!("{base}{path}?version={version}"));
        }
    }
    Some(format!("{base}{path}"))
}

/// The event number for events sent to the server. The newtype
/// ensures that events are incremented correctly.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct EventNumber(usize);

impl EventNumber {
    fn new() -> Self {
        Self(0)
    }

    /// Returns the current event number and increments
    /// it for the next usage. The event number returned
    /// is the event number that should be used for the next
    /// event to send to the server.
    pub fn advance(&mut self) -> usize {
        let next = self.0;
        self.0 += 1;
        next
    }
}

impl From<EventNumber> for usize {
    fn from(value: EventNumber) -> Self {
        value.0
    }
}

impl From<ReplicaId> for session_sharing_protocol::common::InputReplicaId {
    fn from(value: ReplicaId) -> Self {
        value.to_string().into()
    }
}

impl From<session_sharing_protocol::common::InputReplicaId> for ReplicaId {
    fn from(value: session_sharing_protocol::common::InputReplicaId) -> Self {
        ReplicaId::new(value)
    }
}

impl From<&Role> for InteractionState {
    fn from(value: &Role) -> InteractionState {
        match value {
            Role::Reader => InteractionState::Selectable,
            Role::Executor => InteractionState::Editable,
            Role::Full => InteractionState::Editable,
        }
    }
}

/// Decode scrollback blocks from their JSON wire format into [`SerializedBlock`]s.
///
/// Blocks that fail to deserialize are silently dropped.
pub(crate) fn decode_scrollback(scrollback: &Scrollback) -> Vec<SerializedBlock> {
    scrollback
        .blocks
        .iter()
        .filter_map(|block| serde_json::from_slice(&block.raw).ok())
        .collect()
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
