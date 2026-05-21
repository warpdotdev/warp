use pathfinder_color::ColorU;
use pathfinder_geometry::vector::Vector2F;
use warp_core::features::FeatureFlag;
use warpui::{
    elements::{
        Align, ConstrainedBox, Container, CrossAxisAlignment, Empty, Flex, MainAxisAlignment,
        MainAxisSize, ParentElement, Rect, Shrinkable, Stack,
    },
    fonts::Weight,
    ui_components::{
        button::{ButtonVariant, TextAndIcon, TextAndIconAlignment},
        components::{UiComponent, UiComponentStyles},
    },
    Element, ViewContext,
};

use crate::{
    drive::sharing::{ContentEditability, SharingAccessLevel},
    env_vars::{
        active_env_var_collection_data::TrashStatus,
        view::env_var_collection::{EnvVarCollectionAction, EnvVarCollectionView},
    },
    i18n::{self, I18nKey},
    ui_components::{breadcrumb::BreadcrumbState, buttons::icon_button, icons::Icon},
    AppContext, Appearance, SingletonEntity,
};

const VARIABLE_DIVIDER_HEIGHT: f32 = 2.;
const SECTION_FONT_SIZE: f32 = 16.;
const BUTTON_HEIGHT: f32 = 32.;

/// This file contains components that fixed in the view,
/// i.e. the trash banner, breadcrumbs, and variables section header
impl EnvVarCollectionView {
    pub(super) fn update_breadcrumbs(&mut self, ctx: &mut ViewContext<Self>) {
        self.breadcrumbs = self
            .active_env_var_collection_data
            .update(ctx, |data, ctx| {
                data.breadcrumbs(ctx)
                    .map(|breadcrumbs| breadcrumbs.into_iter().map(BreadcrumbState::new).collect())
                    .unwrap_or_default()
            })
    }

    pub(super) fn render_trash_banner(
        &self,
        access_level: SharingAccessLevel,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let deleted = match self
            .active_env_var_collection_data
            .as_ref(app)
            .trash_status(app)
        {
            TrashStatus::Active => return None,
            TrashStatus::Trashed => false,
            TrashStatus::Deleted => true,
        };
        let appearance = Appearance::as_ref(app);

        let mut stack = Stack::new();

        let text = if deleted {
            i18n::tr(app, I18nKey::EnvVarsNoAccess)
        } else {
            i18n::tr(app, I18nKey::EnvVarsMovedToTrash)
        };
        stack.add_child(
            Align::new(
                Flex::row()
                    .with_children([
                        ConstrainedBox::new(
                            Icon::Trash
                                .to_warpui_icon(appearance.theme().foreground())
                                .finish(),
                        )
                        .with_width(16.)
                        .with_height(16.)
                        .finish(),
                        appearance
                            .ui_builder()
                            .span(text)
                            .with_style(UiComponentStyles {
                                font_size: Some(appearance.ui_font_size() + 2.),
                                ..Default::default()
                            })
                            .build()
                            .with_padding_left(8.)
                            .finish(),
                    ])
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Max)
                    .finish(),
            )
            .finish(),
        );

        let action_row = if deleted {
            Shrinkable::new(1., Empty::new().finish()).finish()
        } else {
            let mut action_row = Flex::row()
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center);

            if !FeatureFlag::SharedWithMe.is_enabled() || access_level.can_trash() {
                let ui_builder = appearance.ui_builder().clone();
                let tooltip = i18n::tr(app, I18nKey::EnvVarsRestoreTooltip).to_string();
                action_row.add_child(
                    Align::new(
                        appearance
                            .ui_builder()
                            .button(
                                ButtonVariant::Basic,
                                self.button_mouse_states.restore_from_trash_button.clone(),
                            )
                            .with_tooltip(move || {
                                ui_builder.tool_tip(tooltip.clone()).build().finish()
                            })
                            .with_text_label(i18n::tr(app, I18nKey::CommonRestore).to_string())
                            .build()
                            .on_click(|ctx, _, _| {
                                ctx.dispatch_typed_action(EnvVarCollectionAction::Untrash)
                            })
                            .finish(),
                    )
                    .finish(),
                );
            }

