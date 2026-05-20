use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::agent_view::orchestration_pill_bar::{
    render_agent_avatar_disc, render_orchestrator_avatar_disc,
};
use crate::ai::blocklist::orchestration_topology::descendant_conversation_ids_in_spawn_order;
use crate::ai::blocklist::usage::render_context_window_usage_icon;
use crate::ai::blocklist::usage::rollup::{
    compute_orchestration_rollup, AgentAvatar, OrchestrationCreditRollup, PerAgentCreditEntry,
};
use crate::ai::blocklist::view_util::format_credits;
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::appearance::Appearance;
use crate::persistence::model::{
    token_usage_category_display_name, ModelTokenUsage, FULL_TERMINAL_USE_CATEGORY,
    PRIMARY_AGENT_CATEGORY,
};
use crate::ui_components::blended_colors;
use std::cmp::Ordering;
use std::collections::HashMap;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::Icon;
use warpui::elements::ConstrainedBox;
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::{
    elements::{
        Border, Container, CornerRadius, CrossAxisAlignment, Empty, Flex, Hoverable, MainAxisSize,
        MouseStateHandle, ParentElement, Radius, Text,
    },
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    Settings,
    Footer,
}

pub struct ConversationUsageInfo {
    pub credits_spent: f32,
    // Credits spent over the last block, where the block comprises
    // all agent outputs since the most recent user input.
    pub credits_spent_for_last_block: Option<f32>,
    pub tool_calls: i32,
    pub models: Vec<ModelTokenUsage>,
    pub context_window_usage: f32,
    pub files_changed: i32,
    pub lines_added: i32,
    pub lines_removed: i32,
    pub commands_executed: i32,
}

/// Timing information for the last set of agent responses
/// (all blocks since the last user input, as this is the granularity
/// at which we show the usage footer)
pub struct TimingInfo {
    /// Time to first token for the last block (in milliseconds)
    pub time_to_first_token_ms: i64,
    /// Total response time for the last block (in milliseconds)
    pub total_agent_response_time_ms: i64,
    /// Wall-to-wall response time (in milliseconds) from sending the user query
    /// to the last token in the last set of responses.
    pub wall_to_wall_response_time_ms: Option<i64>,
}

/// Typed actions dispatched by widgets inside [`ConversationUsageView`]. The
/// view uses a single typed action surface for the "View details" /
/// "Hide details" toggle and the "Show N more" affordance so each row's
/// click handler can dispatch through the regular action pipeline without
/// borrowing the view directly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversationUsageViewAction {
    /// Flip the "View details" / "Hide details" toggle.
    ToggleDetailsExpanded,
    /// Reveal the truncated rows beyond the first 5 in the per-agent
    /// breakdown.
    ShowAllAgentRows,
}

/// View to hold a conversation usage info block.
/// This is used for both the usage footer and the usage history page in settings.
pub struct ConversationUsageView {
    pub usage_info: ConversationUsageInfo,
    /// The display mode for this view.
    pub display_mode: DisplayMode,
    /// Optional timing information for the last set of responses (only shown in the footer version of this view).
    pub timing_info: Option<TimingInfo>,
    full_terminal_use_tooltip_mouse_state: MouseStateHandle,
    /// Orchestration credit rollup context. When `Some`, the parent
    /// conversation is an orchestrator with at least one locally-loaded
    /// descendant; the rollup itself is recomputed at render time from
    /// `parent_conversation_id` so descendant updates always read fresh
    /// values.
    parent_conversation_id: Option<AIConversationId>,
    /// Local UI state: whether the "View details" toggle is currently
    /// expanded. Resets to `false` whenever the footer is rebuilt — the
    /// rich-content view backing this struct is dropped and recreated on
    /// every collapse / reopen cycle, satisfying PRODUCT invariant 6.
    details_expanded: bool,
    /// Local UI state: whether the user clicked "Show N more" to reveal the
    /// rows beyond the first 5. Resets on view rebuild for the same reason
    /// as `details_expanded`.
    show_all_clicked: bool,
    /// Per-row mouse states for the "View details" / "Hide details" link
    /// and the "Show N more" link. Stored on the view so hover/click state
    /// survives across renders.
    details_toggle_mouse_state: MouseStateHandle,
    show_more_mouse_state: MouseStateHandle,
}

