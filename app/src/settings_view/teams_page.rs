use super::admin_actions::AdminActions;
use super::settings_page::{render_customer_type_badge, MatchData, PageType, SettingsWidget};
use super::transfer_ownership_confirmation_modal::{
    TransferOwnershipConfirmationEvent, TransferOwnershipConfirmationModal,
};
use super::SettingsSection;
use super::{
    settings_page::{
        render_separator, render_sub_header, SettingsPageMeta, SettingsPageViewHandle,
    },
    tab_menu::Tabs,
};

use crate::ai::AIRequestUsageModel;
use crate::auth::auth_manager::{AuthManager, LoginGatedFeature};
use crate::auth::auth_state::AuthState;
use crate::auth::auth_view_modal::AuthViewVariant;
use crate::auth::{AuthStateProvider, UserUid};
use crate::menu::{self, Menu, MenuItem, MenuItemFields};
use crate::modal::{Modal, ModalEvent, ModalViewState};
use crate::pricing::PricingInfoModel;
use crate::view_components::ToastFlavor;
use crate::workspaces::team::{MembershipRole, TeamDeleteDisabledReason};
use crate::{
    appearance::Appearance,
    channel::ChannelState,
    cloud_object::{model::persistence::CloudModel, CloudObjectEventEntrypoint, Space},
    drive::cloud_action_confirmation_dialog::{
        CloudActionConfirmationDialog, CloudActionConfirmationDialogEvent,
        CloudActionConfirmationDialogVariant,
    },
    editor::{
        EditorView, Event as EditorEvent, InteractionState, SingleLineEditorOptions, TextOptions,
    },
    network::NetworkStatus,
    send_telemetry_from_ctx,
    server::{
        cloud_objects::update_manager::UpdateManager, ids::ServerId, telemetry::TelemetryEvent,
    },
    themes::{self, theme::Blend},
    ui_components::icons::Icon,
    view_components::{ClickableTextInput, ClickableTextInputAction, ClickableTextInputEvent},
    word_block_editor::{ChipEditorState, WordBlockEditorView, WordBlockEditorViewEvent},
    workspace::WorkspaceAction,
    workspaces::{
        team::{DiscoverableTeam, Team},
        update_manager::{TeamUpdateManager, TeamUpdateManagerEvent},
        user_workspaces::{UserWorkspaces, UserWorkspacesEvent},
        workspace::{BillingMetadata, CustomerType, DelinquencyStatus, WorkspaceSizePolicy},
    },
};

use core::default::Default;
use email_address::EmailAddress;
use itertools::Itertools;
use lazy_static::lazy_static;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::{cmp::Ordering, collections::HashSet};
use warp_core::ui::theme::color::internal_colors;
use warpui::FocusContext;

use warpui::{
    clipboard::ClipboardContent,
    elements::{
        Align, Border, ChildAnchor, ClippedScrollStateHandle, ConstrainedBox, Container,
        CornerRadius, CrossAxisAlignment, Element, Flex, Hoverable, MainAxisAlignment,
        MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
        ParentOffsetBounds, Radius, SavePosition, ScrollTarget, ScrollToPositionMode, Shrinkable,
        Stack, Text,
    },
    fonts::{Properties, Weight},
    platform::Cursor,
    presenter::ChildView,
    ui_components::{
        button::{ButtonVariant, TextAndIcon, TextAndIconAlignment},
        components::{Coords, UiComponent, UiComponentStyles},
        switch::SwitchStateHandle,
        text_input::TextInput,
    },
    AppContext, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

const TEAM_MEMBERS_HEADER_POSITION_ID: &str = "team_settings:team_members_header";
// Styling for team create page
const TEAM_NAME_EDITOR_PLACEHOLDER_TEXT: &str = "Team name";
const CREATE_TEAM_BUTTON_LEFT_PADDING: f32 = 10.;
const CREATE_TEAM_DESCRIPTION: &str = "When you create a team, you can collaborate on agent-driven development by sharing cloud agent runs, environments, automations, and artifacts. You can also create a shared knowledge store for teammates and agents alike.";

// Styling for team management page
const LEAVE_TEAM_BUTTON_LABEL: &str = "Leave team";
const DELETE_TEAM_BUTTON_LABEL: &str = "Delete team";
const CREATE_TEAM_BUTTON_LABEL: &str = "Create";
const APPROVE_DOMAINS_PLACEHOLDER: &str = "Domains, comma separated";
const EMAILS_PLACEHOLDER: &str = "Emails, comma separated";
const APPROVE_DOMAINS_BUTTON_LABEL: &str = "Set";
const SEND_EMAIL_INVITES_BUTTON_LABEL: &str = "Invite";
const BUTTON_WIDTH: f32 = 82.;
const BUTTON_HEIGHT: f32 = 40.;
const COPY_LINK_LEFT_PADDING: f32 = 7.;
const LEAVE_TEAM_BUTTON_WIDTH: f32 = 115.;
const SCROLLABLE_LIST_ITEM_PADDING: f32 = 10.;
const CLOSE_BUTTON_ICON_SIZE: f32 = 20.;
const CONTENT_SEPARATION_PADDING: f32 = 24.;
const TEXT_FIELD_TOP_PADDING: f32 = 12.;
const HORIZONTAL_BAR_TO_SUB_HEADER_PADDING: f32 = 9.;
const SUBSECTION_HEADER_FONT_SIZE: f32 = 18.;
const SUBSUBSECTION_HEADER_FONT_SIZE: f32 = 14.;

const INVITE_LINK_PREFIX: &str = "/team/";
const INVALID_DOMAINS_INSTRUCTIONS: &str =
    "Some of the provided domains are invalid, or have already been added.";

const INVITE_LINK_TOGGLE_INSTRUCTIONS: &str = "As an admin, you can choose whether to enable or disable the ability for team members to invite others by invitation link.";
const INVITE_LINK_DOMAIN_RESTRICTIONS_INSTRUCTIONS: &str =
    "Restrict by domain — only allow users with emails at specific domains to join your team through the invite link.";

const INVITE_BY_EMAIL_EXPIRY_INSTRUCTIONS: &str = "Email invitations are valid for 7 days.";
const INVALID_EMAILS_INSTRUCTIONS: &str =
    "Some of the provided email addresses are invalid, already invited, or members of the team.";

const OFFLINE_TEXT: &str = "You are offline.";

const MAX_CHIP_WIDTH: f32 = 280.;

lazy_static! {
    static ref DOMAIN_NAME_REGEX: Regex =
        Regex::new(r"^([a-zA-Z0-9]+(-[a-zA-Z0-9]+)*\.)+[a-zA-Z0-9]{2,}$")
            .expect("regex should not fail to compile");
    static ref EMAIL_INVITE_PENDING_COLOR: ColorU = ColorU::new(243, 185, 17, 255);
    static ref PAST_DUE_BADGE_COLOR: ColorU = ColorU::new(254, 253, 194, 255);
    static ref UNPAID_BADGE_COLOR: ColorU = ColorU::new(255, 130, 114, 255);
    static ref DELINQUENCY_BADGE_TEXT_COLOR: ColorU = ColorU::new(0, 0, 0, 190);
}

#[derive(Debug, Clone)]
pub enum TeamsPageAction {
    LeaveTeam,
    ShowLeaveTeamConfirmationDialog,
    ShowDeleteTeamConfirmationDialog,
    CopyLink(String),
    CreateTeam,
    ChangeInviteViewOption(TeamsInviteOption),
    DeletePendingEmailInvitation {
        team_uid: ServerId,
        invitee_email: String,
    },
    RemoveUserFromTeam {
        user_uid: UserUid,
        team_uid: ServerId,
    },
    ToggleIsInviteLinkEnabled {
        team_uid: ServerId,
        current_state: bool,
    },
    ResetInviteLinks {
        team_uid: ServerId,
    },
    AddDomainRestrictions {
        team_uid: ServerId,
    },
    DeleteDomainRestriction {
        domain_uid: ServerId,
        team_uid: ServerId,
    },
    SendEmailInvites {
        team_uid: ServerId,
    },
    OpenWarpDrive,
    GenerateUpgradeLink {
        team_uid: ServerId,
    },
    GenerateStripeBillingPortalLink {
        team_uid: ServerId,
    },
    OpenAdminPanel {
        team_uid: ServerId,
    },
    ContactSupport,
    ContactSales,
    /// This action is for toggling the discoverability checkbox before a team is created.
    ToggleTeamDiscoverabilityBeforeCreation,
    /// This action is for toggling the discoverability toggle after a team has been created.
    ToggleTeamDiscoverability {
        team_uid: ServerId,
        current_state: bool,
    },
    JoinTeamWithTeamDiscovery {
        team_uid: ServerId,
    },
    ShowTransferOwnershipModal {
        new_owner_email: String,
        new_owner_uid: UserUid,
        team_uid: ServerId,
    },
    OpenMemberActionsMenu {
        index: usize,
    },
    CloseMemberActionsMenu,
    SetTeamMemberRole {
        team_uid: ServerId,
        user_uid: UserUid,
        role: MembershipRole,
    },
}

impl TeamsPageAction {
    pub fn blocked_for_anonymous_user(&self) -> bool {
        use TeamsPageAction::*;
        matches!(
            self,
            LeaveTeam
                | ShowLeaveTeamConfirmationDialog
                | ShowDeleteTeamConfirmationDialog
                | CreateTeam
                | DeletePendingEmailInvitation { .. }
                | RemoveUserFromTeam { .. }
                | AddDomainRestrictions { .. }
                | DeleteDomainRestriction { .. }
                | SendEmailInvites { .. }
                | GenerateUpgradeLink { .. }
                | GenerateStripeBillingPortalLink { .. }
                | OpenAdminPanel { .. }
                | ContactSupport
                | ContactSales
                | ToggleTeamDiscoverabilityBeforeCreation
                | ToggleTeamDiscoverability { .. }
                | JoinTeamWithTeamDiscovery { .. }
        )
    }
}

impl From<&TeamsPageAction> for LoginGatedFeature {
    fn from(val: &TeamsPageAction) -> LoginGatedFeature {
        use TeamsPageAction::*;
        match val {
            LeaveTeam => "Leave Team",
            ShowDeleteTeamConfirmationDialog => "Delete Team",
            CreateTeam => "Create Team",
            DeletePendingEmailInvitation { .. } => "Delete Pending Email Invitation",
            RemoveUserFromTeam { .. } => "Remove User From Team",
            AddDomainRestrictions { .. } => "Add Domain Restrictions",
            DeleteDomainRestriction { .. } => "Delete Domain Restriction",
            SendEmailInvites { .. } => "Send Email Invites",
            GenerateUpgradeLink { .. } => "Generate Upgrade Link",
            GenerateStripeBillingPortalLink { .. } => "Generate Stripe Billing Portal Link",
            OpenAdminPanel { .. } => "Open Admin Panel",
            ContactSupport => "Contact Support",
            ContactSales => "Contact Sales",
            ToggleTeamDiscoverability { .. } | ToggleTeamDiscoverabilityBeforeCreation => {
                "Toggle Team Discoverability"
            }
            JoinTeamWithTeamDiscovery { .. } => "Join Team With Team Discovery",
            _ => "Unknown reason",
        }
    }
}

impl TryFrom<&TeamsPageAction> for TelemetryEvent {
    type Error = anyhow::Error;
    fn try_from(action: &TeamsPageAction) -> Result<Self, Self::Error> {
        match action {
            TeamsPageAction::CopyLink(_) => Ok(TelemetryEvent::TeamLinkCopied),
            TeamsPageAction::ChangeInviteViewOption(option) => {
                Ok(TelemetryEvent::ChangedInviteViewOption(*option))
            }
            TeamsPageAction::SendEmailInvites { .. } => Ok(TelemetryEvent::SendEmailInvites),
            // Some Team events are logged from the server so we do not want to log
            // them from the client as well. For more details see:
            // https://docs.google.com/document/d/1va3_qfkHtDFKZqYaMgNUn5nwU4f8NByzyhg1uolHlck/edit
            _ => Err(anyhow::anyhow!(
                "We do not log this telemetry event from the client."
            )),
        }
    }
}

#[derive(Clone)]
pub enum TeamsPageViewEvent {
    TeamsChanged,
    OpenWarpDrive,
    ShowToast {
        message: String,
        flavor: ToastFlavor,
    },
}

#[derive(Default)]
struct TeamsWidgetMouseHandles {
    copy_link_button: MouseStateHandle,
    send_email_invites_button: MouseStateHandle,
    leave_team_button: MouseStateHandle,
    create_team_button: MouseStateHandle,
    approve_domains_button: MouseStateHandle,
    reset_invite_links_button: MouseStateHandle,
    invite_by_link_toggle_state: SwitchStateHandle,
    upgrade_link: MouseStateHandle,
    stripe_billing_portal_link: MouseStateHandle,
    manage_plan_link: MouseStateHandle,
    enterprise_contact_us_link: MouseStateHandle,
    discoverable_team_toggle_state: SwitchStateHandle,
    checkbox_mouse_state: MouseStateHandle,
    admin_panel_button: MouseStateHandle,
    grow_team_warning_cta_button: MouseStateHandle,
    team_members_count_tooltip: MouseStateHandle,
    outgrow_upgrade_link: MouseStateHandle,
    outgrow_contact_sales_link: MouseStateHandle,
}

/// TeamsInviteOption is whether the user is looking at invite-by-link or invite-by-email.
#[derive(Clone, PartialEq, Eq, Debug, Default, Copy, Serialize, Deserialize)]
pub enum TeamsInviteOption {
    #[default]
    Link,
    Email,
}

impl std::fmt::Display for TeamsInviteOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                TeamsInviteOption::Link => "Link",
                TeamsInviteOption::Email => "Email",
            },
        )
    }
}

impl Tabs for TeamsInviteOption {
    fn action_on_click(&self, selection: TeamsInviteOption) -> TeamsPageAction {
        TeamsPageAction::ChangeInviteViewOption(selection)
    }

    fn label(&self, _team: &Team, _cloud_model: &CloudModel) -> String {
        self.tab_name()
    }
}

/// What's blocking the team from growing right now. Resolved by
/// `grow_team_warning`; consumed by `render_grow_team_warning_alert` and
/// `grow_team_warning_cta`. Priority order is delinquency > over-cap > at-cap.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum GrowTeamWarning {
    /// Team size equals the workspace size policy limit.
    SeatCapReached,
    /// Team size exceeds the workspace size policy limit.
    SeatCapExceeded,
    /// Subscription has a past-due payment.
    PaymentPastDue,
    /// Subscription is unpaid.
    PaymentUnpaid,
}

/// The action an admin can take to resolve a `GrowTeamWarning`. `None`
/// indicates no actionable path (non-admin viewer, enterprise with no
/// self-serve option, or no higher-cap plan available).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum GrowTeamWarningCta {
    /// Self-serve upgrade is available; route to `/upgrade`.
    Upgrade,
    /// Team is on the highest self-serve plan; needs sales for more capacity.
    ContactSales,
    /// Self-serve admin can resolve billing via the Stripe portal.
    UpdateBilling,
    /// Non-self-serve admin (e.g. enterprise) should reach out to support.
    ContactSupport,
    /// No actionable CTA from this viewer in this state.
    None,
}

/// The order of the ItemState enum values determines the ordering of the members and
/// invites list in the team management page (see `impl Ord for Item`` below).
#[derive(Clone, PartialOrd, PartialEq, Eq, Ord)]
enum ItemState {
    Expired,
    Pending,
    Owner,
    Admin,
    Valid,
}

#[derive(Clone)]
struct ItemAction {
    icon: Icon,
    label: String,
    action: TeamsPageAction,
}

/// An item (team member, pending email invite, or domain) consists of its text, and actions associated with it.
#[derive(Clone)]
pub struct Item {
    text: String,
    actions: Vec<ItemAction>,
    state: ItemState,
}

impl PartialEq for Item {
    fn eq(&self, other: &Self) -> bool {
        (&self.state, &self.text) == (&other.state, &other.text)
    }
}

impl Eq for Item {}

impl PartialOrd for Item {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Item {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.state == other.state {
            self.text.cmp(&other.text)
        } else {
            self.state.cmp(&other.state)
        }
    }
}

