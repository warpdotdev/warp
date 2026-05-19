use std::borrow::Cow;

use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::model::persistence::CloudModelEvent;
use crate::cloud_object::model::view::CloudViewModel;
use crate::cloud_object::Owner;
use crate::cloud_object::{CloudObject, ServerGuestSubject};
use crate::editor::PropagateAndNoOpNavigationKeys;
use crate::menu::{self, Menu, MenuItem, MenuItemFields};
use crate::send_telemetry_from_ctx;
use crate::server::cloud_objects::update_manager::{
    ObjectOperation, UpdateManager, UpdateManagerEvent,
};
use crate::server::ids::ServerId;
use crate::server::telemetry::CloudObjectTelemetryMetadata;
use crate::server::telemetry::OpenedSharingDialogEvent;
use crate::server::telemetry::SharingDialogSource;
use crate::terminal::shared_session::permissions_manager::{
    SessionPermissionsEvent, SessionPermissionsManager,
};
use crate::terminal::shared_session::SharedSessionActionSource;
use crate::terminal::TerminalView;
use crate::ui_components::icons::Icon;
use crate::view_components::DismissibleToast;
use crate::word_block_editor::{
    WordBlockEditorStyles, WordBlockEditorView, WordBlockEditorViewEvent, WordBlockLayout,
    WordBlockStyles,
};
use crate::workspace::ToastStack;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::TelemetryEvent;
use email_address::EmailAddress;
use inheritance::{InheritanceDetails, InheritanceState};
use itertools::Itertools;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use qrcode::{types::Color as QrColor, QrCode};
use session_sharing_protocol::common::{Guest, PendingGuest, SessionId, TeamAclData};
use warp_core::ui::appearance::Appearance;
use warp_editor::editor::NavigationKey;
use warpui::elements::{
    Align, ChildAnchor, ChildView, Fill, Highlight, MainAxisSize, MouseStateHandle,
    OffsetPositioning, ParentAnchor, PositionedElementAnchor, PositionedElementOffsetBounds,
    SavePosition, ScrollStateHandle, Scrollable, ScrollableElement, ScrollbarWidth, Shrinkable,
    Stack, UniformList, UniformListState,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::ui_components::button::{ButtonVariant, TextAndIcon, TextAndIconAlignment};
use warpui::ui_components::components::Coords;
use warpui::FocusContext;
use warpui::WeakViewHandle;
use warpui::{
    clipboard::ClipboardContent,
    elements::{
        Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss, Empty, Flex,
        MainAxisAlignment, ParentElement, Radius, Rect,
    },
    keymap::FixedBinding,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use super::{
    style, ContentEditability, LinkSharingSubjectType, ShareableObject, SharingAccessLevel,
    Subject, SubjectExt, TeamKind, UserKind,
};

mod inheritance;

const MENU_WIDTH: f32 = 200.;
const GUEST_LIST_MAX_HEIGHT: f32 = 400.;

/// Width constraints for the invitation email editor. This is both:
const EMAIL_CHIP_WIDTH: f32 = 100.;
const EMAIL_EDITOR_WIDTH: f32 = 100.;

const SHARING_DIALOG_WIDTH: f32 = 425.;
const SESSION_QR_CODE_SIZE: f32 = 240.;
const SESSION_QR_CODE_PADDING: f32 = 16.;

const NO_ACCESS_LABEL: &str = "No access";

#[derive(Default)]
struct UiStateHandles {
    invite_button: MouseStateHandle,
    invite_access_level_button: MouseStateHandle,
    owner_tooltip: MouseStateHandle,
    link_sharing_menu_button: MouseStateHandle,
    team_sharing_menu_button: MouseStateHandle,
    copy_link_button: MouseStateHandle,
    guest_list_state: UniformListState,
    guest_scroll_state: ScrollStateHandle,
}

/// State for which menu is currently open.
///
/// This helps with two things:
/// 1. Ensuring the menu is rendered last, on top of any other elements
/// 2. Ensuring that only one menu is open at a time.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
enum OpenMenuState {
    #[default]
    None,
    LinkSharing,
    TeamSharing,
    InviteAccessLevel,
    Guest(usize),
}

struct InviteFormValidationState {
    /// Invitees with invalid email addresses.
    invalid_emails: Vec<String>,
    /// Invitees that are already guests on the object.
    duplicate_guests: Vec<String>,
    /// All invitees
    invitee_emails: Vec<String>,
}

impl InviteFormValidationState {
    pub fn is_valid(&self) -> bool {
        self.invalid_emails.is_empty()
            && self.duplicate_guests.is_empty()
            && !self.invitee_emails.is_empty()
    }
}

/// UI state for object guests.
struct GuestState {
    menu_button_handle: MouseStateHandle,
    tooltip_handle: MouseStateHandle,
    current_access_level: SharingAccessLevel,
    subject: Subject,
    inheritance: Option<InheritanceState>,
}

/// UI state for link sharing.
#[derive(Default)]
struct LinkSharingState {
    access_level: Option<SharingAccessLevel>,
    tooltip_handle: MouseStateHandle,
    inheritance: Option<InheritanceState>,
}

/// UI state for team sharing.
#[derive(Default)]
struct TeamSharingState {
    team: Option<TeamKind>,
    access_level: Option<SharingAccessLevel>,
    tooltip_handle: MouseStateHandle,
    inheritance: Option<InheritanceState>,
}

/// Container for fields related to the invite-by-email form.
struct EmailInviteForm {
    email_editor: ViewHandle<WordBlockEditorView>,
    selected_access_level: SharingAccessLevel,
    access_level_menu: ViewHandle<Menu<SharingDialogAction>>,
}

pub struct SharingDialog {
    self_handle: WeakViewHandle<SharingDialog>,
    target: Option<ShareableObject>,

    invite_form: EmailInviteForm,

    guest_states: Vec<GuestState>,
    guest_menu: ViewHandle<Menu<SharingDialogAction>>,

    link_sharing_state: LinkSharingState,
    link_sharing_menu: ViewHandle<Menu<SharingDialogAction>>,

    team_sharing_state: TeamSharingState,
    team_sharing_menu: ViewHandle<Menu<SharingDialogAction>>,

    ui_state_handles: UiStateHandles,
    open_menu_state: OpenMenuState,
}

#[derive(Debug, Clone)]
pub enum SharingDialogEvent {
    Close,
}

#[derive(Debug, Clone)]
pub enum SharingDialogAction {
    Close,
    CopyLink,
    #[allow(dead_code)]
    SetLinkPermissions(Option<SharingAccessLevel>),
    ToggleLinkSharingMenu,
    ToggleTeamSharingMenu,
    ToggleInviteAccessLevelMenu,
    SetInviteAccessLevel(SharingAccessLevel),
    ToggleGuestMenu(usize),
    RemoveGuest,
    SetGuestAccessLevel(SharingAccessLevel),
    SendInvitations,
    SetTeamPermissions(Option<SharingAccessLevel>),
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        SharingDialogAction::Close,
        id!(SharingDialog::ui_name()),
    )])
}

impl SharingDialog {
    pub fn new(target: Option<ShareableObject>, ctx: &mut ViewContext<Self>) -> Self {
        let link_sharing_menu =
            ctx.add_typed_action_view(|_ctx| Menu::new().with_drop_shadow().with_width(MENU_WIDTH));
        ctx.subscribe_to_view(&link_sharing_menu, |me, menu, event, ctx| {
            me.handle_menu_event(menu, event, ctx)
        });

        let team_sharing_menu =
            ctx.add_typed_action_view(|_ctx| Menu::new().with_drop_shadow().with_width(MENU_WIDTH));
        ctx.subscribe_to_view(&team_sharing_menu, |me, menu, event, ctx| {
            me.handle_menu_event(menu, event, ctx)
        });

        let update_manager = UpdateManager::handle(ctx);
        ctx.subscribe_to_model(&update_manager, |me, _, event, ctx| {
            me.handle_update_manager_event(event, ctx);
        });

        ctx.subscribe_to_model(&CloudModel::handle(ctx), |me, _, event, ctx| {
            me.handle_cloud_model_event(event, ctx);
        });

        ctx.subscribe_to_model(
            &SessionPermissionsManager::handle(ctx),
            |me, _, event, ctx| {
                me.handle_session_permissions_event(event, ctx);
            },
        );

        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |me, _, event, ctx| {
                me.handle_ai_history_event(event, ctx);
            },
        );

        let invite_form = EmailInviteForm {
            email_editor: ctx.add_typed_action_view(|ctx| {
                let mut view = WordBlockEditorView::new(
                    ctx,
                    "Emails",
                    13.,
                    vec![' ', ','],
                    EMAIL_CHIP_WIDTH,
                    Box::new(EmailAddress::is_valid),
                )
                .with_layout(WordBlockLayout::Horizontal {
                    editor_min_width: EMAIL_EDITOR_WIDTH,
                })
                .with_styles(ctx, Self::email_invite_form_styles);
                view.set_propagate_navigation_keys(PropagateAndNoOpNavigationKeys::Always, ctx);
                view
            }),
            selected_access_level: SharingAccessLevel::View,
            access_level_menu: Self::build_invite_access_level_menu(ctx),
        };
        ctx.subscribe_to_view(&invite_form.email_editor, |me, _, event, ctx| {
            me.handle_email_invite_editor_event(event, ctx);
        });

