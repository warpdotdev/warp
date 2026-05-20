use std::{
    collections::{HashMap, HashSet},
    iter,
};

use futures::future::BoxFuture;
use futures_util::future::join_all;
use itertools::{Either, Itertools};
use pathfinder_color::ColorU;
use rand::Rng;
#[cfg(not(target_arch = "wasm32"))]
use session_sharing_protocol::common::Viewer;
use session_sharing_protocol::common::{
    InputReplicaId, ParticipantInfo, ParticipantList, ParticipantPresenceUpdate, PresenceUpdate,
    Role, RoleRequestId, Selection,
};

use asset_cache::AssetCacheExt as _;
use warpui::{
    assets::asset_cache::{AssetCache, AssetState},
    image_cache::ImageType,
    r#async::SpawnedFutureHandle,
    AppContext, Entity, ModelContext, SingletonEntity,
};

use session_sharing_protocol::common::ParticipantId;

use crate::{
    auth::UserUid,
    editor::{CursorColors, PeerSelectionData},
    terminal::model::{block::BlockId, blocks::BlockList, terminal_model::BlockIndex},
    util::color::coloru_with_opacity,
};

/// Selections have 25% opacity.
pub fn text_selection_color(participant_color: ColorU) -> ColorU {
    coloru_with_opacity(participant_color, 25)
}

pub const MUTED_PARTICIPANT_COLOR: ColorU = ColorU {
    r: 176,
    g: 176,
    b: 176,
    a: 255,
};

pub const MUTED_AVATAR_BORDER_COLOR: ColorU = ColorU {
    r: 138,
    g: 138,
    b: 138,
    a: 255,
};

/// A set of pre-assigned colors that we use for shared session participants.
/// These come from https://www.figma.com/file/chk9pwt35jTJhf9KnHmZyE/Components?type=design&node-id=1650-1410&mode=design&t=RTHbE9G6NLhFRqLQ-0.
const PRESET_COLORS: &[ColorU] = &[
    ColorU {
        r: 93,
        g: 202,
        b: 60,
        a: 255,
    },
    ColorU {
        r: 174,
        g: 67,
        b: 255,
        a: 255,
    },
    ColorU {
        r: 224,
        g: 222,
        b: 19,
        a: 255,
    },
    ColorU {
        r: 255,
        g: 125,
        b: 38,
        a: 255,
    },
    ColorU {
        r: 68,
        g: 233,
        b: 237,
        a: 255,
    },
    ColorU {
        r: 54,
        g: 98,
        b: 236,
        a: 255,
    },
    ColorU {
        r: 255,
        g: 13,
        b: 226,
        a: 255,
    },
];

/// Helper struct containing participant info and anything else necessary for rendering
/// for a present participant.
#[derive(Clone)]
pub struct Participant {
    pub info: ParticipantInfo,

    /// The color assigned to this participant
    pub color: ColorU,

    /// Is None iff participant is sharer.
    pub role: Option<Role>,
}

impl Participant {
    pub fn id(&self) -> &ParticipantId {
        &self.info.id
    }

    pub fn input_replica_id(&self) -> &InputReplicaId {
        &self.info.profile_data.input_replica_id
    }

    /// Returns the selected block index that the avatar should be rendered at.
    /// This is the block at the top of the last continuous selection.
    /// Returns None if the participant doesn't have a block selected.
    pub fn get_selected_block_index_for_avatar(
        &self,
        block_list: &BlockList,
    ) -> Option<BlockIndex> {
        let session_sharing_protocol::common::Selection::Blocks { block_ids } =
            &self.info.selection
        else {
            return None;
        };
        let mut block_index_for_avatar = None;
        // Sort selected block indices in decreasing order.
        let block_indices = block_ids
            .iter()
            .filter_map(|block_id| block_list.block_index_for_id(&(block_id.to_string().into())))
            .sorted_unstable()
            .rev();
        for idx in block_indices {
            let Some(block_index) = block_index_for_avatar else {
                block_index_for_avatar = Some(idx);
                continue;
            };
            // If this is part of the same continuous selection, update the index since we want the avatar at the top of the last continuous selection.
            if idx
                == std::convert::Into::<usize>::into(block_index)
                    .saturating_sub(1)
                    .into()
            {
                block_index_for_avatar = Some(idx);
            } else {
                // Once we reach a smaller index that's not part of the same continuous selection, return
                return block_index_for_avatar;
            }
        }
        block_index_for_avatar
    }
}