impl ConversationUsageView {
    pub fn new(
        usage_info: ConversationUsageInfo,
        display_mode: DisplayMode,
        timing_info: Option<TimingInfo>,
        full_terminal_use_tooltip_mouse_state: MouseStateHandle,
    ) -> Self {
        Self {
            usage_info,
            display_mode,
            timing_info,
            full_terminal_use_tooltip_mouse_state,
            parent_conversation_id: None,
            details_expanded: false,
            show_all_clicked: false,
            details_toggle_mouse_state: MouseStateHandle::default(),
            show_more_mouse_state: MouseStateHandle::default(),
        }
    }

    /// Constructs the view in `DisplayMode::Footer` with orchestration
    /// credit rollup wired in. The view subscribes to
    /// [`BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated`] so it
    /// re-renders whenever any contributing conversation's usage metadata
    /// changes (PRODUCT invariant 7).
    pub fn new_footer_with_rollup(
        usage_info: ConversationUsageInfo,
        timing_info: Option<TimingInfo>,
        full_terminal_use_tooltip_mouse_state: MouseStateHandle,
        parent_conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, move |_, history, event, ctx| {
            // Narrow to events that can actually change *this* orchestrator's
            // rollup. Without this filter the closure wakes on every history
            // event in the app (one terminal view's typing storm fans out to
            // every other rollup subscriber), and `ctx.notify()` forces a
            // full footer re-render on each wake — orders of magnitude more
            // expensive than the subtree walk below.
            //
            // `StartedNewConversation` is intentionally omitted: a freshly
            // spawned descendant always has zero credits, so the rollup
            // result is unchanged until its first
            // `ConversationUsageMetadataUpdated` event (which this filter
            // will then pick up). Invariant 8 ("new descendant’s row appears
            // when it first spends a credit") is satisfied by the
            // credits-update path, not by the spawn event itself.
            let touched_id = match event {
                BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { conversation_id }
                | BlocklistAIHistoryEvent::RemoveConversation {
                    conversation_id, ..
                }
                | BlocklistAIHistoryEvent::DeletedConversation {
                    conversation_id, ..
                } => *conversation_id,
                _ => return,
            };
            if touched_id == parent_conversation_id {
                ctx.notify();
                return;
            }
            // `RemoveConversation` / `DeletedConversation` fire *after* the
            // conversation is dropped from `conversations_by_id`, but the
            // `children_by_parent` index that this walker consults is not
            // cleaned up on remove, so a just-pruned descendant is still
            // listed here. That lets us correctly notify on prune (invariant
            // 9) — the render-time rollup then skips the missing
            // conversation via the loaded-descendants filter and the row
            // disappears.
            let history = history.as_ref(ctx);
            if descendant_conversation_ids_in_spawn_order(history, parent_conversation_id)
                .contains(&touched_id)
            {
                ctx.notify();
            }
        });

