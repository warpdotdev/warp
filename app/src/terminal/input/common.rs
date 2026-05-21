use std::sync::Arc;

use crate::{
    ai::{
        llms::{is_using_api_key_for_provider, LLMPreferences},
        AIRequestUsageModel, BuyCreditsBannerDisplayState,
    },
    appearance::Appearance,
    settings::{AISettings, InputSettings},
    terminal::{
        buy_credits_banner::BuyCreditsBanner,
        input::{Input, InputAction, InputSuggestionsMode, MenuPositioning},
        model::TerminalModel,
        view::{TerminalAction, PADDING_LEFT},
    },
    ui_components::icons::Icon,
    workspaces::user_workspaces::UserWorkspaces,
};
use pathfinder_geometry::vector::vec2f;
use vim::vim::{VimMode, VimState};
use warp_completer::completer::Description;
use warp_core::features::FeatureFlag;
use warpui::{
    elements::{
        AnchorPair, Border, ChildAnchor, ConstrainedBox, Container, CornerRadius,
        CrossAxisAlignment, DispatchEventResult, Element, EventHandler, Flex, OffsetPositioning,
        OffsetType, ParentAnchor, ParentElement, ParentOffsetBounds, PositionedElementOffsetBounds,
        PositioningAxis, Radius, Shrinkable, Stack, Text, XAxisAnchor,
    },
    fonts::Weight,
    presenter::ChildView,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, EntityId, SingletonEntity, ViewHandle,
};

/// Whether the terminal input message bar should be shown.
///
/// The message bar is hidden when AI is disabled, the user has turned it off in settings,
/// or the session is a shared ambient agent session.
pub(super) fn should_show_terminal_input_message_bar(
    model: &TerminalModel,
    app: &AppContext,
) -> bool {
    FeatureFlag::AgentView.is_enabled()
        && !FeatureFlag::AgentViewPromptChip.is_enabled()
        && InputSettings::as_ref(app).is_terminal_input_message_bar_enabled()
        && AISettings::as_ref(app).is_any_ai_enabled(app)
        && !model.is_shared_ambient_agent_session()
}

/// Renders vim status bar
/// Used by: agent.rs, terminal.rs, universal.rs, legacy.rs
pub(super) fn render_vim_status(vim_state: &VimState, appearance: &Appearance) -> Container {
    let theme = appearance.theme();
    let ansi_colors = theme.terminal_colors().bright;
    let icon = match vim_state.mode {
        VimMode::Normal => Icon::VimNormalMode.to_warpui_icon(ansi_colors.green.into()),
        VimMode::Insert => {
            use crate::themes::theme::Blend;
            Icon::VimInsertMode.to_warpui_icon(
                theme
                    .background()
                    .blend(&theme.foreground().with_opacity(50)),
            )
        }
        VimMode::Visual(_) => Icon::VimVisualMode.to_warpui_icon(ansi_colors.blue.into()),
        VimMode::Replace => Icon::VimReplaceMode.to_warpui_icon(ansi_colors.red.into()),
    };
    Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_children([
                Container::new(
                    Text::new_inline(
                        vim_state.showcmd.to_owned(),
                        appearance.monospace_font_family(),
                        12.,
                    )
                    .with_color(appearance.theme().nonactive_ui_text_color().into())
                    .finish(),
                )
                .with_margin_right(8.)
                .with_margin_bottom(2.)
                .finish(),
                ConstrainedBox::new(icon.finish())
                    .with_width(12.)
                    .with_height(12.)
                    .finish(),
            ])
            .finish(),
    )
    .with_margin_right(8.)
    .with_margin_bottom(4.)
}