#[derive(Clone)]
struct DiscoverableTeamState {
    team: DiscoverableTeam,
    mouse_state_handle: MouseStateHandle,
}

impl DiscoverableTeamState {
    pub fn new(team: DiscoverableTeam) -> Self {
        Self {
            team,
            mouse_state_handle: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct OpenTeamsSettingsModalArgs {
    pub invite_email: Option<String>,
}

pub struct TeamsPageView {
    page: PageType<Self>,
    auth_state: Arc<AuthState>,
    create_team_editor: ViewHandle<EditorView>,
    approve_domains_block_editor: ViewHandle<WordBlockEditorView>,
    approve_domains_block_editor_state: ChipEditorState,
    email_invites_block_editor: ViewHandle<WordBlockEditorView>,
    email_invites_block_editor_state: ChipEditorState,
    // Note that rather than storing just the current workspace, we're storing the entire
    // ModelHandle<UserWorkspaces>. That's because eventually we'll be handling more than one workspace.
    user_workspaces: ModelHandle<UserWorkspaces>,
    ai_request_usage_model: ModelHandle<AIRequestUsageModel>,
    pricing_info_model: ModelHandle<PricingInfoModel>,
    cloud_model: ModelHandle<CloudModel>,
    invite_view: TeamsInviteOption,
    team_members_mouse_state_handles: Vec<MouseStateHandle>,
    team_approved_domains_mouse_state_handles: Vec<MouseStateHandle>,
    delete_or_leave_team_confirmation_dialog: ViewHandle<CloudActionConfirmationDialog>,
    show_delete_or_leave_team_confirmation_dialog: bool,
    transfer_ownership_modal_state: ModalViewState<Modal<TransferOwnershipConfirmationModal>>,
    clipped_scroll_state: ClippedScrollStateHandle,
    discoverable_teams_states: Vec<DiscoverableTeamState>,
    rename_team_editor: ViewHandle<ClickableTextInput>,
    checkbox_value: bool,
    member_actions_menu: ViewHandle<Menu<TeamsPageAction>>,
    open_member_actions_menu_index: Option<usize>,
}

impl Entity for TeamsPageView {
    type Event = TeamsPageViewEvent;
}

impl TypedActionView for TeamsPageView {
    type Action = TeamsPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        // Block anonymous users from performing team actions
        if AuthStateProvider::as_ref(ctx)
            .get()
            .is_anonymous_or_logged_out()
            && action.blocked_for_anonymous_user()
        {
            AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                auth_manager.attempt_login_gated_feature(
                    action.into(),
                    AuthViewVariant::RequireLoginCloseable,
                    ctx,
                )
            });
            return;
        }

        match action {
            TeamsPageAction::CopyLink(link) => self.copy_invite_link(link, ctx),
            TeamsPageAction::LeaveTeam => self.leave_team(ctx),
            TeamsPageAction::CreateTeam => self.create_team(ctx),
            TeamsPageAction::RemoveUserFromTeam { user_uid, team_uid } => {
                self.remove_user_from_team(*user_uid, *team_uid, ctx)
            }
            TeamsPageAction::ChangeInviteViewOption(view_option) => {
                self.change_invite_view_option(view_option, ctx);
            }
            TeamsPageAction::SendEmailInvites { team_uid } => {
                self.send_email_invites(*team_uid, ctx);
                ctx.notify();
            }
            TeamsPageAction::OpenWarpDrive => ctx.emit(TeamsPageViewEvent::OpenWarpDrive),
            TeamsPageAction::ShowLeaveTeamConfirmationDialog => {
                self.delete_or_leave_team_confirmation_dialog
                    .update(ctx, |dialog, ctx| {
                        dialog.set_variant(CloudActionConfirmationDialogVariant::LeaveTeam);
                        ctx.notify();
                    });
                self.show_delete_or_leave_team_confirmation_dialog = true;
                self.enable_confirmation_dialog_confirm_button(ctx);
            }
            TeamsPageAction::ShowDeleteTeamConfirmationDialog => {
                self.delete_or_leave_team_confirmation_dialog
                    .update(ctx, |dialog, ctx| {
                        dialog.set_variant(CloudActionConfirmationDialogVariant::DeleteTeam);
                        ctx.notify();
                    });
                self.show_delete_or_leave_team_confirmation_dialog = true;
                self.enable_confirmation_dialog_confirm_button(ctx);
            }
            TeamsPageAction::ToggleIsInviteLinkEnabled {
                team_uid,
                current_state,
            } => {
                let new_value = !current_state;
                self.set_is_invite_link_enabled(*team_uid, new_value, ctx);
                ctx.notify();
            }
            TeamsPageAction::ResetInviteLinks { team_uid } => {
                self.reset_invite_links(*team_uid, ctx);
                ctx.notify();
            }
            TeamsPageAction::DeletePendingEmailInvitation {
                team_uid,
                invitee_email,
            } => {
                self.delete_team_invite(*team_uid, invitee_email.clone(), ctx);
                ctx.notify();
            }
            TeamsPageAction::AddDomainRestrictions { team_uid } => {
                self.add_domain_restrictions(*team_uid, ctx)
            }
            TeamsPageAction::DeleteDomainRestriction {
                domain_uid,
                team_uid,
            } => self.delete_domain_restriction(*team_uid, *domain_uid, ctx),
            TeamsPageAction::GenerateUpgradeLink { team_uid } => {
                self.generate_upgrade_link(*team_uid, ctx)
            }
            TeamsPageAction::GenerateStripeBillingPortalLink { team_uid } => {
                self.generate_stripe_billing_portal_link(*team_uid, ctx)
            }
            TeamsPageAction::OpenAdminPanel { team_uid } => {
                AdminActions::open_admin_panel(*team_uid, ctx);
            }
            TeamsPageAction::ContactSupport => {
                AdminActions::contact_support(ctx);
            }
            TeamsPageAction::ContactSales => {
                AdminActions::contact_sales(ctx);
            }
            TeamsPageAction::ToggleTeamDiscoverability {
                team_uid,
                current_state,
            } => {
                self.set_team_discoverability(*team_uid, !current_state, ctx);
                ctx.notify();
            }
            TeamsPageAction::JoinTeamWithTeamDiscovery { team_uid } => {
                self.join_team_with_team_discovery(*team_uid, ctx);
                ctx.notify();
            }
            TeamsPageAction::ShowTransferOwnershipModal {
                new_owner_email,
                new_owner_uid,
                team_uid,
            } => {
                self.show_transfer_ownership_modal(
                    new_owner_email.clone(),
                    *new_owner_uid,
                    *team_uid,
                    ctx,
                );
            }
            TeamsPageAction::ToggleTeamDiscoverabilityBeforeCreation => {
                self.checkbox_value = !self.checkbox_value;
            }
            TeamsPageAction::OpenMemberActionsMenu { index } => {
                self.open_member_actions_menu_for_item(*index, ctx);
            }
            TeamsPageAction::CloseMemberActionsMenu => {
                self.open_member_actions_menu_index = None;
                ctx.notify();
            }
            TeamsPageAction::SetTeamMemberRole {
                team_uid,
                user_uid,
                role,
            } => {
                self.set_team_member_role(*user_uid, *team_uid, *role, ctx);
            }
        };

        if let Ok(event) = TelemetryEvent::try_from(action) {
            send_telemetry_from_ctx!(event, ctx);
        }
    }
}

impl View for TeamsPageView {
    fn ui_name() -> &'static str {
        "TeamsPageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() && !self.user_workspaces.as_ref(ctx).has_teams() {
            ctx.focus(&self.create_team_editor);
            ctx.notify();
        }
    }
}