/// Helper struct containing presence information about a participant who selected a particular block.
pub struct ParticipantAtSelectedBlock<'a> {
    /// The participant who selected the block.
    pub participant: &'a Participant,
    /// This block is the top of a continuous block selection by this participant.
    /// True for single selected block as well.
    pub is_top_of_continuous_selection: bool,
    /// This block is the bottom of a continuous block selection by this participant.
    /// True for single selected block as well.
    pub is_bottom_of_continuous_selection: bool,
    pub should_show_avatar: bool,
}

/// A viewer who was once part of the session
/// but no longer is.
#[derive(Clone)]
pub struct AbsentViewer {
    /// The last known info we had about the viewer.
    participant_info: ParticipantInfo,
}

impl AbsentViewer {
    pub fn id(&self) -> &ParticipantId {
        &self.participant_info.id
    }

    pub fn input_replica_id(&self) -> &InputReplicaId {
        &self.participant_info.profile_data.input_replica_id
    }
}

/// Manager for assigning colors to shared session participants as they join and leave.
/// This should contain the data needed to render presence-related UIs.
/// The presence manager does not store participant data about ourselves, whether we are the sharer or viewer.
pub struct PresenceManager {
    /// Our own Participant ID.
    id: ParticipantId,

    /// Our own Firebase UID.
    firebase_uid: UserUid,

    /// Our own role, None iff is sharer.
    pub role: Option<Role>,

    /// Participant ID of the sharer.
    sharer_id: ParticipantId,

    /// Is None iff we ourselves are the sharer.
    sharer: Option<Participant>,

    /// The set of viewers who are still part of the session.
    ///
    /// If we are ourselves a viewer, this map does _not_ include our own state.
    present_viewers: HashMap<ParticipantId, Participant>,

    /// The set of viewers who were once part of the session but no longer are.
    /// By default, all of the `get_*` APIs that return a list of participants
    /// _do not_ include the absent viewers.
    absent_viewers: HashMap<ParticipantId, AbsentViewer>,

    chosen_colors: HashSet<ColorU>,

    /// Loading participants is a future because we may need to download an image.
    /// Note even if there is no image, the participant is still loaded as a future.
    load_participants_imgs_future_handle: Option<SpawnedFutureHandle>,

    /// Whether we ourselves are attempting to reconnect to the server.
    /// If this is true, all avatars should have a muted color.
    is_reconnecting: bool,

    // Map from block ID to the shared session participant IDs that have it selected.
    block_id_to_participants_selected: HashMap<BlockId, Vec<ParticipantId>>,

    role_requests: HashMap<ParticipantId, RoleRequestId>,
}

/// Returns the first available preset color, or a random color if all are taken.
pub fn get_available_color(chosen_colors: &HashSet<ColorU>) -> ColorU {
    for color in PRESET_COLORS {
        if !chosen_colors.contains(color) {
            return *color;
        }
    }
    // If we ran out of colors, generate a random one.
    ColorU::new(
        rand::thread_rng().gen_range(0..=255),
        rand::thread_rng().gen_range(0..=255),
        rand::thread_rng().gen_range(0..=255),
        255,
    )
}

impl PresenceManager {
    pub fn new_for_sharer(id: ParticipantId, firebase_uid: UserUid) -> Self {
        Self {
            id: id.clone(),
            firebase_uid,
            role: None,
            sharer_id: id,
            sharer: None,
            present_viewers: HashMap::new(),
            absent_viewers: HashMap::new(),
            chosen_colors: HashSet::new(),
            load_participants_imgs_future_handle: None,
            block_id_to_participants_selected: HashMap::new(),
            is_reconnecting: false,
            role_requests: HashMap::new(),
        }
    }