        Self {
            self_handle: ctx.handle(),
            target,
            invite_form,
            guest_states: vec![],
            guest_menu: Self::build_guest_menu(ctx),
            link_sharing_state: Default::default(),
            link_sharing_menu,
            team_sharing_state: Default::default(),
            team_sharing_menu,
            ui_state_handles: Default::default(),
            open_menu_state: Default::default(),
        }
    }

    fn handle_update_manager_event(
        &mut self,
        event: &UpdateManagerEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let UpdateManagerEvent::ObjectOperationComplete { result } = event else {
            return;
        };

        if let ObjectOperation::UpdatePermissions = result.operation {
            self.refresh_object_permission_states(ctx);
        }
    }

    fn handle_cloud_model_event(&mut self, event: &CloudModelEvent, ctx: &mut ViewContext<Self>) {
        if let Some(target_server_id) = self.target_cloud_object_id(ctx) {
            let event_object_id = match event {
                CloudModelEvent::ObjectMoved { type_and_id, .. } => type_and_id,
                CloudModelEvent::ObjectUpdated { type_and_id, .. } => type_and_id,
                CloudModelEvent::ObjectTrashed { .. } => return,
                CloudModelEvent::ObjectUntrashed { .. } => return,
                CloudModelEvent::NotebookEditorChangedFromServer { .. } => return,
                CloudModelEvent::ObjectCreated { type_and_id } => type_and_id,
                CloudModelEvent::ObjectPermissionsUpdated { type_and_id, .. } => type_and_id,
                CloudModelEvent::ObjectDeleted { .. } => return,
                CloudModelEvent::ObjectForceExpanded { .. } => return,
                CloudModelEvent::ObjectSynced { .. } | CloudModelEvent::InitialLoadCompleted => {
                    return
                }
            };

            if event_object_id.sync_id().into_server() == Some(target_server_id) {
                self.refresh_object_permission_states(ctx);
            }
        }
    }

    fn handle_session_permissions_event(
        &mut self,
        event: &SessionPermissionsEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SessionPermissionsEvent::GuestsUpdated {
                session_id,
                guests,
                pending_guests,
            } => {
                self.update_session_guests(ctx, session_id, guests, pending_guests);
            }
            SessionPermissionsEvent::LinkPermissionsUpdated {
                session_id,
                access_level,
            } => {
                self.update_session_link_permissions(*session_id, *access_level, ctx);
            }
            SessionPermissionsEvent::TeamPermissionsUpdated {
                session_id,
                team_acl,
            } => self.update_session_team_permissions(session_id, team_acl.clone(), ctx),
        }
    }

    fn handle_ai_history_event(
        &mut self,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if let BlocklistAIHistoryEvent::UpdatedConversationMetadata {
            conversation_id, ..
        } = event
        {
            // Check if this event is for the conversation we're currently showing
            if let Some(ShareableObject::AIConversation(target_id)) = &self.target {
                if target_id == conversation_id {
                    self.refresh_object_permission_states(ctx);
                }
            }
        }
    }

    /// Sets the target object whose ACLs are shown.
    pub fn set_target(&mut self, target: Option<ShareableObject>, ctx: &mut ViewContext<Self>) {
        self.target = target;
        self.reset_invite_form(ctx);
        self.refresh_object_permission_states(ctx);
        ctx.notify();
    }

    pub fn has_target(&self) -> bool {
        self.target.is_some()
    }

    pub fn has_shared_session_target(&self) -> bool {
        self.target
            .as_ref()
            .is_some_and(|target| matches!(target, ShareableObject::Session { .. }))
    }

    /// Returns `true` if the target is an AI conversation that cannot be shared.
    /// This happens when the conversation hasn't been synced to the cloud.
    pub fn is_unsharable_conversation(&self, app: &AppContext) -> bool {
        if let Some(ShareableObject::AIConversation(id)) = &self.target {
            !BlocklistAIHistoryModel::as_ref(app).can_conversation_be_shared(id)
        } else {
            false
        }
    }

    /// The Warp Drive server ID for the target object. `None` if the target is not a Warp Drive
    /// object or AI conversation.
    fn target_cloud_object_id(&self, app: &AppContext) -> Option<ServerId> {
        match self.target.as_ref() {
            Some(ShareableObject::WarpDriveObject(id)) => Some(*id),
            Some(ShareableObject::AIConversation(id)) => BlocklistAIHistoryModel::as_ref(app)
                .get_server_conversation_metadata(id)
                .map(|m| ServerId::from_string_lossy(m.metadata.uid.uid())),
            _ => None,
        }
    }

    /// The targeted Warp Drive object, or `None` if the target is not a known Warp Drive object.
    fn target_cloud_object<'a>(&self, app: &'a AppContext) -> Option<&'a dyn CloudObject> {
        self.target_cloud_object_id(app)
            .and_then(|id| CloudModel::as_ref(app).get_by_uid(&id.uid()))
    }

    /// The name of the targeted object.
    fn targeted_object_name(&self, app: &AppContext) -> String {
        self.target
            .as_ref()
            .and_then(|target| match target {
                ShareableObject::WarpDriveObject(server_id) => CloudModel::as_ref(app)
                    .get_by_uid(&server_id.uid())
                    .map(|object| object.display_name()),
                ShareableObject::Session { .. } => Some("session".to_string()),
                ShareableObject::AIConversation(_) => Some("conversation".to_string()),
            })
            .unwrap_or_else(|| "unknown".to_string())
    }

    /// Whether or not the current user is allowed to *edit* sharing settings.
    fn can_edit_access(&self, app: &AppContext) -> bool {
        self.target.is_some() && self.access_level(app).can_edit_access()
    }

    fn can_anyone_with_link_share(&self, app: &AppContext) -> bool {
        UserWorkspaces::as_ref(app).is_anyone_with_link_sharing_enabled()
    }

    fn can_direct_link_share(&self, app: &AppContext) -> bool {
        UserWorkspaces::as_ref(app).is_direct_link_sharing_enabled()
    }

    /// The editability state of the object.
    /// * Users who can edit access have the full sharing dialog
    /// * Users who can edit object contents can see who the object is shared with
    /// * Users that are view-only do not need to see permissions
    pub fn editability(&self, app: &AppContext) -> ContentEditability {
        match self.target.as_ref() {
            // Always treat session contents as "editable," so that the sharing dialog is shown.
            Some(ShareableObject::Session { .. }) => ContentEditability::Editable,
            Some(ShareableObject::WarpDriveObject(id)) => {
                CloudViewModel::as_ref(app).object_editability(&id.uid(), app)
            }
            // Always treat AI conversations as "editable," so that the sharing dialog is shown.
            Some(ShareableObject::AIConversation(_)) => ContentEditability::Editable,
            None => ContentEditability::ReadOnly,
        }
    }

    /// The current user's access level on the shared object.
    fn access_level(&self, app: &AppContext) -> SharingAccessLevel {
        match self.target.as_ref() {
            Some(ShareableObject::WarpDriveObject(id)) => {
                CloudViewModel::as_ref(app).access_level(&id.uid(), app)
            }
            Some(ShareableObject::AIConversation(id)) => {
                // Get access level from conversation metadata permissions
                match BlocklistAIHistoryModel::as_ref(app).get_server_conversation_metadata(id) {
                    Some(server_metadata) => {
                        let permissions = &server_metadata.permissions;
                        // Conversation has server metadata, check permissions
                        AuthStateProvider::as_ref(app)
                            .get()
                            .user_id()
                            .and_then(|user_uid| {
                                // Check if user is owner
                                if let Owner::User {
                                    user_uid: owner_uid,
                                } = permissions.space
                                {
                                    if owner_uid == user_uid {
                                        return Some(SharingAccessLevel::Full);
                                    }
                                }
                                // Check if user is on the owning team (for team-owned conversations)
                                if let Owner::Team { team_uid } = permissions.space {
                                    if UserWorkspaces::as_ref(app).current_team_uid()
                                        == Some(team_uid)
                                    {
                                        return Some(SharingAccessLevel::Full);
                                    }
                                }
                                // Check if user is in guests
                                let user_firebase_uid = user_uid.to_string();
                                permissions.guests.iter().find_map(|guest| {
                                    if let ServerGuestSubject::User { firebase_uid } =
                                        &guest.subject
                                    {
                                        if firebase_uid == &user_firebase_uid {
                                            return Some(guest.access_level.into());
                                        }
                                    }
                                    None
                                })
                            })
                            .unwrap_or(SharingAccessLevel::View)
                    }
                    None => {
                        // No server metadata yet - conversation hasn't been shared
                        // The owner (logged in user) should have full access
                        SharingAccessLevel::Full
                    }
                }
            }
            Some(ShareableObject::Session { ref handle, .. }) => {
                // Sharer always has Full access.
                if handle.upgrade(app).is_some_and(|handle| {
                    handle
                        .as_ref(app)
                        .sharer_session_kind()
                        .is_some_and(|kind| kind.is_sharer())
                }) {
                    return SharingAccessLevel::Full;
                }

                if let Some(owner) = self.owner(app) {
                    // If we are the user owner, we have Full access.
                    if let Some(user_uid) = AuthStateProvider::as_ref(app).get().user_id() {
                        if owner.is_user(user_uid) {
                            return SharingAccessLevel::Full;
                        }
                    }
                    // Team members of owning team have Full access.
                    if let Subject::Team(team_kind) = owner {
                        if UserWorkspaces::as_ref(app)
                            .current_team_uid()
                            .is_some_and(|current| current == team_kind.team_uid())
                        {
                            return SharingAccessLevel::Full;
                        }
                    }
                }

                // For viewers, compute effective access as the max across all channels.
                let mut level = SharingAccessLevel::View;

                if let Some(link_level) = self.link_sharing_state.access_level {
                    level = level.max(link_level);
                }

                if let Some(team_level) = self.team_sharing_state.access_level {
                    if let Some(TeamKind::SharedSessionTeam { ref team_uid, .. }) =
                        self.team_sharing_state.team
                    {
                        if UserWorkspaces::as_ref(app)
                            .current_team_uid()
                            .is_some_and(|current| current == *team_uid)
                        {
                            level = level.max(team_level);
                        }
                    }
                }

                if let Some(user_uid) = AuthStateProvider::as_ref(app).get().user_id() {
                    if let Some(guest_level) = self
                        .guest_states
                        .iter()
                        .find(|guest| guest.subject.is_user(user_uid))
                        .map(|guest| guest.current_access_level)
                    {
                        level = level.max(guest_level);
                    }
                }

                level
            }
            None => SharingAccessLevel::Full,
        }
    }

    /// Report a telemetry event for opening this sharing dialog.
    ///
    /// This should be called by views that contain a sharing dialog whenever they open it (i.e.
    /// panes and the Warp Drive index).
    pub fn report_open(&self, source: SharingDialogSource, ctx: &mut ViewContext<Self>) {
        let event = match self.target.as_ref() {
            Some(ShareableObject::WarpDriveObject(id)) => {
                match CloudModel::as_ref(ctx).get_by_uid(&id.uid()) {
                    Some(object) => TelemetryEvent::OpenedSharingDialog(OpenedSharingDialogEvent {
                        source,
                        object_metadata: Some(CloudObjectTelemetryMetadata {
                            object_type: (&object.cloud_object_type_and_id()).into(),
                            object_uid: object.sync_id().into_server(),
                            space: Some(object.space(ctx).into()),
                            team_uid: match object.permissions().owner {
                                Owner::Team { team_uid, .. } => Some(team_uid),
                                Owner::User { .. } => None,
                            },
                        }),
                        session_id: None,
                    }),
                    None => return,
                }
            }
            Some(ShareableObject::Session { session_id, .. }) => {
                TelemetryEvent::OpenedSharingDialog(OpenedSharingDialogEvent {
                    source,
                    object_metadata: None,
                    session_id: Some(*session_id),
                })
            }
            // Skip telemetry for AI conversations
            Some(ShareableObject::AIConversation(_)) => return,
            None => return,
        };

        send_telemetry_from_ctx!(event, ctx);
    }

    fn reset_editable_state(&mut self, ctx: &mut ViewContext<Self>) {
        self.reset_invite_form(ctx);
        ctx.notify();
    }

    fn owner(&self, app: &AppContext) -> Option<Subject> {
        match self.target.as_ref()? {
            ShareableObject::WarpDriveObject(id) => {
                let owner = CloudModel::as_ref(app)
                    .get_by_uid(&id.uid())?
                    .permissions()
                    .owner;
                Some(Subject::from_owner(owner))
            }
            ShareableObject::Session { handle, .. } => {
                // Check if team has Full access - if so, team is the owner.
                if let Some(TeamKind::SharedSessionTeam { team_uid, name }) =
                    self.team_sharing_state.team.as_ref()
                {
                    if self.team_sharing_state.access_level == Some(SharingAccessLevel::Full) {
                        return Some(Subject::Team(TeamKind::SharedSessionTeam {
                            team_uid: *team_uid,
                            name: name.clone(),
                        }));
                    }
                }

                // Otherwise, the sharer is the owner.
                // The sharer doesn't store their own participant info, so if it's unset, we assume
                // the current user.
                handle
                    .upgrade(app)
                    .and_then(|handle| handle.as_ref(app).shared_session_presence_manager())
                    .and_then(|presence| presence.as_ref(app).get_sharer())
                    .map(|sharer| {
                        UserKind::SharedSessionParticipant(sharer.info.profile_data.clone())
                    })
                    .or_else(|| {
                        AuthStateProvider::as_ref(app)
                            .get()
                            .user_id()
                            .map(UserKind::Account)
                    })
                    .map(Subject::User)
            }
            ShareableObject::AIConversation(id) => {
                // Get owner from conversation's server metadata
                BlocklistAIHistoryModel::as_ref(app)
                    .get_server_conversation_metadata(id)
                    .map(|m| Subject::from_owner(m.permissions.space))
            }
        }
    }

    fn update_session_guests(
        &mut self,
        ctx: &mut ViewContext<Self>,
        session_id: &SessionId,
        guests: &[Guest],
        pending_guests: &[PendingGuest],
    ) {
        // We should only update the guests if the dialog is targeting the
        // correct session.
        match self.target.as_ref() {
            Some(ShareableObject::Session {
                session_id: target_session_id,
                ..
            }) => {
                if session_id != target_session_id {
                    return;
                }
            }
            _ => return,
        }

        let guests_iter = guests.iter().map(|guest| GuestState {
            menu_button_handle: Default::default(),
            tooltip_handle: Default::default(),
            current_access_level: guest.direct_acl.into(),
            subject: Subject::User(UserKind::SharedSessionParticipant(
                guest.profile_data.clone(),
            )),
            inheritance: None,
        });

        let pending_guests_iter = pending_guests.iter().map(|guest| GuestState {
            menu_button_handle: Default::default(),
            tooltip_handle: Default::default(),
            current_access_level: guest.direct_acl.into(),
            subject: Subject::PendingUser {
                email: Some(guest.email.clone()),
            },
            inheritance: None,
        });

        self.guest_states = guests_iter.chain(pending_guests_iter).collect();

        self.guest_states
            .sort_by_cached_key(|guest| guest.subject.name(ctx));

        ctx.notify();
    }

    fn update_session_link_permissions(
        &mut self,
        session_id: SessionId,
        access_level: Option<SharingAccessLevel>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Ensure we're targeting the correct session.
        let Some(ShareableObject::Session {
            session_id: target_session_id,
            ..
        }) = self.target.as_ref()
        else {
            return;
        };
        if &session_id != target_session_id {
            return;
        }

        self.link_sharing_state = LinkSharingState {
            access_level,
            tooltip_handle: Default::default(),
            inheritance: None,
        };
        ctx.notify()
    }

    fn update_session_team_permissions(
        &mut self,
        session_id: &SessionId,
        team_acl: Option<TeamAclData>,
        ctx: &mut ViewContext<Self>,
    ) {
        // Ensure we're targeting the correct session.
        match self.target.as_ref() {
            Some(ShareableObject::Session {
                session_id: target_session_id,
                ..
            }) => {
                if session_id != target_session_id {
                    return;
                }
            }
            _ => return,
        }

        self.team_sharing_state = TeamSharingState {
            access_level: team_acl.as_ref().map(|team_acl| team_acl.acl.into()),
            team: team_acl.map(|team_acl| TeamKind::SharedSessionTeam {
                team_uid: ServerId::from_string_lossy(team_acl.uid),
                name: team_acl.name,
            }),
            tooltip_handle: Default::default(),
            inheritance: None,
        };
        ctx.notify()
    }

    /// Refreshes all permissions that have cached UI state.
    fn refresh_object_permission_states(&mut self, ctx: &mut ViewContext<Self>) {
        // The permission states for shared sessions are managed differently
        // than other cloud objects. We return to avoid resetting the
        // permissions for sessions.
        if matches!(self.target.as_ref(), Some(ShareableObject::Session { .. })) {
            return;
        }

        // Handle AI conversations separately
        if let Some(ShareableObject::AIConversation(conversation_id)) = &self.target {
            // Use the helper that checks both loaded conversations and historical metadata
            if let Some(server_metadata) = BlocklistAIHistoryModel::as_ref(ctx)
                .get_server_conversation_metadata(conversation_id)
            {
                let permissions = &server_metadata.permissions;
                // Populate guest states from conversation's server permissions
                self.guest_states = permissions
                    .guests
                    .iter()
                    .filter_map(|guest| {
                        // Convert ServerGuestSubject to Subject
                        let subject = match &guest.subject {
                            ServerGuestSubject::User { firebase_uid } => {
                                let user_uid = crate::auth::UserUid::new(firebase_uid);
                                Some(super::Subject::User(super::UserKind::Account(user_uid)))
                            }
                            ServerGuestSubject::PendingUser { email } => {
                                Some(super::Subject::PendingUser {
                                    email: email.clone(),
                                })
                            }
                            ServerGuestSubject::Team { team_uid } => {
                                Some(super::Subject::Team(super::TeamKind::Team {
                                    team_uid: *team_uid,
                                }))
                            }
                        }?;

                        Some(GuestState {
                            menu_button_handle: Default::default(),
                            subject,
                            current_access_level: guest.access_level.into(),
                            tooltip_handle: Default::default(),
                            inheritance: None, // AI conversations don't support inheritance yet
                        })
                    })
                    .collect();

                // Handle link sharing state
                self.link_sharing_state = match &permissions.anyone_link_sharing {
                    Some(link_sharing) => LinkSharingState {
                        access_level: Some(link_sharing.access_level.into()),
                        tooltip_handle: Default::default(),
                        inheritance: None,
                    },
                    None => Default::default(),
                };

                self.guest_states
                    .sort_by_cached_key(|guest| guest.subject.name(ctx));
                ctx.notify();
                return;
            }
            // If permissions not found, clear states
            self.guest_states.clear();
            self.guest_states.shrink_to_fit();
            self.link_sharing_state = Default::default();
            self.team_sharing_state = Default::default();
            ctx.notify();
            return;
        }

        match self.target_cloud_object(ctx) {
            Some(object) => {
                let object_id = object.sync_id();
                self.guest_states = object
                    .permissions()
                    .guests
                    .iter()
                    .map(move |guest| GuestState {
                        menu_button_handle: Default::default(),
                        subject: guest.subject.clone(),
                        current_access_level: guest.access_level,
                        tooltip_handle: Default::default(),
                        inheritance: InheritanceState::from_object_and_source(
                            &object_id,
                            guest.source.as_ref(),
                        ),
                    })
                    .collect();

                self.link_sharing_state = match &object.permissions().anyone_with_link {
                    Some(link_sharing) => LinkSharingState {
                        access_level: Some(link_sharing.access_level),
                        tooltip_handle: Default::default(),
                        inheritance: InheritanceState::from_object_and_source(
                            &object_id,
                            link_sharing.source.as_ref(),
                        ),
                    },
                    None => Default::default(),
                }
            }
            None => {
                self.guest_states.clear();
                self.guest_states.shrink_to_fit();
                self.link_sharing_state = Default::default();
                self.team_sharing_state = Default::default();
            }
        }

        self.guest_states
            .sort_by_cached_key(|guest| guest.subject.name(ctx));

        ctx.notify();
    }

    /// Saved position ID for a particular guest's menu button.
    fn guest_access_button_id(&self, idx: usize) -> String {
        format!(
            "sharing_dialog_guest_access_button_{}_{}",
            idx,
            self.self_handle.id()
        )
    }

    /// Copy the object's URL to the clipboard.
    pub fn copy_link(&self, ctx: &mut ViewContext<Self>) {
        if let Some(url) = self.target.as_ref().and_then(|target| target.link(ctx)) {
            let event = match self.target.as_ref() {
                Some(ShareableObject::Session { .. }) => {
                    Some(TelemetryEvent::CopiedSharedSessionLink {
                        source: SharedSessionActionSource::SharingDialog,
                    })
                }
                Some(ShareableObject::WarpDriveObject(_))
                | Some(ShareableObject::AIConversation(_)) => {
                    Some(TelemetryEvent::ObjectLinkCopied { link: url.clone() })
                }
                None => None,
            };
            if let Some(event) = event {
                send_telemetry_from_ctx!(event, ctx);
            }

            ctx.clipboard().write(ClipboardContent::plain_text(url));

            let window_id = ctx.window_id();
            let object_name = self.targeted_object_name(ctx);
            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                let toast = DismissibleToast::default(format!("Copied link to {object_name}."));
                toast_stack.add_ephemeral_toast(toast, window_id, ctx);
            });
        }
    }

    /// Create the menu for editing guest access.
    fn build_guest_menu(ctx: &mut ViewContext<Self>) -> ViewHandle<Menu<SharingDialogAction>> {
        let menu = ctx.add_typed_action_view(|_| {
            Menu::new()
                .prevent_interaction_with_other_elements()
                .with_drop_shadow()
        });
        ctx.subscribe_to_view(&menu, |me, menu, event, ctx| {
            me.handle_menu_event(menu, event, ctx);
        });
        menu
    }

    /// Configure the guest dropdown menu for a newly-selected guest.
    fn reset_guest_menu(&mut self, guest_index: usize, ctx: &mut ViewContext<Self>) {
        if let Some(guest) = self.guest_states.get(guest_index) {
            let current_access_level = guest.current_access_level;
            let inherited_access = guest.inheritance.is_some();
            let is_ai_conversation = matches!(
                self.target.as_ref(),
                Some(ShareableObject::AIConversation(_))
            );
            // Check if this is a team guest - team removal is only supported for non-session targets
            let is_team_guest = matches!(guest.subject, Subject::Team(_));
            let is_session = matches!(self.target.as_ref(), Some(ShareableObject::Session { .. }));

            self.guest_menu.update(ctx, |menu, ctx| {
                let mut items = vec![MenuItemFields::new(SharingAccessLevel::View.label())
                    .with_on_select_action(SharingDialogAction::SetGuestAccessLevel(
                        SharingAccessLevel::View,
                    ))
                    .with_disabled(
                        inherited_access && current_access_level >= SharingAccessLevel::View,
                    )
                    .into_item()];

                // Only add Edit option if not an AI conversation
                if !is_ai_conversation {
                    items.push(
                        MenuItemFields::new(SharingAccessLevel::Edit.label())
                            .with_on_select_action(SharingDialogAction::SetGuestAccessLevel(
                                SharingAccessLevel::Edit,
                            ))
                            .with_disabled(
                                inherited_access
                                    && current_access_level >= SharingAccessLevel::Edit,
                            )
                            .into_item(),
                    );
                }

                // Add Remove option for non-team guests, or for team guests in non-session contexts
                // (team removal is supported for WarpDrive objects and AI conversations, but not sessions)
                if !is_team_guest || !is_session {
                    items.push(MenuItem::Separator);
                    items.push(
                        MenuItemFields::new("Remove")
                            .with_on_select_action(SharingDialogAction::RemoveGuest)
                            .with_disabled(inherited_access)
                            .into_item(),
                    );
                }

                menu.set_items(items, ctx);
                menu.set_selected_by_index(
                    match current_access_level {
                        SharingAccessLevel::View => 0,
                        SharingAccessLevel::Edit => {
                            if is_ai_conversation {
                                0
                            } else {
                                1
                            }
                        }
                        // Not yet supported, so default to view.
                        SharingAccessLevel::Full => 0,
                    },
                    ctx,
                );
            })
        }
    }

    /// Remove the guest currently targeted via dropdown menu,
    fn remove_targeted_guest(&mut self, ctx: &mut ViewContext<Self>) {
        let OpenMenuState::Guest(idx) = self.open_menu_state else {
            return;
        };

        let Some(guest) = self.guest_states.get(idx) else {
            return;
        };
        // Ensure we don't try to update inherited ACLs.
        if guest.inheritance.is_some() {
            return;
        }

        match &self.target {
            Some(ShareableObject::WarpDriveObject(object_id)) => {
                let guest_identifier = guest.subject.to_guest_identifier(ctx);
                if let Some(guest_identifier) = guest_identifier {
                    let object_id = *object_id;
                    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                        update_manager.remove_object_guest(object_id, guest_identifier, ctx);
                    });
                }
            }
            Some(ShareableObject::Session { handle, .. }) => {
                self.remove_targeted_guest_for_session(idx, handle.clone(), ctx);
            }
            Some(ShareableObject::AIConversation(conversation_id)) => {
                self.remove_targeted_guest_for_conversation(idx, *conversation_id, ctx);
            }
            None => (),
        }

        self.set_open_menu(OpenMenuState::None, ctx);
    }

    fn remove_targeted_guest_for_session(
        &mut self,
        guest_idx: usize,
        handle: WeakViewHandle<TerminalView>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(guest) = self.guest_states.get(guest_idx) else {
            return;
        };

        let Some(handle) = handle.upgrade(ctx) else {
            log::error!(
                "Unable to upgrade handle to TerminalView when removing guest from session"
            );
            return;
        };

        if let Some(user_uid) = guest.subject.user_uid() {
            // User is a full guest.
            handle.update(ctx, |view, ctx| {
                view.remove_guest(user_uid, ctx);
            });
        } else if let Some(email) = guest.subject.email(ctx) {
            // User is a pending guest.
            let email = email.to_owned();
            handle.update(ctx, |view, ctx| {
                view.remove_pending_guest(email, ctx);
            });
        }
    }

    /// Set the currently-targeted guest's access level.
    fn set_targeted_guest_access(
        &mut self,
        access_level: SharingAccessLevel,
        ctx: &mut ViewContext<Self>,
    ) {
        let OpenMenuState::Guest(idx) = self.open_menu_state else {
            return;
        };
        let Some(guest) = self.guest_states.get_mut(idx) else {
            return;
        };
        // Optimistically update the dialog label.
        guest.current_access_level = access_level;
        ctx.notify();

        match &self.target {
            Some(ShareableObject::WarpDriveObject(object_id)) => {
                self.set_targeted_guest_access_for_object(idx, access_level, *object_id, ctx);
            }
            Some(ShareableObject::Session { handle, .. }) => {
                self.set_targeted_guest_access_for_session(idx, access_level, handle.clone(), ctx);
            }
            Some(ShareableObject::AIConversation(conversation_id)) => {
                self.set_targeted_guest_access_for_conversation(
                    idx,
                    access_level,
                    *conversation_id,
                    ctx,
                );
            }
            None => (),
        };
    }

    fn set_targeted_guest_access_for_object(
        &mut self,
        guest_idx: usize,
        access_level: SharingAccessLevel,
        object_id: ServerId,
        ctx: &mut ViewContext<Self>,
    ) {
        let (guest_email, is_inherited) = match self.guest_states.get(guest_idx) {
            Some(guest) => match guest.subject.email(ctx) {
                Some(email) => (email.to_owned(), guest.inheritance.is_some()),
                None => return,
            },
            None => return,
        };

        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            // If there's only an inherited guest ACL, we have to add a new guest to the descendant
            // object.
            if is_inherited {
                update_manager.add_object_guests(
                    object_id,
                    vec![guest_email],
                    access_level.into(),
                    ctx,
                );
            } else {
                update_manager.update_object_guests(
                    object_id,
                    vec![guest_email],
                    access_level.into(),
                    ctx,
                );
            }
        });

        self.set_open_menu(OpenMenuState::None, ctx);
    }

    fn set_targeted_guest_access_for_session(
        &mut self,
        guest_idx: usize,
        access_level: SharingAccessLevel,
        handle: WeakViewHandle<TerminalView>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(guest) = self.guest_states.get(guest_idx) else {
            return;
        };

        let Some(handle) = handle.upgrade(ctx) else {
            log::error!(
                "Unable to upgrade handle to TerminalView when setting guest ACL for session"
            );
            return;
        };

        if let Some(user_uid) = guest.subject.user_uid() {
            // User is a full guest.
            handle.update(ctx, |view, ctx| {
                view.update_role_for_user(user_uid.to_owned(), access_level.into(), ctx);
            });
        } else if let Some(email) = guest.subject.email(ctx) {
            // User is a pending guest.
            let email = email.to_owned();
            handle.update(ctx, |view, ctx| {
                view.update_role_for_pending_user(email, access_level.into(), ctx);
            });
        }

        self.set_open_menu(OpenMenuState::None, ctx);
    }

    fn remove_targeted_guest_for_conversation(
        &mut self,
        guest_idx: usize,
        conversation_id: crate::ai::agent::conversation::AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(guest) = self.guest_states.get(guest_idx) else {
            return;
        };

        // Get the conversation's server_id from metadata
        let server_id = match BlocklistAIHistoryModel::as_ref(ctx)
            .get_server_conversation_metadata(&conversation_id)
            .map(|m| ServerId::from_string_lossy(m.metadata.uid.uid()))
        {
            Some(id) => id,
            None => {
                log::warn!(
                    "AI conversation {:?} has no server_id for permission update",
                    conversation_id
                );
                return;
            }
        };

        let guest_identifier = guest.subject.to_guest_identifier(ctx);
        let Some(guest_identifier) = guest_identifier else {
            return;
        };

        // Call UpdateManager to remove the guest
        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager.remove_ai_conversation_guest(
                server_id,
                conversation_id,
                guest_identifier,
                ctx,
            );
        });
    }

    fn set_targeted_guest_access_for_conversation(
        &mut self,
        guest_idx: usize,
        access_level: SharingAccessLevel,
        conversation_id: crate::ai::agent::conversation::AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let guest_email = match self.guest_states.get(guest_idx) {
            Some(guest) => match guest.subject.email(ctx) {
                Some(email) => email.to_owned(),
                None => return,
            },
            None => return,
        };

        // Get the conversation's server_id from metadata
        let server_id = match BlocklistAIHistoryModel::as_ref(ctx)
            .get_server_conversation_metadata(&conversation_id)
            .map(|m| ServerId::from_string_lossy(m.metadata.uid.uid()))
        {
            Some(id) => id,
            None => {
                log::warn!(
                    "AI conversation {:?} has no server_id for permission update",
                    conversation_id
                );
                return;
            }
        };

        // Call UpdateManager to update the guest's access level
        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager.update_ai_conversation_guests(
                server_id,
                conversation_id,
                vec![guest_email],
                access_level.into(),
                ctx,
            );
        });
    }

    fn add_guests_for_conversation(
        &mut self,
        guest_emails: Vec<String>,
        access_level: SharingAccessLevel,
        conversation_id: crate::ai::agent::conversation::AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        // Get the conversation's server_id from metadata
        let server_id = match BlocklistAIHistoryModel::as_ref(ctx)
            .get_server_conversation_metadata(&conversation_id)
            .map(|m| ServerId::from_string_lossy(m.metadata.uid.uid()))
        {
            Some(id) => id,
            None => {
                log::warn!(
                    "AI conversation {:?} has no server_id for permission update",
                    conversation_id
                );
                return;
            }
        };

        // Call UpdateManager to add guests
        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager.add_ai_conversation_guests(
                server_id,
                conversation_id,
                guest_emails,
                access_level.into(),
                ctx,
            );
        });
    }

    /// Create the access level selector dropdown for the email invitation form.
    fn build_invite_access_level_menu(
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<Menu<SharingDialogAction>> {
        let menu = ctx.add_typed_action_view(|_| {
            let mut menu = Menu::new();
            // Note: Items will be updated dynamically in reset_invite_access_level_menu
            // based on whether the target is an AI conversation
            menu.add_items([
                MenuItemFields::new(SharingAccessLevel::View.label())
                    .with_on_select_action(SharingDialogAction::SetInviteAccessLevel(
                        SharingAccessLevel::View,
                    ))
                    .into_item(),
                MenuItemFields::new(SharingAccessLevel::Edit.label())
                    .with_on_select_action(SharingDialogAction::SetInviteAccessLevel(
                        SharingAccessLevel::Edit,
                    ))
                    .into_item(),
            ]);
            menu.prevent_interaction_with_other_elements()
                .with_drop_shadow()
        });
        ctx.subscribe_to_view(&menu, |me, menu, event, ctx| {
            me.handle_menu_event(menu, event, ctx);
        });
        menu
    }

    /// Reset the invite access level menu based on the current target.
    fn reset_invite_access_level_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let is_ai_conversation = matches!(
            self.target.as_ref(),
            Some(ShareableObject::AIConversation(_))
        );

        self.invite_form.access_level_menu.update(ctx, |menu, ctx| {
            let mut items = vec![MenuItemFields::new(SharingAccessLevel::View.label())
                .with_on_select_action(SharingDialogAction::SetInviteAccessLevel(
                    SharingAccessLevel::View,
                ))
                .into_item()];

            // Only add Edit option if not an AI conversation
            if !is_ai_conversation {
                items.push(
                    MenuItemFields::new(SharingAccessLevel::Edit.label())
                        .with_on_select_action(SharingDialogAction::SetInviteAccessLevel(
                            SharingAccessLevel::Edit,
                        ))
                        .into_item(),
                );
            }

            menu.set_items(items, ctx);
            // Always select View (index 0) for AI conversations
            menu.set_selected_by_index(
                match self.invite_form.selected_access_level {
                    SharingAccessLevel::View => 0,
                    SharingAccessLevel::Edit => {
                        if is_ai_conversation {
                            0
                        } else {
                            1
                        }
                    }
                    SharingAccessLevel::Full => 0,
                },
                ctx,
            );
        });
    }

    fn invite_access_level_button_id(&self) -> String {
        format!("invite_form_access_level_{}", self.self_handle.id())
    }

    /// Render the form for inviting new guests.
    fn render_invite_form(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let editor = ChildView::new(&self.invite_form.email_editor).finish();

        let access_level = SavePosition::new(
            appearance
                .ui_builder()
                .button(
                    ButtonVariant::Text,
                    self.ui_state_handles.invite_access_level_button.clone(),
                )
                .with_centered_text_label(
                    self.invite_form.selected_access_level.label().to_string(),
                )
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(SharingDialogAction::ToggleInviteAccessLevelMenu);
                })
                .finish(),
            &self.invite_access_level_button_id(),
        )
        .finish();

        let editor_container = Container::new(
            Flex::row()
                .with_children([Shrinkable::new(1., editor).finish(), access_level])
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        )
        .with_border(Border::all(1.).with_border_color(style::form_border_color(appearance)))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .with_margin_right(style::ACL_ITEM_PADDING / 2.)
        // Match the word block editor's left padding.
        .with_padding_right(8.)
        .finish();

        let validation_state = self.invite_form_state(app);

        let mut invite_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                self.ui_state_handles.invite_button.clone(),
            )
            .with_centered_text_label("Invite".into())
            .with_style(UiComponentStyles {
                // Adjust the height to match the email editor's padding.
                height: Some(style::ACL_ITEM_HEIGHT + 6.),
                ..Default::default()
            });

        if !validation_state.is_valid() {
            invite_button = invite_button.disabled();
        }

        // For Warp Drive targets, we can't update permissions while there's a pending change.
        if self
            .target_cloud_object(app)
            .is_some_and(|object| object.metadata().has_pending_online_only_change())
        {
            invite_button = invite_button.disabled();
        }

        let invite_button = invite_button
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(SharingDialogAction::SendInvitations);
            })
            .finish();

        let form = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., editor_container).finish())
            .with_child(invite_button)
            .finish();

        let contents = if validation_state.is_valid() {
            form
        } else {
            let mut contents = Flex::column()
                .with_child(form)
                .with_main_axis_alignment(MainAxisAlignment::Start)
                .with_cross_axis_alignment(CrossAxisAlignment::Start);

            if !validation_state.duplicate_guests.is_empty() {
                let error_text = format!(
                    "Already shared with {}",
                    validation_state.duplicate_guests.iter().format(", ")
                );
                contents.add_child(self.render_error_message(error_text, appearance));
            }

            if !validation_state.invalid_emails.is_empty() {
                let error_text = format!(
                    "Invalid address: {}",
                    validation_state.invalid_emails.iter().format(", ")
                );
                contents.add_child(self.render_error_message(error_text, appearance));
            }

            contents.finish()
        };

        Container::new(contents)
            .with_horizontal_padding(style::ACL_ITEM_PADDING)
            .with_vertical_margin(style::ACL_ITEM_GAP / 2.)
            .finish()
    }

    fn render_error_message(
        &self,
        error: impl Into<Cow<'static, str>>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .paragraph(error)
            .with_style(UiComponentStyles {
                font_color: Some(appearance.theme().ui_error_color()),
                ..Default::default()
            })
            .build()
            .with_vertical_margin(style::ACL_ITEM_GAP / 2.)
            .finish()
    }

    fn email_invite_form_styles(app: &AppContext) -> WordBlockEditorStyles {
        let appearance = Appearance::as_ref(app);
        let text_color = style::acl_primary_text_color(appearance);

        WordBlockEditorStyles {
            font_family: appearance.ui_font_family(),
            editor_font_color: text_color,
            background: style::dialog_background(appearance).into(),
            valid_word_styles: WordBlockStyles {
                font_color: text_color,
                background: style::form_chip_background(appearance),
            },
            invalid_word_styles: WordBlockStyles {
                font_color: text_color,
                background: appearance.theme().ui_error_color().into(),
            },
        }
    }

    fn invite_form_state(&self, app: &AppContext) -> InviteFormValidationState {
        let invite_editor = self.invite_form.email_editor.as_ref(app);

        let invitees = invite_editor.get_list_of_words(app);
        let owner = self.owner(app);
        let duplicate_guests = self
            .guest_states
            .iter()
            .map(|g| &g.subject)
            .chain(owner.as_ref())
            .filter_map(|guest| {
                invitees
                    .iter()
                    .find(|invitee| guest.matches_email(invitee, app))
                    .cloned()
            })
            .collect();

        InviteFormValidationState {
            invalid_emails: invite_editor.get_list_of_invalid_words(app),
            duplicate_guests,
            invitee_emails: invitees,
        }
    }

    fn handle_email_invite_editor_event(
        &mut self,
        event: &WordBlockEditorViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            WordBlockEditorViewEvent::WordListValidityChanged => {
                // Re-render to enable/disable the invite button.
                ctx.notify();
            }
            WordBlockEditorViewEvent::Enter => {
                self.send_invitations(ctx);
            }
            WordBlockEditorViewEvent::Escape => ctx.emit(SharingDialogEvent::Close),
            WordBlockEditorViewEvent::Navigate(key) => match key {
                NavigationKey::Tab | NavigationKey::Down => {
                    self.set_open_menu(OpenMenuState::InviteAccessLevel, ctx);
                }
                _ => (),
            },
        }
    }

    /// Send all pending email invitations.
    fn send_invitations(&mut self, ctx: &mut ViewContext<Self>) {
        let form_state = self.invite_form_state(ctx);
        if !form_state.is_valid() {
            return;
        }

        match &self.target {
            Some(ShareableObject::WarpDriveObject(object_id)) => {
                UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                    update_manager.add_object_guests(
                        *object_id,
                        form_state.invitee_emails,
                        self.invite_form.selected_access_level.into(),
                        ctx,
                    );
                });
            }
            Some(ShareableObject::Session { handle, .. }) => {
                let Some(handle) = handle.upgrade(ctx) else {
                    log::error!("Unable to upgrade handle to TerminalView when sending email invitations for session");
                    return;
                };

                handle.update(ctx, |view, ctx| {
                    view.add_guests(
                        form_state.invitee_emails,
                        self.invite_form.selected_access_level.into(),
                        ctx,
                    );
                });
            }
            Some(ShareableObject::AIConversation(conversation_id)) => {
                self.add_guests_for_conversation(
                    form_state.invitee_emails,
                    self.invite_form.selected_access_level,
                    *conversation_id,
                    ctx,
                );
            }
            None => return,
        }

        self.reset_invite_form(ctx);
        ctx.notify();
    }

    /// Reset all state for the invite form.
    fn reset_invite_form(&mut self, ctx: &mut ViewContext<Self>) {
        self.invite_form
            .email_editor
            .update(ctx, |editor, ctx| editor.clear_list_of_words(ctx));
        self.invite_form.selected_access_level = SharingAccessLevel::View;
        self.invite_form
            .access_level_menu
            .update(ctx, |menu, ctx| menu.set_selected_by_index(0, ctx))
    }

    pub fn add_invitee_email(&mut self, invitee_email: String, ctx: &mut ViewContext<Self>) {
        self.invite_form.email_editor.update(ctx, |editor, ctx| {
            editor.clear_list_of_words(ctx);
            editor.set_editor_buffer_text(&invitee_email, ctx);
            ctx.notify();
        });
        ctx.focus(&self.invite_form.email_editor);
    }

    fn handle_menu_event(
        &mut self,
        _menu: ViewHandle<Menu<SharingDialogAction>>,
        event: &menu::Event,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            menu::Event::ItemSelected | menu::Event::ItemHovered => {}
            menu::Event::Close { .. } => {
                self.set_open_menu(OpenMenuState::None, ctx);
            }
        }
    }

    /// Toggle a particular menu open or closed.
    fn toggle_menu(&mut self, menu: OpenMenuState, ctx: &mut ViewContext<Self>) {
        let new_state = if menu == self.open_menu_state {
            OpenMenuState::None
        } else {
            menu
        };
        self.set_open_menu(new_state, ctx);
    }

    /// Set which menu/dropdown is currently open.
    fn set_open_menu(&mut self, open_menu: OpenMenuState, ctx: &mut ViewContext<Self>) {
        match open_menu {
            OpenMenuState::None => {
                if self.open_menu_state == OpenMenuState::InviteAccessLevel {
                    // If the invite form menu _was_ open, switch focus to the email editor.
                    ctx.focus(&self.invite_form.email_editor);
                } else {
                    ctx.focus_self();
                }
            }
            OpenMenuState::LinkSharing => {
                self.reset_link_sharing_menu(ctx);
                ctx.focus(&self.link_sharing_menu)
            }
            OpenMenuState::TeamSharing => {
                self.reset_team_sharing_menu(ctx);
                ctx.focus(&self.team_sharing_menu)
            }
            OpenMenuState::InviteAccessLevel => {
                self.reset_invite_access_level_menu(ctx);
                ctx.focus(&self.invite_form.access_level_menu)
            }
            OpenMenuState::Guest(idx) => {
                self.reset_guest_menu(idx, ctx);
                ctx.focus(&self.guest_menu);
            }
        }
        self.open_menu_state = open_menu;
        ctx.notify();
    }

    /// Render the open menu, if any.
    fn render_menu(&self, stack: &mut Stack, app: &AppContext) {
        if !self.can_edit_access(app) {
            return;
        }

        match self.open_menu_state {
            OpenMenuState::None => (),
            OpenMenuState::LinkSharing => {
                stack.add_positioned_overlay_child(
                    ChildView::new(&self.link_sharing_menu).finish(),
                    OffsetPositioning::offset_from_save_position_element(
                        self.link_sharing_menu_button_id(),
                        vec2f(0., 0.),
                        PositionedElementOffsetBounds::WindowByPosition,
                        PositionedElementAnchor::TopRight,
                        ChildAnchor::TopLeft,
                    ),
                );
            }
            OpenMenuState::TeamSharing => {
                stack.add_positioned_overlay_child(
                    ChildView::new(&self.team_sharing_menu).finish(),
                    OffsetPositioning::offset_from_save_position_element(
                        self.team_sharing_menu_button_id(),
                        vec2f(0., 0.),
                        PositionedElementOffsetBounds::WindowByPosition,
                        PositionedElementAnchor::TopRight,
                        ChildAnchor::TopLeft,
                    ),
                );
            }
            OpenMenuState::InviteAccessLevel => stack.add_positioned_overlay_child(
                ChildView::new(&self.invite_form.access_level_menu).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    self.invite_access_level_button_id(),
                    vec2f(0., 4.),
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::BottomLeft,
                    ChildAnchor::TopLeft,
                ),
            ),
            OpenMenuState::Guest(idx) => {
                // Check that the guest is in bounds, in case it was removed but we haven't yet
                // refreshed `self.guest_states`.
                if idx < self.guest_states.len() {
                    stack.add_positioned_overlay_child(
                        ChildView::new(&self.guest_menu).finish(),
                        OffsetPositioning::offset_from_save_position_element(
                            self.guest_access_button_id(idx),
                            vec2f(0., 4.),
                            PositionedElementOffsetBounds::WindowByPosition,
                            PositionedElementAnchor::BottomLeft,
                            ChildAnchor::TopLeft,
                        ),
                    )
                }
            }
        }
    }

    /// Render the "Who has access" header shown above the ACL list.
    fn render_access_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .span("Who has access")
            .with_style(UiComponentStyles {
                font_color: Some(style::label_text(appearance)),
                font_size: Some(style::PRIMARY_TEXT_SIZE),
                ..Default::default()
            })
            .build()
            .with_horizontal_padding(style::ACL_ITEM_PADDING)
            .with_vertical_margin(style::ACL_ITEM_GAP / 2.)
            .finish()
    }

    /// Renders a header with live session details only for shared session sharers.
    fn render_session_header(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let Some(ShareableObject::Session { started_at, .. }) = self.target.as_ref() else {
            return None;
        };

        if !self.can_edit_access(app) {
            return None;
        }

        let text = appearance
            .ui_builder()
            .wrappable_text(
                format!(
                    "Live session started at {} on {}",
                    started_at.format("%l:%M%P"),
                    started_at.format("%m/%d"),
                ),
                true,
            )
            .with_style(UiComponentStyles {
                font_color: Some(style::acl_primary_text_color(appearance)),
                font_size: Some(style::HEADER_TEXT_SIZE),
                ..Default::default()
            })
            .build()
            .finish();

        Some(
            Container::new(text)
                .with_horizontal_padding(style::ACL_ITEM_PADDING)
                .with_padding_top(style::ACL_ITEM_PADDING / 2.)
                .with_padding_bottom(style::ACL_ITEM_PADDING)
                .finish(),
        )
    }

    /// Renders a clarification label if the user is not allowed to edit permissions.
    fn render_restricted_access_label(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let access_level = self.access_level(app);
        if access_level.can_edit_access() {
            return None;
        }

        const PREFIX: &str = "You must have full access to manage permissions. You have ";
        const SUFFIX: &str = " access.";
        let access_level_start = PREFIX.chars().count();
        let access_level_end = access_level_start + access_level.name().chars().count();

        let text = appearance
            .ui_builder()
            .wrappable_text(format!("{PREFIX}{}{SUFFIX}", access_level.name()), true)
            .with_style(UiComponentStyles {
                font_color: Some(style::label_text(appearance)),
                ..Default::default()
            })
            .with_highlights(
                (access_level_start..access_level_end).collect(),
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
            )
            .build()
            .finish();

        Some(
            Container::new(text)
                .with_horizontal_padding(style::ACL_ITEM_PADDING)
                .with_vertical_margin(style::ACL_ITEM_GAP / 2.)
                .finish(),
        )
    }

    fn render_owner(&self, appearance: &Appearance, app: &AppContext) -> Option<Box<dyn Element>> {
        let owner = self.owner(app)?;

        let tooltip_text = match owner {
            Subject::Team(_) => "Team objects automatically grant full permissions to team members",
            _ => "Owners always have full permissions on their objects",
        };
        let owner_access_label = render_with_detail_tooltip(
            tooltip_text,
            self.ui_state_handles.owner_tooltip.clone(),
            appearance
                .ui_builder()
                .span(SharingAccessLevel::Full.label())
                .with_style(UiComponentStyles {
                    font_color: Some(
                        appearance
                            .theme()
                            .disabled_text_color(style::dialog_background(appearance).into())
                            .into_solid(),
                    ),
                    ..Default::default()
                })
                .build()
                .finish(),
            appearance,
        );

        Some(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(self.render_subject(&owner, None, appearance, app))
                    .with_child(owner_access_label)
                    .finish(),
            )
            .with_horizontal_padding(style::ACL_ITEM_PADDING)
            .with_vertical_margin(style::ACL_ITEM_GAP / 2.)
            .finish(),
        )
    }

    fn link_sharing_menu_button_id(&self) -> String {
        format!("link_sharing_menu_button_{}", self.self_handle.id())
    }

    fn render_link_sharing_subject(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let link_sharing_subject_type = match self.link_sharing_state.access_level {
            Some(_) => LinkSharingSubjectType::Anyone,
            None => LinkSharingSubjectType::None,
        };
        let can_edit_access = self.can_edit_access(app);

        if !can_edit_access && self.link_sharing_state.access_level.is_none() {
            return None;
        }

        let (inherited_label, inherited_tooltip) = self
            .link_sharing_state
            .inheritance
            .as_ref()
            .map(|inheritance| {
                let InheritanceDetails {
                    source_label,
                    tooltip_text,
                } = inheritance.details(appearance, app);
                (source_label, tooltip_text)
            })
            .unzip();

        let mut subject_row = Flex::row();

        subject_row.add_child(
            Container::new(self.render_subject(
                &Subject::AnyoneWithLink(link_sharing_subject_type),
                inherited_label,
                appearance,
                app,
            ))
            .with_horizontal_padding(style::ACL_ITEM_PADDING)
            .with_vertical_margin(style::ACL_ITEM_GAP / 2.)
            .finish(),
        );

        let menu_button_label = match self.link_sharing_state.access_level {
            Some(access_level) => access_level.label(),
            None => NO_ACCESS_LABEL,
        };
        let mut menu_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Text,
                self.ui_state_handles.link_sharing_menu_button.clone(),
            )
            .with_centered_text_label(menu_button_label.to_string())
            .with_style(UiComponentStyles {
                padding: Some(Coords::default()),
                ..Default::default()
            });
        if !can_edit_access {
            menu_button = menu_button.disabled();
        }
        let menu_button = SavePosition::new(
            menu_button
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(SharingDialogAction::ToggleLinkSharingMenu);
                })
                .finish(),
            &self.link_sharing_menu_button_id(),
        )
        .finish();

        subject_row.add_child(match inherited_tooltip {
            Some(tooltip) => render_with_detail_tooltip(
                tooltip,
                self.link_sharing_state.tooltip_handle.clone(),
                menu_button,
                appearance,
            ),
            None => menu_button,
        });

        Some(
            Container::new(
                subject_row
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .finish(),
            )
            .with_padding_right(style::ACL_ITEM_PADDING)
            .finish(),
        )
    }

    fn reset_link_sharing_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let inherited_access = self.link_sharing_state.inheritance.is_some();
        let current_access_level = self.link_sharing_state.access_level;
        let is_ai_conversation = matches!(
            self.target.as_ref(),
            Some(ShareableObject::AIConversation(_))
        );

        let mut items = vec![
            MenuItemFields::new("Only people invited")
                .with_on_select_action(SharingDialogAction::SetLinkPermissions(None))
                .with_icon(Icon::Lock)
                .with_disabled(inherited_access)
                .into_item(),
            MenuItem::Separator,
            MenuItemFields::new("Anyone with the link")
                .with_no_interaction_on_hover()
                .with_icon(Icon::Globe)
                .into_item(),
            MenuItemFields::new(SharingAccessLevel::View.label())
                .with_on_select_action(SharingDialogAction::SetLinkPermissions(Some(
                    SharingAccessLevel::View,
                )))
                .with_disabled(
                    inherited_access && current_access_level >= Some(SharingAccessLevel::View),
                )
                .into_item(),
        ];

        // Only add Edit option if not an AI conversation
        if !is_ai_conversation {
            items.push(
                MenuItemFields::new(SharingAccessLevel::Edit.label())
                    .with_on_select_action(SharingDialogAction::SetLinkPermissions(Some(
                        SharingAccessLevel::Edit,
                    )))
                    .with_disabled(
                        inherited_access && current_access_level >= Some(SharingAccessLevel::Edit),
                    )
                    .into_item(),
            );
        }

        self.link_sharing_menu.update(ctx, |menu, ctx| {
            menu.set_items(items, ctx);
            menu.set_selected_by_index(
                match current_access_level {
                    None => 0,
                    Some(SharingAccessLevel::View) => 3,
                    Some(SharingAccessLevel::Edit) => {
                        if is_ai_conversation {
                            3
                        } else {
                            4
                        }
                    }
                    Some(SharingAccessLevel::Full) => 3,
                },
                ctx,
            );
        })
    }

    fn team_sharing_menu_button_id(&self) -> String {
        format!("team_sharing_menu_button_{}", self.self_handle.id())
    }

    /// Renders the entry for the team sharing ACL, if the correct conditions
    /// are met.
    fn render_team_sharing_subject(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        // Currently, we only allow editing the team ACL for sessions.
        if !matches!(self.target.as_ref(), Some(ShareableObject::Session { .. })) {
            return None;
        }

        let can_edit_access = self.can_edit_access(app);

        // If there's no team ACL, and the user can't edit it, show nothing.
        if !can_edit_access && self.team_sharing_state.access_level.is_none() {
            return None;
        }

        let (inherited_label, inherited_tooltip) = self
            .team_sharing_state
            .inheritance
            .as_ref()
            .map(|inheritance| {
                let InheritanceDetails {
                    source_label,
                    tooltip_text,
                } = inheritance.details(appearance, app);
                (source_label, tooltip_text)
            })
            .unzip();

        // This logic assumes that the sharer's team is the one we would want
        // to add permissions for.
        let team_kind = if can_edit_access {
            TeamKind::Team {
                team_uid: UserWorkspaces::as_ref(app).current_team_uid()?,
            }
        } else {
            self.team_sharing_state.team.clone()?
        };
        // If this team is the owner of the object, don't render this team sharing ACL since
        // we already rendered the team as the owner (and you can't change ACLs on it).
        if let Some(Subject::Team(team_owner)) = self.owner(app) {
            if team_owner.team_uid() == team_kind.team_uid() {
                return None;
            }
        }

        let mut subject_row = Flex::row();
        subject_row.add_child(
            Container::new(self.render_subject(
                &Subject::Team(team_kind),
                inherited_label,
                appearance,
                app,
            ))
            .with_horizontal_padding(style::ACL_ITEM_PADDING)
            .with_vertical_margin(style::ACL_ITEM_GAP / 2.)
            .finish(),
        );

        let menu_button = {
            let label = match self.team_sharing_state.access_level {
                Some(access_level) => access_level.label(),
                None => NO_ACCESS_LABEL,
            };
            let button = appearance
                .ui_builder()
                .button(
                    ButtonVariant::Text,
                    self.ui_state_handles.team_sharing_menu_button.clone(),
                )
                .with_centered_text_label(label.to_string())
                .with_style(UiComponentStyles {
                    padding: Some(Coords::default()),
                    ..Default::default()
                });
            if can_edit_access {
                button
            } else {
                button.disabled()
            }
        };

        let menu_button = SavePosition::new(
            menu_button
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(SharingDialogAction::ToggleTeamSharingMenu);
                })
                .finish(),
            &self.team_sharing_menu_button_id(),
        )
        .finish();

        subject_row.add_child(match inherited_tooltip {
            Some(tooltip) => render_with_detail_tooltip(
                tooltip,
                self.team_sharing_state.tooltip_handle.clone(),
                menu_button,
                appearance,
            ),
            None => menu_button,
        });

        Some(
            Container::new(
                subject_row
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .finish(),
            )
            .with_padding_right(style::ACL_ITEM_PADDING)
            .finish(),
        )
    }

    fn reset_team_sharing_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let inherited_access = self.team_sharing_state.inheritance.is_some();
        let current_access_level = self.team_sharing_state.access_level;
        let items = [
            MenuItemFields::new("Only invited teammates")
                .with_on_select_action(SharingDialogAction::SetTeamPermissions(None))
                .with_icon(Icon::Lock)
                .with_disabled(inherited_access)
                .into_item(),
            MenuItem::Separator,
            MenuItemFields::new("Teammates with the link")
                .with_no_interaction_on_hover()
                .with_icon(Icon::Users)
                .into_item(),
            MenuItemFields::new(SharingAccessLevel::View.label())
                .with_on_select_action(SharingDialogAction::SetTeamPermissions(Some(
                    SharingAccessLevel::View,
                )))
                .with_disabled(
                    inherited_access && current_access_level >= Some(SharingAccessLevel::View),
                )
                .into_item(),
            MenuItemFields::new(SharingAccessLevel::Edit.label())
                .with_on_select_action(SharingDialogAction::SetTeamPermissions(Some(
                    SharingAccessLevel::Edit,
                )))
                .with_disabled(
                    inherited_access && current_access_level >= Some(SharingAccessLevel::Edit),
                )
                .into_item(),
        ];

        self.team_sharing_menu.update(ctx, |menu, ctx| {
            menu.set_items(items, ctx);
            menu.set_selected_by_index(
                match current_access_level {
                    None => 0,
                    Some(SharingAccessLevel::View) => 3,
                    Some(SharingAccessLevel::Edit) => 4,
                    // Not yet supported, so default to view.
                    Some(SharingAccessLevel::Full) => 3,
                },
                ctx,
            );
        })
    }

    /// Renders a guest ACL.
    fn render_guest(
        &self,
        guest: &GuestState,
        index: usize,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let (inherited_label, inherited_tooltip) = guest
            .inheritance
            .as_ref()
            .map(|inheritance| {
                let InheritanceDetails {
                    source_label,
                    tooltip_text,
                } = inheritance.details(appearance, app);
                (source_label, tooltip_text)
            })
            .unzip();

        let mut access_level_button = appearance
            .ui_builder()
            .button(ButtonVariant::Text, guest.menu_button_handle.clone())
            .with_centered_text_label(guest.current_access_level.label().to_string())
            .with_style(UiComponentStyles {
                padding: Some(Coords::default()),
                ..Default::default()
            });
        if !self.can_edit_access(app) {
            access_level_button = access_level_button.disabled();
        }
        let access_level_label = SavePosition::new(
            access_level_button
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(SharingDialogAction::ToggleGuestMenu(index));
                })
                .finish(),
            &self.guest_access_button_id(index),
        )
        .finish();

        let row = Flex::row()
            .with_children([
                self.render_subject(&guest.subject, inherited_label, appearance, app),
                match inherited_tooltip {
                    Some(tooltip) => render_with_detail_tooltip(
                        tooltip,
                        guest.tooltip_handle.clone(),
                        access_level_label,
                        appearance,
                    ),
                    None => access_level_label,
                },
            ])
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish();

        Container::new(
            ConstrainedBox::new(row)
                // Each guest row must have the exact same height for UniformList to scroll correctly.
                .with_height(style::ACL_GUEST_HEIGHT)
                .finish(),
        )
        .with_horizontal_padding(style::ACL_ITEM_PADDING)
        .with_vertical_margin(style::ACL_ITEM_GAP / 2.)
        .finish()
    }

    /// Render all guest ACLs on the object. If there are no guests, returns `None` to not take up
    /// space.
    fn render_guests(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        if self.guest_states.is_empty() {
            return None;
        }

        let self_handle = self.self_handle.clone();
        let list = UniformList::new(
            self.ui_state_handles.guest_list_state.clone(),
            self.guest_states.len(),
            move |range, app| {
                let appearance = Appearance::as_ref(app);
                let me = self_handle
                    .upgrade(app)
                    .expect("Dialog handle must be upgradeable if rendered")
                    .as_ref(app);
                // Create a temporary Vec to avoid borrowing AppContext for too long.
                let guests = me
                    .guest_states
                    .iter()
                    .enumerate()
                    .skip(range.start)
                    .take(range.end - range.start)
                    .map(|(idx, guest)| me.render_guest(guest, idx, appearance, app))
                    .collect_vec();
                guests.into_iter()
            },
        );

        let scrollable_list = Scrollable::vertical(
            self.ui_state_handles.guest_scroll_state.clone(),
            list.finish_scrollable(),
            ScrollbarWidth::Auto,
            appearance.theme().nonactive_ui_detail().into(),
            appearance.theme().active_ui_detail().into(),
            Fill::None,
        )
        .with_overlayed_scrollbar()
        .finish();
        Some(
            ConstrainedBox::new(scrollable_list)
                .with_max_height(GUEST_LIST_MAX_HEIGHT)
                .finish(),
        )
    }

    /// Renders a single subject, with their name, avatar, and detail subtext.
    fn render_subject(
        &self,
        subject: &Subject,
        detail_override: Option<Box<dyn Element>>,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let avatar = Container::new(subject.avatar(appearance, app).build().finish())
            .with_padding_right(10.)
            .finish();

        let name_text = subject.name(app).unwrap_or(Cow::Borrowed("Unknown"));
        let name_label = appearance
            .ui_builder()
            .span(name_text)
            .with_style(UiComponentStyles {
                font_size: Some(style::PRIMARY_TEXT_SIZE),
                font_color: Some(style::acl_primary_text_color(appearance)),
                ..Default::default()
            })
            .build()
            .finish();

        let detail_element = detail_override.or_else(|| {
            subject
                .detail(app)
                .map(|detail| style::detail_text(detail, appearance).build().finish())
        });

        let info = match detail_element {
            Some(detail) => Flex::column()
                .with_children([name_label, detail])
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .finish(),
            None => name_label,
        };

        Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_children([avatar, info])
            .finish()
    }

    /// Renders a link to the shared object, with a CTA to copy the URL.
    fn render_object_link(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let url = match self.target.as_ref().and_then(|target| target.link(app)) {
            Some(url) => url,
            None => return Empty::new().finish(),
        };

        let link_text = appearance
            .ui_builder()
            .span(url)
            .with_style(UiComponentStyles {
                font_color: Some(style::acl_secondary_text_color(appearance)),
                ..Default::default()
            })
            .build()
            .finish();

        let link = Container::new(Align::new(link_text).left().finish())
            .with_border(
                Border::new(1.)
                    .with_sides(true, true, true, false)
                    .with_border_color(style::form_border_color(appearance)),
            )
            .with_corner_radius(CornerRadius::with_left(Radius::Pixels(4.)))
            .with_uniform_padding(8.)
            .finish();

        let copy_button_background = appearance.theme().surface_2();
        let copy_button_foreground = appearance.theme().main_text_color(copy_button_background);
        let copy_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Basic,
                self.ui_state_handles.copy_link_button.clone(),
            )
            .with_text_and_icon_label(
                TextAndIcon::new(
                    TextAndIconAlignment::IconFirst,
                    "Copy link",
                    Icon::Link.to_warpui_icon(copy_button_foreground),
                    MainAxisSize::Min,
                    MainAxisAlignment::SpaceBetween,
                    vec2f(12., 12.),
                )
                .with_inner_padding(4.),
            )
            .with_style(UiComponentStyles {
                font_color: Some(copy_button_foreground.into()),
                background: Some(copy_button_background.into()),
                border_color: Some(style::form_border_color(appearance).into()),
                border_radius: Some(CornerRadius::with_right(Radius::Pixels(4.))),
                ..Default::default()
            })
            .with_clicked_styles(UiComponentStyles {
                background: Some(appearance.theme().surface_2().into()),
                ..Default::default()
            })
            .with_hovered_styles(UiComponentStyles {
                background: Some(appearance.theme().surface_3().into()),
                ..Default::default()
            })
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(SharingDialogAction::CopyLink))
            .finish();

        let link_form = ConstrainedBox::new(
            Flex::row()
                .with_children([Shrinkable::new(1., link).finish(), copy_button])
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        )
        .with_min_height(style::ACL_ITEM_HEIGHT)
        .finish();

        Container::new(link_form)
            .with_horizontal_padding(style::ACL_ITEM_PADDING)
            .with_vertical_padding(style::ACL_ITEM_PADDING / 2.)
            .with_vertical_margin(style::ACL_ITEM_GAP / 2.)
            .finish()
    }

    fn render_session_qr_code(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        if !matches!(self.target.as_ref(), Some(ShareableObject::Session { .. })) {
            return None;
        }

        let url = self.target.as_ref().and_then(|target| target.link(app))?;
        let code = QrCode::new(url.as_bytes()).ok()?;
        let width = code.width();
        let cell_size = SESSION_QR_CODE_SIZE / width as f32;
        let mut rows = Flex::column();

        for y in 0..width {
            let mut row = Flex::row();
            for x in 0..width {
                let color = if code[(x, y)] == QrColor::Dark {
                    ColorU::black()
                } else {
                    ColorU::white()
                };

                row.add_child(
                    ConstrainedBox::new(Rect::new().with_background_color(color).finish())
                        .with_width(cell_size)
                        .with_height(cell_size)
                        .finish(),
                );
            }
            rows.add_child(row.finish());
        }

        let title = appearance
            .ui_builder()
            .span("Remote control QR code")
            .with_style(UiComponentStyles {
                font_color: Some(style::acl_primary_text_color(appearance)),
                font_size: Some(style::PRIMARY_TEXT_SIZE),
                ..Default::default()
            })
            .build()
            .finish();

        let subtitle = appearance
            .ui_builder()
            .span("Scan to open this session on another device.")
            .with_style(UiComponentStyles {
                font_color: Some(style::acl_secondary_text_color(appearance)),
                ..Default::default()
            })
            .build()
            .finish();

        let qr_code = Container::new(rows.finish())
            .with_uniform_padding(SESSION_QR_CODE_PADDING)
            .with_background_color(ColorU::white())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_border(Border::all(1.).with_border_color(style::form_border_color(appearance)))
            .finish();

        Some(
            Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(8.)
                    .with_child(title)
                    .with_child(qr_code)
                    .with_child(subtitle)
                    .finish(),
            )
            .with_horizontal_padding(style::ACL_ITEM_PADDING)
            .with_vertical_padding(style::ACL_ITEM_PADDING / 2.)
            .with_vertical_margin(style::ACL_ITEM_GAP / 2.)
            .finish(),
        )
    }
}