impl TeamsPageView {
    fn editor<F>(
        mut event_handler: F,
        placeholder: &str,
        ui_font_size: f32,
        ctx: &mut ViewContext<TeamsPageView>,
    ) -> ViewHandle<EditorView>
    where
        F: 'static + FnMut(&mut TeamsPageView, &EditorEvent, &mut ViewContext<TeamsPageView>),
    {
        let editor = ctx.add_typed_action_view(|ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_size_override: Some(ui_font_size),
                    ..Default::default()
                },
                ..Default::default()
            };
            EditorView::single_line(options, ctx)
        });

        editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
            editor.set_placeholder_text(placeholder, ctx);
        });

        ctx.subscribe_to_view(&editor, move |me, _, event, ctx| {
            event_handler(me, event, ctx);
        });

        editor
    }

    pub fn new(ctx: &mut ViewContext<TeamsPageView>) -> Self {
        let user_workspaces = UserWorkspaces::handle(ctx);
        ctx.observe(&user_workspaces, |me, _, ctx| {
            me.update_team_members_state(ctx);
            me.update_approved_domains_state(ctx);
        });
        ctx.subscribe_to_model(&user_workspaces, |me, _handle, event, ctx| {
            me.handle_model_event(event, ctx);
            ctx.notify();
        });

        let team_update_manager = TeamUpdateManager::handle(ctx);
        ctx.subscribe_to_model(&team_update_manager, |me, _handle, event, ctx| {
            me.handle_team_update_event(event, ctx);
            ctx.notify();
        });

        let cloud_model = CloudModel::handle(ctx);
        ctx.observe(&cloud_model, |me, _, ctx| {
            me.update_team_members_state(ctx);
            me.update_approved_domains_state(ctx);
        });

        let pricing_info_model = PricingInfoModel::handle(ctx);

        let appearance = Appearance::as_ref(ctx);
        let font_size = appearance.ui_font_size();
        let create_team_editor = Self::editor(
            |me, event, ctx| me.handle_editor_event(event, ctx),
            TEAM_NAME_EDITOR_PLACEHOLDER_TEXT,
            font_size,
            ctx,
        );

        let approve_domains_block_editor = ctx.add_typed_action_view(|ctx| {
            WordBlockEditorView::new(
                ctx,
                APPROVE_DOMAINS_PLACEHOLDER,
                font_size,
                vec![',', ' '],
                MAX_CHIP_WIDTH,
                Box::new(Self::is_valid_domain),
            )
        });
        ctx.subscribe_to_view(&approve_domains_block_editor, |me, _, event, ctx| {
            me.handle_approve_domains_block_editor_event(event, ctx);
        });

        let email_invites_block_editor = ctx.add_typed_action_view(|ctx| {
            WordBlockEditorView::new(
                ctx,
                EMAILS_PLACEHOLDER,
                font_size,
                vec![',', ' '],
                MAX_CHIP_WIDTH,
                Box::new(EmailAddress::is_valid),
            )
        });
        ctx.subscribe_to_view(&email_invites_block_editor, |me, _, event, ctx| {
            me.handle_email_invites_block_editor_event(event, ctx);
        });

        let current_user_team = user_workspaces.as_ref(ctx).current_team();

        let team_members_mouse_state_handles =
            current_user_team.map_or_else(Vec::new, |user_team| {
                user_team
                    .members
                    .iter()
                    .map(|_| Default::default())
                    .collect()
            });

        let team_approved_domains_mouse_state_handles =
            current_user_team.map_or_else(Vec::new, |user_team| {
                user_team
                    .invite_link_domain_restrictions
                    .iter()
                    .map(|_| Default::default())
                    .collect()
            });

        let team_name = current_user_team
            .map_or_else(|| "", |team| &team.name)
            .to_string();
        let rename_team_editor = ctx.add_typed_action_view(|ctx| {
            let mut input = ClickableTextInput::new(team_name, ctx);
            input.set_placeholder_text("Your new team name", ctx);
            input
        });
        ctx.subscribe_to_view(&rename_team_editor, |me, _, event, ctx| {
            me.handle_rename_team_editor_event(event, ctx);
        });

        ctx.subscribe_to_model(&NetworkStatus::handle(ctx), move |_, _, _, ctx| {
            ctx.notify()
        });

        let delete_or_leave_team_confirmation_dialog =
            ctx.add_typed_action_view(|_| CloudActionConfirmationDialog::new());
        ctx.subscribe_to_view(
            &delete_or_leave_team_confirmation_dialog,
            |me, _, event, ctx| {
                me.handle_cloud_action_confirmation_dialog_event(event, ctx);
            },
        );

        let transfer_ownership_modal_body =
            ctx.add_typed_action_view(|_| TransferOwnershipConfirmationModal::new());
        ctx.subscribe_to_view(&transfer_ownership_modal_body, |me, _, event, ctx| {
            me.handle_transfer_ownership_modal_event(event, ctx);
        });
        let transfer_ownership_modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(
                Some("Transfer team ownership?".to_string()),
                transfer_ownership_modal_body,
                ctx,
            )
            .with_modal_style(UiComponentStyles {
                height: Some(220.),
                ..Default::default()
            })
            .with_header_style(UiComponentStyles {
                padding: Some(Coords::uniform(24.).bottom(16.)),
                ..Default::default()
            })
            .with_body_style(UiComponentStyles {
                padding: Some(Coords::uniform(24.).top(0.).bottom(12.)),
                height: Some(150.),
                ..Default::default()
            })
        });
        ctx.subscribe_to_view(&transfer_ownership_modal, |me, _, event, ctx| {
            me.handle_transfer_ownership_modal_close_event(event, ctx);
        });

        let member_actions_menu = ctx.add_typed_action_view(|_| Menu::new().with_drop_shadow());
        ctx.subscribe_to_view(&member_actions_menu, |me, _, event, ctx| {
            if let menu::Event::Close { .. } = event {
                me.open_member_actions_menu_index = None;
                ctx.notify();
            }
        });

        let page = PageType::new_monolith(TeamsWidget::default(), None, true);
        TeamsPageView {
            page,
            auth_state: AuthStateProvider::as_ref(ctx).get().clone(),
            create_team_editor,
            approve_domains_block_editor,
            approve_domains_block_editor_state: ChipEditorState {
                is_valid: false,
                is_empty: true,
                num_chips: 0,
            },
            email_invites_block_editor,
            email_invites_block_editor_state: ChipEditorState {
                is_valid: false,
                is_empty: true,
                num_chips: 0,
            },
            user_workspaces,
            ai_request_usage_model: AIRequestUsageModel::handle(ctx),
            pricing_info_model,
            cloud_model,
            invite_view: TeamsInviteOption::default(),
            team_members_mouse_state_handles,
            team_approved_domains_mouse_state_handles,
            clipped_scroll_state: Default::default(),
            delete_or_leave_team_confirmation_dialog,
            show_delete_or_leave_team_confirmation_dialog: false,
            transfer_ownership_modal_state: ModalViewState::new(transfer_ownership_modal),
            discoverable_teams_states: Vec::new(),
            rename_team_editor,
            checkbox_value: true,
            member_actions_menu,
            open_member_actions_menu_index: None,
        }
    }

    fn open_member_actions_menu_for_item(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        let Some(team) = self.user_workspaces.as_ref(ctx).current_team() else {
            return;
        };
        let Some(current_user_email) = self.auth_state.user_email() else {
            return;
        };
        let items = self.team_to_item_list(team, &current_user_email);
        let items_sorted = items.iter().sorted().collect_vec();

        let Some(item) = items_sorted.get(index) else {
            return;
        };

        if item.actions.is_empty() {
            return;
        }

        let menu_items: Vec<MenuItem<TeamsPageAction>> = item
            .actions
            .iter()
            .map(|item_action| {
                MenuItem::Item(
                    MenuItemFields::new(item_action.label.clone())
                        .with_on_select_action(item_action.action.clone())
                        .with_icon(item_action.icon),
                )
            })
            .collect();

        self.member_actions_menu.update(ctx, |menu, ctx| {
            menu.set_items(menu_items, ctx);
        });
        self.open_member_actions_menu_index = Some(index);
        ctx.notify();
    }

    fn handle_model_event(
        &mut self,
        event: &UserWorkspacesEvent,
        ctx: &mut ViewContext<TeamsPageView>,
    ) {
        match event {
            UserWorkspacesEvent::EmailInviteSent => {
                self.email_invites_block_editor.update(ctx, |editor, ctx| {
                    editor.clear_list_of_words(ctx);
                });
                self.update_team_members_state(ctx);
            }
            UserWorkspacesEvent::EmailInviteRejected(err) => {
                self.update_team_members_state(ctx);
                self.show_error("Failed to send invite", Some(err), ctx)
            }
            UserWorkspacesEvent::TeamsChanged => {
                self.update_team_members_state(ctx);
                self.update_approved_domains_state(ctx);

                AIRequestUsageModel::handle(ctx).update(ctx, |usage_model, ctx| {
                    usage_model.refresh_request_usage_async(ctx);
                });

                ctx.emit(TeamsPageViewEvent::TeamsChanged);
            }
            UserWorkspacesEvent::ToggleInviteLinksSuccess => {
                self.show_success("Toggled invite links", ctx);
                ctx.notify();
            }
            UserWorkspacesEvent::ToggleInviteLinksRejected(err) => {
                self.show_error("Failed to toggle invite links", Some(err), ctx);
            }
            UserWorkspacesEvent::ResetInviteLinks => {
                self.show_success("Reset invite links", ctx);
                ctx.notify();
            }
            UserWorkspacesEvent::ResetInviteLinksRejected(err) => {
                self.show_error("Failed to reset invite links", Some(err), ctx);
            }
            UserWorkspacesEvent::DeleteTeamInvite => {
                self.update_team_members_state(ctx);
                self.show_success("Deleted invite", ctx);
            }
            UserWorkspacesEvent::DeleteTeamInviteRejected(err) => {
                self.show_error("Failed to delete invite", Some(err), ctx);
            }
            UserWorkspacesEvent::AddDomainRestrictionsSuccess => {
                self.approve_domains_block_editor
                    .update(ctx, |editor, ctx| {
                        editor.clear_list_of_words(ctx);
                    });
                self.update_approved_domains_state(ctx);
            }
            UserWorkspacesEvent::AddDomainRestrictionsRejected(err) => {
                self.show_error("Failed to add domain restriction", Some(err), ctx)
            }
            UserWorkspacesEvent::DeleteDomainRestrictionSuccess => {
                self.update_approved_domains_state(ctx);
            }
            UserWorkspacesEvent::DeleteDomainRestrictionRejected(err) => {
                self.show_error("Failed to delete domain restriction", Some(err), ctx)
            }
            UserWorkspacesEvent::GenerateUpgradeLink(upgrade_link) => {
                ctx.open_url(upgrade_link);
            }
            UserWorkspacesEvent::GenerateUpgradeLinkRejected(err) => self.show_error(
                "Failed to generate upgrade link. Please contact us at feedback@warp.dev",
                Some(err),
                ctx,
            ),
            UserWorkspacesEvent::GenerateStripeBillingPortalLink(billing_session_link) => {
                ctx.open_url(billing_session_link);
            }
            UserWorkspacesEvent::GenerateStripeBillingPortalLinkRejected(err) => self.show_error(
                "Failed to generate billing link. Please contact us at feedback@warp.dev",
                Some(err),
                ctx,
            ),
            UserWorkspacesEvent::ToggleTeamDiscoverabilitySuccess => {
                self.show_success("Toggled team discoverability", ctx);
                ctx.notify();
            }
            UserWorkspacesEvent::ToggleTeamDiscoverabilityRejected(err) => {
                self.show_error("Failed to toggle team discoverability", Some(err), ctx);
            }
            UserWorkspacesEvent::JoinTeamWithTeamDiscoverySuccess => {
                // Force refresh of Warp Drive objects after joining a team
                UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
                    update_manager.refresh_updated_objects(ctx);
                });

                let message = self
                    .user_workspaces
                    .as_ref(ctx)
                    .current_team()
                    .map_or("Successfully joined team".to_string(), |team| {
                        format!("Successfully joined {}", team.name)
                    });
                self.show_success(message, ctx);
                ctx.notify();
            }
            UserWorkspacesEvent::JoinTeamWithTeamDiscoveryRejected(err) => {
                self.show_error("Failed to join team", Some(err), ctx);
            }
            UserWorkspacesEvent::FetchDiscoverableTeamsSuccess(teams) => {
                self.discoverable_teams_states = teams
                    .iter()
                    .map(|team| DiscoverableTeamState::new(team.clone()))
                    .collect();
                ctx.notify();
            }
            UserWorkspacesEvent::FetchDiscoverableTeamsRejected(e) => {
                // Don't show toast, only log to sentry
                log::error!("Failed to fetch discoverable teams: {e:?}");
            }
            UserWorkspacesEvent::TransferTeamOwnershipSuccess => {
                self.show_success("Successfully transferred team ownership", ctx);
                ctx.notify();
            }
            UserWorkspacesEvent::TransferTeamOwnershipRejected(err) => {
                self.show_error("Failed to transfer team ownership", Some(err), ctx);
            }
            UserWorkspacesEvent::SetTeamMemberRoleSuccess => {
                self.update_team_members_state(ctx);
                self.show_success("Successfully updated team member role", ctx);
            }
            UserWorkspacesEvent::SetTeamMemberRoleRejected(err) => {
                self.show_error("Failed to update team member role", Some(err), ctx);
            }
            UserWorkspacesEvent::UpdateWorkspaceSettingsSuccess => {
                // as of right now, this is only emitted on the billing & usage page
            }
            UserWorkspacesEvent::UpdateWorkspaceSettingsRejected(_) => {
                // as of right now, this is only emitted on the billing & usage page
            }
            UserWorkspacesEvent::AiOveragesUpdated => {
                // AI overages update doesn't affect teams page display
            }
            UserWorkspacesEvent::PurchaseAddonCreditsSuccess => {
                // Addon credits purchase success is handled in billing_and_usage_page
            }
            UserWorkspacesEvent::PurchaseAddonCreditsRejected(_) => {
                // Addon credits purchase rejection is handled in billing_and_usage_page
            }
            UserWorkspacesEvent::CodebaseContextEnablementChanged => {}
            UserWorkspacesEvent::SunsettedToBuildDataUpdated => {
                // Build plan migration modal is handled by OneTimeModalModel
            }
        }
    }

    /// Scroll to the team membership settings. If an email is provided, it's prepopulated in the
    /// invite editor.
    pub fn open_team_members(&mut self, email: Option<&String>, ctx: &mut ViewContext<Self>) {
        if let Some(email) = email {
            self.email_invites_block_editor.update(
                ctx,
                |invite_editor: &mut WordBlockEditorView, ctx| {
                    invite_editor.clear_list_of_words(ctx);
                    invite_editor.add_word(email, ctx);
                    ctx.focus(&self.email_invites_block_editor);
                },
            );
        }
        self.clipped_scroll_state.scroll_to_position(ScrollTarget {
            position_id: TEAM_MEMBERS_HEADER_POSITION_ID.to_string(),
            mode: ScrollToPositionMode::FullyIntoView,
        });
        ctx.notify();
    }

    fn handle_team_update_event(
        &mut self,
        event: &TeamUpdateManagerEvent,
        ctx: &mut ViewContext<TeamsPageView>,
    ) {
        match event {
            TeamUpdateManagerEvent::LeaveError => {
                let error = "Error leaving team".to_string();
                self.show_error(error, None, ctx);
            }
            TeamUpdateManagerEvent::LeaveSuccess => {
                self.show_success("Successfully left team", ctx);
                ctx.notify();
            }
            TeamUpdateManagerEvent::RenameTeamSuccess => {
                self.show_success("Successfully renamed team", ctx)
            }
            TeamUpdateManagerEvent::RenameTeamError => {
                self.show_error("Failed to rename team", None, ctx)
            }
        }
    }

    fn handle_cloud_action_confirmation_dialog_event(
        &mut self,
        event: &CloudActionConfirmationDialogEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CloudActionConfirmationDialogEvent::Cancel => {
                self.show_delete_or_leave_team_confirmation_dialog = false;
                ctx.notify();
            }
            CloudActionConfirmationDialogEvent::Confirm => {
                self.leave_team(ctx);
                self.show_delete_or_leave_team_confirmation_dialog = false;
            }
        }
    }

    fn handle_transfer_ownership_modal_event(
        &mut self,
        event: &TransferOwnershipConfirmationEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TransferOwnershipConfirmationEvent::Confirm {
                new_owner_uid,
                team_uid,
            } => {
                self.set_team_member_role(*new_owner_uid, *team_uid, MembershipRole::Owner, ctx);
                self.transfer_ownership_modal_state.close();
                ctx.notify();
            }
            TransferOwnershipConfirmationEvent::Cancel => {
                self.transfer_ownership_modal_state.close();
                ctx.notify();
            }
        }
    }

    fn handle_transfer_ownership_modal_close_event(
        &mut self,
        event: &ModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ModalEvent::Close => {
                self.transfer_ownership_modal_state.close();
                ctx.notify();
            }
        }
    }

    fn show_transfer_ownership_modal(
        &mut self,
        new_owner_email: String,
        new_owner_uid: UserUid,
        team_uid: ServerId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.transfer_ownership_modal_state
            .view
            .update(ctx, |modal, ctx| {
                modal.body().update(ctx, |body, ctx| {
                    body.set_new_owner(new_owner_email, new_owner_uid, team_uid);
                    ctx.notify();
                });
            });
        self.transfer_ownership_modal_state.open();
        ctx.notify();
    }

    fn handle_approve_domains_block_editor_event(
        &mut self,
        event: &WordBlockEditorViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            WordBlockEditorViewEvent::WordListValidityChanged => {
                let editor = self.approve_domains_block_editor.as_ref(ctx);
                self.approve_domains_block_editor_state.is_empty =
                    editor.get_list_of_words(ctx).is_empty();
                self.approve_domains_block_editor_state.is_valid =
                    editor.get_list_of_invalid_words(ctx).is_empty()
                        && !self.approve_domains_block_editor_state.is_empty;
                self.approve_domains_block_editor_state.num_chips = editor.num_chips();
                ctx.notify();
            }
            WordBlockEditorViewEvent::Enter | WordBlockEditorViewEvent::Navigate(_) => (),
            WordBlockEditorViewEvent::Escape => ctx.focus_self(),
        }
    }

    fn handle_email_invites_block_editor_event(
        &mut self,
        event: &WordBlockEditorViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            WordBlockEditorViewEvent::WordListValidityChanged => {
                let editor = self.email_invites_block_editor.as_ref(ctx);
                self.email_invites_block_editor_state.is_empty =
                    editor.get_list_of_words(ctx).is_empty();
                self.email_invites_block_editor_state.is_valid =
                    editor.get_list_of_invalid_words(ctx).is_empty()
                        && !self.email_invites_block_editor_state.is_empty;
                self.email_invites_block_editor_state.num_chips = editor.num_chips();
                ctx.notify();
            }
            WordBlockEditorViewEvent::Enter | WordBlockEditorViewEvent::Navigate(_) => (),
            WordBlockEditorViewEvent::Escape => ctx.focus_self(),
        }
    }

    fn update_approved_domains_state(&mut self, ctx: &mut ViewContext<Self>) {
        self.update_approved_domains_mouse_state_handles(ctx);
        self.update_domains_validator(ctx);
    }

    fn update_approved_domains_mouse_state_handles(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(team) = self.user_workspaces.as_ref(ctx).current_team() {
            self.team_approved_domains_mouse_state_handles = team
                .invite_link_domain_restrictions
                .iter()
                .map(|_| Default::default())
                .collect();
        }
        ctx.notify();
    }

    // Updates the validator used by `approve_domains_block_editor` to determine what color
    // to use when rendering the different word chips on the editor.
    fn update_domains_validator(&mut self, ctx: &mut ViewContext<Self>) {
        let current_domain_restrictions =
            if let Some(team) = self.user_workspaces.as_ref(ctx).current_team() {
                team.invite_link_domain_restrictions
                    .iter()
                    .map(|domain_restriction| domain_restriction.domain.clone())
                    .collect()
            } else {
                Vec::new()
            };
        self.approve_domains_block_editor
            .update(ctx, |editor, ctx| {
                editor.with_validator(
                    ctx,
                    Box::new(move |word| {
                        // word chip is a valid domain restriction if it's parsable AND it isn't already an existing domain restriction
                        let lowercase_word = word.to_ascii_lowercase();
                        Self::is_valid_domain(&lowercase_word)
                            && !current_domain_restrictions
                                .iter()
                                .any(|s| s == &lowercase_word)
                    }),
                );
                ctx.notify();
            });
    }

    fn update_team_members_state(&mut self, ctx: &mut ViewContext<Self>) {
        self.update_team_member_mouse_state_handles(ctx);
        self.update_email_validator(ctx);
        self.update_team_name(ctx);
        self.update_email_editor_interaction_state(ctx);
    }

    /// Disables the invite-by-email chip editor whenever the grow-team
    /// warning banner is showing (seat cap reached, delinquent billing,
    /// etc.). Visual styling is unchanged; only interaction is blocked.
    fn update_email_editor_interaction_state(&mut self, ctx: &mut ViewContext<Self>) {
        let blocked = self
            .user_workspaces
            .as_ref(ctx)
            .current_team()
            .map(|team| TeamsWidget::grow_team_warning(team).is_some())
            .unwrap_or(false);
        let state = if blocked {
            InteractionState::Disabled
        } else {
            InteractionState::Editable
        };
        self.email_invites_block_editor.update(ctx, |editor, ctx| {
            editor.set_interaction_state(state, ctx);
        });
    }

    fn update_team_member_mouse_state_handles(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(team) = self.user_workspaces.as_ref(ctx).current_team() {
            let total_length = team.pending_email_invites.len() + team.members.len();
            self.team_members_mouse_state_handles =
                (0..total_length).map(|_| Default::default()).collect();
        }
        ctx.notify();
    }

    fn update_team_name(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(team) = self.user_workspaces.as_ref(ctx).current_team() {
            let team_name = team.name.clone();
            self.rename_team_editor.update(ctx, |editor, ctx| {
                editor.handle_action(&ClickableTextInputAction::UpdateText(team_name), ctx)
            });
        }
    }

    // Updates the validator used by `email_invites_block_editor` to determine what color
    // to use when rendering the different word chips on the editor.
    fn update_email_validator(&mut self, ctx: &mut ViewContext<Self>) {
        let (member_emails, invitee_emails) =
            if let Some(team) = self.user_workspaces.as_ref(ctx).current_team() {
                (
                    team.members
                        .iter()
                        .map(|member| member.email.clone())
                        .collect(),
                    team.pending_email_invites
                        .iter()
                        .map(|invite| invite.invitee_email.clone())
                        .collect(),
                )
            } else {
                (Vec::new(), Vec::new())
            };
        self.email_invites_block_editor.update(ctx, |editor, ctx| {
            editor.with_validator(
                ctx,
                Box::new(move |word| {
                    // word chip is a valid email if its parsable AND it isn't already a team member / invitee
                    let lowercase_word = word.to_ascii_lowercase();
                    EmailAddress::is_valid(&lowercase_word)
                        && !member_emails.iter().any(|s| s == &lowercase_word)
                        && !invitee_emails.iter().any(|s| s == &lowercase_word)
                }),
            );
            ctx.notify();
        });
    }

    fn enable_confirmation_dialog_confirm_button(&mut self, ctx: &mut ViewContext<Self>) {
        self.delete_or_leave_team_confirmation_dialog
            .update(ctx, |dialog, _ctx| {
                dialog.set_confirmation_button_enabled(true);
            })
    }

    fn show_toast(
        &mut self,
        message: impl Into<String>,
        flavor: ToastFlavor,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(TeamsPageViewEvent::ShowToast {
            message: message.into(),
            flavor,
        });
        ctx.notify();
    }

    fn show_success(&mut self, message: impl Into<String>, ctx: &mut ViewContext<Self>) {
        self.show_toast(message, ToastFlavor::Success, ctx);
    }

    fn show_error(
        &mut self,
        error_msg: impl Into<String>,
        error: Option<&anyhow::Error>,
        ctx: &mut ViewContext<Self>,
    ) {
        let message = error_msg.into();
        self.show_toast(message.clone(), ToastFlavor::Error, ctx);

        // Log error to sentry
        if let Some(error) = error {
            log::error!("{message}: {error:#}");
        } else {
            log::error!("{message}");
        }
    }

    fn change_invite_view_option(
        &mut self,
        view_option: &TeamsInviteOption,
        ctx: &mut ViewContext<Self>,
    ) {
        self.invite_view = *view_option;
        self.update_team_members_state(ctx);
    }

    fn copy_invite_link(&mut self, link: &str, ctx: &mut ViewContext<Self>) {
        ctx.clipboard()
            .write(ClipboardContent::plain_text(link.to_string()));
        self.show_toast("Link copied to clipboard!", ToastFlavor::Default, ctx);
    }

    fn remove_user_from_team(
        &mut self,
        user_uid: UserUid,
        team_uid: ServerId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.user_workspaces
            .update(ctx, move |user_workspaces, ctx| {
                user_workspaces.remove_user_from_team(
                    user_uid,
                    team_uid,
                    CloudObjectEventEntrypoint::TeamSettings,
                    ctx,
                );
            });
    }

    fn leave_team(&mut self, ctx: &mut ViewContext<Self>) {
        let team_uid = self.user_workspaces.as_ref(ctx).current_team_uid();
        if let Some(team_uid) = team_uid {
            TeamUpdateManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.leave_team(team_uid, CloudObjectEventEntrypoint::TeamSettings, ctx)
            });
        }
    }

    fn create_team(&mut self, ctx: &mut ViewContext<Self>) {
        let team_name = self.create_team_editor.as_ref(ctx).buffer_text(ctx);
        TeamUpdateManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.create_team(
                team_name,
                CloudObjectEventEntrypoint::TeamSettings,
                Some(self.checkbox_value),
                ctx,
            );
        });
        ctx.dispatch_typed_action(&WorkspaceAction::OpenWarpDrive);
    }

    fn set_team_member_role(
        &mut self,
        user_uid: UserUid,
        team_uid: ServerId,
        role: MembershipRole,
        ctx: &mut ViewContext<Self>,
    ) {
        self.user_workspaces
            .update(ctx, move |user_workspaces, ctx| {
                user_workspaces.set_team_member_role(user_uid, team_uid, role, ctx);
            });
    }

    fn is_valid_domain(domain: &str) -> bool {
        DOMAIN_NAME_REGEX.is_match(domain)
    }

    fn add_domain_restrictions(&mut self, team_uid: ServerId, ctx: &mut ViewContext<Self>) {
        let editor = self.approve_domains_block_editor.as_ref(ctx);

        // Verify no invalid domains before continuing
        let invalid_domains = editor.get_list_of_invalid_words(ctx);
        if !invalid_domains.is_empty() {
            let error = format!("Invalid domains: {}", invalid_domains.len());
            self.show_error(error, None, ctx);
            return;
        }

        // Don't do anything if list of domains is empty
        let domains = editor.get_list_of_words(ctx);
        if domains.is_empty() {
            return;
        }

        // Lowercase and deduplicate domains
        let unique_domains: Vec<String> = domains
            .into_iter()
            .map(|word| word.to_ascii_lowercase())
            .collect::<HashSet<String>>()
            .into_iter()
            .collect();

        self.show_success(
            format!("Domain restrictions added: {}", unique_domains.len()),
            ctx,
        );
        self.user_workspaces
            .update(ctx, move |user_workspaces, ctx| {
                user_workspaces.add_invite_link_domain_restrictions(team_uid, unique_domains, ctx);
            });
    }

    fn delete_domain_restriction(
        &mut self,
        team_uid: ServerId,
        domain_uid: ServerId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.user_workspaces
            .update(ctx, move |user_workspaces, ctx| {
                user_workspaces.delete_invite_link_domain_restriction(team_uid, domain_uid, ctx);
            });
    }

    fn send_email_invites(&mut self, team_uid: ServerId, ctx: &mut ViewContext<Self>) {
        let editor = self.email_invites_block_editor.as_ref(ctx);

        // Verify no invalid emails before continuing
        let invalid_emails = editor.get_list_of_invalid_words(ctx);
        if !invalid_emails.is_empty() {
            let error = format!("Invalid emails: {}", invalid_emails.len());
            self.show_error(error, None, ctx);
            return;
        }

        // Don't do anything if list of emails is empty
        let emails = editor.get_list_of_words(ctx);
        if emails.is_empty() {
            return;
        }

        // Lowercase and deduplicate emails
        let unique_emails: Vec<String> = emails
            .into_iter()
            .map(|word| word.to_ascii_lowercase())
            .collect::<HashSet<String>>()
            .into_iter()
            .collect();

        let message = if unique_emails.len() == 1 {
            "Your invite is on the way!".to_string()
        } else {
            format!("Your {} invites are on the way!", unique_emails.len())
        };
        self.show_success(message, ctx);
        self.user_workspaces
            .update(ctx, move |user_workspaces, ctx| {
                user_workspaces.send_email_invites(team_uid, unique_emails, ctx);
            })
    }

    fn set_is_invite_link_enabled(
        &mut self,
        team_uid: ServerId,
        new_value: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.user_workspaces
            .update(ctx, move |user_workspaces, ctx| {
                user_workspaces.set_is_invite_link_enabled(team_uid, new_value, ctx);
            });
    }

    fn reset_invite_links(&mut self, team_uid: ServerId, ctx: &mut ViewContext<Self>) {
        self.user_workspaces
            .update(ctx, move |user_workspaces, ctx| {
                user_workspaces.reset_invite_links(team_uid, ctx);
            });
    }

    fn set_team_discoverability(
        &mut self,
        team_uid: ServerId,
        discoverable: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.user_workspaces
            .update(ctx, move |user_workspaces, ctx| {
                user_workspaces.set_team_discoverability(team_uid, discoverable, ctx);
            });
    }

    fn join_team_with_team_discovery(&mut self, team_uid: ServerId, ctx: &mut ViewContext<Self>) {
        self.user_workspaces
            .update(ctx, move |user_workspaces, ctx| {
                user_workspaces.join_team_with_team_discovery(team_uid, ctx);
            });
    }

    fn delete_team_invite(
        &mut self,
        team_uid: ServerId,
        invitee_email: String,
        ctx: &mut ViewContext<Self>,
    ) {
        self.user_workspaces
            .update(ctx, move |user_workspaces, ctx| {
                user_workspaces.delete_team_invite(team_uid, invitee_email, ctx);
            });
    }

    fn generate_upgrade_link(&mut self, team_uid: ServerId, ctx: &mut ViewContext<Self>) {
        self.user_workspaces
            .update(ctx, move |user_workspaces, ctx| {
                user_workspaces.generate_upgrade_link(team_uid, ctx);
            });
    }

    fn generate_stripe_billing_portal_link(
        &mut self,
        team_uid: ServerId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.user_workspaces
            .update(ctx, move |user_workspaces, ctx| {
                user_workspaces.generate_stripe_billing_portal_link(team_uid, ctx);
            });
    }

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_) => ctx.notify(),
            EditorEvent::Escape => ctx.focus_self(),
            _ => (),
        }
    }

    fn handle_rename_team_editor_event(
        &mut self,
        event: &ClickableTextInputEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ClickableTextInputEvent::Submit(new_name) => {
                TeamUpdateManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.rename_team(new_name.to_string(), ctx)
                });
                self.rename_team_editor.update(ctx, |editor, ctx| {
                    editor.handle_action(
                        &ClickableTextInputAction::UpdateText(new_name.to_string()),
                        ctx,
                    )
                });
                ctx.notify();
            }
        }
    }

    /// Find view handle of first input. If user team does not exist, this will default to
    /// the create_team_editor.
    fn focus_on_next_input(&mut self, ctx: &mut ViewContext<Self>) {
        let workspaces: &UserWorkspaces = self.user_workspaces.as_ref(ctx);

        if let Some(team) = workspaces.current_team() {
            if team.organization_settings.is_invite_link_enabled {
                ctx.focus(&self.approve_domains_block_editor);
            } else {
                ctx.focus(&self.email_invites_block_editor);
            }
        } else {
            ctx.focus(&self.create_team_editor);
        }
        ctx.notify();
    }

    fn team_to_item_list(&self, team: &Team, current_user_email: &str) -> Vec<Item> {
        let mut combined = Vec::new();
        let current_user_has_admin_permissions = team.has_admin_permissions(current_user_email);
        let current_user_has_owner_permissions = team.has_owner_permissions(current_user_email);

        // pending email invites
        team.pending_email_invites.iter().for_each(|email_invite| {
            let state = if email_invite.expired {
                ItemState::Expired
            } else {
                ItemState::Pending
            };

            let actions = if current_user_has_admin_permissions {
                vec![ItemAction {
                    icon: Icon::X,
                    label: "Cancel invite".to_string(),
                    action: TeamsPageAction::DeletePendingEmailInvitation {
                        team_uid: team.uid,
                        invitee_email: email_invite.invitee_email.clone(),
                    },
                }]
            } else {
                vec![]
            };

            combined.push(Item {
                text: email_invite.invitee_email.clone(),
                actions,
                state,
            });
        });

        // team members
        team.members.iter().for_each(|member| {
            let team_member_has_owner_permissions = team.has_owner_permissions(&member.email);
            let team_member_has_admin_permissions = team.has_admin_permissions(&member.email);

            let state = if team_member_has_owner_permissions {
                ItemState::Owner
            } else if team_member_has_admin_permissions {
                ItemState::Admin
            } else {
                ItemState::Valid
            };

            let mut actions = Vec::new();

            if member.email != *current_user_email {
                // Owner can transfer ownership to non-owner members
                if current_user_has_owner_permissions && !team_member_has_owner_permissions {
                    actions.push(ItemAction {
                        icon: Icon::Users,
                        label: "Transfer ownership".to_string(),
                        action: TeamsPageAction::ShowTransferOwnershipModal {
                            new_owner_email: member.email.clone(),
                            new_owner_uid: member.uid,
                            team_uid: team.uid,
                        },
                    });
                }

                // Admins can promote and demote other admins
                if team.is_multi_admin_enabled()
                    && current_user_has_admin_permissions
                    && !team_member_has_owner_permissions
                {
                    if team_member_has_admin_permissions {
                        actions.push(ItemAction {
                            icon: Icon::ArrowDown,
                            label: "Demote from admin".to_string(),
                            action: TeamsPageAction::SetTeamMemberRole {
                                team_uid: team.uid,
                                user_uid: member.uid,
                                role: MembershipRole::User,
                            },
                        });
                    } else {
                        actions.push(ItemAction {
                            icon: Icon::ArrowUp,
                            label: "Promote to admin".to_string(),
                            action: TeamsPageAction::SetTeamMemberRole {
                                team_uid: team.uid,
                                user_uid: member.uid,
                                role: MembershipRole::Admin,
                            },
                        });
                    }
                }

                // Admins can remove non-owner members
                if current_user_has_admin_permissions && !team_member_has_owner_permissions {
                    actions.push(ItemAction {
                        icon: Icon::X,
                        label: "Remove from team".to_string(),
                        action: TeamsPageAction::RemoveUserFromTeam {
                            user_uid: member.uid,
                            team_uid: team.uid,
                        },
                    });
                }
            }

            combined.push(Item {
                text: member.email.clone(),
                actions,
                state,
            });
        });

        combined
    }
}

