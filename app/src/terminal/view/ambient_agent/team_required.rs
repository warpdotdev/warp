use warpui::elements::{
    Align, ChildView, ConstrainedBox, Container, CrossAxisAlignment, Expanded, Flex, ParentElement,
    Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
use crate::view_components::action_button::{ActionButton, PrimaryTheme};
use crate::workspaces::user_workspaces::UserWorkspaces;

pub(crate) const TITLE: &str = "Cloud agents need a team";
pub(crate) const BODY_ADMIN: &str = "You’re in this workspace but not on a team, so you can’t start cloud runs. Join or create a team, then try again.";
pub(crate) const BODY_NON_ADMIN: &str = "You’re in this workspace but not on a team, so you can’t start cloud runs. Join a team, then try again.";
pub(crate) const PRIMARY_CTA_LABEL: &str = "Open Teams settings";
pub(crate) const TOAST_ADMIN: &str =
    "Cloud agents need a team. Join or create a team to start cloud runs.";
pub(crate) const TOAST_NON_ADMIN: &str =
    "Cloud agents need a team. Join a team to start cloud runs.";

fn is_current_user_workspace_admin(app: &AppContext) -> bool {
    let Some(email) = AuthStateProvider::as_ref(app).get().user_email() else {
        return false;
    };
    UserWorkspaces::as_ref(app)
        .current_workspace()
        .is_some_and(|workspace| workspace.is_workspace_admin(&email))
}

fn body_text(app: &AppContext) -> &'static str {
    if is_current_user_workspace_admin(app) {
        BODY_ADMIN
    } else {
        BODY_NON_ADMIN
    }
}

pub(crate) fn should_render(team_required: bool, is_in_setup: bool, is_configuring: bool) -> bool {
    team_required && (is_in_setup || is_configuring)
}

/// Toast copy for submissions blocked by the team requirement, mirroring the
/// admin/non-admin split of the inline blocker's body text.
pub(crate) fn toast_message(app: &AppContext) -> &'static str {
    if is_current_user_workspace_admin(app) {
        TOAST_ADMIN
    } else {
        TOAST_NON_ADMIN
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloudAgentTeamRequiredViewEvent {
    OpenTeamsSettings,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloudAgentTeamRequiredViewAction {
    OpenTeamsSettings,
}

pub struct CloudAgentTeamRequiredView {
    open_teams_settings_button: ViewHandle<ActionButton>,
}

impl CloudAgentTeamRequiredView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let open_teams_settings_button = ctx.add_typed_action_view(|_| {
            ActionButton::new(PRIMARY_CTA_LABEL, PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(CloudAgentTeamRequiredViewAction::OpenTeamsSettings);
            })
        });

        Self {
            open_teams_settings_button,
        }
    }
}

impl Entity for CloudAgentTeamRequiredView {
    type Event = CloudAgentTeamRequiredViewEvent;
}

impl TypedActionView for CloudAgentTeamRequiredView {
    type Action = CloudAgentTeamRequiredViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        let event = match action {
            CloudAgentTeamRequiredViewAction::OpenTeamsSettings => {
                CloudAgentTeamRequiredViewEvent::OpenTeamsSettings
            }
        };
        ctx.emit(event);
    }
}

impl View for CloudAgentTeamRequiredView {
    fn ui_name() -> &'static str {
        "CloudAgentTeamRequiredView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(16.)
            .with_child(
                Text::new(TITLE, appearance.ui_font_family(), 20.)
                    .with_style(Properties::default().weight(Weight::Semibold))
                    .with_color(theme.active_ui_text_color().into_solid())
                    .finish(),
            )
            .with_child(
                Text::new(
                    body_text(app),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(theme.nonactive_ui_text_color().into_solid())
                .soft_wrap(true)
                .finish(),
            )
            .with_child(ChildView::new(&self.open_teams_settings_button).finish())
            .finish();

        Flex::column()
            .with_child(
                Expanded::new(
                    1.,
                    Container::new(
                        Align::new(ConstrainedBox::new(content).with_max_width(480.).finish())
                            .finish(),
                    )
                    .with_background(theme.background())
                    .with_uniform_padding(24.)
                    .finish(),
                )
                .finish(),
            )
            .finish()
    }
}