/// Render a tooltip that explains an ACL.
fn render_with_detail_tooltip(
    tooltip: impl Into<String>,
    mouse_state: MouseStateHandle,
    element: Box<dyn Element>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    appearance.ui_builder().tool_tip_on_element(
        tooltip.into(),
        mouse_state,
        element,
        ParentAnchor::TopLeft,
        ChildAnchor::TopRight,
        vec2f(-4., 0.),
    )
}

impl Entity for SharingDialog {
    type Event = SharingDialogEvent;
}

impl View for SharingDialog {
    fn ui_name() -> &'static str {
        "SharingDialog"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut contents = Flex::column();

        contents.extend(self.render_session_header(appearance, app));

        if self.can_edit_access(app) && self.can_direct_link_share(app) {
            contents.add_child(self.render_invite_form(appearance, app));
        }
        contents.add_child(self.render_access_header(appearance));
        contents.extend(self.render_restricted_access_label(appearance, app));

        if self.can_anyone_with_link_share(app) {
            contents.extend(self.render_link_sharing_subject(appearance, app));
        }
        contents.extend(self.render_team_sharing_subject(appearance, app));
        contents.extend(self.render_owner(appearance, app));

        if let Some(guest_list) = self.render_guests(appearance) {
            contents.add_child(guest_list);
        }

