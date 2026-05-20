mod header_revamp;

use crate::code_review::code_review_view::{
    CodeReviewHeaderFields, CodeReviewView, CONTENT_TOP_MARGIN,
};
use crate::{
    appearance::Appearance,
    code_review::{
        code_review_view::{get_discard_button_disabled_tooltip, CodeReviewAction, LoadedState},
        diff_state::DiffStateModel,
    },
    menu::Menu,
    ui_components::icons::Icon,
    view_components::action_button::ActionButton,
};
use pathfinder_geometry::vector::vec2f;
use warp_core::features::FeatureFlag;
use warpui::elements::{Hoverable, ParentElement};
use warpui::platform::Cursor;
use warpui::ui_components::components::{Coords, UiComponent};
use warpui::{
    elements::{
        Align, ChildAnchor, ChildView, Clipped, ConstrainedBox, Container, CrossAxisAlignment,
        Flex, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
        ParentOffsetBounds, Shrinkable, SizeConstraintCondition, SizeConstraintSwitch, Stack,
    },
    fonts::{Properties, Weight},
    ui_components::{
        button::{ButtonVariant, TextAndIcon, TextAndIconAlignment},
        components::UiComponentStyles,
    },
    AppContext, Element, ModelHandle, ViewHandle,
};

// This is a best effort guess of the size of all of the elements in the header to know when we should start to wrap to the second row
const HEADER_WRAP_BREAKPOINT: f32 = 450.;

pub(crate) const HEADER_BUTTON_PADDING: Coords = Coords {
    top: 2.,
    bottom: 2.,
    left: 6.,
    right: 6.,
};

#[derive(Default)]
struct StateHandles {
    branch_name_tooltip: MouseStateHandle,
    discard_all_button: MouseStateHandle,
    add_diff_set_context_button: MouseStateHandle,
}

pub struct CodeReviewHeader {
    state_handles: StateHandles,
}

impl CodeReviewHeader {
    pub fn new() -> Self {
        Self {
            state_handles: StateHandles::default(),
        }
    }

    pub fn render(
        &self,
        state: &LoadedState,
        appearance: &Appearance,
        code_review_header_fields: &CodeReviewHeaderFields,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let wide_layout =
            self.render_wide_layout(state, appearance, code_review_header_fields, app);

        let compact_layout =
            self.render_compact_layout(state, appearance, code_review_header_fields, app);

        let header_switch = SizeConstraintSwitch::new(
            wide_layout,
            vec![(
                SizeConstraintCondition::WidthLessThan(HEADER_WRAP_BREAKPOINT),
                compact_layout,
            )],
        )
        .finish();

        Container::new(Clipped::new(Shrinkable::new(1., header_switch).finish()).finish())
            .with_margin_top(CONTENT_TOP_MARGIN)
            .with_margin_bottom(12.)
            .finish()
    }

    fn render_wide_layout(
        &self,
        state: &LoadedState,
        appearance: &Appearance,
        code_review_header_fields: &CodeReviewHeaderFields,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut left_section_wide = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        left_section_wide.add_child(
            Shrinkable::new(
                100.0,
                self.create_branch_tooltip(
                    &code_review_header_fields.diff_state_model,
                    appearance,
                    app,
                ),
            )
            .finish(),
        );
        left_section_wide.add_child(
            Container::new(CodeReviewView::render_diff_stats(
                &state.to_diff_stats(),
                appearance,
            ))
            .with_margin_right(8.)
            .finish(),
        );

        let mut right_section_wide = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(ChildView::new(&code_review_header_fields.diff_selector).finish());

        let has_no_changes = state.to_diff_stats().has_no_changes();

        if FeatureFlag::DiscardPerFileAndAllChanges.is_enabled() {
            right_section_wide.add_child(self.create_discard_button(
                state,
                &code_review_header_fields.diff_state_model,
                appearance,
                app,
            ));
        }

        if FeatureFlag::DiffSetAsContext.is_enabled() && !has_no_changes {
            if FeatureFlag::FileAndDiffSetComments.is_enabled() {
                right_section_wide.add_child(self.render_header_dropdown_button(
                    &code_review_header_fields.header_dropdown_button,
                    &code_review_header_fields.header_menu,
                    code_review_header_fields.header_menu_open,
                ));
            } else {
                right_section_wide.add_child(self.render_add_diff_set_context_button(appearance));
            }
        }

        if code_review_header_fields.is_in_split_pane {
            right_section_wide = right_section_wide.with_child(self.render_maximize_pane_button(
                &code_review_header_fields.maximize_button,
                appearance,
            ));
        }

        Clipped::new(
            Shrinkable::new(
                1.,
                Flex::row()
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(Shrinkable::new(2., left_section_wide.finish()).finish())
                    .with_child(right_section_wide.finish())
                    .finish(),
            )
            .finish(),
        )
        .finish()
    }