impl SettingsPageMeta for TeamsPageView {
    fn section() -> SettingsSection {
        SettingsSection::Teams
    }

    fn on_page_selected(&mut self, allow_steal_focus: bool, ctx: &mut ViewContext<Self>) {
        if allow_steal_focus {
            self.focus_on_next_input(ctx);
        }
        self.create_team_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
        });
        // We want to immediately see if the user is part of a workspace rather than wait for the next poll.
        std::mem::drop(
            TeamUpdateManager::handle(ctx)
                .update(ctx, |manager, ctx| manager.refresh_workspace_metadata(ctx)),
        );
        self.update_team_members_state(ctx);
        self.update_approved_domains_state(ctx);
        if NetworkStatus::as_ref(ctx).is_online() {
            self.user_workspaces
                .update(ctx, move |user_workspaces, ctx| {
                    user_workspaces.fetch_discoverable_teams(ctx);
                });
        }
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        true
    }

    fn on_tab_pressed(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus_on_next_input(ctx);
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<TeamsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<TeamsPageView>) -> Self {
        SettingsPageViewHandle::Teams(view_handle)
    }
}

#[derive(Default)]
struct TeamsWidget {
    mouse_state_handles: TeamsWidgetMouseHandles,
}

impl TeamsWidget {
    /// Gets the per-seat costs (monthly and yearly) for the current team plan.
    /// Returns None if pricing info is unavailable or the plan doesn't support per-seat pricing.
    fn get_per_seat_costs(
        &self,
        team_metadata: &Team,
        pricing_info_model: &PricingInfoModel,
    ) -> Option<(f64, f64)> {
        let stripe_subscription_plan = (&team_metadata.billing_metadata).try_into().ok()?;
        let plan_pricing = pricing_info_model.plan_pricing(&stripe_subscription_plan)?;
        let monthly_cost = plan_pricing.monthly_plan_price_per_month_usd_cents as f64 / 100.;
        let yearly_cost = plan_pricing.yearly_plan_price_per_month_usd_cents as f64 * 12. / 100.;
        Some((monthly_cost, yearly_cost))
    }

    fn grow_team_warning(team: &Team) -> Option<GrowTeamWarning> {
        match team.billing_metadata.delinquency_status {
            DelinquencyStatus::PastDue => return Some(GrowTeamWarning::PaymentPastDue),
            DelinquencyStatus::Unpaid => return Some(GrowTeamWarning::PaymentUnpaid),
            DelinquencyStatus::NoDelinquency
            // team limit is split into 2 cases below
            | DelinquencyStatus::TeamLimitExceeded
            | DelinquencyStatus::Unknown => {}
        }
        let policy = team.billing_metadata.tier.workspace_size_policy?;
        if policy.is_unlimited {
            return None;
        }
        let team_size = i64::try_from(team.members.len()).unwrap_or(i64::MAX);
        if team_size > policy.limit {
            return Some(GrowTeamWarning::SeatCapExceeded);
        }
        if team_size >= policy.limit {
            return Some(GrowTeamWarning::SeatCapReached);
        }
        None
    }

    /// Maps an admin's actionable path out of a `GrowTeamWarning`.
    fn grow_team_warning_cta(
        warning: GrowTeamWarning,
        has_admin_permissions: bool,
        billing_metadata: &BillingMetadata,
        pricing_info: &PricingInfoModel,
    ) -> GrowTeamWarningCta {
        if !has_admin_permissions {
            return GrowTeamWarningCta::None;
        }
        match warning {
            GrowTeamWarning::PaymentPastDue | GrowTeamWarning::PaymentUnpaid => {
                // Self-serve admins should be able to fix billing themselves;
                // everyone else (enterprise / legacy) needs to reach support.
                if billing_metadata.is_on_stripe_paid_plan() {
                    GrowTeamWarningCta::UpdateBilling
                } else {
                    GrowTeamWarningCta::ContactSupport
                }
            }
            GrowTeamWarning::SeatCapReached | GrowTeamWarning::SeatCapExceeded => {
                // Build Business is the top of the self-serve ladder; the only
                // path to more seats is an enterprise / sales conversation.
                if billing_metadata.is_on_build_business_plan() {
                    return GrowTeamWarningCta::ContactSales;
                }
                if billing_metadata.is_enterprise_plan() {
                    return GrowTeamWarningCta::None;
                }
                let Some(policy) = billing_metadata.tier.workspace_size_policy else {
                    return GrowTeamWarningCta::None;
                };
                if Self::has_higher_seat_cap_plan_available(&policy, pricing_info) {
                    GrowTeamWarningCta::Upgrade
                } else {
                    GrowTeamWarningCta::None
                }
            }
        }
    }

    fn has_higher_seat_cap_plan_available(
        workspace_size_policy: &WorkspaceSizePolicy,
        pricing_info: &PricingInfoModel,
    ) -> bool {
        if workspace_size_policy.is_unlimited {
            return false;
        }
        pricing_info
            .plans()
            .iter()
            .filter_map(|plan| plan.max_team_size)
            .any(|max| i64::from(max) > workspace_size_policy.limit)
    }