        contents.add_child(self.render_object_link(appearance, app));
        contents.extend(self.render_session_qr_code(appearance, app));

        let mut stack = Stack::new();
        stack.add_child(contents.finish());
        self.render_menu(&mut stack, app);

        let dialog = Container::new(stack.finish())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_border(Border::all(1.).with_border_color(theme.surface_3().into()))
            .with_vertical_padding(style::ACL_ITEM_PADDING)
            .with_background(style::dialog_background(appearance));

        Dismiss::new(
            ConstrainedBox::new(dialog.finish())
                .with_width(SHARING_DIALOG_WIDTH)
                .finish(),
        )
        .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(SharingDialogAction::Close))
        .prevent_interaction_with_other_elements()
        .finish()
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.invite_form.email_editor);
        }
    }
}

impl TypedActionView for SharingDialog {
    type Action = SharingDialogAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SharingDialogAction::Close => {
                self.reset_editable_state(ctx);
                ctx.emit(SharingDialogEvent::Close)
            }
            SharingDialogAction::SetLinkPermissions(access_level) => {
                self.set_open_menu(OpenMenuState::None, ctx);
                if let Some(ShareableObject::WarpDriveObject(id)) = self.target.as_ref() {
                    UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
                        update_manager.set_object_link_permissions(*id, *access_level, ctx);
                    });
                } else if let Some(ShareableObject::Session { handle, .. }) = self.target.as_ref() {
                    if let Some(view) = handle.upgrade(ctx) {
                        let role = access_level.map(|access_level| access_level.into());
                        view.update(ctx, |view, ctx| {
                            view.update_session_link_permissions(role, ctx)
                        });
                    }
                } else if let Some(ShareableObject::AIConversation(conversation_id)) =
                    self.target.as_ref()
                {
                    // Get the conversation's server_id from metadata
                    if let Some(server_id) = BlocklistAIHistoryModel::as_ref(ctx)
                        .get_server_conversation_metadata(conversation_id)
                        .map(|m| ServerId::from_string_lossy(m.metadata.uid.uid()))
                    {
                        UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
                            update_manager.set_ai_conversation_link_permissions(
                                server_id,
                                *conversation_id,
                                *access_level,
                                ctx,
                            );
                        });
                    } else {
                        log::warn!(
                            "AI conversation {:?} has no server_id for link permission update",
                            conversation_id
                        );
                    }
                }
                ctx.notify();
            }
            SharingDialogAction::SetTeamPermissions(access_level) => {
                self.set_open_menu(OpenMenuState::None, ctx);
                // So far, we only support setting team permissions for sessions.
                if let Some(ShareableObject::Session { handle, .. }) = self.target.as_ref() {
                    let Some(view) = handle.upgrade(ctx) else {
                        return;
                    };
                    let Some(team_uid) = UserWorkspaces::as_ref(ctx).current_team_uid() else {
                        return;
                    };

                    let role = access_level.map(|access_level| access_level.into());
                    view.update(ctx, |view, ctx| {
                        view.update_session_team_permissions(role, team_uid.uid(), ctx);
                    });
                }
                ctx.notify();
            }
            SharingDialogAction::CopyLink => self.copy_link(ctx),
            SharingDialogAction::ToggleLinkSharingMenu => {
                self.toggle_menu(OpenMenuState::LinkSharing, ctx);
            }
            SharingDialogAction::ToggleTeamSharingMenu => {
                self.toggle_menu(OpenMenuState::TeamSharing, ctx);
            }
            SharingDialogAction::ToggleInviteAccessLevelMenu => {
                self.toggle_menu(OpenMenuState::InviteAccessLevel, ctx);
            }
            SharingDialogAction::SetInviteAccessLevel(level) => {
                self.invite_form.selected_access_level = *level;
                ctx.notify();
            }
            SharingDialogAction::SendInvitations => self.send_invitations(ctx),
            SharingDialogAction::ToggleGuestMenu(index) => {
                self.toggle_menu(OpenMenuState::Guest(*index), ctx);
            }
            SharingDialogAction::SetGuestAccessLevel(level) => {
                self.set_targeted_guest_access(*level, ctx)
            }
            SharingDialogAction::RemoveGuest => self.remove_targeted_guest(ctx),
        }
    }
}
