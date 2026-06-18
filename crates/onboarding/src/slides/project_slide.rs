use ui_components::{button, keyboard_shortcut, Component as _, Options as _};
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::color::coloru_with_opacity;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::Icon;
use warpui_core::elements::{
    Align, ClippedScrollStateHandle, ConstrainedBox, Container, CrossAxisAlignment, Flex,
    MouseStateHandle, ParentElement, Shrinkable,
};
use warpui_core::fonts::Weight;
use warpui_core::keymap::Keystroke;
use warpui_core::platform::file_picker::{FilePickerConfiguration, FilePickerError};
use warpui_core::prelude::{MainAxisAlignment, MainAxisSize, Vector2F};
use warpui_core::ui_components::button::{ButtonVariant, TextAndIcon, TextAndIconAlignment};
use warpui_core::ui_components::components::{UiComponent as _, UiComponentStyles};
use warpui_core::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity as _, TypedActionView, View,
    ViewContext,
};

use super::OnboardingSlide;
use crate::model::OnboardingStateModel;
use crate::slides::{bottom_nav, layout, slide_content};
use crate::telemetry::OnboardingEvent;
use crate::visuals::project_visual;

const LEFT_COLUMN_W: f32 = 428.;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ProjectOnboardingSettings {
    #[default]
    NoProject,
    Project {
        selected_local_folder: String,
        initialize_projects_automatically: bool,
    },
}

impl ProjectOnboardingSettings {
    pub fn from_path(path: Option<String>) -> Self {
        match path {
            None => ProjectOnboardingSettings::NoProject,
            Some(path) => ProjectOnboardingSettings::Project {
                selected_local_folder: path,
                initialize_projects_automatically: true,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub enum ProjectSlideAction {
    BackClicked,
    NextClicked,
    SkipClicked,
    OpenLocalFolderClicked,
    LocalFolderSelected(Result<String, FilePickerError>),
    ToggleInitializeProjectsAutomatically,
}

pub struct ProjectSlide {
    onboarding_state: ModelHandle<OnboardingStateModel>,
    open_folder_mouse_state: MouseStateHandle,
    back_button: button::Button,
    next_button: button::Button,
    initialize_projects_automatically_mouse_state: MouseStateHandle,
    scroll_state: ClippedScrollStateHandle,
}

impl ProjectSlide {
    pub(crate) fn new(onboarding_state: ModelHandle<OnboardingStateModel>) -> Self {
        Self {
            onboarding_state,
            open_folder_mouse_state: MouseStateHandle::default(),
            back_button: button::Button::default(),
            next_button: button::Button::default(),
            initialize_projects_automatically_mouse_state: MouseStateHandle::default(),
            scroll_state: ClippedScrollStateHandle::new(),
        }
    }

    fn project_settings<'a>(&self, app: &'a AppContext) -> &'a ProjectOnboardingSettings {
        self.onboarding_state.as_ref(app).project_settings()
    }

    fn render_content(
        &self,
        appearance: &Appearance,
        settings: &ProjectOnboardingSettings,
        agent_modality_enabled: bool,
    ) -> Box<dyn Element> {
        let mut children = vec![
            Align::new(self.render_header(appearance)).finish(),
            Align::new(self.render_open_folder_button(appearance, settings)).finish(),
        ];

        // Only show the "Initialize project automatically" checkbox when AgentView is NOT enabled.
        // When AgentView is enabled, initialization is handled differently through the callout flow.
        if !agent_modality_enabled {
            if let ProjectOnboardingSettings::Project {
                initialize_projects_automatically,
                ..
            } = settings
            {
                children.push(
                    Align::new(
                        self.render_project_options(*initialize_projects_automatically, appearance),
                    )
                    .finish(),
                );
            }
        }

        let bottom_nav = Align::new(self.render_bottom_nav(appearance, settings)).finish();
        slide_content::onboarding_slide_content(
            children,
            bottom_nav,
            self.scroll_state.clone(),
            appearance,
        )
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let title = appearance
            .ui_builder()
            .paragraph("Open a project")
            .with_style(UiComponentStyles {
                font_size: Some(36.),
                font_weight: Some(Weight::Medium),
                ..Default::default()
            })
            .build()
            .finish();

        let subtitle = appearance
            .ui_builder()
            .paragraph("Set up a project to optimize it for coding in Warp.")
            .with_style(UiComponentStyles {
                font_size: Some(20.),
                font_weight: Some(Weight::Normal),
                font_color: Some(internal_colors::text_sub(
                    appearance.theme(),
                    appearance.theme().background(),
                )),
                ..Default::default()
            })
            .build()
            .finish();

        ConstrainedBox::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(title)
                .with_child(Container::new(subtitle).with_margin_top(8.).finish())
                .finish(),
        )
        .with_max_width(LEFT_COLUMN_W)
        .finish()
    }

    fn render_open_folder_button(
        &self,
        appearance: &Appearance,
        settings: &ProjectOnboardingSettings,
    ) -> Box<dyn Element> {
        // Match the intention/agent layout: a wide button within the left column.

        let theme = appearance.theme();
        let (label, variant) = match settings {
            ProjectOnboardingSettings::Project {
                selected_local_folder,
                ..
            } => (
                TextAndIcon::new(
                    TextAndIconAlignment::IconFirst,
                    selected_local_folder.to_owned(),
                    Icon::Folder.to_warpui_icon(theme.foreground()),
                    MainAxisSize::Max,
                    MainAxisAlignment::Center,
                    Vector2F::new(16., 16.),
                )
                .with_inner_padding(8.),
                ButtonVariant::Secondary,
            ),
            ProjectOnboardingSettings::NoProject => {
                let enter = Keystroke::parse("enter").unwrap_or_default();
                let text_color = theme.foreground().into();
                let border_color = coloru_with_opacity(text_color, 60);

                let shortcut = keyboard_shortcut::KeyboardShortcut.render(
                    appearance,
                    keyboard_shortcut::Params {
                        keystroke: enter,
                        options: keyboard_shortcut::Options {
                            font_color: Some(text_color),
                            background: None,
                            border_fill: Some(border_color.into()),
                            sizing: keyboard_shortcut::Sizing {
                                font_size: 12.,
                                padding: 2.,
                            },
                        },
                    },
                );

                let folder_icon =
                    ConstrainedBox::new(Icon::Folder.to_warpui_icon(theme.foreground()).finish())
                        .with_width(16.)
                        .with_height(16.)
                        .finish();

                let folder_text = Container::new(
                    appearance
                        .ui_builder()
                        .paragraph("Open local folder")
                        .with_style(UiComponentStyles {
                            font_color: Some(text_color),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_margin_left(8.)
                .finish();

                let enter_shortcut = Container::new(shortcut).with_margin_left(8.).finish();

                let label = Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_children([folder_icon, folder_text, enter_shortcut])
                    .finish();

                let button = appearance
                    .ui_builder()
                    .button(ButtonVariant::Accent, self.open_folder_mouse_state.clone())
                    .with_custom_label(label)
                    .build()
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(ProjectSlideAction::OpenLocalFolderClicked)
                    })
                    .finish();

                return Container::new(
                    ConstrainedBox::new(button)
                        .with_width(LEFT_COLUMN_W)
                        .finish(),
                )
                .with_margin_top(24.)
                .finish();
            }
        };

        let button = appearance
            .ui_builder()
            .button(variant, self.open_folder_mouse_state.clone())
            .with_text_and_icon_label(label)
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(ProjectSlideAction::OpenLocalFolderClicked)
            })
            .finish();

        Container::new(
            ConstrainedBox::new(button)
                .with_width(LEFT_COLUMN_W)
                .finish(),
        )
        .with_margin_top(24.)
        .finish()
    }

