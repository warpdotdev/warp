use warpui::elements::{
    Border, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Expanded, Flex, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentElement, Radius, ScrollbarWidth, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::appearance::Appearance;
use crate::server::ids::ServerId;
use crate::workspaces::team::DiscoverableTeam;
const SUBTITLE: &str = "You can join any open team in your Warp workspace.";
const MAX_VISIBLE_TEAMS: usize = 4;
const TEAM_LIST_MAX_HEIGHT: f32 = 264.;

struct JoinableTeamState {
    team: DiscoverableTeam,
    join_button_mouse_state: MouseStateHandle,
}

pub(crate) struct JoinTeamsModal {
    teams: Vec<JoinableTeamState>,
    joining_team_uid: Option<ServerId>,
    scroll_state: ClippedScrollStateHandle,
}

#[derive(Debug)]
pub(crate) enum JoinTeamsModalAction {
    Join { team_uid: ServerId },
}

pub(crate) enum JoinTeamsModalEvent {
    Join { team_uid: ServerId },
}

impl JoinTeamsModal {
    pub(crate) fn new() -> Self {
        Self {
            teams: Vec::new(),
            joining_team_uid: None,
            scroll_state: Default::default(),
        }
    }

    pub(crate) fn set_teams(
        &mut self,
        mut teams: Vec<DiscoverableTeam>,
        ctx: &mut ViewContext<Self>,
    ) {
        teams.sort_by_key(|team| (-team.num_members, team.name.to_ascii_lowercase()));
        self.teams = teams
            .into_iter()
            .map(|team| JoinableTeamState {
                team,
                join_button_mouse_state: Default::default(),
            })
            .collect();
        self.joining_team_uid = None;
        self.scroll_state = Default::default();
        ctx.notify();
    }

    pub(crate) fn set_joining_team(
        &mut self,
        team_uid: Option<ServerId>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.joining_team_uid = team_uid;
        ctx.notify();
    }

    fn render_team(
        &self,
        team_state: &JoinableTeamState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let team_uid = ServerId::from_string_lossy(&team_state.team.team_uid);
        let is_joining = self.joining_team_uid == Some(team_uid);
        let teammate_count = match team_state.team.num_members {
            1 => "1 teammate".to_string(),
            count => format!("{count} teammates"),
        };

        let team_details = Flex::column()
            .with_child(
                Text::new_inline(
                    team_state.team.name.clone(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_style(Properties::default().weight(Weight::Medium))
                .with_color(theme.active_ui_text_color().into())
                .finish(),
            )
            .with_child(
                Text::new_inline(
                    teammate_count,
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(theme.sub_text_color(theme.surface_2()).into())
                .finish(),
            )
            .finish();

        let mut join_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                team_state.join_button_mouse_state.clone(),
            )
            .with_centered_text_label(if is_joining {
                "Joining…".to_string()
            } else {
                "Join".to_string()
            })
            .with_style(UiComponentStyles {
                width: Some(88.),
                height: Some(36.),
                font_weight: Some(Weight::Medium),
                ..Default::default()
            });
        if self.joining_team_uid.is_some() {
            join_button = join_button.disabled();
        }

        let join_button = join_button
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(JoinTeamsModalAction::Join { team_uid });
            })
            .finish();

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Expanded::new(1., team_details).finish())
                .with_child(join_button)
                .finish(),
        )
        .with_uniform_padding(12.)
        .with_border(Border::all(1.).with_border_fill(theme.outline()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .finish()
    }
}

impl Entity for JoinTeamsModal {
    type Event = JoinTeamsModalEvent;
}

impl View for JoinTeamsModal {
    fn ui_name() -> &'static str {
        "JoinTeamsModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let mut teams = Flex::column().with_spacing(8.);
        for team in &self.teams {
            teams.add_child(self.render_team(team, appearance));
        }
        let teams = teams.finish();
        let teams = if self.teams.len() > MAX_VISIBLE_TEAMS {
            ConstrainedBox::new(
                ClippedScrollable::vertical(
                    self.scroll_state.clone(),
                    teams,
                    ScrollbarWidth::Auto,
                    theme.nonactive_ui_text_color().into(),
                    theme.active_ui_text_color().into(),
                    warpui::elements::Fill::None,
                )
                .finish(),
            )
            .with_height(TEAM_LIST_MAX_HEIGHT)
            .finish()
        } else {
            teams
        };

        Flex::column()
            .with_child(
                Text::new(
                    SUBTITLE.to_string(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(theme.sub_text_color(theme.surface_2()).into())
                .finish(),
            )
            .with_child(Container::new(teams).with_margin_top(16.).finish())
            .finish()
    }
}

impl TypedActionView for JoinTeamsModal {
    type Action = JoinTeamsModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            JoinTeamsModalAction::Join { team_uid } if self.joining_team_uid.is_none() => {
                self.set_joining_team(Some(*team_uid), ctx);
                ctx.emit(JoinTeamsModalEvent::Join {
                    team_uid: *team_uid,
                });
            }
            JoinTeamsModalAction::Join { .. } => {}
        }
    }
}
