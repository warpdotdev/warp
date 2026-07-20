use std::borrow::Cow;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::Vector2F;
use warp_core::features::FeatureFlag;
use warp_core::ui::Icon;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::color::blend::Blend as _;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DropShadow, Expanded,
    Flex, Hoverable, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement as _,
    ParentOffsetBounds, Radius, Stack,
};
use warpui::fonts::Weight;
use warpui::keymap::EditableBinding;
use warpui::platform::file_picker::FilePickerError;
use warpui::platform::{Cursor, FilePickerConfiguration};
use warpui::ui_components::components::{UiComponent as _, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
};

use crate::util::bindings::{BindingGroup, CustomAction, keybinding_name_to_display_string};

const BUTTON_MIN_WIDTH: f32 = 149.;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_editable_bindings([
        EditableBinding::new(
            "project_buttons:open_repository",
            "Open repository",
            ProjectButtonsAction::OpenRepository,
        )
        .with_context_predicate(id!("ProjectButons"))
        .with_group(BindingGroup::Folders.as_str())
        .with_custom_action(CustomAction::OpenRepository),
        EditableBinding::new(
            "project_buttons:create_new_project",
            "Create new project",
            ProjectButtonsAction::CreateProject,
        )
        .with_context_predicate(id!("ProjectButons"))
        .with_enabled(|| FeatureFlag::CreateProjectFlow.is_enabled())
        .with_mac_key_binding("cmd-shift-N")
        .with_linux_or_windows_key_binding("alt-shift-N"),
    ]);
}

#[derive(Default)]
struct StateHandles {
    open_repo_button: MouseStateHandle,
    create_project_button: MouseStateHandle,
    clone_repo_button: MouseStateHandle,
}

pub struct ProjectButtons {
    state_handles: StateHandles,
}

struct TooltipData {
    text: String,
    keybinding: Option<String>,
}

impl ProjectButtons {
    pub fn new(_ctx: &mut ViewContext<Self>) -> Self {
        Self {
            state_handles: Default::default(),
        }
    }

    pub fn open_repository(ctx: &mut ViewContext<Self>) {
        ctx.open_file_picker(
            move |result, ctx| {
                if let Some(path_result) = result.map(|paths| paths.into_iter().next()).transpose()
                {
                    ctx.emit(ProjectButtonsEvent::OpenRepository(path_result));
                }
            },
            FilePickerConfiguration::new().folders_only(),
        );
    }

    fn glowing_button(
        &self,
        label_text: impl Into<Cow<'static, str>> + Clone,
        icon: Icon,
        action: ProjectButtonsAction,
        tooltip: TooltipData,
        mouse_state: MouseStateHandle,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let icon_color = internal_colors::fg_overlay_6(theme);
        let label = appearance
            .ui_builder()
            .paragraph(label_text.clone())
            .with_style(UiComponentStyles {
                font_weight: Some(Weight::Semibold),
                ..Default::default()
            })
            .build()
            .finish();

        Hoverable::new(mouse_state, move |state| {
            let icon_el = Container::new(
                ConstrainedBox::new(icon.to_warpui_icon(icon_color).finish())
                    .with_height(20.)
                    .with_width(20.)
                    .finish(),
            )
            .finish();

            let vertical_content = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(icon_el)
                .with_child(Container::new(label).with_margin_top(4.).finish())
                .finish();

            let base = ConstrainedBox::new(
                Container::new(
                    Container::new(vertical_content)
                        .with_uniform_padding(16.)
                        .finish(),
                )
                .with_border(theme.outline().into_solid())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_drop_shadow(
                    DropShadow::new_with_standard_offset_and_spread(ColorU::new(255, 143, 253, 15))
                        .with_offset(Vector2F::zero()),
                )
                .with_background(if state.is_hovered() {
                    theme
                        .background()
                        .blend(&theme.surface_overlay_1())
                        .into_solid()
                } else {
                    theme.background().into_solid()
                })
                .finish(),
            )
            .with_min_width(BUTTON_MIN_WIDTH)
            .finish();

            // Optional tooltip with keybinding string
            if state.is_hovered() {
                let tooltip = if let Some(keybinding) = tooltip.keybinding {
                    appearance
                        .ui_builder()
                        .tool_tip_with_sublabel(tooltip.text, keybinding)
                        .build()
                        .finish()
                } else {
                    appearance
                        .ui_builder()
                        .tool_tip(tooltip.text)
                        .build()
                        .finish()
                };

                let mut stack = Stack::new();
                stack.add_child(base);

                let offset = OffsetPositioning::offset_from_parent(
                    Vector2F::new(0., 4.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomMiddle,
                    ChildAnchor::TopMiddle,
                );

                stack.add_positioned_overlay_child(tooltip, offset);
                stack.finish()
            } else {
                base
            }
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action))
        .finish()
    }
}

pub enum ProjectButtonsEvent {
    OpenRepository(Result<String, FilePickerError>),
    CreateProject,
    CloneRepository,
}

impl Entity for ProjectButtons {
    type Event = ProjectButtonsEvent;
}

#[derive(Clone, Copy, Debug)]
pub enum ProjectButtonsAction {
    OpenRepository,
    CreateProject,
    CloneRepository,
}

impl TypedActionView for ProjectButtons {
    type Action = ProjectButtonsAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ProjectButtonsAction::OpenRepository => Self::open_repository(ctx),
            ProjectButtonsAction::CreateProject => ctx.emit(ProjectButtonsEvent::CreateProject),
            ProjectButtonsAction::CloneRepository => ctx.emit(ProjectButtonsEvent::CloneRepository),
        }
    }
}

impl View for ProjectButtons {
    fn ui_name() -> &'static str {
        "ProjectButons"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let mut row = Flex::row();

        if FeatureFlag::CreateProjectFlow.is_enabled() {
            row.add_children([
                Container::new(self.glowing_button(
                    "Create new project",
                    Icon::Plus,
                    ProjectButtonsAction::CreateProject,
                    TooltipData {
                        text: "Create and initialize a brand new project".to_string(),
                        keybinding: keybinding_name_to_display_string(
                            "project_buttons:create_new_project",
                            app,
                        ),
                    },
                    self.state_handles.create_project_button.clone(),
                    app,
                ))
                .with_margin_right(16.)
                .finish(),
                Container::new(self.glowing_button(
                    "Open repository",
                    Icon::Folder,
                    ProjectButtonsAction::OpenRepository,
                    TooltipData {
                        text: "Open an existing local folder or repository".to_string(),
                        keybinding: keybinding_name_to_display_string(
                            "project_buttons:open_repository",
                            app,
                        ),
                    },
                    self.state_handles.open_repo_button.clone(),
                    app,
                ))
                .with_margin_right(16.)
                .finish(),
                self.glowing_button(
                    "Clone repository",
                    Icon::Duplicate,
                    ProjectButtonsAction::CloneRepository,
                    TooltipData {
                        text: "Clone a repo from GitHub or another source".to_string(),
                        keybinding: None,
                    },
                    self.state_handles.clone_repo_button.clone(),
                    app,
                ),
            ]);
        } else {
            row.add_child(
                Expanded::new(
                    1.,
                    self.glowing_button(
                        "Open repository",
                        Icon::Plus,
                        ProjectButtonsAction::CreateProject,
                        TooltipData {
                            text: "Open an existing local folder or repository".to_string(),
                            keybinding: keybinding_name_to_display_string(
                                "project_buttons:open_repository",
                                app,
                            ),
                        },
                        self.state_handles.create_project_button.clone(),
                        app,
                    ),
                )
                .finish(),
            );
        }

        row.finish()
    }
}