    fn render_bottom_nav(
        &self,
        appearance: &Appearance,
        settings: &ProjectOnboardingSettings,
    ) -> Box<dyn Element> {
        let back_button = self.back_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Back".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(ProjectSlideAction::BackClicked);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let theme_picker_last =
            warp_core::features::FeatureFlag::OpenWarpNewSettingsModes.is_enabled();

        let (label, keystroke, action) = match settings {
            ProjectOnboardingSettings::Project { .. } => (
                if theme_picker_last {
                    "Next"
                } else {
                    "Get Warping"
                },
                Keystroke::parse("enter").unwrap_or_default(),
                ProjectSlideAction::NextClicked,
            ),
            ProjectOnboardingSettings::NoProject => (
                "Skip",
                Keystroke::parse("cmdorctrl-enter").unwrap_or_default(),
                ProjectSlideAction::SkipClicked,
            ),
        };

        let next_button = self.next_button.render(
            appearance,
            button::Params {
                content: button::Content::Label(label.into()),
                theme: &button::themes::Primary,
                options: button::Options {
                    keystroke: Some(keystroke),
                    on_click: Some(Box::new(move |ctx, _app, _pos| {
                        ctx.dispatch_typed_action(action.clone());
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        // The project slide is unreachable in the new flow (ThirdParty → ThemePicker),
        // so only the legacy step counts apply.
        let (step_index, step_count) = (3, 4);
        bottom_nav::onboarding_bottom_nav(
            appearance,
            step_index,
            step_count,
            Some(back_button),
            Some(next_button),
        )
    }

    fn render_option(
        &self,
        appearance: &Appearance,
        mouse_state: MouseStateHandle,
        checked: bool,
        title: &'static str,
        description: &'static str,
        action: ProjectSlideAction,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let action = action.clone();

        let checkbox = appearance
            .ui_builder()
            .checkbox(mouse_state, Some(12.))
            .check(checked)
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
            .finish();

        let title = appearance
            .ui_builder()
            .wrappable_text(title, true)
            .with_style(UiComponentStyles {
                font_size: Some(12.),
                font_weight: Some(Weight::Normal),
                font_color: Some(theme.sub_text_color(theme.background()).into_solid()),
                ..Default::default()
            })
            .build()
            .finish();

        let description = appearance
            .ui_builder()
            .wrappable_text(description, true)
            .with_style(UiComponentStyles {
                font_size: Some(12.),
                font_weight: Some(Weight::Normal),
                font_color: Some(theme.disabled_text_color(theme.background()).into_solid()),
                ..Default::default()
            })
            .build()
            .finish();

        let text_col = Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(title)
                .with_child(Container::new(description).with_margin_top(4.).finish())
                .finish(),
        )
        .with_uniform_padding(3.0)
        .finish();

        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(checkbox)
            .with_child(Shrinkable::new(1., text_col).finish())
            .finish()
    }

    fn render_project_options(
        &self,
        initialize_projects_automatically: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let initialize = self.render_option(
            appearance,
            self.initialize_projects_automatically_mouse_state.clone(),
            initialize_projects_automatically,
            "Initialize project automatically",
            "Prepares the project environment, builds an index of your code, and generates project rules—giving the agent deeper understanding and better performance.",
            ProjectSlideAction::ToggleInitializeProjectsAutomatically,
        );

        // Keep this aligned with the folder button width.
        ConstrainedBox::new(Container::new(initialize).with_margin_top(16.).finish())
            .with_width(LEFT_COLUMN_W)
            .finish()
    }

    fn render_visual(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let panel_background = internal_colors::neutral_2(theme);

        let pill_color = internal_colors::fg_overlay_1(theme).into_solid();
        let center_icon_color = internal_colors::neutral_5(theme);
        let side_icon_color = internal_colors::neutral_4(theme);

        Container::new(project_visual(
            panel_background,
            pill_color,
            center_icon_color,
            side_icon_color,
        ))
        .with_background_color(internal_colors::neutral_1(theme))
        .finish()
    }
}

impl Entity for ProjectSlide {
    type Event = ();
}

impl View for ProjectSlide {
    fn ui_name() -> &'static str {
        "ProjectSlide"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let settings = self.project_settings(app);
        let agent_modality_enabled = self.onboarding_state.as_ref(app).agent_modality_enabled();

        layout::static_left(
            || self.render_content(appearance, settings, agent_modality_enabled),
            || self.render_visual(appearance),
        )
    }
}

impl ProjectSlide {
    fn open_local_folder(&mut self, ctx: &mut ViewContext<Self>) {
        send_telemetry_from_ctx!(OnboardingEvent::FolderSelectionStarted, ctx);
        ctx.open_file_picker(
            |result, ctx| {
                if let Some(path_result) = result.map(|paths| paths.into_iter().next()).transpose()
                {
                    ctx.dispatch_typed_action(&ProjectSlideAction::LocalFolderSelected(
                        path_result,
                    ));
                }
            },
            FilePickerConfiguration::new().folders_only(),
        );
    }

    fn next(&mut self, ctx: &mut ViewContext<Self>) {
        if !matches!(
            self.project_settings(ctx),
            ProjectOnboardingSettings::Project { .. }
        ) {
            return;
        }

        self.onboarding_state.update(ctx, |model, ctx| {
            if warp_core::features::FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
                model.next(ctx);
            } else {
                model.complete(ctx);
            }
        });
    }

    fn skip(&mut self, ctx: &mut ViewContext<Self>) {
        self.onboarding_state.update(ctx, |model, ctx| {
            model.set_project_selected_local_folder(None, ctx);
            if warp_core::features::FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
                model.next(ctx);
            } else {
                model.complete(ctx);
            }
        });
    }
}

impl OnboardingSlide for ProjectSlide {
    fn on_enter(&mut self, ctx: &mut ViewContext<Self>) {
        match self.project_settings(ctx) {
            ProjectOnboardingSettings::NoProject => self.open_local_folder(ctx),
            ProjectOnboardingSettings::Project { .. } => self.next(ctx),
        }
    }

    fn on_cmd_or_ctrl_enter(&mut self, ctx: &mut ViewContext<Self>) {
        if matches!(
            self.project_settings(ctx),
            ProjectOnboardingSettings::NoProject
        ) {
            self.skip(ctx);
        }
    }
}

impl TypedActionView for ProjectSlide {
    type Action = ProjectSlideAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ProjectSlideAction::BackClicked => {
                let onboarding_state = self.onboarding_state.clone();
                onboarding_state.update(ctx, |model, ctx| {
                    model.back(ctx);
                });
            }
            ProjectSlideAction::NextClicked => {
                self.next(ctx);
            }
            ProjectSlideAction::SkipClicked => {
                self.skip(ctx);
            }
            ProjectSlideAction::OpenLocalFolderClicked => {
                self.open_local_folder(ctx);
            }
            ProjectSlideAction::LocalFolderSelected(result) => match result {
                Ok(path) => {
                    let onboarding_state = self.onboarding_state.clone();
                    onboarding_state.update(ctx, |model, ctx| {
                        model.set_project_selected_local_folder(Some(path.clone()), ctx);
                    });
                    ctx.notify();
                }
                Err(err) => {
                    log::warn!("File picker error during onboarding: {err}");
                }
            },
            ProjectSlideAction::ToggleInitializeProjectsAutomatically => {
                let onboarding_state = self.onboarding_state.clone();
                onboarding_state.update(ctx, |model, ctx| {
                    model.toggle_project_initialize_projects_automatically(ctx);
                });
                ctx.notify();
            }
        }
    }
}