    fn render_compact_layout(
        &self,
        state: &LoadedState,
        appearance: &Appearance,
        code_review_header_fields: &CodeReviewHeaderFields,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut left_section_compact = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        left_section_compact.add_child(
            Shrinkable::new(
                100.0,
                self.create_branch_tooltip(
                    &code_review_header_fields.diff_state_model,
                    appearance,
                    app,
                ),
            )
            .finish(),
        );
        left_section_compact.add_child(
            Container::new(CodeReviewView::render_diff_stats(
                &state.to_diff_stats(),
                appearance,
            ))
            .with_margin_right(8.)
            .finish(),
        );

        let mut right_subsection_compact = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        if FeatureFlag::DiscardPerFileAndAllChanges.is_enabled() {
            right_subsection_compact.add_child(self.create_discard_button(
                state,
                &code_review_header_fields.diff_state_model,
                appearance,
                app,
            ));
        }

        let has_no_changes = state.to_diff_stats().has_no_changes();

        if FeatureFlag::DiffSetAsContext.is_enabled() && !has_no_changes {
            if FeatureFlag::FileAndDiffSetComments.is_enabled() {
                right_subsection_compact.add_child(self.render_header_dropdown_button(
                    &code_review_header_fields.header_dropdown_button,
                    &code_review_header_fields.header_menu,
                    code_review_header_fields.header_menu_open,
                ));
            } else {
                right_subsection_compact
                    .add_child(self.render_add_diff_set_context_button(appearance));
            }
        }

        if code_review_header_fields.is_in_split_pane {
            right_subsection_compact.add_child(self.render_maximize_pane_button(
                &code_review_header_fields.maximize_button,
                appearance,
            ));
        }

        let right_section_compact = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(ChildView::new(&code_review_header_fields.diff_selector).finish())
            .with_child(Container::new(right_subsection_compact.finish()).finish());

        Clipped::new(
            Shrinkable::new(
                1.,
                Flex::column()
                    .with_main_axis_alignment(MainAxisAlignment::Start)
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_child(Align::new(left_section_compact.finish()).left().finish())
                    .with_child(
                        Container::new(Align::new(right_section_compact.finish()).right().finish())
                            .with_margin_top(8.)
                            .finish(),
                    )
                    .finish(),
            )
            .finish(),
        )
        .finish()
    }

    fn create_branch_name_element(
        diff_state_model: &ModelHandle<DiffStateModel>,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let header_text = Self::get_header_text(diff_state_model, app);

        Container::new(
            warpui::elements::Text::new_inline(
                header_text,
                appearance.ui_font_family(),
                appearance.ui_font_size() + 2.,
            )
            .with_style(Properties::default().weight(Weight::Semibold))
            .with_color(theme.main_text_color(theme.background()).into())
            .finish(),
        )
        .with_margin_right(8.)
        .finish()
    }

    fn create_branch_tooltip(
        &self,
        diff_state_model: &ModelHandle<DiffStateModel>,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let header_text = Self::get_header_text(diff_state_model, app);
        appearance.ui_builder().overlay_tool_tip_on_element(
            header_text,
            self.state_handles.branch_name_tooltip.clone(),
            Self::create_branch_name_element(diff_state_model, appearance, app),
            ParentAnchor::BottomLeft,
            ChildAnchor::TopLeft,
            vec2f(0., 4.),
        )
    }

    fn create_discard_button(
        &self,
        state: &LoadedState,
        diff_state_model: &ModelHandle<DiffStateModel>,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let has_no_changes = state.to_diff_stats().has_no_changes();
        let git_operation_blocked = diff_state_model.as_ref(app).is_git_operation_blocked(app);
        let is_disabled = has_no_changes || git_operation_blocked;

        let sub_text_color = theme.sub_text_color(theme.background());
        let mut button_builder = appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.state_handles.discard_all_button.clone(),
            )
            .with_style(UiComponentStyles::default().set_padding(HEADER_BUTTON_PADDING))
            .with_style(UiComponentStyles {
                font_color: Some(sub_text_color.into()),
                ..Default::default()
            })
            .with_text_and_icon_label(
                TextAndIcon::new(
                    TextAndIconAlignment::IconFirst,
                    "Discard all".to_string(),
                    Icon::ReverseLeft.to_warpui_icon(warp_core::ui::theme::Fill::Solid(
                        sub_text_color.into_solid(),
                    )),
                    MainAxisSize::Min,
                    MainAxisAlignment::SpaceBetween,
                    vec2f(16., 16.),
                )
                .with_inner_padding(4.),
            );