/// Renders the vim status indicator in the bottom right corner of the given stack.
pub(super) fn add_vim_status_to_stack(
    stack: &mut Stack,
    vim_state: &VimState,
    appearance: &Appearance,
    use_adjusted_padding: bool,
) {
    let terminal_padding = if use_adjusted_padding {
        *PADDING_LEFT / 1.5
    } else {
        0.
    };
    let status_bar = render_vim_status(vim_state, appearance)
        .with_padding_bottom(4.)
        .with_uniform_margin(terminal_padding)
        .finish();
    stack.add_positioned_child(
        status_bar,
        OffsetPositioning::offset_from_parent(
            vec2f(0.0, 0.0),
            ParentOffsetBounds::Unbounded,
            ParentAnchor::BottomRight,
            ChildAnchor::BottomRight,
        ),
    )
}

/// Wraps the given column, assumed to represent the full input content, with appropriate
/// left padding to be consistent with the terminal content, as well as an event handler to
/// focus the input view when clicked.
pub(super) fn wrap_input_with_terminal_padding_and_focus_handler(
    is_active_session: bool,
    column: Box<dyn Element>,
    use_adjusted_padding: bool,
) -> Box<dyn Element> {
    let terminal_padding = if use_adjusted_padding {
        *PADDING_LEFT / 1.5
    } else {
        *PADDING_LEFT
    };

    if is_active_session {
        let mut flex_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::End);

        flex_row.add_child(
            Shrinkable::new(
                1.,
                Container::new(column)
                    .with_padding_left(terminal_padding)
                    .finish(),
            )
            .finish(),
        );

        EventHandler::new(flex_row.finish())
            .on_left_mouse_down(move |ctx, _, _| {
                ctx.dispatch_typed_action(TerminalAction::ClearSelectionsWhenShellMode);
                ctx.dispatch_typed_action(InputAction::FocusInputBox);
                DispatchEventResult::StopPropagation
            })
            .finish()
    } else {
        Container::new(column)
            .with_padding_left(terminal_padding)
            .finish()
    }
}

/// Renders the selected workflow info overlay over the input.
pub(super) fn add_workflow_info_overlay(
    stack: &mut Stack,
    selected_workflow_state: &super::SelectedWorkflowState,
    pane_height_px: f32,
    menu_positioning: MenuPositioning,
) {
    let workflows_info_view =
        Container::new(ChildView::new(&selected_workflow_state.more_info_view).finish())
            .with_margin_left(16.)
            .with_margin_right(16.)
            .finish();

    stack.add_positioned_overlay_child(
        ConstrainedBox::new(workflows_info_view)
            .with_max_height(pane_height_px * 0.35)
            .finish(),
        OffsetPositioning::from_axes(
            PositioningAxis::relative_to_parent(
                ParentOffsetBounds::ParentByPosition,
                OffsetType::Pixel(0.),
                AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
            ),
            PositioningAxis::relative_to_parent(
                ParentOffsetBounds::Unbounded,
                OffsetType::Pixel(0.),
                menu_positioning.workflows_info_y_anchor(),
            ),
        ),
    );
}

/// Renders the voltron overlay over the input.
pub(super) fn add_voltron_overlay(
    stack: &mut Stack,
    voltron_view: &ViewHandle<crate::voltron::Voltron>,
    menu_positioning: MenuPositioning,
) {
    stack.add_positioned_overlay_child(
        ChildView::new(voltron_view).finish(),
        OffsetPositioning::offset_from_parent(
            menu_positioning.voltron_offset(),
            ParentOffsetBounds::Unbounded,
            menu_positioning.voltron_parent_anchor(),
            menu_positioning.voltron_child_anchor(),
        ),
    );
}