    /// Renders the red warning alert at the top of the invite section.
    fn render_grow_team_warning_alert(
        &self,
        team: &Team,
        warning: GrowTeamWarning,
        has_admin_permissions: bool,
        pricing_info: &PricingInfoModel,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let horizontal_padding = 16.;
        let theme = appearance.theme();
        let active_text = theme.active_ui_text_color();

        let alert_icon = Container::new(
            ConstrainedBox::new(
                Icon::AlertCircle
                    .to_warpui_icon(active_text.with_opacity(90))
                    .finish(),
            )
            .with_max_height(20.)
            .with_max_width(20.)
            .finish(),
        )
        .with_margin_right(horizontal_padding)
        .finish();

        let title = match warning {
            GrowTeamWarning::SeatCapReached => "Your team is full",
            GrowTeamWarning::SeatCapExceeded => "You've exceeded your member limit",
            GrowTeamWarning::PaymentPastDue => "Payment past due",
            GrowTeamWarning::PaymentUnpaid => "Subscription unpaid",
        };
        let title_element = self.render_subsection_header(title.to_owned(), appearance);

        let cta = Self::grow_team_warning_cta(
            warning,
            has_admin_permissions,
            &team.billing_metadata,
            pricing_info,
        );

        let body_prefix = match warning {
            GrowTeamWarning::SeatCapReached => "You've reached your plan's member limit.",
            GrowTeamWarning::SeatCapExceeded => {
                "You've exceeded your plan's member limit. Existing team members keep their access, but you won't be able to add new members."
            }
            GrowTeamWarning::PaymentPastDue => {
                "Team invites have been restricted due to a past-due payment."
            }
            GrowTeamWarning::PaymentUnpaid => {
                "Team invites have been restricted due to an unpaid subscription."
            }
        };

        let is_delinquency = matches!(
            warning,
            GrowTeamWarning::PaymentPastDue | GrowTeamWarning::PaymentUnpaid
        );
        let cta_sentence = if !has_admin_permissions {
            if is_delinquency {
                "Contact a team admin to restore access."
            } else {
                "Contact a team admin to grow the team."
            }
        } else {
            match cta {
                GrowTeamWarningCta::Upgrade => "Upgrade to grow your team.",
                GrowTeamWarningCta::ContactSales => "Contact sales to grow your team.",
                GrowTeamWarningCta::UpdateBilling => {
                    "Update your payment information to restore access."
                }
                GrowTeamWarningCta::ContactSupport => "Contact support to restore access.",
                GrowTeamWarningCta::None => {
                    if is_delinquency {
                        "Contact support to restore access."
                    } else {
                        "Contact sales to grow your team."
                    }
                }
            }
        };
        let body_text = format!("{body_prefix} {cta_sentence}");
        let body = self.render_sub_text(body_text, appearance, None);
        let title_container = Container::new(title_element)
            .with_margin_bottom(4.)
            .finish();
        let text_column = Flex::column()
            .with_child(title_container)
            .with_child(body)
            .finish();
        let left_content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(alert_icon)
            .with_child(Shrinkable::new(1., text_column).finish())
            .finish();

        let mut content_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Shrinkable::new(1., left_content).finish());