    pub fn new_for_viewer(
        id: ParticipantId,
        firebase_uid: UserUid,
        participants: ParticipantList,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        // Populate sharer info, remaining fields for sharer and viewer
        // will be populated in the call to `update_participants`.
        let mut chosen_colors = HashSet::new();
        let color = get_available_color(&chosen_colors);
        chosen_colors.insert(color);

        let sharer = Participant {
            info: participants.sharer.info.clone(),
            color,
            role: None,
        };

        let mut manager = Self {
            id,
            firebase_uid,
            role: Some(Role::default()),
            sharer_id: participants.sharer.info.id.clone(),
            sharer: Some(sharer),
            present_viewers: HashMap::new(),
            absent_viewers: HashMap::new(),
            chosen_colors,
            load_participants_imgs_future_handle: None,
            block_id_to_participants_selected: HashMap::new(),
            is_reconnecting: false,
            role_requests: HashMap::new(),
        };
        manager.update_participants(participants, ctx);
        manager
    }

    /// Returns our own participant id.
    pub fn id(&self) -> ParticipantId {
        self.id.clone()
    }

    /// Returns our own Firebase UID.
    pub fn firebase_uid(&self) -> UserUid {
        self.firebase_uid
    }

    /// Returns the sharer's participant id.
    pub fn sharer_id(&self) -> ParticipantId {
        self.sharer_id.clone()
    }

    /// Returns our own role, `None` iff we are the sharer.
    pub fn role(&self) -> Option<Role> {
        self.role
    }

    /// Returns the viewer's role, if the viewer is known to us.
    pub fn viewer_role(&self, viewer_id: &ParticipantId) -> Option<Role> {
        self.present_viewers.get(viewer_id).and_then(|v| v.role)
    }

    /// Returns a viewer's role request id given their participant id,
    /// `None` if the viewer does not have a pending request.
    pub fn get_role_request(&self, participant_id: &ParticipantId) -> Option<&RoleRequestId> {
        self.role_requests.get(participant_id)
    }

    /// Returns the present viewers of this shared session, not including ourselves.
    /// There is no guarantee of the ordering of viewers, so callers should sort by ID for a stable ordering.
    pub fn get_present_viewers(&self) -> impl Iterator<Item = &Participant> {
        self.present_viewers.values()
    }

    /// Returns the sharer of this shared session.
    /// Returns None if we are the sharer ourselves (we should not need presence data for ourselves).
    pub fn get_sharer(&self) -> Option<&Participant> {
        self.sharer.as_ref()
    }

    /// Returns all present participants of this shared session, including sharer and viewers,
    /// but not including ourselves.
    pub fn all_present_participants(&self) -> impl Iterator<Item = &Participant> {
        if let Some(sharer) = self.get_sharer() {
            return Either::Left(iter::once(sharer).chain(self.get_present_viewers()));
        }
        Either::Right(self.get_present_viewers())
    }

    /// Returns the participant identified by id iff the participant is present.
    pub fn get_participant(&self, id: &ParticipantId) -> Option<&Participant> {
        if let Some(viewer) = self.present_viewers.get(id) {
            return Some(viewer);
        } else if let Some(sharer) = self.sharer.as_ref() {
            if self.sharer_id == *id {
                return Some(sharer);
            }
        }
        None
    }

    /// Returns the participants who have the block at the block index selected.
    pub fn get_participants_selected_block_index(
        &self,
        block_index: BlockIndex,
        block_list: &BlockList,
    ) -> Vec<&Participant> {
        let Some(block) = block_list.block_at(block_index) else {
            return vec![];
        };
        let Some(participant_ids) = self.block_id_to_participants_selected.get(block.id()) else {
            return vec![];
        };
        participant_ids
            .iter()
            .filter_map(|id| self.get_participant(id))
            .collect_vec()
    }

    /// Returns the participants who have the block at the block index selected,
    /// with some additional info helpful for rendering.
    pub fn get_participants_at_selected_block(
        &self,
        block_index: BlockIndex,
        block_list: &BlockList,
    ) -> Vec<ParticipantAtSelectedBlock<'_>> {
        let participants_selected_this_block =
            self.get_participants_selected_block_index(block_index, block_list);
        if participants_selected_this_block.is_empty() {
            return vec![];
        }