            action_row.finish()
        };

        stack.add_child(Align::new(action_row).right().finish());

        Some(
            Container::new(
                ConstrainedBox::new(stack.finish())
                    .with_min_height(40.)
                    .finish(),
            )
            .with_horizontal_padding(16.)
            .with_background(appearance.theme().surface_2())
            .finish(),
        )
    }

    pub(super) fn render_variables_section_header(
        &self,
        editability: ContentEditability,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut variables_section_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        variables_section_row.add_child(
            Shrinkable::new(
                2.,
                appearance
                    .ui_builder()
                    .span(i18n::tr(app, I18nKey::CommonVariables).to_string())
                    .with_style(UiComponentStyles {
                        font_size: Some(SECTION_FONT_SIZE),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .finish(),
        );

        if !FeatureFlag::SharedWithMe.is_enabled() || editability.can_edit() {
            variables_section_row.add_child(
                Shrinkable::new(
                    1.,
                    Flex::row()
                        .with_main_axis_alignment(MainAxisAlignment::End)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(
                            icon_button(
                                appearance,
                                Icon::Plus,
                                false,
                                self.button_mouse_states.add_variable_state.clone(),
                            )
                            .build()
                            .on_click(|ctx, _, _| {
                                ctx.dispatch_typed_action(EnvVarCollectionAction::AddVariable)
                            })
                            .finish(),
                        )
                        .finish(),
                )
                .finish(),
            );
        }

        variables_section_row.finish()
    }

    pub(super) fn render_divider(&self, appearance: &Appearance, index: usize) -> Box<dyn Element> {
        Shrinkable::new(
            1.,
            ConstrainedBox::new(
                Rect::new()
                    .with_background_color(if index != self.variable_rows.len() - 1 {
                        appearance.theme().surface_2().into()
                    } else {
                        ColorU::transparent_black()
                    })
                    .finish(),
            )
            .with_height(VARIABLE_DIVIDER_HEIGHT)
            .finish(),
        )
        .finish()
    }

    pub(super) fn render_invoke_button(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.button_mouse_states.invoke_mouse_state.clone(),
            )
            .with_style(UiComponentStyles {
                font_size: Some(14.),
                font_weight: Some(Weight::Bold),
                width: Some(80.),
                height: Some(BUTTON_HEIGHT),
                ..Default::default()
            })
            .with_text_and_icon_label(
                TextAndIcon::new(
                    TextAndIconAlignment::TextFirst,
                    "Load",
                    Icon::TerminalInput.to_warpui_icon(appearance.theme().active_ui_text_color()),
                    MainAxisSize::Min,
                    MainAxisAlignment::SpaceBetween,
                    Vector2F::new(10., 10.),
                )
                .with_inner_padding(4.),
            );

        if self.should_disable_invoke(app) {
            button = button.disabled();
        }

        button
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(EnvVarCollectionAction::Invoke))
            .finish()
    }

    pub(super) fn render_save_button(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let is_save_disabled = self.should_disable_save(app);
        let mut button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                self.button_mouse_states.save_mouse_state.clone(),
            )
            .with_style(UiComponentStyles {
                font_color: if is_save_disabled {
                    Some(
                        appearance
                            .theme()
                            .disabled_text_color(appearance.theme().background())
                            .into_solid(),
                    )
                } else {
                    Some(
                        appearance
                            .theme()
                            .main_text_color(appearance.theme().accent())
                            .into_solid(),
                    )
                },
                font_weight: Some(Weight::Bold),
                width: Some(100.),
                height: Some(BUTTON_HEIGHT),
                font_size: Some(14.),
                ..Default::default()
            })
            .with_centered_text_label(i18n::tr(app, I18nKey::CommonSave).to_owned());

        if is_save_disabled {
            button = button.disabled();
        }

        button
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(EnvVarCollectionAction::SaveVariables)
            })
            .finish()
    }
}