        // CTA button only renders when there's an actionable path. A single
        // mouse state handle is fine because at most one CTA shows at a time.
        if let Some((cta_label, cta_action)) = match cta {
            GrowTeamWarningCta::Upgrade => Some((
                "Upgrade",
                TeamsPageAction::GenerateUpgradeLink { team_uid: team.uid },
            )),
            GrowTeamWarningCta::ContactSales => {
                Some(("Contact sales", TeamsPageAction::ContactSales))
            }
            GrowTeamWarningCta::UpdateBilling => Some((
                "Update billing",
                TeamsPageAction::GenerateStripeBillingPortalLink { team_uid: team.uid },
            )),
            GrowTeamWarningCta::ContactSupport => {
                Some(("Contact support", TeamsPageAction::ContactSupport))
            }
            GrowTeamWarningCta::None => None,
        } {
            let cta_mouse_state = self
                .mouse_state_handles
                .grow_team_warning_cta_button
                .clone();
            let cta_styles = UiComponentStyles {
                font_weight: Some(Weight::Medium),
                font_size: Some(13.),
                height: Some(32.),
                padding: Some(Coords {
                    top: 6.,
                    bottom: 6.,
                    left: 14.,
                    right: 14.,
                }),
                ..Default::default()
            };
            let error_color = theme.ui_error_color();
            let cta_button = appearance
                .ui_builder()
                .button(ButtonVariant::Secondary, cta_mouse_state)
                .with_style(cta_styles)
                .with_centered_text_label(cta_label.to_owned())
                .with_hovered_styles(UiComponentStyles {
                    background: Some(
                        themes::theme::Fill::from(error_color)
                            .with_opacity(20)
                            .into(),
                    ),
                    border_color: Some(themes::theme::Fill::from(error_color).into()),
                    ..Default::default()
                })
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(cta_action.clone()))
                .finish();
            content_row =
                content_row.with_child(Container::new(cta_button).with_margin_left(16.).finish());
        }

        let error_color = theme.ui_error_color();
        let background_fill = themes::theme::Fill::from(error_color).with_opacity(10);
        let border_fill = themes::theme::Fill::from(error_color);
        Container::new(content_row.finish())
            .with_vertical_padding(12.)
            .with_horizontal_padding(horizontal_padding)
            .with_background(background_fill)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_border(Border::all(1.).with_border_fill(border_fill))
            .finish()
    }

    fn render_team_member_cost_info(
        &self,
        team_metadata: &Team,
        pricing_info_model: &PricingInfoModel,
        appearance: &Appearance,
        has_admin_permissions: bool,
    ) -> Box<dyn Element> {
        let prorated_message = if has_admin_permissions {
            "You'll be charged for a portion of the team member's usage of Warp."
        } else {
            "Your admin will be charged for a portion of the team member's usage of Warp."
        };

        let additional_members_cost_money_msg = if let Some((monthly_cost, yearly_cost)) =
            self.get_per_seat_costs(team_metadata, pricing_info_model)
        {
            format!(
                "Additional members are billed at your plan's per-user rate: ${monthly_cost:.0}/month or ${yearly_cost:.0}/year, depending on your billing interval. {prorated_message}"
            )
        } else {
            format!(
                "Additional members are billed at your plan's per-user rate. {prorated_message}"
            )
        };

        let horizontal_padding = 16.;
        let theme = appearance.theme();
        let currency_icon = Container::new(
            ConstrainedBox::new(Icon::CoinsStacked.to_warpui_icon(theme.accent()).finish())
                .with_max_height(20.)
                .with_max_width(20.)
                .finish(),
        )
        .with_margin_right(horizontal_padding)
        .finish();

        let member_pricing_info =
            self.render_sub_text(additional_members_cost_money_msg, appearance, None);

        let content_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(currency_icon)
            .with_child(Shrinkable::new(1., member_pricing_info).finish());

        // Wrap in a container with an accent-tinted alert background.
        Container::new(content_row.finish())
            .with_vertical_padding(20.)
            .with_horizontal_padding(horizontal_padding)
            .with_background(internal_colors::accent_overlay_1(theme))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .finish()
    }

    fn render_team_management_page(
        &self,
        team_metadata: &Team,
        cloud_model: &CloudModel,
        ai_request_usage_model: &AIRequestUsageModel,
        view: &TeamsPageView,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let current_user_email = view.auth_state.user_email().unwrap_or_default();
        let has_admin_permissions = team_metadata.has_admin_permissions(&current_user_email);
        let is_owner = team_metadata.has_owner_permissions(&current_user_email);
        let remaining_workspace_credits =
            ai_request_usage_model.total_current_workspace_bonus_credits_remaining(app);
        let delete_disabled_reason = team_metadata
            .get_delete_disabled_reason(&current_user_email, remaining_workspace_credits);

        let mut main_content = Flex::column();
        let chip_editor_style = UiComponentStyles::default()
            .set_background(appearance.theme().background().into())
            .set_border_radius(CornerRadius::with_all(Radius::Pixels(3.)))
            .set_border_width(1.)
            .set_border_color(appearance.theme().foreground().with_opacity(20).into())
            .set_padding(Coords::uniform(0.).top(4.).right(5.));

        // 1) Team name header
        main_content.add_child(self.render_header(
            has_admin_permissions,
            team_metadata,
            view,
            appearance,
        ));

        // has_plan_limit will be true if the team has any shared object policy that
        // is not unlimited.
        let has_plan_limit = team_metadata
            .billing_metadata
            .tier
            .shared_notebooks_policy
            .map(|policy| !policy.is_unlimited)
            .unwrap_or_else(|| false)
            || team_metadata
                .billing_metadata
                .tier
                .shared_workflows_policy
                .map(|policy| !policy.is_unlimited)
                .unwrap_or_else(|| false);
        if has_plan_limit {
            // Render plan usage and limits
            main_content.add_child(
                Container::new(self.render_plan_usage(team_metadata, cloud_model, appearance, app))
                    .with_padding_top(CONTENT_SEPARATION_PADDING)
                    .finish(),
            )
        }

        // 2) Horizontal separator
        main_content.add_child(
            Container::new(render_separator(appearance))
                .with_padding_top(CONTENT_SEPARATION_PADDING)
                .with_margin_bottom(HORIZONTAL_BAR_TO_SUB_HEADER_PADDING)
                .finish(),
        );

        // 3) Team invitation flows (invite link / email invites / discovery)
        main_content.add_child(self.render_team_invitation_section(
            team_metadata,
            has_admin_permissions,
            view,
            appearance,
            chip_editor_style,
            app,
        ));

        // 4) Horizontal separator between the invite flows and the team members
        // list. 32px of breathing room above and below to match the design.
        main_content.add_child(
            Container::new(render_separator(appearance))
                .with_padding_top(32.)
                .with_padding_bottom(32.)
                .finish(),
        );

        // 5) Team members
        main_content.add_child(self.render_team_members_section(
            team_metadata,
            &current_user_email,
            view,
            appearance,
        ));

        // 6) Optional outgrow CTA
        let pricing_info_model = view.pricing_info_model.as_ref(app);
        if let Some(cta) = self.render_outgrow_cta(
            team_metadata,
            has_admin_permissions,
            pricing_info_model,
            appearance,
        ) {
            main_content.add_child(
                Container::new(cta)
                    .with_padding_top(CONTENT_SEPARATION_PADDING)
                    .finish(),
            );
        }

        // 7) Deleting/leaving teams
        let mut button_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        let is_enterprise_team =
            team_metadata.billing_metadata.customer_type == CustomerType::Enterprise;
        // We don't allow users on enterprise teams to leave or delete their team,
        // since their enterprise agreement is tied to it, and it helps enforce that others
        // can't join some other team that doesn't have stricter security guarantees
        if !is_enterprise_team {
            button_row.add_child(
                Container::new(self.render_leave_or_delete_team_button(
                    is_owner,
                    delete_disabled_reason.is_none(),
                    view,
                    appearance,
                ))
                .with_padding_right(24.)
                .finish(),
            );
        }
        // We show some help text if a team can't be deleted...
        if let Some(delete_disabled_reason) = delete_disabled_reason {
            // and if the current user actually has the perms to delete the team
            if has_admin_permissions && !is_enterprise_team {
                button_row.add_child(
                    Container::new(self.render_delete_disabled_help_text(
                        delete_disabled_reason,
                        team_metadata.uid,
                        appearance,
                    ))
                    .with_padding_right(24.)
                    .finish(),
                );
            }
        }
        main_content.add_child(button_row.finish());
        main_content.finish()
    }

    fn render_header(
        &self,
        has_admin_permissions: bool,
        team: &Team,
        view: &TeamsPageView,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut team_name_header = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);

        let mut left_side = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::Start);

        if has_admin_permissions {
            left_side.add_child(ChildView::new(&view.rename_team_editor).finish());
        } else {
            left_side.add_child(
                Text::new_inline(team.name.clone(), appearance.ui_font_family(), 24.)
                    .with_style(Properties::default().weight(Weight::Bold))
                    .with_color(appearance.theme().active_ui_text_color().into())
                    .finish(),
            );
        }

        if team.billing_metadata.customer_type != CustomerType::Unknown {
            left_side.add_child(
                Container::new(render_customer_type_badge(
                    appearance,
                    team.billing_metadata.customer_type.to_display_string(),
                ))
                .with_margin_left(12.)
                .finish(),
            );
        }

        match team.billing_metadata.delinquency_status {
            DelinquencyStatus::PastDue => {
                left_side.add_child(
                    Container::new(self.render_delinquency_badge(
                        appearance,
                        "PAST DUE".into(),
                        themes::theme::Fill::from(*PAST_DUE_BADGE_COLOR).into(),
                    ))
                    .with_margin_left(8.)
                    .finish(),
                );
            }
            DelinquencyStatus::Unpaid => {
                left_side.add_child(
                    Container::new(self.render_delinquency_badge(
                        appearance,
                        "UNPAID".into(),
                        themes::theme::Fill::from(*UNPAID_BADGE_COLOR).into(),
                    ))
                    .with_margin_left(8.)
                    .finish(),
                );
            }
            DelinquencyStatus::NoDelinquency
            | DelinquencyStatus::TeamLimitExceeded
            | DelinquencyStatus::Unknown => (),
        }

        team_name_header.add_child(left_side.finish());

        // Upgrade / billing links
        if has_admin_permissions {
            team_name_header.add_child(self.render_billing_links(team, appearance));
        }

        team_name_header.finish()
    }

    fn render_contact_support_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .button(
                ButtonVariant::Link,
                self.mouse_state_handles.enterprise_contact_us_link.clone(),
            )
            .with_text_and_icon_label(
                TextAndIcon::new(
                    TextAndIconAlignment::IconFirst,
                    "Contact support",
                    Icon::Phone.to_warpui_icon(appearance.theme().accent()),
                    MainAxisSize::Min,
                    MainAxisAlignment::Center,
                    vec2f(14., 14.),
                )
                .with_inner_padding(4.),
            )
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(TeamsPageAction::ContactSupport);
            })
            .finish()
    }

    fn render_manage_billing_button(
        &self,
        team_uid: ServerId,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .button(
                ButtonVariant::Link,
                self.mouse_state_handles.stripe_billing_portal_link.clone(),
            )
            .with_text_and_icon_label(
                TextAndIcon::new(
                    TextAndIconAlignment::IconFirst,
                    "Manage billing",
                    Icon::CoinsStacked.to_warpui_icon(appearance.theme().accent()),
                    MainAxisSize::Min,
                    MainAxisAlignment::Center,
                    vec2f(14., 14.),
                )
                .with_inner_padding(4.),
            )
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(TeamsPageAction::GenerateStripeBillingPortalLink {
                    team_uid,
                });
            })
            .finish()
    }

    fn render_admin_panel_button(
        &self,
        team_uid: ServerId,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .button(
                ButtonVariant::Link,
                self.mouse_state_handles.admin_panel_button.clone(),
            )
            .with_text_and_icon_label(
                TextAndIcon::new(
                    TextAndIconAlignment::IconFirst,
                    "Open admin panel",
                    Icon::Users.to_warpui_icon(appearance.theme().accent()),
                    MainAxisSize::Min,
                    MainAxisAlignment::Center,
                    vec2f(14., 14.),
                )
                .with_inner_padding(4.),
            )
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(TeamsPageAction::OpenAdminPanel { team_uid });
            })
            .finish()
    }

    fn render_billing_links(&self, team: &Team, appearance: &Appearance) -> Box<dyn Element> {
        let mut billing_links = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min);

        let team_uid = team.uid;

        // For enterprise we actually hide both upgrade/billing links and have a contact support link instead
        if team.billing_metadata.customer_type == CustomerType::Enterprise {
            billing_links.add_child(
                Container::new(self.render_contact_support_button(appearance))
                    .with_margin_left(12.)
                    .finish(),
            );
        } else {
            // If the team is upgradeable to self-serve tier, show them the upgrade link.
            if team.billing_metadata.can_upgrade_to_higher_tier_plan() {
                let description = if team.billing_metadata.can_upgrade_to_build_plan() {
                    "Upgrade to Build"
                } else {
                    match team.billing_metadata.customer_type {
                        CustomerType::Prosumer => "Upgrade to Turbo plan",
                        CustomerType::Turbo => "Upgrade to Lightspeed plan",
                        _ => "Compare plans",
                    }
                };
                billing_links.add_child(
                    Container::new(self.render_compare_plans_button(
                        description,
                        self.mouse_state_handles.upgrade_link.clone(),
                        team_uid,
                        appearance,
                        None,
                    ))
                    .with_margin_left(12.)
                    .finish(),
                );
            } else if team.has_billing_history {
                billing_links.add_child(
                    Container::new(self.render_manage_billing_button(team_uid, appearance))
                        .with_margin_left(12.)
                        .finish(),
                );
            }
        }

        billing_links.add_child(
            Container::new(self.render_admin_panel_button(team_uid, appearance))
                .with_margin_left(12.)
                .finish(),
        );

        billing_links.finish()
    }

    fn render_plan_usage(
        &self,
        team: &Team,
        cloud_model: &CloudModel,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut section = Flex::column();
        let sub_header_text = match team.billing_metadata.customer_type {
            CustomerType::Free => "Free plan usage limits",
            _ => "Plan usage limits",
        };
        section.add_child(self.render_subsection_header(sub_header_text.into(), appearance));

        let mut shared_objects_usage_row =
            Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        if let Some(policy) = team.billing_metadata.tier.shared_notebooks_policy {
            if !policy.is_unlimited {
                let mut shared_notebooks_column = Flex::column();
                shared_notebooks_column.add_child(
                    self.render_plan_usage_header("Shared Notebooks".into(), appearance),
                );
                let num_shared_notebooks = cloud_model
                    .active_notebooks_in_space(Space::Team { team_uid: team.uid }, app)
                    .count();
                shared_notebooks_column.add_child(
                    Container::new(self.render_plan_usage_text(
                        format!("{}/{}", num_shared_notebooks, policy.limit),
                        appearance,
                    ))
                    .with_margin_top(4.)
                    .finish(),
                );
                shared_objects_usage_row.add_child(
                    Container::new(shared_notebooks_column.finish())
                        .with_margin_right(64.)
                        .finish(),
                );
            }
        }

        if let Some(policy) = team.billing_metadata.tier.shared_workflows_policy {
            if !policy.is_unlimited {
                let mut shared_workflows_column = Flex::column();
                shared_workflows_column.add_child(
                    self.render_plan_usage_header("Shared Workflows".into(), appearance),
                );
                let num_shared_workflows = cloud_model
                    .active_workflows_in_space(Space::Team { team_uid: team.uid }, app)
                    .count();
                shared_workflows_column.add_child(
                    Container::new(self.render_plan_usage_text(
                        format!("{}/{}", num_shared_workflows, policy.limit),
                        appearance,
                    ))
                    .with_margin_top(4.)
                    .finish(),
                );
                shared_objects_usage_row.add_child(shared_workflows_column.finish());
            }
        }

        section.add_child(
            Container::new(shared_objects_usage_row.finish())
                .with_margin_top(16.)
                .finish(),
        );

        section.finish()
    }

    fn render_team_invitation_section(
        &self,
        team_metadata: &Team,
        has_admin_permissions: bool,
        view: &TeamsPageView,
        appearance: &Appearance,
        chip_editor_style: UiComponentStyles,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut invitation_section = Flex::column();

        // "team is full" or "billing issue" or some other alert thats restricting you from adding team members
        let warning = Self::grow_team_warning(team_metadata);
        let pricing_info_model = view.pricing_info_model.as_ref(app);
        if let Some(warning) = warning {
            let alert = self.render_grow_team_warning_alert(
                team_metadata,
                warning,
                has_admin_permissions,
                pricing_info_model,
                appearance,
            );
            invitation_section.add_child(Container::new(alert).with_padding_bottom(24.).finish());
        }

        invitation_section.add_child(
            Container::new(
                self.render_subsection_header("Invite team members".to_owned(), appearance),
            )
            .with_padding_bottom(16.)
            .finish(),
        );

        if team_metadata.billing_metadata.is_on_stripe_paid_plan() {
            let pricing_alert = self.render_team_member_cost_info(
                team_metadata,
                pricing_info_model,
                appearance,
                has_admin_permissions,
            );
            invitation_section.add_child(
                Container::new(pricing_alert)
                    .with_padding_bottom(24.)
                    .finish(),
            );
        }

        // Invite by link section
        // Only show invite-by-link if user is admin OR if invite links are enabled
        if team_metadata.organization_settings.is_invite_link_enabled || has_admin_permissions {
            invitation_section.add_child(self.render_invite_by_link_section(
                team_metadata,
                has_admin_permissions,
                view,
                appearance,
                chip_editor_style,
            ));
        }

        // Invite by email. Disabled whenever the warning banner is showing —
        // the banner owns the explanation + recovery CTA.
        invitation_section.add_child(self.render_invite_by_email_section(
            team_metadata,
            view,
            appearance,
            chip_editor_style,
            warning.is_some(),
        ));

        // By discovery — third invitation method, same hierarchical level as
        // By link / By email. Gated on non-Enterprise, admin viewer, and the
        // team being eligible for discovery.
        let current_user_email = view.auth_state.user_email().unwrap_or_default();
        if team_metadata.billing_metadata.customer_type != CustomerType::Enterprise
            && has_admin_permissions
            && team_metadata.is_eligible_for_discovery
        {
            invitation_section.add_child(self.render_discoverability_toggle_section(
                team_metadata,
                &current_user_email,
                appearance,
            ));
        }

        invitation_section.finish()
    }

    fn render_invite_by_link_section(
        &self,
        team: &Team,
        has_admin_permissions: bool,
        view: &TeamsPageView,
        appearance: &Appearance,
        chip_editor_style: UiComponentStyles,
    ) -> Box<dyn Element> {
        let mut section = Flex::column();

        // Header + admin-only subtext on the left, toggle on the right. The
        // text is stacked so the toggle centers against the whole block.
        let header = self.render_subsubsection_header("By link".to_owned(), appearance);
        let text_column = if has_admin_permissions {
            Flex::column()
                .with_child(header)
                .with_child(
                    Container::new(self.render_sub_text(
                        INVITE_LINK_TOGGLE_INSTRUCTIONS.into(),
                        appearance,
                        Some(Coords::uniform(0.).right(48.)),
                    ))
                    .with_padding_top(8.)
                    .finish(),
                )
                .finish()
        } else {
            Flex::column().with_child(header).finish()
        };

        let mut invite_by_link_header_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(Shrinkable::new(1., text_column).finish());

        // Toggle on the right only renders if user is admin
        if has_admin_permissions {
            let team_uid = team.uid;
            let current_state = team.organization_settings.is_invite_link_enabled;
            let invite_by_link_toggle = appearance
                .ui_builder()
                .switch(self.mouse_state_handles.invite_by_link_toggle_state.clone())
                .check(team.organization_settings.is_invite_link_enabled)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(TeamsPageAction::ToggleIsInviteLinkEnabled {
                        team_uid,
                        current_state,
                    })
                });

            invite_by_link_header_row.add_child(invite_by_link_toggle.finish());
        }

        section.add_child(invite_by_link_header_row.finish());

        // 3) Invite link + domain restrictions
        // Only renders if invite by link is enabled
        if team.organization_settings.is_invite_link_enabled {
            section.add_child(self.render_copy_link_row(team, appearance));

            // Render invite link reset text if admin user
            if has_admin_permissions {
                let team_uid = team.uid;
                section.add_child(
                    Align::new(
                        appearance
                            .ui_builder()
                            .link(
                                "Reset links".into(),
                                None,
                                Some(Box::new(move |ctx| {
                                    ctx.dispatch_typed_action(TeamsPageAction::ResetInviteLinks {
                                        team_uid,
                                    });
                                })),
                                self.mouse_state_handles.reset_invite_links_button.clone(),
                            )
                            .soft_wrap(false)
                            .build()
                            .with_margin_top(8.)
                            .finish(),
                    )
                    .left()
                    .finish(),
                );
            }

            // Don't render restricted domains section if user is not an admin AND there are no domain restrictions
            if has_admin_permissions || !team.invite_link_domain_restrictions.is_empty() {
                section.add_child(self.render_approved_domains_section(
                    team,
                    has_admin_permissions,
                    view,
                    appearance,
                    chip_editor_style,
                ));
            }
        }

        section.finish()
    }

    fn render_invite_by_email_section(
        &self,
        team: &Team,
        view: &TeamsPageView,
        appearance: &Appearance,
        chip_editor_style: UiComponentStyles,
        force_disabled: bool,
    ) -> Box<dyn Element> {
        let mut section = Flex::column();

        // "By email" subsection header
        section.add_child(
            Container::new(self.render_subsubsection_header("By email".to_owned(), appearance))
                .with_padding_top(CONTENT_SEPARATION_PADDING)
                .with_padding_bottom(8.)
                .finish(),
        );

        // Form stays visually unchanged when blocked; the chip editor is
        // disabled via `update_email_editor_interaction_state` and the send
        // button is force-disabled below. The warning banner at the top of
        // the invitation section owns the explanation + recovery CTA.
        section.add_child(
            Container::new(self.render_sub_text(
                INVITE_BY_EMAIL_EXPIRY_INSTRUCTIONS.into(),
                appearance,
                Some(Coords::uniform(0.).right(48.)),
            ))
            .with_padding_bottom(TEXT_FIELD_TOP_PADDING)
            .finish(),
        );

        section.add_child(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Shrinkable::new(
                        1.,
                        TextInput::new(view.email_invites_block_editor.clone(), chip_editor_style)
                            .build()
                            .finish(),
                    )
                    .finish(),
                )
                .with_child(self.render_send_email_invites_button(
                    team.uid,
                    view,
                    appearance,
                    force_disabled,
                ))
                .finish(),
        );

        // Skip the "invalid emails" hint when the form is disabled.
        if !force_disabled
            && !view.email_invites_block_editor_state.is_valid
            && !view.email_invites_block_editor_state.is_empty
            && view.email_invites_block_editor_state.num_chips > 0
        {
            section.add_child(
                Container::new(
                    self.render_error_sub_text(INVALID_EMAILS_INSTRUCTIONS.into(), appearance),
                )
                .with_padding_top(8.)
                .finish(),
            )
        }

        section.finish()
    }

    fn render_team_members_section(
        &self,
        team: &Team,
        user_email: &str,
        view: &TeamsPageView,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut section = Flex::column().with_main_axis_size(MainAxisSize::Min);

        // 1) "Team members" header row
        let header_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(self.render_subsection_header("Team members".to_owned(), appearance))
            .with_child(self.render_team_members_count(team, appearance))
            .finish();
        section.add_child(
            SavePosition::new(
                Container::new(header_row).with_padding_bottom(16.).finish(),
                TEAM_MEMBERS_HEADER_POSITION_ID,
            )
            .finish(),
        );

        // 2) List of team members
        section.add_child(self.render_item_list(
            view.team_to_item_list(team, user_email),
            view.team_members_mouse_state_handles.clone(),
            view,
            appearance,
        ));

        section.finish()
    }

    /// Right-aligned "{N} team members" label next to the section header.
    /// On finite-cap plans, appends an info icon with a capacity tooltip.
    fn render_team_members_count(&self, team: &Team, appearance: &Appearance) -> Box<dyn Element> {
        let count = team.members.len();
        let count_label = if count == 1 {
            "1 team member".to_string()
        } else {
            format!("{count} team members")
        };
        let theme = appearance.theme();
        let count_color = theme.active_ui_text_color();
        // Info icon uses the muted gray that matches other secondary UI hints.
        let muted_color = theme.active_ui_text_color().with_opacity(60);

        let count_text = appearance
            .ui_builder()
            .span(count_label)
            .with_style(UiComponentStyles {
                font_family_id: Some(appearance.ui_font_family()),
                font_color: Some(count_color.into()),
                font_size: Some(12.),
                ..Default::default()
            })
            .build()
            .finish();

        // No capacity tooltip when the plan is unlimited (or workspace size
        // policy is missing). Just render the count text on its own.
        let policy = team.billing_metadata.tier.workspace_size_policy;
        let finite_cap = match policy {
            Some(p) if !p.is_unlimited => Some(p.limit),
            _ => None,
        };
        let Some(cap) = finite_cap else {
            return count_text;
        };

        let plan_display = team.billing_metadata.customer_type.to_display_string();
        let tooltip_text =
            format!("Your plan ({plan_display}) has a maximum capacity of {cap} members.");

        let info_icon = Container::new(
            ConstrainedBox::new(Icon::Info.to_warpui_icon(muted_color).finish())
                .with_max_height(14.)
                .with_max_width(14.)
                .finish(),
        )
        .with_margin_left(6.)
        .finish();

        let info_icon_with_tooltip = appearance.ui_builder().overlay_tool_tip_on_element(
            tooltip_text,
            self.mouse_state_handles.team_members_count_tooltip.clone(),
            info_icon,
            ParentAnchor::TopRight,
            ChildAnchor::BottomRight,
            vec2f(0., -5.),
        );

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(count_text)
            .with_child(info_icon_with_tooltip)
            .finish()
    }

    // "Want to upgrade your team? <Do X>"
    fn render_outgrow_cta(
        &self,
        team: &Team,
        has_admin_permissions: bool,
        pricing_info: &PricingInfoModel,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        if team.billing_metadata.is_delinquent_due_to_payment_issue() {
            return None;
        }
        let cta = Self::grow_team_warning_cta(
            GrowTeamWarning::SeatCapReached,
            has_admin_permissions,
            &team.billing_metadata,
            pricing_info,
        );
        match cta {
            GrowTeamWarningCta::Upgrade => {
                Some(self.render_outgrow_upgrade_line(team.uid, appearance))
            }
            GrowTeamWarningCta::ContactSales => {
                Some(self.render_outgrow_contact_sales_line(appearance))
            }
            GrowTeamWarningCta::UpdateBilling
            | GrowTeamWarningCta::ContactSupport
            | GrowTeamWarningCta::None => None,
        }
    }

    /// "Want to grow your team? <Upgrade>" — routes through self-serve upgrade.
    fn render_outgrow_upgrade_line(
        &self,
        team_uid: ServerId,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let prefix = self.render_sub_text("Want to grow your team? ".to_string(), appearance, None);
        let link = appearance
            .ui_builder()
            .link(
                "Upgrade".to_string(),
                None,
                Some(Box::new(move |ctx| {
                    ctx.dispatch_typed_action(TeamsPageAction::GenerateUpgradeLink { team_uid });
                })),
                self.mouse_state_handles.outgrow_upgrade_link.clone(),
            )
            .soft_wrap(false)
            .build()
            .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(prefix)
            .with_child(link)
            .finish()
    }

    /// "Want to grow your team? <Contact sales>" — opens the contact sales page.
    fn render_outgrow_contact_sales_line(&self, appearance: &Appearance) -> Box<dyn Element> {
        let prefix = self.render_sub_text("Want to grow your team? ".to_string(), appearance, None);
        let link = appearance
            .ui_builder()
            .link(
                "Contact sales".into(),
                None,
                Some(Box::new(move |ctx| {
                    ctx.dispatch_typed_action(TeamsPageAction::ContactSales);
                })),
                self.mouse_state_handles.outgrow_contact_sales_link.clone(),
            )
            .soft_wrap(false)
            .build()
            .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(prefix)
            .with_child(link)
            .finish()
    }

    fn render_approved_domains_section(
        &self,
        team: &Team,
        has_admin_permissions: bool,
        view: &TeamsPageView,
        appearance: &Appearance,
        chip_editor_style: UiComponentStyles,
    ) -> Box<dyn Element> {
        let mut section = Flex::column();

        // 1) Instruction text for domain restrictions + Domain approval mechanism (input box + button)
        if has_admin_permissions {
            section.add_child(
                Container::new(self.render_sub_text(
                    INVITE_LINK_DOMAIN_RESTRICTIONS_INSTRUCTIONS.into(),
                    appearance,
                    Some(Coords::uniform(0.).right(48.)),
                ))
                .with_padding_top(16.)
                .finish(),
            );

            section.add_child(
                Container::new(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(
                            Shrinkable::new(
                                1.,
                                TextInput::new(
                                    view.approve_domains_block_editor.clone(),
                                    chip_editor_style,
                                )
                                .build()
                                .finish(),
                            )
                            .finish(),
                        )
                        .with_child(self.render_approve_domains_button(team.uid, view, appearance))
                        .finish(),
                )
                .with_padding_top(TEXT_FIELD_TOP_PADDING)
                .finish(),
            );

            if !view.approve_domains_block_editor_state.is_valid
                && !view.approve_domains_block_editor_state.is_empty
                && view.approve_domains_block_editor_state.num_chips > 0
            {
                section.add_child(
                    Container::new(
                        self.render_error_sub_text(INVALID_DOMAINS_INSTRUCTIONS.into(), appearance),
                    )
                    .with_padding_top(8.)
                    .finish(),
                )
            }
        }

        // 3) List of approved domains
        let domains_as_items: Vec<Item> = team
            .invite_link_domain_restrictions
            .iter()
            .map(|domain_restriction| {
                let actions = if has_admin_permissions {
                    vec![ItemAction {
                        icon: Icon::X,
                        label: "Remove domain".to_string(),
                        action: TeamsPageAction::DeleteDomainRestriction {
                            domain_uid: domain_restriction.uid,
                            team_uid: team.uid,
                        },
                    }]
                } else {
                    vec![]
                };
                Item {
                    text: domain_restriction.domain.clone(),
                    actions,
                    state: ItemState::Valid,
                }
            })
            .collect();

        if !domains_as_items.is_empty() {
            section.add_child(
                Container::new(self.render_item_list(
                    domains_as_items,
                    view.team_approved_domains_mouse_state_handles.clone(),
                    view,
                    appearance,
                ))
                .with_padding_top(CONTENT_SEPARATION_PADDING)
                .finish(),
            );
        }

        section.finish()
    }

    fn render_approve_domains_button(
        &self,
        team_uid: ServerId,
        view: &TeamsPageView,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        // Only render enabled button with action if domain list is valid.
        let (action, variant) = if view.approve_domains_block_editor_state.is_valid {
            (
                Some(TeamsPageAction::AddDomainRestrictions { team_uid }),
                ButtonVariant::Accent,
            )
        } else {
            (None, ButtonVariant::Basic)
        };
        Container::new(self.render_button(
            APPROVE_DOMAINS_BUTTON_LABEL,
            variant,
            self.mouse_state_handles.approve_domains_button.clone(),
            action,
            self.button_properties(),
            appearance,
        ))
        .with_padding_left(COPY_LINK_LEFT_PADDING)
        .finish()
    }

    fn render_send_email_invites_button(
        &self,
        team_uid: ServerId,
        view: &TeamsPageView,
        appearance: &Appearance,
        force_disabled: bool,
    ) -> Box<dyn Element> {
        // Only render enabled button with action if email list is valid AND
        // the caller hasn't forced the disabled state (e.g. team at seat cap).
        let (action, variant) = if !force_disabled && view.email_invites_block_editor_state.is_valid
        {
            (
                Some(TeamsPageAction::SendEmailInvites { team_uid }),
                ButtonVariant::Accent,
            )
        } else {
            (None, ButtonVariant::Basic)
        };
        Container::new(self.render_button(
            SEND_EMAIL_INVITES_BUTTON_LABEL,
            variant,
            self.mouse_state_handles.send_email_invites_button.clone(),
            action,
            self.button_properties(),
            appearance,
        ))
        .with_padding_left(COPY_LINK_LEFT_PADDING)
        .finish()
    }

    fn button_properties(&self) -> UiComponentStyles {
        UiComponentStyles {
            font_weight: Some(Weight::Semibold),
            width: Some(BUTTON_WIDTH),
            height: Some(BUTTON_HEIGHT),
            padding: Some(Coords {
                top: 8.,
                bottom: 8.,
                left: 12.,
                right: 12.,
            }),
            font_size: Some(14.),
            ..Default::default()
        }
    }

    fn render_discoverability_toggle_section(
        &self,
        team: &Team,
        current_user_email: &str,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        // Same layout as the "By link" header row: text column on the left,
        // toggle on the right.
        let header = self.render_subsubsection_header("By discovery".to_owned(), appearance);

        let domain = current_user_email.split('@').nth(1).unwrap_or("");
        let team_discoverability_instructions =
            format!("Allow Warp users with an @{domain} email to find and join the team.");
        let subtext = self.render_sub_text(
            team_discoverability_instructions,
            appearance,
            Some(Coords::uniform(0.).right(48.)),
        );
        let text_column = Flex::column()
            .with_child(header)
            .with_child(Container::new(subtext).with_padding_top(8.).finish())
            .finish();

        let team_uid = team.uid;
        let current_state = team.organization_settings.is_discoverable;
        let discoverable_team_toggle = appearance
            .ui_builder()
            .switch(
                self.mouse_state_handles
                    .discoverable_team_toggle_state
                    .clone(),
            )
            .check(team.organization_settings.is_discoverable)
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(TeamsPageAction::ToggleTeamDiscoverability {
                    team_uid,
                    current_state,
                })
            });

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(Shrinkable::new(1., text_column).finish())
            .with_child(discoverable_team_toggle.finish())
            .finish();

        Container::new(row)
            .with_padding_top(CONTENT_SEPARATION_PADDING)
            .finish()
    }

    fn render_leave_or_delete_team_button(
        &self,
        is_team_owner: bool,
        can_team_be_deleted: bool,
        view: &TeamsPageView,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut stack = Stack::new();

        let (label, action) = if is_team_owner {
            (
                DELETE_TEAM_BUTTON_LABEL,
                TeamsPageAction::ShowDeleteTeamConfirmationDialog,
            )
        } else {
            (
                LEAVE_TEAM_BUTTON_LABEL,
                TeamsPageAction::ShowLeaveTeamConfirmationDialog,
            )
        };

        let ui_builder = appearance.ui_builder().clone();
        let button = ui_builder
            .button(
                ButtonVariant::Outlined,
                self.mouse_state_handles.leave_team_button.clone(),
            )
            .with_style(
                self.button_properties()
                    .set_font_color(appearance.theme().active_ui_text_color().into())
                    .set_width(LEAVE_TEAM_BUTTON_WIDTH),
            )
            .with_centered_text_label(label.to_owned());
        let hoverable = if is_team_owner && !can_team_be_deleted {
            button
                .with_disabled_styles(UiComponentStyles {
                    background: Some(appearance.theme().surface_3().into()),
                    border_color: Some(appearance.theme().surface_3().into()),
                    font_color: Some(
                        appearance
                            .theme()
                            .disabled_text_color(appearance.theme().background())
                            .into(),
                    ),
                    ..Default::default()
                })
                .disabled()
                .build()
        } else {
            button
                .build()
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        };

        stack.add_child(
            Container::new(hoverable.finish())
                .with_padding_top(CONTENT_SEPARATION_PADDING)
                .with_padding_bottom(CONTENT_SEPARATION_PADDING)
                .finish(),
        );

        if view.show_delete_or_leave_team_confirmation_dialog {
            stack.add_positioned_overlay_child(
                ChildView::new(&view.delete_or_leave_team_confirmation_dialog).finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::Center,
                    ChildAnchor::BottomMiddle,
                ),
            );
        }

        stack.finish()
    }

    fn render_delete_disabled_help_text(
        &self,
        delete_disabled_reason: TeamDeleteDisabledReason,
        team_uid: ServerId,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let description = self.render_sub_text(
            delete_disabled_reason.user_facing_message().into(),
            appearance,
            None,
        );

        let mut children = vec![description];

        if delete_disabled_reason == TeamDeleteDisabledReason::ActivePaidSubscription {
            let link = appearance
                .ui_builder()
                .link(
                    "Manage plan".into(),
                    None,
                    Some(Box::new(move |ctx| {
                        ctx.dispatch_typed_action(
                            TeamsPageAction::GenerateStripeBillingPortalLink { team_uid },
                        );
                    })),
                    self.mouse_state_handles.manage_plan_link.clone(),
                )
                .build()
                .finish();
            children.push(link);
        }

        Flex::column().with_children(children).finish()
    }

    fn render_delinquency_badge(
        &self,
        appearance: &Appearance,
        text: String,
        chip_color: ColorU,
    ) -> Box<dyn Element> {
        Container::new(
            Text::new_inline(text, appearance.ui_font_family(), appearance.ui_font_size())
                .with_color(*DELINQUENCY_BADGE_TEXT_COLOR)
                .with_style(Properties::default().weight(Weight::Medium))
                .finish(),
        )
        .with_uniform_padding(4.)
        .with_background(chip_color)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
        .finish()
    }

    fn render_state_chip(
        &self,
        appearance: &Appearance,
        text: String,
        text_color: ColorU,
        chip_color: ColorU,
        font_size: f32,
        font_weight: Weight,
    ) -> Box<dyn Element> {
        Container::new(
            Text::new_inline(text, appearance.ui_font_family(), font_size)
                .with_color(text_color)
                .with_style(Properties::default().weight(font_weight))
                .finish(),
        )
        .with_uniform_padding(6.)
        .with_background(chip_color)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
        .with_margin_left(10.)
        .finish()
    }

    fn render_item_list(
        &self,
        items: Vec<Item>,
        mouse_state_handles: Vec<MouseStateHandle>,
        view: &TeamsPageView,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let all_items = items
            .iter()
            .sorted()
            .zip(mouse_state_handles.iter())
            .enumerate()
            .map(|(idx, (item, handle))| {
                let mut row = Flex::row()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_child(
                        Shrinkable::new(
                            1.,
                            Text::new_inline(
                                item.text.clone(),
                                appearance.ui_font_family(),
                                appearance.ui_font_size() - 1.,
                            )
                            .with_color(appearance.theme().active_ui_text_color().into())
                            .finish(),
                        )
                        .finish(),
                    );

                let mut pending_and_close_row = Flex::row()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Min);

                match item.state {
                    ItemState::Expired => {
                        pending_and_close_row.add_child(
                            self.render_state_chip(
                                appearance,
                                "EXPIRED".into(),
                                appearance.theme().ui_error_color(),
                                themes::theme::Fill::from(appearance.theme().ui_error_color())
                                    .with_opacity(30)
                                    .into(),
                                appearance.ui_font_size() - 1.,
                                Weight::Normal,
                            ),
                        );
                    }
                    ItemState::Pending => {
                        pending_and_close_row.add_child(
                            self.render_state_chip(
                                appearance,
                                "PENDING".into(),
                                *EMAIL_INVITE_PENDING_COLOR,
                                themes::theme::Fill::from(*EMAIL_INVITE_PENDING_COLOR)
                                    .with_opacity(30)
                                    .into(),
                                appearance.ui_font_size() - 1.,
                                Weight::Normal,
                            ),
                        );
                    }
                    ItemState::Owner => {
                        pending_and_close_row.add_child(self.render_state_chip(
                            appearance,
                            "OWNER".into(),
                            appearance.theme().accent().into(),
                            appearance.theme().accent().with_opacity(30).into(),
                            appearance.ui_font_size() - 1.,
                            Weight::Normal,
                        ));
                    }
                    ItemState::Admin => {
                        pending_and_close_row.add_child(
                            self.render_state_chip(
                                appearance,
                                "ADMIN".into(),
                                appearance
                                    .theme()
                                    .background()
                                    .blend(&appearance.theme().foreground().with_opacity(60))
                                    .into(),
                                appearance
                                    .theme()
                                    .background()
                                    .blend(&appearance.theme().foreground().with_opacity(25))
                                    .into(),
                                appearance.ui_font_size() - 1.,
                                Weight::Normal,
                            ),
                        );
                    }
                    ItemState::Valid => (),
                }

                match item.actions.len() {
                    0 => {
                        // No actions - no button
                    }
                    1 => {
                        // Single action - show the action's icon
                        let item_action = &item.actions[0];
                        let action = item_action.action.clone();
                        let icon = item_action.icon;
                        pending_and_close_row.add_child(
                            Container::new(
                                Hoverable::new(handle.clone(), move |_mouse_state| {
                                    Container::new(
                                        ConstrainedBox::new(
                                            icon.to_warpui_icon(
                                                appearance
                                                    .theme()
                                                    .active_ui_text_color()
                                                    .with_opacity(70),
                                            )
                                            .finish(),
                                        )
                                        .with_max_height(CLOSE_BUTTON_ICON_SIZE)
                                        .with_max_width(CLOSE_BUTTON_ICON_SIZE)
                                        .finish(),
                                    )
                                    .with_uniform_padding(2.)
                                    .finish()
                                })
                                .with_cursor(Cursor::PointingHand)
                                .on_click(move |ctx, _, _| {
                                    ctx.dispatch_typed_action(action.clone())
                                })
                                .finish(),
                            )
                            .with_margin_left(10.)
                            .finish(),
                        );
                    }
                    _ => {
                        // Multiple actions - show dots menu
                        let menu_is_open = view.open_member_actions_menu_index == Some(idx);
                        let mut stack = Stack::new();
                        let dots_button = Hoverable::new(handle.clone(), |_mouse_state| {
                            Container::new(
                                ConstrainedBox::new(
                                    Icon::DotsVertical
                                        .to_warpui_icon(
                                            appearance
                                                .theme()
                                                .active_ui_text_color()
                                                .with_opacity(70),
                                        )
                                        .finish(),
                                )
                                .with_max_height(CLOSE_BUTTON_ICON_SIZE)
                                .with_max_width(CLOSE_BUTTON_ICON_SIZE)
                                .finish(),
                            )
                            .with_uniform_padding(2.)
                            .finish()
                        })
                        .with_cursor(Cursor::PointingHand)
                        .on_click(move |ctx, _, _| {
                            if menu_is_open {
                                ctx.dispatch_typed_action(TeamsPageAction::CloseMemberActionsMenu);
                            } else {
                                ctx.dispatch_typed_action(TeamsPageAction::OpenMemberActionsMenu {
                                    index: idx,
                                });
                            }
                        })
                        .finish();
                        stack.add_child(dots_button);

                        // Show menu if this is the open index
                        if view.open_member_actions_menu_index == Some(idx) {
                            stack.add_positioned_overlay_child(
                                ChildView::new(&view.member_actions_menu).finish(),
                                OffsetPositioning::offset_from_parent(
                                    vec2f(0., 0.),
                                    ParentOffsetBounds::WindowByPosition,
                                    ParentAnchor::BottomRight,
                                    ChildAnchor::TopRight,
                                ),
                            );
                        }

                        pending_and_close_row.add_child(
                            Container::new(stack.finish())
                                .with_margin_left(10.)
                                .finish(),
                        );
                    }
                }

                row.add_child(pending_and_close_row.finish());

                let list_element =
                    Container::new(row.finish()).with_uniform_padding(SCROLLABLE_LIST_ITEM_PADDING);

                let container = if idx % 2 == 0 {
                    list_element
                        .with_background(internal_colors::fg_overlay_1(appearance.theme()))
                        .finish()
                } else {
                    list_element.finish()
                };

                container
            })
            .collect::<Vec<_>>()
            .into_iter();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_children(all_items)
            .finish()
    }

    fn render_copy_link_row(
        &self,
        team_metadata: &Team,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut section = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        let (link_text, button_enabled) = match &team_metadata.invite_code {
            Some(invite_code) => {
                let link = format!(
                    "{}{}{}",
                    ChannelState::server_root_url(),
                    INVITE_LINK_PREFIX,
                    invite_code.code
                );
                (link, true)
            }
            None => ("Failed to load invite link.".into(), false),
        };
        let theme = appearance.theme();

        let mut copy_button = appearance
            .ui_builder()
            .copy_button(14., self.mouse_state_handles.copy_link_button.clone())
            .build();

        if !button_enabled {
            copy_button = copy_button.disable();
        }

        // 1) Invite link (with copy button)
        section.add_child(
            Shrinkable::new(
                1.,
                Container::new(
                    Flex::row()
                        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_children([
                            Shrinkable::new(
                                1.,
                                Container::new(
                                    Align::new(
                                        appearance
                                            .ui_builder()
                                            .span(link_text.clone())
                                            .with_style(UiComponentStyles {
                                                font_color: Some(
                                                    theme
                                                        .main_text_color(theme.background())
                                                        .into_solid(),
                                                ),
                                                ..Default::default()
                                            })
                                            .build()
                                            .finish(),
                                    )
                                    .left()
                                    .finish(),
                                )
                                .with_padding_bottom(10.)
                                .with_padding_top(10.)
                                .finish(),
                            )
                            .finish(),
                            copy_button
                                .on_click(move |ctx, _, _| {
                                    ctx.dispatch_typed_action(TeamsPageAction::CopyLink(
                                        link_text.clone(),
                                    ));
                                })
                                .finish(),
                        ])
                        .finish(),
                )
                .with_background(theme.background())
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_padding_left(8.)
                .with_padding_right(8.)
                .finish(),
            )
            .finish(),
        );

        Container::new(section.finish())
            .with_margin_top(TEXT_FIELD_TOP_PADDING)
            .finish()
    }

    fn render_sub_header(&self, text: String, appearance: &Appearance) -> Box<dyn Element> {
        Align::new(
            appearance
                .ui_builder()
                .span(text)
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_weight: Some(Weight::Light),
                    font_color: Some(
                        appearance
                            .theme()
                            .active_ui_text_color()
                            .with_opacity(90)
                            .into(),
                    ),
                    font_size: Some(12.),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .left()
        .finish()
    }

    fn render_subsection_header(&self, text: String, appearance: &Appearance) -> Box<dyn Element> {
        Align::new(
            appearance
                .ui_builder()
                .span(text)
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_weight: Some(Weight::Medium),
                    font_color: Some(appearance.theme().active_ui_text_color().into()),
                    font_size: Some(SUBSECTION_HEADER_FONT_SIZE),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .left()
        .finish()
    }

    /// Smaller in-page header used under a `render_subsection_header`
    /// (e.g. "By link" / "By email" / "By discovery").
    fn render_subsubsection_header(
        &self,
        text: String,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Align::new(
            appearance
                .ui_builder()
                .span(text)
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_weight: Some(Weight::Medium),
                    font_color: Some(appearance.theme().active_ui_text_color().into()),
                    font_size: Some(SUBSUBSECTION_HEADER_FONT_SIZE),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .left()
        .finish()
    }

    fn render_description(&self, text: String, appearance: &Appearance) -> Box<dyn Element> {
        Text::new(text, appearance.ui_font_family(), 12.)
            .with_color(
                appearance
                    .theme()
                    .active_ui_text_color()
                    .with_opacity(90)
                    .into(),
            )
            .finish()
    }

    fn render_sub_header_with_subtext_color(
        &self,
        appearance: &Appearance,
        text: String,
    ) -> Box<dyn Element> {
        Container::new(
            Align::new(
                appearance
                    .ui_builder()
                    .span(text)
                    .with_style(UiComponentStyles {
                        font_family_id: Some(appearance.ui_font_family()),
                        font_color: Some(
                            appearance
                                .theme()
                                .sub_text_color(appearance.theme().background())
                                .into_solid(),
                        ),
                        font_size: Some(14.),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .left()
            .finish(),
        )
        .with_padding_bottom(4.)
        .finish()
    }

    fn render_plan_usage_header(&self, text: String, appearance: &Appearance) -> Box<dyn Element> {
        Align::new(
            appearance
                .ui_builder()
                .span(text)
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_weight: Some(Weight::Light),
                    font_color: Some(
                        appearance
                            .theme()
                            .active_ui_text_color()
                            .with_opacity(40)
                            .into(),
                    ),
                    font_size: Some(13.),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .left()
        .finish()
    }

    fn render_plan_usage_text(&self, text: String, appearance: &Appearance) -> Box<dyn Element> {
        Align::new(
            appearance
                .ui_builder()
                .span(text)
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_weight: Some(Weight::Light),
                    font_color: Some(
                        appearance
                            .theme()
                            .active_ui_text_color()
                            .with_opacity(60)
                            .into(),
                    ),
                    font_size: Some(20.),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .left()
        .finish()
    }

    fn render_sub_text(
        &self,
        text: String,
        appearance: &Appearance,
        margin: Option<Coords>,
    ) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .paragraph(text)
            .with_style(UiComponentStyles {
                margin,
                font_color: Some(
                    appearance
                        .theme()
                        .active_ui_text_color()
                        .with_opacity(60)
                        .into(),
                ),
                ..Default::default()
            })
            .build()
            .finish()
    }

    fn render_error_sub_text(&self, text: String, appearance: &Appearance) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .paragraph(text)
            .with_style(UiComponentStyles {
                margin: Some(Coords::uniform(0.).right(48.)),
                font_color: Some(appearance.theme().ui_error_color()),
                ..Default::default()
            })
            .build()
            .finish()
    }

    fn render_create_team_page_with_banner(
        &self,
        view: &TeamsPageView,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut column = Flex::column();

        column.add_child(self.render_create_team_page(view, appearance, app));

        column.finish()
    }

    fn render_create_team_page(
        &self,
        view: &TeamsPageView,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut page = Flex::column();

        // Title, subtitle, and description
        page.add_child(render_sub_header(appearance, "Teams".to_string(), None));
        page.add_child(
            self.render_sub_header_with_subtext_color(appearance, "Create a team".to_string()),
        );
        page.add_child(
            Container::new(
                self.render_description(CREATE_TEAM_DESCRIPTION.to_string(), appearance),
            )
            .with_padding_top(6.)
            .finish(),
        );

        if view.auth_state.is_on_work_domain().unwrap_or_default() {
            let checkbox = Container::new(
                appearance
                    .ui_builder()
                    .checkbox(self.mouse_state_handles.checkbox_mouse_state.clone(), None)
                    .check(view.checkbox_value)
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(
                            TeamsPageAction::ToggleTeamDiscoverabilityBeforeCreation,
                        )
                    })
                    .finish(),
            )
            .with_margin_left(-4.)
            .finish();
            let checkbox_row_text = if let Some(domain) = view.auth_state.user_email_domain() {
                format!("Allow Warp users with an @{domain} email to find and join the team.")
            } else {
                "Allow Warp users with the same email domain as you to find and join the team."
                    .to_string()
            };
            let checkbox_row = Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(checkbox)
                    .with_child(
                        Shrinkable::new(1., self.render_description(checkbox_row_text, appearance))
                            .finish(),
                    )
                    .finish(),
            )
            .with_padding_top(6.)
            .finish();
            page.add_child(checkbox_row);
        }

        // Team name editor
        page.add_child(
            Container::new(self.render_create_team_actions(view, appearance, app))
                .with_padding_top(12.)
                .with_padding_bottom(12.)
                .finish(),
        );

        if !view.discoverable_teams_states.is_empty() {
            // Separator and subtitle
            page.add_child(render_separator(appearance));
            page.add_child(self.render_sub_header_with_subtext_color(
                appearance,
                "Or, join an existing team within your company".to_string(),
            ));

            // Team discovery
            page.add_child(self.render_team_discovery_section(view, appearance));
        }

        page.finish()
    }

    fn render_create_team_actions(
        &self,
        view: &TeamsPageView,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        Flex::column()
            .with_child(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_child(
                        Shrinkable::new(
                            1.,
                            self.render_editor(appearance, view.create_team_editor.clone(), None),
                        )
                        .finish(),
                    )
                    .with_child(
                        Container::new(self.render_create_team_button(view, appearance, app))
                            .with_padding_left(CREATE_TEAM_BUTTON_LEFT_PADDING)
                            .finish(),
                    )
                    .finish(),
            )
            .finish()
    }

    fn render_team_discovery_section(
        &self,
        view: &TeamsPageView,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut team_discovery = Flex::column();
        // Sort teams so teams accepting invites with most teammates appear on top
        let mut sorted_teams = view.discoverable_teams_states.clone();
        sorted_teams.sort_by_key(|team_state| {
            (
                !team_state.team.team_accepting_invites,
                -team_state.team.num_members,
            )
        });

        // Render box for each team
        for team_state in &sorted_teams {
            team_discovery.add_child(
                Container::new(self.render_single_team_in_team_discovery(team_state, appearance))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                    .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
                    .with_uniform_padding(16.)
                    .with_margin_top(12.)
                    .finish(),
            );
        }
        team_discovery.finish()
    }

    fn render_single_team_in_team_discovery(
        &self,
        team_state: &DiscoverableTeamState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut single_team = Flex::column();

        // Team name
        single_team.add_child(self.render_sub_header(team_state.team.name.to_string(), appearance));

        // Number of teammates
        let teammate_string = if team_state.team.num_members == 1 {
            "1 teammate".to_string()
        } else {
            format!("{} teammates", team_state.team.num_members)
        };
        single_team.add_child(self.render_sub_text(teammate_string, appearance, None));

        // Call to action
        single_team.add_child(
            Container::new(
                self.render_sub_text(
                    "Join this team and start collaborating on workflows, notebooks, and more."
                        .to_string(),
                    appearance,
                    None,
                ),
            )
            .with_padding_top(12.)
            .with_padding_bottom(12.)
            .finish(),
        );

        // Join button
        single_team.add_child(
            Container::new(self.render_join_team_button(team_state, appearance))
                .with_padding_top(12.)
                .finish(),
        );

        single_team.finish()
    }

    fn render_editor(
        &self,
        appearance: &Appearance,
        editor: ViewHandle<EditorView>,
        width: Option<f32>,
    ) -> Box<dyn Element> {
        let element = ConstrainedBox::new(
            appearance
                .ui_builder()
                .text_input(editor)
                .with_style(UiComponentStyles {
                    background: Some(appearance.theme().background().into()),
                    font_color: Some(
                        appearance
                            .theme()
                            .main_text_color(appearance.theme().background())
                            .into_solid(),
                    ),
                    ..Default::default()
                })
                .build()
                .finish(),
        );

        if let Some(element_width) = width {
            element.with_width(element_width).finish()
        } else {
            element.finish()
        }
    }

    fn render_compare_plans_button(
        &self,
        text: &str,
        mouse_state_handle: MouseStateHandle,
        team_uid: ServerId,
        appearance: &Appearance,
        style: Option<UiComponentStyles>,
    ) -> Box<dyn Element> {
        let icon_color = appearance.theme().accent();

        let mut button = appearance
            .ui_builder()
            .button(ButtonVariant::Link, mouse_state_handle)
            .with_text_and_icon_label(
                TextAndIcon::new(
                    TextAndIconAlignment::IconFirst,
                    text.to_string(),
                    Icon::CoinsStacked.to_warpui_icon(icon_color),
                    MainAxisSize::Min,
                    MainAxisAlignment::Center,
                    vec2f(14., 14.),
                )
                .with_inner_padding(4.),
            );

        if let Some(style) = style {
            button = button.with_style(style);
        }

        button
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(TeamsPageAction::GenerateUpgradeLink { team_uid });
            })
            .finish()
    }

    fn render_button(
        &self,
        label: &str,
        variant: ButtonVariant,
        mouse_state_handle: MouseStateHandle,
        action: Option<TeamsPageAction>,
        styles: UiComponentStyles,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut builder = appearance
            .ui_builder()
            .button(variant, mouse_state_handle)
            .with_style(styles)
            .with_centered_text_label(label.to_owned());

        // No action → render as truly disabled, otherwise hover styling still applies.
        if action.is_none() {
            builder = builder.disabled();
        }
        let button = builder.build();

        if let Some(action) = action {
            button
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
                .finish()
        } else {
            button.finish()
        }
    }

    fn render_create_team_button(
        &self,
        view: &TeamsPageView,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                self.mouse_state_handles.create_team_button.clone(),
            )
            .with_centered_text_label(CREATE_TEAM_BUTTON_LABEL.to_owned())
            .with_style(UiComponentStyles {
                font_color: Some(
                    appearance
                        .theme()
                        .main_text_color(appearance.theme().accent())
                        .into_solid(),
                ),
                font_weight: Some(Weight::Medium),
                width: Some(100.),
                height: Some(38.),
                font_size: Some(14.),
                ..Default::default()
            });

        if view
            .create_team_editor
            .as_ref(app)
            .buffer_text(app)
            .trim()
            .is_empty()
        {
            button = button
                .with_style(UiComponentStyles {
                    font_color: Some(
                        appearance
                            .theme()
                            .disabled_text_color(appearance.theme().background())
                            .into(),
                    ),
                    ..Default::default()
                })
                .disabled();
        }

        button
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(TeamsPageAction::CreateTeam))
            .finish()
    }

    fn render_join_team_button(
        &self,
        team_state: &DiscoverableTeamState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        if team_state.team.team_accepting_invites {
            self.render_button(
                "Join",
                ButtonVariant::Accent,
                team_state.mouse_state_handle.clone(),
                Some(TeamsPageAction::JoinTeamWithTeamDiscovery {
                    team_uid: ServerId::from_string_lossy(&team_state.team.team_uid),
                }),
                UiComponentStyles {
                    font_color: Some(
                        appearance
                            .theme()
                            .main_text_color(appearance.theme().accent())
                            .into_solid(),
                    ),
                    font_weight: Some(Weight::Medium),
                    height: Some(38.),
                    font_size: Some(14.),
                    ..Default::default()
                },
                appearance,
            )
        } else {
            appearance
                .ui_builder()
                .button(ButtonVariant::Accent, team_state.mouse_state_handle.clone())
                .with_style(UiComponentStyles {
                    font_weight: Some(Weight::Medium),
                    height: Some(38.),
                    font_size: Some(14.),
                    ..Default::default()
                })
                .with_centered_text_label("Contact Admin to request access".to_string())
                .disabled()
                .build()
                .finish()
        }
    }
}