        let participant_ids_selected_prev_block = if block_index == 0.into() {
            HashSet::new()
        } else {
            HashSet::<_>::from_iter(
                self.get_participants_selected_block_index(block_index - 1.into(), block_list)
                    .into_iter()
                    .map(|p| p.info.id.clone()),
            )
        };
        let participant_ids_selected_next_block = HashSet::<_>::from_iter(
            self.get_participants_selected_block_index(block_index + 1.into(), block_list)
                .into_iter()
                .map(|p| p.info.id.clone()),
        );

        participants_selected_this_block
            .into_iter()
            .map(|participant| {
                let should_show_avatar = participant
                    .get_selected_block_index_for_avatar(block_list)
                    .is_some_and(|idx| idx == block_index);
                ParticipantAtSelectedBlock {
                    participant,
                    is_top_of_continuous_selection: !participant_ids_selected_prev_block
                        .contains(&participant.info.id),
                    is_bottom_of_continuous_selection: !participant_ids_selected_next_block
                        .contains(&participant.info.id),
                    should_show_avatar,
                }
            })
            .collect_vec()
    }

    pub fn update_participants(
        &mut self,
        participants: ParticipantList,
        ctx: &mut ModelContext<Self>,
    ) {
        // If there was a previous in-flight future updating participants, cancel it since our new list is more up to date.
        if let Some(old_abort_handle) = self.load_participants_imgs_future_handle.take() {
            old_abort_handle.abort();
        }

        // The new or updated participants.
        let mut latest_participants = Vec::new();

        // A list of futures. Each one represents a profile image that's being loaded for a participant.
        let mut participant_image_loading_futures = Vec::new();

        // Update sharer info
        let incoming_sharer_info = participants.sharer.info;
        if let Some(sharer) = self.sharer.as_mut() {
            sharer.info = incoming_sharer_info.clone();

            if let Some(future) = Self::when_profile_image_is_loaded(sharer, ctx) {
                participant_image_loading_futures.push(future);
            }
            latest_participants.push(sharer.clone());
        }

        for viewer in participants.viewers {
            if !viewer.is_present {
                if let Some(viewer) = self.present_viewers.remove(&viewer.info.id) {
                    self.chosen_colors.remove(&viewer.color);
                }
                self.absent_viewers.insert(
                    viewer.info.id.clone(),
                    AbsentViewer {
                        participant_info: viewer.info,
                    },
                );
                continue;
            }

            let info = viewer.info;
            // Only store role data for ourselves.
            if info.id == self.id {
                self.role = Some(viewer.role);
                continue;
            }

            // If this participant already existed, update the info and role
            // while keeping their color.
            if let Some(existing_participant) = self.present_viewers.get_mut(&info.id) {
                existing_participant.info = info;
                existing_participant.role = Some(viewer.role);
                continue;
            };

            // Otherwise, pick an available color and add them.
            let color = get_available_color(&self.chosen_colors);
            self.chosen_colors.insert(color);

            let new_viewer = Participant {
                info,
                color,
                role: Some(viewer.role),
            };

            if let Some(future) = Self::when_profile_image_is_loaded(&new_viewer, ctx) {
                participant_image_loading_futures.push(future);
            }
            latest_participants.push(new_viewer);
        }

        // Spawn a future that waits for all the new profile images to be loaded into memory.
        let load_participants_future_handle = ctx.spawn(
            async move {
                join_all(participant_image_loading_futures).await;
            },
            |manager, _, ctx| {
                manager.on_participant_images_loaded(latest_participants, ctx);
            },
        );
        self.load_participants_imgs_future_handle = Some(load_participants_future_handle.clone());
    }

    /// Returns a future that resolves when the participant's profile image is loaded. If the participant
    /// doesn't have a profile image OR their image is already available in memory, returns None.
    fn when_profile_image_is_loaded(
        participant: &Participant,
        app: &AppContext,
    ) -> Option<BoxFuture<'static, ()>> {
        let url = participant.info.profile_data.photo_url.as_ref()?;
        let asset_cache = AssetCache::as_ref(app);

        // Make a non-blocking check to see if the image is loaded yet. If the image hasn't been seen
        // before, this call spawns a task to fetch the bytes and parse it into an image.
        match asset_cache.load_asset_from_url::<ImageType>(url, None) {
            AssetState::Loading { handle } => handle.when_loaded(asset_cache),
            _ => None,
        }
    }

    pub fn update_participant_presence(
        &mut self,
        update: ParticipantPresenceUpdate,
        _ctx: &mut ModelContext<Self>,
    ) {
        let participant = if self.sharer_id == update.participant_id {
            self.sharer.as_mut()
        } else {
            self.present_viewers.get_mut(&update.participant_id)
        };

        let Some(participant) = participant else {
            if self.id != update.participant_id {
                log::warn!("Received shared session participant presence update for participant that doesn't exist");
            }
            return;
        };
        let PresenceUpdate::Selection(selection) = update.update;

        // Selection info is needed for rendering remote cursors in input
        participant.info.selection = selection;
        self.refresh_block_id_to_participants_selected();
    }

    pub fn update_participant_role(
        &mut self,
        participant_id: &ParticipantId,
        role: Role,
        _ctx: &mut ModelContext<Self>,
    ) {
        if participant_id == &self.id {
            self.role = Some(role);
        } else {
            let Some(participant) = self.present_viewers.get_mut(participant_id) else {
                log::warn!("Received shared session participant role update for participant that doesn't exist");
                return;
            };
            participant.role = Some(role);
        }
    }

    pub fn make_all_participants_readers(&mut self, _ctx: &mut ModelContext<Self>) {
        for viewer in self.present_viewers.values_mut() {
            viewer.role = Some(Role::Reader);
        }
    }

    /// Called when the sharer is notified of a role request from a viewer.
    pub fn on_role_requested(
        &mut self,
        participant_id: ParticipantId,
        role_request_id: RoleRequestId,
        role: Role,
        _ctx: &mut ModelContext<Self>,
    ) {
        // TODO: handle pending role requests on reconnection
        // Ensure only the sharer can update its role requests
        if self.sharer_id != self.id {
            return;
        }

        // Ensure viewer doesn't already have requested role
        if let Some(old_role) = self.viewer_role(&participant_id) {
            if role == old_role {
                return;
            }
        }

        self.role_requests
            .insert(participant_id.clone(), role_request_id.clone());
    }

    /// Called when the sharer is notified of a cancelled role request
    pub fn on_role_request_cancelled(
        &mut self,
        participant_id: ParticipantId,
        _ctx: &mut ModelContext<Self>,
    ) {
        // Ensure only the sharer can remove its role requests
        if self.sharer_id != self.id {
            return;
        }

        self.role_requests.remove(&participant_id);
    }

    /// Called as the sharer responds to a role request
    pub fn on_role_request_responded_to(
        &mut self,
        participant_id: ParticipantId,
        _ctx: &mut ModelContext<Self>,
    ) {
        // Ensure only the sharer can remove its role requests
        if self.sharer_id != self.id {
            return;
        }

        self.role_requests.remove(&participant_id);
    }

    pub fn set_is_reconnecting(
        &mut self,
        is_self_reconnecting: bool,
        _ctx: &mut ModelContext<Self>,
    ) {
        self.is_reconnecting = is_self_reconnecting;
    }

    pub fn is_reconnecting(&self) -> bool {
        self.is_reconnecting
    }

    /// Refreshes the block ID to participants selected cache to be consistent with the current participant data stored.
    fn refresh_block_id_to_participants_selected(&mut self) {
        self.block_id_to_participants_selected.clear();
        let participants = if self.sharer.is_some() {
            Either::Left(
                iter::once(self.sharer.as_ref().expect("sharer should exist"))
                    .chain(self.present_viewers.values()),
            )
        } else {
            Either::Right(self.present_viewers.values())
        };
        for participant in participants {
            if let session_sharing_protocol::common::Selection::Blocks { block_ids } =
                &participant.info.selection
            {
                for block_id in block_ids {
                    self.block_id_to_participants_selected
                        .entry(block_id.to_string().into())
                        .or_default()
                        .push(participant.info.id.clone());
                }
            }
        }
    }

    fn on_participant_images_loaded(
        &mut self,
        latest_participants: Vec<Participant>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Once all participant futures have completed, update the participant list and emit an event.
        for participant in latest_participants {
            if let session_sharing_protocol::common::Selection::Blocks { block_ids } =
                &participant.info.selection
            {
                for block_id in block_ids {
                    self.block_id_to_participants_selected
                        .entry(block_id.to_string().into())
                        .or_default()
                        .push(participant.info.id.clone());
                }
            }

            if participant.info.id == self.sharer_id {
                self.sharer = Some(participant);
            } else {
                self.present_viewers
                    .insert(participant.info.id.clone(), participant);
            }
        }
        self.refresh_block_id_to_participants_selected();
        ctx.emit(Event::ParticipantListUpdated);
    }

    pub fn input_data_for_participant(
        &self,
        participant: &Participant,
    ) -> (InputReplicaId, PeerSelectionData) {
        let input_replica_id = participant.input_replica_id().clone();
        let participant_color = if self.is_reconnecting() {
            MUTED_PARTICIPANT_COLOR
        } else {
            participant.color
        };
        let colors = CursorColors {
            cursor: participant_color.into(),
            selection: text_selection_color(participant_color).into(),
        };

        let cursor_data = PeerSelectionData {
            colors,
            display_name: participant.info.profile_data.display_name.clone(),
            image_url: participant.info.profile_data.photo_url.clone(),
            should_draw_cursors: matches!(participant.info.selection, Selection::None),
        };

        (input_replica_id, cursor_data)
    }

    pub fn absent_viewers(&self) -> impl Iterator<Item = &AbsentViewer> + '_ {
        self.absent_viewers.values()
    }

    /// Returns a viewer's firebase uid, if the viewer is known to us.
    pub fn viewer_firebase_uid(&self, viewer_id: &ParticipantId) -> Option<UserUid> {
        if *viewer_id == self.id {
            return Some(self.firebase_uid);
        }

        self.present_viewers
            .get(viewer_id)
            .map(|v| v.info.profile_data.firebase_uid.as_str())
            .or_else(|| {
                self.absent_viewers
                    .get(viewer_id)
                    .map(|v| v.participant_info.profile_data.firebase_uid.as_str())
            })
            .map(UserUid::new)
    }

    /// Returns all of the present viewer IDs associated with the given Firebase
    /// UID, including ourselves if applicable.
    pub fn present_viewer_ids_for_uid(
        &self,
        viewer_uid: UserUid,
    ) -> impl Iterator<Item = &ParticipantId> + '_ {
        let is_viewer_self = self.firebase_uid == viewer_uid;
        let viewer_ids = self
            .present_viewers
            .values()
            .filter(move |v| viewer_uid.as_string() == v.info.profile_data.firebase_uid)
            .map(|v| &v.info.id);
        viewer_ids.chain(is_viewer_self.then_some(&self.id))
    }

    /// Returns a participant ID for a participant associated with the given
    /// Firebase UID.
    pub fn present_viewer_id_for_uid(&self, viewer_uid: UserUid) -> Option<&ParticipantId> {
        self.present_viewer_ids_for_uid(viewer_uid).next()
    }

    /// Returns the only distinct present viewer UID. Multiple present viewers
    /// with the same UID count as one user.
    pub fn single_distinct_present_viewer_uid(&self) -> Option<&str> {
        Self::single_distinct_uid(
            self.get_present_viewers()
                .map(|v| v.info.profile_data.firebase_uid.as_str()),
        )
    }

    /// Like `single_distinct_present_viewer_uid`, but reads directly from a
    /// participant list before the presence manager finishes processing it.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn single_distinct_present_viewer_uid_from_viewers<'a>(
        viewers: impl Iterator<Item = &'a Viewer>,
    ) -> Option<&'a str> {
        Self::single_distinct_uid(
            viewers
                .filter(|v| v.is_present)
                .map(|v| v.info.profile_data.firebase_uid.as_str()),
        )
    }

    fn single_distinct_uid<'a>(mut uids: impl Iterator<Item = &'a str>) -> Option<&'a str> {
        let uid = uids.next()?;
        uids.all(|other_uid| other_uid == uid).then_some(uid)
    }
}

pub enum Event {
    ParticipantListUpdated,
}

impl Entity for PresenceManager {
    type Event = Event;
}

#[cfg(test)]
#[path = "presence_manager_tests.rs"]
mod tests;