        if is_disabled {
            let disabled_styles = UiComponentStyles {
                font_color: Some(theme.disabled_text_color(theme.background()).into_solid()),
                ..Default::default()
            };
            button_builder = button_builder.with_style(disabled_styles).with_cursor(None);
        }

        let mut button_hoverable = button_builder.build();

        if !is_disabled {
            button_hoverable = button_hoverable.on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(CodeReviewAction::ShowDiscardConfirmDialog(None));
            });
        }

        let button_element = button_hoverable.finish();

        if is_disabled {
            let tooltip_text = get_discard_button_disabled_tooltip(git_operation_blocked);
            Container::new(CodeReviewHeader::wrap_disabled_button_with_tooltip(
                button_element,
                tooltip_text,
                self.state_handles.discard_all_button.clone(),
                appearance,
            ))
            .with_margin_left(4.)
            .finish()
        } else {
            Container::new(button_element).with_margin_left(4.).finish()
        }
    }

    fn wrap_disabled_button_with_tooltip(
        button_element: Box<dyn Element>,
        tooltip_text: String,
        mouse_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        Hoverable::new(mouse_state, move |state| {
            let mut stack = Stack::new().with_child(button_element);
            if state.is_hovered() {
                let disabled_tooltip = ui_builder.tool_tip(tooltip_text).build().finish();
                let tooltip_offset = OffsetPositioning::offset_from_parent(
                    vec2f(0., -8.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopMiddle,
                    ChildAnchor::BottomMiddle,
                );
                stack.add_positioned_overlay_child(disabled_tooltip, tooltip_offset);
            }
            stack.finish()
        })
        .with_cursor(Cursor::Arrow)
        .finish()
    }

    pub(super) fn render_maximize_pane_button(
        &self,
        maximize_button: &ViewHandle<ActionButton>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(ChildView::new(maximize_button).finish())
                .with_height(appearance.ui_font_size() + 10.)
                .with_width(appearance.ui_font_size() + 10.)
                .finish(),
        )
        .with_margin_left(8.)
        .with_margin_right(6.)
        .finish()
    }

    fn render_add_diff_set_context_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let ui_builder = appearance.ui_builder().clone();

        let button = ui_builder
            .button(
                ButtonVariant::Secondary,
                self.state_handles.add_diff_set_context_button.clone(),
            )
            .with_text_and_icon_label(TextAndIcon::new(
                TextAndIconAlignment::IconFirst,
                "",
                Icon::Paperclip.to_warpui_icon(warp_core::ui::theme::Fill::Solid(
                    theme.main_text_color(theme.background()).into(),
                )),
                MainAxisSize::Min,
                MainAxisAlignment::SpaceBetween,
                vec2f(16., 16.),
            ))
            // manual overrides so it matches the branch dropdown and discard all button
            .with_style(UiComponentStyles::default().set_padding(Coords {
                top: 6.,
                bottom: 6.,
                left: 6.,
                right: 6.,
            }))
            .with_tooltip(move || {
                ui_builder
                    .tool_tip("Add diff set as context".to_owned())
                    .build()
                    .finish()
            })
            .with_tooltip_position(warpui::ui_components::button::ButtonTooltipPosition::AboveLeft)
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(CodeReviewAction::AddDiffSetAsContext(
                    crate::code_review::DiffSetScope::All,
                ));
            })
            .finish();

        Container::new(button).with_margin_left(4.).finish()
    }

    /// Renders the header dropdown trigger
    ///
    /// This button dispatches a CodeReviewAction and, when the header menu is open, renders the
    /// attached menu in a Stack overlay positioned relative to the button (like other overflow
    /// buttons in the app, e.g. Drive's "create new" button).
    fn render_header_dropdown_button(
        &self,
        header_dropdown_button: &ViewHandle<ActionButton>,
        header_menu: &ViewHandle<Menu<CodeReviewAction>>,
        header_menu_open: bool,
    ) -> Box<dyn Element> {
        let button_container = Container::new(
            ConstrainedBox::new(ChildView::new(header_dropdown_button).finish())
                .with_height(warp_core::ui::icons::ICON_DIMENSIONS)
                .with_width(warp_core::ui::icons::ICON_DIMENSIONS)
                .finish(),
        )
        .with_margin_left(4.)
        .finish();

        let mut stack = Stack::new().with_child(button_container);

        if header_menu_open {
            stack.add_positioned_overlay_child(
                ChildView::new(header_menu).finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            );
        }

        stack.finish()
    }

    fn get_header_text(diff_state_model: &ModelHandle<DiffStateModel>, app: &AppContext) -> String {
        let branch_name =
            diff_state_model.read(app, |model, ctx| model.get_current_branch_name(ctx));
        branch_name.unwrap_or("Reviewing open changes".to_string())
    }
}