        Self {
            usage_info,
            display_mode: DisplayMode::Footer,
            timing_info,
            full_terminal_use_tooltip_mouse_state,
            parent_conversation_id: Some(parent_conversation_id),
            details_expanded: false,
            show_all_clicked: false,
            details_toggle_mouse_state: MouseStateHandle::default(),
            show_more_mouse_state: MouseStateHandle::default(),
        }
    }

    /// Returns the current orchestration rollup for this view, or `None`
    /// when the view is in settings mode, the parent conversation isn't
    /// known, or the orchestrator has no locally-loaded descendants with
    /// non-zero credits. The feature is self-gating: settings-mode views
    /// and conversations without descendants short-circuit before any
    /// rollup-specific UI is built, so no feature flag is needed.
    fn rollup(&self, app: &AppContext) -> Option<OrchestrationCreditRollup> {
        if self.display_mode != DisplayMode::Footer {
            return None;
        }
        let parent_id = self.parent_conversation_id?;
        let history = BlocklistAIHistoryModel::as_ref(app);
        compute_orchestration_rollup(parent_id, history)
    }

    /// Helper to collect models grouped by category.
    /// Returns a HashMap mapping category name to list of (model_id, is_byok) tuples.
    /// Handles both category-based fields and legacy warp_tokens/byok_tokens fields.
    fn collect_models_by_category(&self) -> HashMap<String, Vec<(String, bool)>> {
        let mut entries_by_category: HashMap<String, Vec<(String, bool)>> = HashMap::new();

        // Collect from category-based fields
        for model in &self.usage_info.models {
            for (category, &tokens) in &model.warp_token_usage_by_category {
                if tokens > 0 {
                    entries_by_category
                        .entry(category.clone())
                        .or_default()
                        .push((model.model_id.clone(), false));
                }
            }
            for (category, &tokens) in &model.byok_token_usage_by_category {
                if tokens > 0 {
                    entries_by_category
                        .entry(category.clone())
                        .or_default()
                        .push((model.model_id.clone(), true));
                }
            }
        }

        // Fallback to legacy fields for backwards compatibility
        if entries_by_category.is_empty() {
            for model in &self.usage_info.models {
                if model.warp_tokens > 0 {
                    entries_by_category
                        .entry(PRIMARY_AGENT_CATEGORY.to_string())
                        .or_default()
                        .push((model.model_id.clone(), false));
                }
                if model.byok_tokens > 0 {
                    entries_by_category
                        .entry(PRIMARY_AGENT_CATEGORY.to_string())
                        .or_default()
                        .push((model.model_id.clone(), true));
                }
            }
        }

        entries_by_category
    }

    fn render_unified_layout(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let font_size = appearance.ui_font_size() + 2.;
        let text_color = blended_colors::text_main(theme, theme.surface_2());

        let rollup = self.rollup(app);

        let mut labels: Vec<Box<dyn Element>> = vec![];
        let mut values: Vec<Box<dyn Element>> = vec![];

        // Usage summary
        labels.push(render_section_header(
            "USAGE SUMMARY".to_string(),
            appearance,
        ));
        values.push(render_section_header("".to_string(), appearance));

        // "Credits spent (total)" value: use the rollup total when available,
        // otherwise the orchestrator's own self total (today's behavior).
        // PRODUCT invariants 2a, 11.
        let total_credits_value = rollup
            .as_ref()
            .map(|r| r.total_credits)
            .unwrap_or(self.usage_info.credits_spent);

        if self.display_mode == DisplayMode::Footer
            && self.usage_info.credits_spent_for_last_block.is_some()
        {
            let last_block_credits = self.usage_info.credits_spent_for_last_block.unwrap();
            labels.push(render_label_text(
                "Credits spent (last response)",
                appearance,
            ));
            values.push(render_value_text(
                format_credits(last_block_credits),
                appearance,
            ));

            labels.push(render_label_text("Credits spent (total)", appearance));
            values.push(self.render_total_credits_value_row(
                total_credits_value,
                rollup.as_ref(),
                appearance,
            ));
        } else {
            labels.push(render_label_text("Credits spent", appearance));
            values.push(self.render_total_credits_value_row(
                total_credits_value,
                rollup.as_ref(),
                appearance,
            ));
        }

        // Per-agent breakdown rows render immediately beneath the
        // "Credits spent (total)" row so they read as a drill-down of
        // that value, not as a separate section appended at the bottom
        // of the card. The rows are pushed into the same two-column
        // label/value layout as the rest of the usage summary; the
        // existing flex spacing handles indentation.
        self.append_per_agent_rows(&mut labels, &mut values, rollup.as_ref(), appearance);

        labels.push(render_label_text("Tool calls", appearance));
        values.push(render_value_text(
            format_value_text(self.usage_info.tool_calls, "call"),
            appearance,
        ));

        let entries_by_category = self.collect_models_by_category();
        let mut categories: Vec<_> = entries_by_category.keys().cloned().collect();
        categories.sort_by(|a, b| match (a.as_str(), b.as_str()) {
            (PRIMARY_AGENT_CATEGORY, _) => Ordering::Less,
            (_, PRIMARY_AGENT_CATEGORY) => Ordering::Greater,
            _ => a.cmp(b),
        });

        for category in categories {
            let models = entries_by_category.get(&category).unwrap();
            if models.is_empty() {
                break;
            }

            let label_text = if category == PRIMARY_AGENT_CATEGORY && entries_by_category.len() == 1
            {
                "Models".to_string()
            } else {
                format!("Models ({})", token_usage_category_display_name(&category))
            };

            // For FULL_TERMINAL_USE_CATEGORY, add an info icon with tooltip
            if category == FULL_TERMINAL_USE_CATEGORY {
                let label_element = render_label_text(&label_text, appearance);

                let hoverable_icon = appearance
                    .ui_builder()
                    .info_button_with_tooltip(
                        font_size * 0.85,
                        "You can change which model is used for full terminal use in the AI settings page",
                        self.full_terminal_use_tooltip_mouse_state.clone(),
                    )
                    .finish();

                labels.push(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(label_element)
                        .with_child(Container::new(hoverable_icon).with_margin_left(4.).finish())
                        .finish(),
                );
            } else {
                labels.push(render_label_text(&label_text, appearance));
            }

            // Build comma-separated list of models, with BYOK indicator using Icon::Key
            let mut model_elements: Vec<Box<dyn Element>> = vec![];
            let mut sorted_models: Vec<_> = models.iter().collect();
            sorted_models.sort_by(|a, b| a.0.cmp(&b.0));

            for (i, (model_id, is_byok)) in sorted_models.iter().enumerate() {
                if i > 0 {
                    model_elements.push(
                        Text::new(", ".to_string(), appearance.ui_font_family(), font_size)
                            .with_color(text_color)
                            .finish(),
                    );
                }

                if *is_byok {
                    model_elements.push(
                        ConstrainedBox::new(Icon::Key.to_warpui_icon(text_color.into()).finish())
                            .with_width(font_size)
                            .with_height(font_size)
                            .finish(),
                    );
                    model_elements.push(
                        Container::new(
                            Text::new((*model_id).clone(), appearance.ui_font_family(), font_size)
                                .with_color(text_color)
                                .finish(),
                        )
                        .with_margin_left(4.)
                        .finish(),
                    );
                } else {
                    model_elements.push(
                        Text::new((*model_id).clone(), appearance.ui_font_family(), font_size)
                            .with_color(text_color)
                            .finish(),
                    );
                }
            }

            values.push(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_children(model_elements)
                    .finish(),
            );
        }

        labels.push(render_label_text("Context window used", appearance));
        let context_usage_str =
            format!("{}%", (self.usage_info.context_window_usage * 100.).round());
        let context_window_element = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.)
            .with_child(
                Text::new(context_usage_str, appearance.ui_font_family(), font_size)
                    .with_color(text_color)
                    .finish(),
            )
            .with_child(
                ConstrainedBox::new(render_context_window_usage_icon(
                    self.usage_info.context_window_usage,
                    theme,
                    None,
                ))
                .with_width(font_size)
                .with_height(font_size)
                .finish(),
            )
            .finish();
        values.push(context_window_element);

        // Space between sections
        labels.push(
            Container::new(Empty::new().finish())
                .with_margin_top(12.)
                .finish(),
        );
        values.push(
            Container::new(Empty::new().finish())
                .with_margin_top(12.)
                .finish(),
        );

        // Tool call summary
        labels.push(render_section_header(
            "TOOL CALL SUMMARY".to_string(),
            appearance,
        ));
        values.push(render_section_header("".to_string(), appearance));

        labels.push(render_label_text("Files changed", appearance));
        values.push(render_value_text(
            format_value_text(self.usage_info.files_changed, "file"),
            appearance,
        ));

        labels.push(render_label_text("Diffs applied", appearance));
        let diffs_element = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Text::new(
                    format!("+ {}", self.usage_info.lines_added),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(theme.ansi_fg_green())
                .finish(),
            )
            .with_child(
                Container::new(
                    ConstrainedBox::new(Empty::new().finish())
                        .with_width(4.)
                        .with_height(4.)
                        .finish(),
                )
                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                .with_background(internal_colors::neutral_6(theme))
                .with_margin_left(8.)
                .with_margin_right(8.)
                .finish(),
            )
            .with_child(
                Text::new(
                    format!("- {}", self.usage_info.lines_removed),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(theme.ansi_fg_red())
                .finish(),
            )
            .finish();
        values.push(diffs_element);

        labels.push(render_label_text("Commands executed", appearance));
        values.push(render_value_text(
            format_value_text(self.usage_info.commands_executed, "command"),
            appearance,
        ));

        // Last response time
        if self.display_mode == DisplayMode::Footer {
            if let Some(timing) = &self.timing_info {
                if timing.time_to_first_token_ms != 0
                    || timing.total_agent_response_time_ms != 0
                    || timing.wall_to_wall_response_time_ms.is_some()
                {
                    // Space between sections
                    labels.push(
                        Container::new(Empty::new().finish())
                            .with_margin_top(12.)
                            .finish(),
                    );
                    values.push(
                        Container::new(Empty::new().finish())
                            .with_margin_top(12.)
                            .finish(),
                    );

                    // Section header
                    labels.push(render_section_header(
                        "LAST RESPONSE TIME".to_string(),
                        appearance,
                    ));
                    values.push(render_section_header("".to_string(), appearance));

                    labels.push(render_label_text("Time to first token", appearance));
                    values.push(render_value_text(
                        format!(
                            "{:.1} seconds",
                            timing.time_to_first_token_ms as f64 / 1000.0
                        ),
                        appearance,
                    ));

                    labels.push(render_label_text("Total agent response time", appearance));
                    values.push(render_value_text(
                        format!(
                            "{:.1} seconds",
                            timing.total_agent_response_time_ms as f64 / 1000.0
                        ),
                        appearance,
                    ));

                    if let Some(wall_ms) = timing.wall_to_wall_response_time_ms {
                        if wall_ms != 0 {
                            labels.push(render_label_text(
                                "Total time (including tool calls)",
                                appearance,
                            ));
                            values.push(render_value_text(
                                format!("{:.1} seconds", wall_ms as f64 / 1000.0),
                                appearance,
                            ));
                        }
                    }
                }
            }
        }

        Container::new(
            Flex::row()
                .with_spacing(8.)
                .with_child(Flex::column().with_children(labels).finish())
                .with_child(Flex::column().with_children(values).finish())
                .finish(),
        )
        .with_uniform_margin(16.)
        .finish()
    }

    /// Pushes the per-agent breakdown rows (and the optional "Show N
    /// more" link) into the two-column layout when the rollup is active
    /// and the user has expanded the details. Pushed in two-column
    /// (label, value) pairs so they slot into the existing flex layout.
    /// The label column carries the avatar + display name; the value
    /// column carries the credit value.
    fn append_per_agent_rows(
        &self,
        labels: &mut Vec<Box<dyn Element>>,
        values: &mut Vec<Box<dyn Element>>,
        rollup: Option<&OrchestrationCreditRollup>,
        appearance: &Appearance,
    ) {
        let Some(rollup) = rollup else {
            return;
        };
        if !self.details_expanded {
            return;
        }
        let total_entries = rollup.per_agent.len();
        let shown_entries: usize =
            if total_entries > PER_AGENT_BREAKDOWN_TRUNCATION_CAP && !self.show_all_clicked {
                PER_AGENT_BREAKDOWN_TRUNCATION_CAP
            } else {
                total_entries
            };
        for entry in rollup.per_agent.iter().take(shown_entries) {
            let (label_el, value_el) = self.render_per_agent_row(entry, appearance);
            labels.push(label_el);
            values.push(value_el);
        }
        if total_entries > shown_entries {
            let hidden_count = total_entries - shown_entries;
            // "Show N more" sits on a row of its own. We push a value-
            // side placeholder that mirrors the link's natural line
            // height so the right column stays in lock-step with the
            // left and the subsequent "Tool calls" / value row pair
            // doesn't slip out of alignment.
            labels.push(self.render_show_more_link(hidden_count, appearance));
            values.push(render_value_text_placeholder(appearance));
        }
    }

    /// Renders the "Credits spent (total)" value cell. When a rollup
    /// applies, the cell is a row with the value followed by a
    /// "View details ▾" / "Hide details ▴" toggle.
    fn render_total_credits_value_row(
        &self,
        total_credits: f32,
        rollup: Option<&OrchestrationCreditRollup>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let value_text = render_value_text(format_credits(total_credits), appearance);
        if rollup.is_none() {
            return value_text;
        }

        let toggle = self.render_details_toggle(appearance);
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(8.)
            .with_child(value_text)
            .with_child(toggle)
            .finish()
    }

    /// Renders the "View details ▾" / "Hide details ▴" toggle link
    /// rendered to the right of the "Credits spent (total)" value when
    /// a rollup applies. The link is styled in the theme's hyperlink
    /// color (`ansi_fg_blue`) — the same color the `FormattedTextElement`
    /// uses for in-line hyperlinks throughout Agent Mode — so it reads
    /// as a clickable affordance rather than a passive label. The
    /// `Hoverable` carries a `PointingHand` cursor on hover; we don't
    /// also flip the color or weight on hover because `Text` doesn't
    /// expose an underline knob and changing color in this two-token
    /// theme system tends to push the link into the accent space.
    fn render_details_toggle(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let font_size = appearance.ui_font_size() + 2.;
        let link_color = theme.ansi_fg_blue();
        let icon_size = font_size;
        let (label, icon) = if self.details_expanded {
            ("Hide details", Icon::ChevronUp)
        } else {
            ("View details", Icon::ChevronDown)
        };
        Hoverable::new(
            self.details_toggle_mouse_state.clone(),
            move |_hover_state| {
                let text_element =
                    Text::new(label.to_string(), appearance.ui_font_family(), font_size)
                        .with_color(link_color)
                        .with_selectable(false)
                        .finish();
                let icon_element =
                    ConstrainedBox::new(icon.to_warpui_icon(link_color.into()).finish())
                        .with_width(icon_size)
                        .with_height(icon_size)
                        .finish();
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_spacing(4.)
                    .with_child(text_element)
                    .with_child(icon_element)
                    .finish()
            },
        )
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(ConversationUsageViewAction::ToggleDetailsExpanded);
        })
        .finish()
    }

    /// Renders the avatar + label cell for a per-agent breakdown row,
    /// plus the credit value cell, returned as a `(label, value)` pair so
    /// the caller can append them to the existing two-column flex layout.
    ///
    /// Color choices:
    /// * Agent name uses the same color as the "USAGE SUMMARY" section
    ///   header (the disabled-text token) so the rollup rows read as a
    ///   sub-list of that section rather than competing with primary
    ///   labels.
    /// * Credit value uses the label-row color (`text_sub`) so it
    ///   visually echoes the "Credits spent" label rather than the
    ///   primary credit count beside it.
    ///
    /// Name length: agent names are clipped to the same max width and
    /// ellipsis treatment used by the orchestration pill bar
    /// ([`PER_AGENT_LABEL_MAX_WIDTH`]) so a long child-agent name in the
    /// footer doesn't push the credit-value column off-screen.
    fn render_per_agent_row(
        &self,
        entry: &PerAgentCreditEntry,
        appearance: &Appearance,
    ) -> (Box<dyn Element>, Box<dyn Element>) {
        let theme = appearance.theme();
        let bg = theme.surface_2();
        let font_size = appearance.ui_font_size() + 2.;
        const ROW_AVATAR_SIZE: f32 = 16.;
        let avatar = match entry.avatar {
            AgentAvatar::Orchestrator => {
                render_orchestrator_avatar_disc(ROW_AVATAR_SIZE, theme, appearance)
            }
            AgentAvatar::Child => {
                render_agent_avatar_disc(&entry.display_name, ROW_AVATAR_SIZE, theme, appearance)
            }
        };
        let name_text = Text::new(
            entry.display_name.clone(),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(blended_colors::text_disabled(theme, bg))
        .soft_wrap(false)
        .with_clip(ClipConfig::ellipsis())
        .finish();
        let name_element = ConstrainedBox::new(name_text)
            .with_max_width(PER_AGENT_LABEL_MAX_WIDTH)
            .finish();
        let label = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(8.)
            .with_child(avatar)
            .with_child(name_element)
            .finish();
        let value = Text::new(
            format_credits(entry.credits_spent),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(blended_colors::text_sub(theme, bg))
        .finish();
        (label, value)
    }

    /// Renders the "Show N more" link row shown beneath the first 5
    /// per-agent rows when the breakdown has more entries than the
    /// truncation cap. Clicking the link replaces the truncated list with
    /// the full list on the next render (PRODUCT invariant 5f). Uses the
    /// same hyperlink-blue color as the "View details" toggle so the
    /// affordances visually match.
    fn render_show_more_link(
        &self,
        hidden_count: usize,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let font_size = appearance.ui_font_size() + 2.;
        let link_color = theme.ansi_fg_blue();
        let label = format!("Show {hidden_count} more");
        Hoverable::new(self.show_more_mouse_state.clone(), move |_hover_state| {
            Text::new(label.clone(), appearance.ui_font_family(), font_size)
                .with_color(link_color)
                .with_style(Properties {
                    weight: Weight::Normal,
                    ..Default::default()
                })
                .with_selectable(false)
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(ConversationUsageViewAction::ShowAllAgentRows);
        })
        .finish()
    }

    /// Render the card container with display mode-specific styling.
    fn render_card_container(
        &self,
        content: Box<dyn Element>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let mut card_container = Container::new(content).with_background(theme.surface_2());

        if let DisplayMode::Footer = self.display_mode {
            card_container = card_container
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .with_border(Border::all(1.0).with_border_fill(theme.outline()))
                .with_uniform_margin(16.);
        } else {
            card_container =
                card_container.with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(6.)));
        }

        let mut res = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        if let DisplayMode::Footer = self.display_mode {
            res = res.with_child(
                // Top divider
                Container::new(Empty::new().finish())
                    .with_border(Border::top(2.0).with_border_fill(theme.outline()))
                    .with_overdraw_bottom(0.)
                    .finish(),
            );
        }

        res.with_child(card_container.finish()).finish()
    }
}