/// Renders the appropriate input suggestions overlay over the input, based on the current input
/// suggestions mode (if any).
pub(super) fn add_input_suggestions_overlays(
    input: &Input,
    stack: &mut Stack,
    appearance: &Appearance,
    menu_positioning: MenuPositioning,
    app: &AppContext,
) {
    match input.suggestions_mode_model().as_ref(app).mode() {
        InputSuggestionsMode::HistoryUp { .. } => {
            stack.add_positioned_overlay_child(
                input.render_history_up_menu(appearance, menu_positioning),
                OffsetPositioning::from_axes(
                    PositioningAxis::relative_to_parent(
                        ParentOffsetBounds::WindowByPosition,
                        OffsetType::Pixel(0.),
                        AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                    ),
                    PositioningAxis::relative_to_parent(
                        ParentOffsetBounds::Unbounded,
                        menu_positioning.history_y_offset(),
                        menu_positioning.history_y_anchor(),
                    ),
                ),
            );
        }
        InputSuggestionsMode::CompletionSuggestions { menu_position, .. } => {
            let relative_position_id = menu_position.to_position_id(input.editor.id());
            stack.add_positioned_overlay_child(
                input.render_completion_suggestions_menu(appearance, menu_positioning),
                OffsetPositioning::from_axes(
                    PositioningAxis::relative_to_stack_child(
                        &relative_position_id,
                        PositionedElementOffsetBounds::WindowByPosition,
                        OffsetType::Pixel(0.),
                        AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                    ),
                    PositioningAxis::relative_to_stack_child(
                        &relative_position_id,
                        PositionedElementOffsetBounds::Unbounded,
                        OffsetType::Pixel(0.),
                        menu_positioning.completion_suggestions_y_anchor(),
                    ),
                ),
            );
        }
        InputSuggestionsMode::StaticWorkflowEnumSuggestions { menu_position, .. } => {
            let relative_position_id = menu_position.to_position_id(input.editor.id());
            stack.add_positioned_overlay_child(
                input.render_workflow_enum_suggestions_menu(appearance, menu_positioning),
                OffsetPositioning::from_axes(
                    PositioningAxis::relative_to_stack_child(
                        &relative_position_id,
                        PositionedElementOffsetBounds::WindowByPosition,
                        OffsetType::Pixel(0.),
                        AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                    ),
                    PositioningAxis::relative_to_stack_child(
                        &relative_position_id,
                        PositionedElementOffsetBounds::Unbounded,
                        OffsetType::Pixel(0.),
                        menu_positioning.completion_suggestions_y_anchor(),
                    ),
                ),
            );
        }
        InputSuggestionsMode::DynamicWorkflowEnumSuggestions {
            menu_position,
            dynamic_enum_status,
            command,
            suggestions,
            ..
        } => {
            let relative_position_id = menu_position.to_position_id(input.editor.id());
            stack.add_positioned_overlay_child(
                input.render_dynamic_workflow_enum_menu(
                    appearance,
                    menu_positioning,
                    command.clone(),
                    dynamic_enum_status.clone(),
                    suggestions,
                ),
                OffsetPositioning::from_axes(
                    PositioningAxis::relative_to_stack_child(
                        &relative_position_id,
                        PositionedElementOffsetBounds::WindowByPosition,
                        OffsetType::Pixel(0.),
                        AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                    ),
                    PositioningAxis::relative_to_stack_child(
                        &relative_position_id,
                        PositionedElementOffsetBounds::Unbounded,
                        OffsetType::Pixel(0.),
                        menu_positioning.completion_suggestions_y_anchor(),
                    ),
                ),
            );
        }
        InputSuggestionsMode::AIContextMenu { .. } => {
            input.render_ai_context_menu(stack, &menu_positioning, app);
        }
        // SlashCommandsMenu is rendered separately via inline_slash_commands_menu_view
        InputSuggestionsMode::SlashCommands => {}
        // Conversation menu is rendered separately via inline_conversation_menu_view
        InputSuggestionsMode::ConversationMenu => {}
        // Model selector is rendered separately via inline_model_selector_view
        InputSuggestionsMode::ModelSelector => {}
        // Profile selector is rendered separately via inline_profile_selector_view
        InputSuggestionsMode::ProfileSelector => {}
        // Prompts menu is rendered separately via inline_prompts_menu_view
        InputSuggestionsMode::PromptsMenu => {}
        // Skill menu is rendered separately via inline_skill_selector_view
        InputSuggestionsMode::SkillMenu => {}
        // User query menu is rendered separately via user_query_menu_view
        InputSuggestionsMode::UserQueryMenu { .. } => {}
        // Inline history menu is rendered separately via inline_history_menu_view
        InputSuggestionsMode::InlineHistoryMenu { .. } => {}
        // Repos menu is rendered separately via inline_repos_menu_view
        InputSuggestionsMode::IndexedReposMenu => {}
        // Plan menu is rendered separately via inline_plan_menu_view
        InputSuggestionsMode::PlanMenu { .. } => {}
        InputSuggestionsMode::Closed => {}
    }
}

