//! An adapter to make session-sharing work with the [`TerminalView`].

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, Local};
use markdown_parser::FormattedTextFragment;
use session_sharing_protocol::common::{ParticipantId, ParticipantList, Role, SessionId};
use session_sharing_protocol::sharer::SessionSourceType;
use warp_core::features::FeatureFlag;
use warpui::elements::MouseStateHandle;
use warpui::{AppContext, Element, ModelHandle, ViewContext, ViewHandle};

use super::sharer::Sharer;
use super::viewer::Viewer;
use crate::auth::UserUid;
use crate::banner::{Banner, BannerTextContent};
use crate::terminal::shared_session::participant_avatar_view::ParticipantAvatarView;
use crate::terminal::shared_session::presence_manager::PresenceManager;
use crate::terminal::shared_session::render_util::{
    ParticipantAvatarParams, participant_avatar_for_selected_block,
};
use crate::terminal::view::{TerminalAction, TerminalView, throttle};
use crate::ui_components::icons::Icon;

/// The kind of shared session this is.
pub enum Kind {
    /// This [`TerminalView`] is being shared.
    Sharer(Sharer),

    /// This [`TerminalView`] is being viewed.
    Viewer(Viewer),
}

impl Kind {
    pub fn as_viewer(&self) -> Option<&Viewer> {
        match self {
            Self::Viewer(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_viewer_mut(&mut self) -> Option<&mut Viewer> {
        match self {
            Self::Viewer(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_sharer(&self) -> Option<&Sharer> {
        match self {
            Self::Sharer(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_sharer_mut(&mut self) -> Option<&mut Sharer> {
        match self {
            Self::Sharer(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_sharer(&self) -> bool {
        self.as_sharer().is_some()
    }

    pub fn is_viewer(&self) -> bool {
        self.as_viewer().is_some()
    }
}

pub struct Participant {
    pub avatar: ViewHandle<ParticipantAvatarView>,
    pub block_selection_mouse_state_handle: MouseStateHandle,
}

impl Participant {
    pub fn new(avatar: ViewHandle<ParticipantAvatarView>) -> Self {
        Self {
            avatar,
            block_selection_mouse_state_handle: Default::default(),
        }
    }
}

/// An adapter to make session-sharing work with the [`TerminalView`].
pub struct Adapter {
    kind: Kind,
    presence_manager: ModelHandle<PresenceManager>,
    viewers: HashMap<ParticipantId, Participant>,
    reconnecting_banner: ViewHandle<Banner<TerminalAction>>,
    is_reconnecting_banner_open: bool,
    session_id: SessionId,
    started_at: DateTime<Local>,
    source_type: SessionSourceType,
}

impl Adapter {
    fn new(
        kind: Kind,
        presence_manager: ModelHandle<PresenceManager>,
        session_id: SessionId,
        started_at: DateTime<Local>,
        source_type: SessionSourceType,
        ctx: &mut ViewContext<TerminalView>,
    ) -> Self {
        let reconnecting_banner = ctx.add_typed_action_view(|_| {
            Banner::new_without_close(BannerTextContent::formatted_text(vec![
                FormattedTextFragment::plain_text("Offline, trying to reconnect..."),
            ]))
            .with_icon(Icon::CloudOffline)
        });

        ctx.subscribe_to_model(&presence_manager, |view, model, event, ctx| {
            view.handle_presence_manager_event(event, model, ctx);
        });

        Self {
            viewers: HashMap::new(),
            presence_manager,
            kind,
            reconnecting_banner,
            is_reconnecting_banner_open: false,
            session_id,
            started_at,
            source_type,
        }
    }

    pub fn new_for_viewer(
        viewer_id: ParticipantId,
        firebase_uid: UserUid,
        participant_list: Box<ParticipantList>,
        session_id: SessionId,
        started_at: DateTime<Local>,
        source_type: SessionSourceType,
        ctx: &mut ViewContext<TerminalView>,
    ) -> Self {
        let presence_manager = ctx.add_model(|ctx| {
            PresenceManager::new_for_viewer(viewer_id, firebase_uid, *participant_list, ctx)
        });
        let viewer = Kind::Viewer(Viewer::new(ctx));
        Self::new(
            viewer,
            presence_manager,
            session_id,
            started_at,
            source_type,
            ctx,
        )
    }

    pub fn new_for_sharer(
        sharer_id: ParticipantId,
        firebase_uid: UserUid,
        session_id: SessionId,
        started_at: DateTime<Local>,
        source_type: SessionSourceType,
        ctx: &mut ViewContext<TerminalView>,
    ) -> Self {
        let presence_manager =
            ctx.add_model(|_| PresenceManager::new_for_sharer(sharer_id, firebase_uid));

        // The inactivity timer is reset every 10 seconds
        // as long as sharer activity was detected during the interval.
        // For ambient agent sessions, we skip the inactivity timer entirely.
        let (activity_tx, activity_rx) = async_channel::unbounded();
        if !matches!(source_type, SessionSourceType::AmbientAgent { .. }) {
            let throttled_activity_rx = throttle(Duration::from_secs(10), activity_rx);
            ctx.spawn_stream_local(
                throttled_activity_rx,
                |view, _, ctx| view.reset_sharer_inactivity_timer(ctx),
                |_, _| {},
            );
        }

        let sharer = Kind::Sharer(Sharer::new(activity_tx, ctx));
        Self::new(
            sharer,
            presence_manager,
            session_id,
            started_at,
            source_type,
            ctx,
        )
    }

    pub fn started_at(&self) -> &DateTime<Local> {
        &self.started_at
    }

    pub fn presence_manager(&self) -> &ModelHandle<PresenceManager> {
        &self.presence_manager
    }

    pub fn sharer(&self) -> Option<&Participant> {
        self.kind
            .as_viewer()
            .and_then(|viewer| viewer.sharer.as_ref())
    }

    pub fn viewers(&self) -> &HashMap<ParticipantId, Participant> {
        &self.viewers
    }

    pub fn remove_viewer(&mut self, participant_id: &ParticipantId) {
        self.viewers.remove(participant_id);
    }

    pub fn add_viewer(
        &mut self,
        participant_id: ParticipantId,
        avatar: ViewHandle<ParticipantAvatarView>,
    ) {
        self.viewers
            .insert(participant_id, Participant::new(avatar));
    }

    pub fn update_participant_role(
        &mut self,
        participant_id: &ParticipantId,
        role: Role,
        ctx: &mut ViewContext<TerminalView>,
    ) {
        if !FeatureFlag::SessionSharingAcls.is_enabled() {
            self.update_participant_role_internal(participant_id, role, ctx);
            return;
        }

        let presence_manager = self.presence_manager.as_ref(ctx);
        if let Some(firebase_uid) = presence_manager.viewer_firebase_uid(participant_id) {
            // Update the local state for all participants that have the same UID.
            let participant_ids: Vec<ParticipantId> = presence_manager
                .present_viewer_ids_for_uid(firebase_uid)
                .cloned()
                .collect();
            for participant_id in participant_ids {
                self.update_participant_role_internal(&participant_id, role, ctx);
            }
        } else {
            log::warn!(
                "Unable to find firebase uid for viewer {participant_id:?} when updating role"
            );
            self.update_participant_role_internal(participant_id, role, ctx);
        }
    }

    fn update_participant_role_internal(
        &mut self,
        participant_id: &ParticipantId,
        role: Role,
        ctx: &mut ViewContext<TerminalView>,
    ) {
        self.presence_manager()
            .update(ctx, |presence_manager, ctx| {
                presence_manager.update_participant_role(participant_id, role, ctx);
            });

        if let Some(participant) = self.viewers.get(participant_id) {
            participant.avatar.update(ctx, |avatar, ctx| {
                avatar.set_role(Some(role));
                ctx.notify();
            });
        }
    }

    pub fn kind(&self) -> &Kind {
        &self.kind
    }

    pub(super) fn kind_mut(&mut self) -> &mut Kind {
        &mut self.kind
    }

    pub fn on_reconnection_status_changed(
        &mut self,
        is_reconnecting: bool,
        ctx: &mut ViewContext<TerminalView>,
    ) {
        self.is_reconnecting_banner_open = is_reconnecting;

        self.presence_manager()
            .update(ctx, |presence_manager, ctx| {
                presence_manager.set_is_reconnecting(is_reconnecting, ctx);
            });

        if let Some(viewer) = self.kind.as_viewer_mut() {
            viewer.set_is_reconnecting(is_reconnecting);
            if let Some(sharer) = &viewer.sharer {
                sharer.avatar.update(ctx, |avatar, ctx| {
                    avatar.set_is_muted(is_reconnecting);
                    ctx.notify();
                });
            }
        }

        for viewer in self.viewers.values_mut() {
            viewer.avatar.update(ctx, |avatar, ctx| {
                avatar.set_is_muted(is_reconnecting);
                ctx.notify();
            });
        }
    }

    pub fn reconnecting_banner(&self) -> Option<&ViewHandle<Banner<TerminalAction>>> {
        self.is_reconnecting_banner_open
            .then_some(&self.reconnecting_banner)
    }

    pub fn presence_avatars(&self, app: &AppContext) -> HashMap<ParticipantId, Box<dyn Element>> {
        let mut avatars = HashMap::new();
        let is_self_reconnecting = self.presence_manager.as_ref(app).is_reconnecting();

        // TODO: we should only be creating avatars for participants that have block selections.
        // TODO: we shouldn't need to consult two different sources (presence manager and our own map)
        //       to construct these avatars.
        for viewer in self.presence_manager.as_ref(app).get_present_viewers() {
            let Some(mouse_state_handle) = self
                .viewers
                .get(viewer.id())
                .map(|v| v.block_selection_mouse_state_handle.clone())
            else {
                continue;
            };

            avatars.insert(
                viewer.id().clone(),
                participant_avatar_for_selected_block(
                    ParticipantAvatarParams::new(viewer, is_self_reconnecting),
                    mouse_state_handle,
                    app,
                ),
            );
        }

        if let Some(sharer) = self.presence_manager.as_ref(app).get_sharer()
            && let Some(mouse_state_handle) = self
                .sharer()
                .map(|s| s.block_selection_mouse_state_handle.clone())
        {
            avatars.insert(
                sharer.id().clone(),
                participant_avatar_for_selected_block(
                    ParticipantAvatarParams::new(sharer, is_self_reconnecting),
                    mouse_state_handle,
                    app,
                ),
            );
        }

        avatars
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub fn source_type(&self) -> &SessionSourceType {
        &self.source_type
    }

    /// Retrieves the viewer avatars we want to render on the right side of the
    /// pane header.
    ///
    /// If the ACL feature flag is turned on, this method will filter out avatars
    /// for participants that are:
    ///     - Duplicate users, i.e. they share the same Firebase UID
    ///     - Same user as the current viewer
    ///     - Same user as the sharer
    pub fn pane_header_viewer_avatars(
        &self,
        ctx: &AppContext,
    ) -> Vec<ViewHandle<ParticipantAvatarView>> {
        let presence_manager = self.presence_manager.as_ref(ctx);
        if FeatureFlag::SessionSharingAcls.is_enabled() {
            let self_uid = presence_manager.firebase_uid();
            let sharer_uid = presence_manager
                .get_sharer()
                .map(|s| s.info.profile_data.firebase_uid.as_str());
            let mut seen_uids = HashSet::new();
            self.viewers
                .iter()
                .filter(|(participant_id, _)| {
                    let Some(viewer_uid) = presence_manager.viewer_firebase_uid(participant_id)
                    else {
                        // If we can't find a Firebase UID for the viewer,
                        // default to showing them in the session header.
                        log::warn!("Couldn't find firebase_uid for viewer {participant_id:?}");
                        return true;
                    };
                    let is_duplicate = !seen_uids.insert(viewer_uid);
                    let is_same_user_as_self = self_uid == viewer_uid;
                    let is_same_user_as_sharer =
                        sharer_uid.is_some_and(|sharer_uid| sharer_uid == viewer_uid.as_str());

                    !is_duplicate && !is_same_user_as_self && !is_same_user_as_sharer
                })
                .map(|(_, viewer)| viewer.avatar.clone())
                .collect()
        } else {
            self.viewers.values().map(|p| p.avatar.clone()).collect()
        }
    }
}