impl SettingsWidget for TeamsWidget {
    type View = TeamsPageView;

    fn search_terms(&self) -> &str {
        "invites teams team members"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        // Main teams content: create a team, error state, team management.
        // We only want to show the teams page if the user is online. Otherwise,
        // we may not have some of the data we need to render the page, and
        // furthermore, it wouldn't be useful because the user can't do anything
        // related to team administration when offline.
        let content = if NetworkStatus::as_ref(app).is_online() {
            let teams = view.user_workspaces.as_ref(app);
            let cloud_model = view.cloud_model.as_ref(app);
            let ai_request_usage_model = view.ai_request_usage_model.as_ref(app);

            match teams.current_team() {
                Some(team) => self.render_team_management_page(
                    team,
                    cloud_model,
                    ai_request_usage_model,
                    view,
                    appearance,
                    app,
                ),
                None => self.render_create_team_page_with_banner(view, appearance, app),
            }
        } else {
            appearance
                .ui_builder()
                .span(OFFLINE_TEXT.to_string())
                .build()
                .finish()
        };

        let mut stack = Stack::new();
        stack.add_child(Flex::column().with_child(content).finish());

        if view.transfer_ownership_modal_state.is_open() {
            stack.add_positioned_overlay_child(
                view.transfer_ownership_modal_state.render(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            );
        }

        stack.finish()
    }
}

#[cfg(test)]
#[test]
pub fn test_valid_domains() {
    assert!(!TeamsPageView::is_valid_domain("@warp.dev"));
    assert!(!TeamsPageView::is_valid_domain("warp,"));
    assert!(!TeamsPageView::is_valid_domain("warpdev"));
    assert!(!TeamsPageView::is_valid_domain(".dev"));
    assert!(!TeamsPageView::is_valid_domain("warp..dev"));
    assert!(!TeamsPageView::is_valid_domain(" "));
    assert!(!TeamsPageView::is_valid_domain("warp!.dev"));
    assert!(!TeamsPageView::is_valid_domain("warp.dev>"));
    assert!(!TeamsPageView::is_valid_domain("warp.dev."));
    assert!(TeamsPageView::is_valid_domain("app.warp.dev"));
    assert!(TeamsPageView::is_valid_domain("warp0.dev0"));
    assert!(TeamsPageView::is_valid_domain("warp.dev"));
    assert!(TeamsPageView::is_valid_domain("miniclip.com"));
}