/// Renders the command xray overlay on the input using the command x ray-specific position id.
pub(super) fn add_command_xray_overlay(
    input: &Input,
    stack: &mut Stack,
    token_description: &Arc<Description>,
    appearance: &Appearance,
    menu_positioning: MenuPositioning,
    app: &AppContext,
) {
    let command_x_ray_position_id = format!("editor:command_x_ray_{}", input.editor.id());
    let line_height = input
        .editor
        .as_ref(app)
        .line_height(app.font_cache(), appearance);
    let offset = match menu_positioning {
        MenuPositioning::AboveInputBox => OffsetType::Pixel(0.),
        MenuPositioning::BelowInputBox => OffsetType::Pixel(line_height),
    };
    stack.add_positioned_overlay_child(
        render_command_token_description(token_description, appearance),
        OffsetPositioning::from_axes(
            PositioningAxis::relative_to_stack_child(
                &command_x_ray_position_id,
                PositionedElementOffsetBounds::ParentByPosition,
                OffsetType::Pixel(0.),
                AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
            ),
            PositioningAxis::relative_to_stack_child(
                &command_x_ray_position_id,
                PositionedElementOffsetBounds::Unbounded,
                offset,
                menu_positioning.command_xray_y_anchor(),
            ),
        ),
    );
}

