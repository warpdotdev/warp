use warpui::elements::{
    Align, ChildView, ConstrainedBox, Container, CrossAxisAlignment, Expanded, Flex,
    MainAxisAlignment, MainAxisSize, ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
use crate::view_components::action_button::{ActionButton, PrimaryTheme, SecondaryTheme};
use crate::workspaces::user_workspaces::UserWorkspaces;

pub(crate) const TITLE: &str = "Cloud agents need a team";
pub(crate) const BODY: &str = "You’re in this workspace but not on a team, so you can’t start cloud runs. Join or create a team, then try again.";

pub(crate) fn should_render(team_required: bool, is_in_setup: bool, is_configuring: bool) -> bool {
    team_required && (is_in_setup || is_configuring)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloudAgentTeamRequiredViewEvent {
    OpenTeamsSettings,
    OpenAdminPanel,
    BackToTerminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrimaryCta {
    OpenTeamsSettings,
    OpenAdminPanel,
}

impl PrimaryCta {
    fn for_workspace_admin(is_workspace_admin: bool) -> Self {
        if is_workspace_admin {
            Self::OpenAdminPanel
        } else {
            Self::OpenTeamsSettings
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::OpenTeamsSettings => "Open Teams settings",
            Self::OpenAdminPanel => "Open admin panel",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloudAgentTeamRequiredViewAction {
    OpenTeamsSettings,
    OpenAdminPanel,
    BackToTerminal,
}

pub struct CloudAgentTeamRequiredView {
    open_teams_settings_button: ViewHandle<ActionButton>,
    open_admin_panel_button: ViewHandle<ActionButton>,
    back_to_terminal_button: ViewHandle<ActionButton>,
}

impl CloudAgentTeamRequiredView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let open_teams_settings_button = ctx.add_typed_action_view(|_| {
            ActionButton::new(PrimaryCta::OpenTeamsSettings.label(), PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(CloudAgentTeamRequiredViewAction::OpenTeamsSettings);
            })
        });
        let open_admin_panel_button = ctx.add_typed_action_view(|_| {
            ActionButton::new(PrimaryCta::OpenAdminPanel.label(), PrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(CloudAgentTeamRequiredViewAction::OpenAdminPanel);
            })
        });
        let back_to_terminal_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Back to terminal", SecondaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(CloudAgentTeamRequiredViewAction::BackToTerminal);
            })
        });

        Self {
            open_teams_settings_button,
            open_admin_panel_button,
            back_to_terminal_button,
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
            CloudAgentTeamRequiredViewAction::OpenAdminPanel => {
                CloudAgentTeamRequiredViewEvent::OpenAdminPanel
            }
            CloudAgentTeamRequiredViewAction::BackToTerminal => {
                CloudAgentTeamRequiredViewEvent::BackToTerminal
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
        let user_email = AuthStateProvider::as_ref(app).get().user_email();
        let is_workspace_admin = UserWorkspaces::as_ref(app)
            .current_workspace()
            .zip(user_email.as_deref())
            .is_some_and(|(workspace, email)| workspace.is_workspace_admin(email));
        let primary_button = match PrimaryCta::for_workspace_admin(is_workspace_admin) {
            PrimaryCta::OpenTeamsSettings => &self.open_teams_settings_button,
            PrimaryCta::OpenAdminPanel => &self.open_admin_panel_button,
        };

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
                Text::new(BODY, appearance.ui_font_family(), appearance.ui_font_size())
                    .with_color(theme.nonactive_ui_text_color().into_solid())
                    .soft_wrap(true)
                    .finish(),
            )
            .with_child(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Start)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(8.)
                    .with_child(ChildView::new(primary_button).finish())
                    .with_child(ChildView::new(&self.back_to_terminal_button).finish())
                    .finish(),
            )
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

#[cfg(test)]
#[path = "team_required_tests.rs"]
mod tests;