impl View for ConversationUsageView {
    fn ui_name() -> &'static str {
        "ConversationUsageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        self.render_card_container(self.render_unified_layout(app), appearance)
    }
}

impl Entity for ConversationUsageView {
    type Event = ();
}

impl TypedActionView for ConversationUsageView {
    type Action = ConversationUsageViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ConversationUsageViewAction::ToggleDetailsExpanded => {
                self.details_expanded = !self.details_expanded;
                // Collapsing the breakdown resets the "Show N more"
                // expansion so the user lands back on the truncated list
                // the next time they expand.
                if !self.details_expanded {
                    self.show_all_clicked = false;
                }
                ctx.notify();
            }
            ConversationUsageViewAction::ShowAllAgentRows => {
                self.show_all_clicked = true;
                ctx.notify();
            }
        }
    }
}

/// Render the main header for a usage section.
fn render_section_header(header_label: String, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let background = theme.surface_2();

    Container::new(
        Text::new(
            header_label,
            appearance.overline_font_family(),
            appearance.overline_font_size(),
        )
        .with_color(blended_colors::text_disabled(theme, background))
        .finish(),
    )
    .with_margin_bottom(4.)
    .finish()
}

/// Format a value and a label into one usage string,
/// making the label plural if the value is not 1.
fn format_value_text(value: i32, label: &str) -> String {
    format!("{} {}{}", value, label, if value == 1 { "" } else { "s" })
}