fn render_command_token_description(
    description: &Arc<Description>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    // Append an ellipsis to the description if the token has more characters than the max
    // number of characters that are allowed.
    const MAX_XRAY_LABEL_CHARS: usize = 16;
    const TOKEN_DESCRIPTION_PADDING: f32 = 12.;
    const TOKEN_DESCRIPTION_MARGIN: f32 = 10.;
    const TOKEN_DESCRIPTION_WIDTH: f32 = 240.;
    const TOKEN_LABEL_HORIZONTAL_PADDING: f32 = 8.;
    const TOKEN_LABEL_VERTICAL_PADDING: f32 = 4.;

    let truncated_label = match description
        .token
        .item
        .char_indices()
        .nth(MAX_XRAY_LABEL_CHARS)
    {
        None => description.token.item.clone(),
        Some((byte_index, _)) => format!("{}...", &description.token[..byte_index]),
    };

    let theme = appearance.theme();
    let ui_builder = appearance.ui_builder();

    let mut command_description = Flex::column().with_child(
        Flex::row()
            .with_child(
                Container::new(
                    ui_builder
                        .paragraph(truncated_label)
                        .with_style(UiComponentStyles {
                            font_family_id: Some(appearance.monospace_font_family()),
                            font_color: Some(theme.active_ui_text_color().into()),
                            font_size: Some(appearance.monospace_font_size()),
                            font_weight: Some(Weight::Bold),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_padding_top(2.)
                .finish(),
            )
            .with_child(
                Container::new(
                    ui_builder
                        .paragraph(description.suggestion_type.to_name().to_string())
                        .with_style(UiComponentStyles {
                            font_family_id: Some(appearance.ui_font_family()),
                            font_color: Some(theme.active_ui_text_color().into()),
                            font_size: Some(appearance.monospace_font_size() * 0.75),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_background(theme.outline())
                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                .with_margin_left(TOKEN_DESCRIPTION_MARGIN)
                .with_padding_left(TOKEN_LABEL_HORIZONTAL_PADDING)
                .with_padding_right(TOKEN_LABEL_HORIZONTAL_PADDING)
                .with_padding_top(TOKEN_LABEL_VERTICAL_PADDING)
                .with_padding_bottom(TOKEN_LABEL_VERTICAL_PADDING)
                .finish(),
            )
            .finish(),
    );

    if let Some(description_text) = description.description_text.clone() {
        command_description.add_child(
            Container::new(
                ui_builder
                    .paragraph(description_text)
                    .with_style(UiComponentStyles {
                        font_family_id: Some(appearance.ui_font_family()),
                        font_color: Some(theme.sub_text_color(theme.surface_2()).into()),
                        font_size: Some(appearance.monospace_font_size() * 0.9),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_margin_top(TOKEN_DESCRIPTION_MARGIN)
            .finish(),
        );
    }

    ConstrainedBox::new(
        Container::new(command_description.finish())
            .with_uniform_padding(TOKEN_DESCRIPTION_PADDING)
            .with_margin_bottom(TOKEN_DESCRIPTION_MARGIN)
            .with_border(Border::all(1.).with_border_fill(theme.split_pane_border_color()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_background_color(theme.surface_2().into_solid())
            .finish(),
    )
    .with_width(TOKEN_DESCRIPTION_WIDTH)
    .finish()
}

/// Conditionally adds the "buy credits" banner overlay.
/// The overlay only is shown if all of the following is true:
/// - The user is on a team that can purchase addon credits
/// - The user is out of credits (or at their auto-reload limit)
/// - The input is focused
/// - There is not a BYO API key for the current model
pub(super) fn maybe_add_buy_credits_banner(
    stack: &mut Stack,
    buy_credits_banner: &ViewHandle<BuyCreditsBanner>,
    is_focused: bool,
    terminal_view_id: EntityId,
    is_input_at_top: bool,
    app: &AppContext,
) {
    let can_purchase_addon_credits = UserWorkspaces::as_ref(app)
        .current_team()
        .and_then(|team| team.billing_metadata.tier.purchase_add_on_credits_policy)
        .is_some_and(|policy| policy.enabled);

    // Show buy credits banner if billing policy allows purchasing, input is focused,
    // and either:
    // 1. OutOfCredits: for users that are not auto-reload enabled
    // 2. MonthlyLimitReached: Auto-reload enabled and is blocked by monthly limit
    let ai_request_usage = AIRequestUsageModel::as_ref(app);
    let should_show_banner = !matches!(
        ai_request_usage.compute_buy_addon_credits_banner_display_state(app),
        BuyCreditsBannerDisplayState::Hidden
    );
    let is_using_api_key_for_current_model = is_using_api_key_for_provider(
        &LLMPreferences::as_ref(app)
            .get_active_base_model(app, Some(terminal_view_id))
            .provider,
        app,
    );
    if can_purchase_addon_credits
        && is_focused
        && should_show_banner
        && !is_using_api_key_for_current_model
    {
        add_buy_credits_banner_overlay(stack, buy_credits_banner, is_input_at_top);
    }
}

/// Adds buy credits banner overlay to stack
fn add_buy_credits_banner_overlay(
    stack: &mut Stack,
    buy_credits_banner: &ViewHandle<BuyCreditsBanner>,
    is_input_at_top: bool,
) {
    use pathfinder_geometry::vector::vec2f;

    let (parent_anchor, child_anchor, y_offset) = if is_input_at_top {
        (ParentAnchor::BottomLeft, ChildAnchor::TopLeft, 8.)
    } else {
        (ParentAnchor::TopLeft, ChildAnchor::BottomLeft, -8.)
    };
    stack.add_positioned_child(
        ChildView::new(buy_credits_banner).finish(),
        OffsetPositioning::offset_from_parent(
            vec2f(0., y_offset),
            ParentOffsetBounds::Unbounded,
            parent_anchor,
            child_anchor,
        ),
    );
}