/// Helper to build a text element with consistent styling for labels.
fn render_label_text(text: &str, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let font_size = appearance.ui_font_size() + 2.;

    Text::new(text.to_string(), appearance.ui_font_family(), font_size)
        .with_color(blended_colors::text_sub(theme, theme.surface_2()))
        .finish()
}

/// Helper to build a text element with consistent styling for values.
fn render_value_text(text: String, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let font_size = appearance.ui_font_size() + 2.;
    let text_color = blended_colors::text_main(theme, theme.surface_2());

    Text::new(text, appearance.ui_font_family(), font_size)
        .with_color(text_color)
        .finish()
}

/// Renders a placeholder value cell that occupies one full line of the
/// value column without painting any visible text. Used opposite the
/// "Show N more" link so the two-column flex stays row-aligned for the
/// subsequent rows.
///
/// A simple `Empty` element would also keep the slot count matched, but
/// `Empty` has zero height, so the value column collapses by one line
/// and "Tool calls" ends up paired with "Show N more" instead of with
/// the next labels-column row. Pushing a `Text` element with a
/// single-space content forces a real line-height equal to the link's
/// own line-height.
fn render_value_text_placeholder(appearance: &Appearance) -> Box<dyn Element> {
    let font_size = appearance.ui_font_size() + 2.;
    Text::new(" ".to_string(), appearance.ui_font_family(), font_size).finish()
}

/// Maximum rendered width of an agent name in a per-agent breakdown row.
/// Mirrors `PILL_LABEL_MAX_WIDTH` in the orchestration pill bar so the
/// footer's name treatment never exceeds what the pill bar already
/// enforces at the top of the agent view.
const PER_AGENT_LABEL_MAX_WIDTH: f32 = 110.;

/// Maximum number of rows shown in the per-agent breakdown before the
/// "Show N more" affordance truncates the list. Matches PRODUCT
/// invariants 5e (≤ 5 rows render in full) and 5f (> 5 rows render the
/// first 5 followed by a "Show N more" link).
const PER_AGENT_BREAKDOWN_TRUNCATION_CAP: usize = 5;

#[cfg(test)]
#[path = "conversation_usage_view_tests.rs"]
mod tests;
