//! The "Conversation" usage popover, anchored to the footer's usage icon.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use thousands::Separable;
use warp_core::ui::Icon;
use warp_multi_agent_api as api;
use warpui::elements::{
    Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss,
    DispatchEventResult, DropShadow, Empty, EventHandler, Expanded, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Shrinkable, Stack, Text,
};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::history_model::BlocklistAIHistoryEvent;
use crate::ai::blocklist::usage::colors::chart_color;
use crate::ai::blocklist::view_util::format_credits;
use crate::ai::llms::LLMPreferences;
use crate::appearance::Appearance;
use crate::persistence::model::{
    FULL_TERMINAL_USE_CATEGORY, ModelTokenUsage, PRIMARY_AGENT_CATEGORY,
};
use crate::settings::UsageDisplayUnit;
use crate::settings::ai::{AISettings, AISettingsChangedEvent};
use crate::settings_view::SettingsSection;
use crate::ui_components::blended_colors;
use crate::workspace::WorkspaceAction;

/// Fixed popover width, matching the Figma reference (`336px`).
const POPOVER_WIDTH: f32 = 336.;
/// Height of the segmented usage/context-window bars.
const BAR_HEIGHT: f32 = 6.;
/// Width/height of the small color swatch next to each row label.
const SWATCH_SIZE: f32 = 8.;
/// Shown in place of a figure the client has no value for. Distinct from
/// `$0.00`, which is a known zero.
const EM_DASH: &str = "\u{2014}";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsagePopoverAction {
    ToggleModelUsageSection,
    ToggleToolCallSummarySection,
    ToggleResponseTimeSection,
    /// Dispatched by the [`Dismiss`] underlay when the user clicks outside
    /// the popover.
    RequestClose,
    /// Toggles the per-model token/cost breakdown subsection for the given
    /// model id.
    ToggleModelExpanded(String),
}

/// Emitted when the popover should be closed, so the footer (which owns
/// `usage_popover_open`) can react to an outside click.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsagePopoverEvent {
    Close,
}

/// What a collapsible section shows in place of its content while collapsed.
#[derive(Default)]
struct CollapsedSummary {
    text: Option<String>,
    /// Disambiguates an abbreviated `text`, e.g. the exact count behind
    /// "144.3k tokens".
    tooltip: Option<String>,
}

impl CollapsedSummary {
    fn new(text: String) -> Self {
        Self {
            text: Some(text),
            tooltip: None,
        }
    }

    fn with_tooltip(mut self, tooltip: Option<String>) -> Self {
        self.tooltip = tooltip;
        self
    }
}

/// The click target that expands and collapses a section.
struct SectionToggle {
    mouse_state: MouseStateHandle,
    action: UsagePopoverAction,
}

/// A `Flex::row` preconfigured for `label ... value` rows: `SpaceBetween`
/// alone has no effect unless the row also claims the max available width.
fn space_between_row() -> Flex {
    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_main_axis_size(MainAxisSize::Max)
}

/// Floating "Conversation" usage popover. The footer owns a single
/// long-lived instance and calls [`Self::reset_for_conversation`] each time
/// the popover opens.
pub struct UsagePopoverView {
    /// `None` until the footer first opens the popover and points it at the
    /// active conversation.
    conversation_id: Option<AIConversationId>,
    model_usage_section_expanded: bool,
    tool_call_summary_section_expanded: bool,
    response_time_section_expanded: bool,
    /// Model ids whose per-model breakdown subsection is currently expanded.
    /// Keyed by model id rather than a fixed set of fields since the list of
    /// models is dynamic per-conversation.
    expanded_model_ids: HashSet<String>,
    /// Hover state per tooltip, keyed by a string unique to each hoverable
    /// instance. The handles must persist across renders: the hover-in delay
    /// never fires on a handle rebuilt every frame.
    hover_states: RefCell<HashMap<String, MouseStateHandle>>,
    model_usage_toggle_mouse_state: MouseStateHandle,
    tool_call_summary_toggle_mouse_state: MouseStateHandle,
    response_time_toggle_mouse_state: MouseStateHandle,
    view_account_usage_mouse_state: MouseStateHandle,
}

impl UsagePopoverView {
    pub fn new(conversation_id: Option<AIConversationId>, ctx: &mut ViewContext<Self>) -> Self {
        // Cost rows render in the user's credits/dollars unit, so the open
        // popover must re-render when the unit flips.
        ctx.subscribe_to_model(&AISettings::handle(ctx), |_, _, event, ctx| {
            if matches!(event, AISettingsChangedEvent::UsageDisplayUnit { .. }) {
                ctx.notify();
            }
        });
        // Usage updates notify the history model's subscribers; the footer
        // re-renders itself but not this child view, so the open popover must
        // listen for its own conversation's usage changes.
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |me, _, event, ctx| {
                let touched_conversation_id = match event {
                    BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated {
                        conversation_id,
                    }
                    | BlocklistAIHistoryEvent::RemoveConversation {
                        conversation_id, ..
                    }
                    | BlocklistAIHistoryEvent::DeletedConversation {
                        conversation_id, ..
                    } => Some(*conversation_id),
                    _ => None,
                };
                if touched_conversation_id.is_some_and(|id| Some(id) == me.conversation_id) {
                    ctx.notify();
                }
            },
        );
        Self {
            conversation_id,
            model_usage_section_expanded: true,
            tool_call_summary_section_expanded: true,
            response_time_section_expanded: true,
            expanded_model_ids: HashSet::new(),
            hover_states: RefCell::new(HashMap::new()),
            model_usage_toggle_mouse_state: MouseStateHandle::default(),
            tool_call_summary_toggle_mouse_state: MouseStateHandle::default(),
            response_time_toggle_mouse_state: MouseStateHandle::default(),
            view_account_usage_mouse_state: MouseStateHandle::default(),
        }
    }

    /// The conversation this popover is currently pointed at.
    pub fn conversation_id(&self) -> Option<AIConversationId> {
        self.conversation_id
    }

    /// Points this (reused) popover at `conversation_id` and resets all
    /// section-collapse state back to [`Self::new`]'s defaults. Subscriptions
    /// are deliberately not re-registered: they are bound to the view's
    /// identity, which outlives any single conversation.
    pub fn reset_for_conversation(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.conversation_id = Some(conversation_id);
        self.model_usage_section_expanded = true;
        self.tool_call_summary_section_expanded = true;
        self.response_time_section_expanded = true;
        self.expanded_model_ids.clear();
        self.hover_states.borrow_mut().clear();
        ctx.notify();
    }

    /// Renders the header's total: the headline figure the footer's tooltip
    /// shows too.
    fn render_header(
        &self,
        conversation: &AIConversation,
        usage_display_unit: UsageDisplayUnit,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let title = Text::new(
            "Conversation".to_string(),
            appearance.ui_font_family(),
            appearance.ui_font_size() + 4.,
        )
        .with_color(blended_colors::text_main(theme, background))
        .finish();

        let total = Text::new(
            conversation_total_text(conversation, usage_display_unit),
            appearance.ui_font_family(),
            appearance.ui_font_size() + 4.,
        )
        .with_color(blended_colors::text_sub(theme, background))
        .finish();
        let title = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(8.)
            .with_child(title)
            .with_child(total)
            .finish();

        let link_color = blended_colors::text_sub(theme, background);
        let font_family = appearance.ui_font_family();
        let font_size = appearance.ui_font_size();
        let link = Hoverable::new(self.view_account_usage_mouse_state.clone(), move |_state| {
            Text::new("View account usage".to_string(), font_family, font_size)
                .with_color(link_color)
                .with_selectable(false)
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::ShowSettingsPage(
                SettingsSection::BillingAndUsage,
            ));
        })
        .finish();

        space_between_row()
            .with_child(title)
            .with_child(link)
            .finish()
    }

    /// Renders a collapsible section header: an overline `label` on the
    /// left and, on the right, a chevron indicating expand state. When the
    /// section is collapsed, `summary`'s text (e.g. "144.3k tokens / $0.21",
    /// "12 tool calls") is shown just before the chevron, so key information
    /// stays visible without expanding the section.
    fn render_section_header(
        &self,
        label: &str,
        expanded: bool,
        summary: CollapsedSummary,
        toggle: SectionToggle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let CollapsedSummary {
            text: collapsed_summary,
            tooltip: collapsed_summary_tooltip,
        } = summary;
        let SectionToggle {
            mouse_state,
            action,
        } = toggle;
        let theme = appearance.theme();
        let background = theme.surface_2();
        let label_color = blended_colors::text_disabled(theme, background);
        let summary_color = blended_colors::text_sub(theme, background);
        let icon = if expanded {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };
        // Fetched up front (rather than inside the closure below) so the
        // closure never needs to capture `self`.
        let summary_hover_state = collapsed_summary_tooltip
            .is_some()
            .then(|| self.hover_state_for(format!("value:header:{label}")));
        let label = label.to_string();
        let overline_font_family = appearance.overline_font_family();
        // A couple points larger than the raw overline size so the section
        // headers read more clearly against the row content below them.
        let overline_font_size = appearance.overline_font_size() + 2.;
        let summary_font_family = appearance.ui_font_family();
        let summary_font_size = appearance.ui_font_size();

        Hoverable::new(mouse_state, move |_state| {
            let label_element = Text::new(label.clone(), overline_font_family, overline_font_size)
                .with_color(label_color)
                .finish();
            let icon_element =
                ConstrainedBox::new(icon.to_warpui_icon(label_color.into()).finish())
                    .with_width(overline_font_size)
                    .with_height(overline_font_size)
                    .finish();
            let mut right = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.);
            if !expanded && let Some(summary) = &collapsed_summary {
                let summary_element =
                    Text::new(summary.clone(), summary_font_family, summary_font_size)
                        .with_color(summary_color)
                        .finish();
                let summary_element = match (&summary_hover_state, &collapsed_summary_tooltip) {
                    (Some(hover_state), Some(tooltip_text)) => with_tooltip(
                        hover_state.clone(),
                        summary_element,
                        tooltip_text.clone(),
                        appearance,
                    ),
                    _ => summary_element,
                };
                right.add_child(summary_element);
            }
            right.add_child(icon_element);
            space_between_row()
                .with_child(label_element)
                .with_child(right.finish())
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(action.clone());
        })
        .finish()
    }

    /// Renders a non-collapsible section header with a value on the right
    /// (same visual treatment as a collapsible header's collapsed-state
    /// summary, minus the chevron): an overline `label` on the left, `value`
    /// on the right, no click handling.
    fn render_static_section_header_with_value(
        &self,
        label: &str,
        value: String,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let label_color = blended_colors::text_disabled(theme, background);
        let value_color = blended_colors::text_sub(theme, background);

        space_between_row()
            .with_child(
                Text::new(
                    label.to_string(),
                    appearance.overline_font_family(),
                    appearance.overline_font_size() + 2.,
                )
                .with_color(label_color)
                .finish(),
            )
            .with_child(
                Text::new(
                    value,
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(value_color)
                .finish(),
            )
            .finish()
    }

    /// Renders the per-model breakdown.
    ///
    /// Returns `None` when there is nothing to break down, so the caller can
    /// omit the section rather than emit an empty element that still consumes
    /// the parent column's spacing.
    fn render_usage_breakdown_section(
        &self,
        conversation: &AIConversation,
        charged_usage_by_key: &HashMap<ModelChargeKey, ModelChargedUsage>,
        custom_endpoint_label: impl Fn(&str) -> String,
        usage_display_unit: UsageDisplayUnit,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let rows = model_usage_rows(
            conversation.token_usage(),
            charged_usage_by_key,
            custom_endpoint_label,
        );
        if rows.is_empty() {
            return None;
        }
        let totals = RowTotals::of_model_rows(&rows);

        let mut column = Flex::column().with_spacing(8.);
        column.add_child(
            self.render_section_header(
                "INFERENCE USAGE",
                self.model_usage_section_expanded,
                CollapsedSummary::new(format_tokens_and_cost(
                    totals.tokens,
                    totals.cost,
                    usage_display_unit,
                ))
                .with_tooltip(totals.tokens.and_then(exact_token_count_tooltip)),
                SectionToggle {
                    mouse_state: self.model_usage_toggle_mouse_state.clone(),
                    action: UsagePopoverAction::ToggleModelUsageSection,
                },
                appearance,
            ),
        );
        if self.model_usage_section_expanded {
            column.add_child(self.render_model_usage_rows(&rows, usage_display_unit, appearance));
        }
        Some(column.finish())
    }

    /// Renders the non-collapsible "PLATFORM USAGE" section: Warp's platform
    /// fee, which unlike inference cost isn't attributable to any single
    /// model.
    ///
    /// Omitted entirely when the server sent no charged usage at all. A charged
    /// usage whose platform fee is zero renders as a known zero, matching how
    /// the rest of the popover distinguishes a known zero from an unknown
    /// figure.
    fn render_platform_usage_section(
        &self,
        conversation: &AIConversation,
        usage_display_unit: UsageDisplayUnit,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let charged_usage = conversation.usage_totals().charged_usage?;

        let value = match usage_display_unit {
            UsageDisplayUnit::Credits => format_credits(charged_usage.platform_cost_in_credits),
            UsageDisplayUnit::Dollars => format_dollars(charged_usage.platform_cost_in_cents),
        };
        Some(self.render_static_section_header_with_value("PLATFORM USAGE", value, appearance))
    }

    fn render_model_usage_rows(
        &self,
        rows: &[ModelUsageRow],
        usage_display_unit: UsageDisplayUnit,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let totals = RowTotals::of_model_rows(rows);
        let total_tokens = totals.tokens.unwrap_or(0);
        let theme = appearance.theme();
        let background = theme.surface_2();
        let font_size = appearance.ui_font_size();

        let mut column = Flex::column().with_spacing(6.);

        let all_models_value = Text::new(
            format_tokens_and_cost(totals.tokens, totals.cost, usage_display_unit),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(blended_colors::text_main(theme, background))
        .finish();
        let all_models_value = self.maybe_with_tooltip(
            "value:all_models".to_string(),
            all_models_value,
            exact_token_count_tooltip(total_tokens),
            appearance,
        );
        column.add_child(
            space_between_row()
                .with_child(
                    Text::new(
                        "All models".to_string(),
                        appearance.ui_font_family(),
                        font_size,
                    )
                    .with_color(blended_colors::text_sub(theme, background))
                    .finish(),
                )
                .with_child(all_models_value)
                .finish(),
        );

        // Segment widths track the same quantity the rows display, so the bar
        // and the numbers beside it can't tell different stories.
        let segments: Vec<(ColorU, f32)> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let pct = if total_tokens == 0 {
                    0.
                } else {
                    (row.tokens as f32 / total_tokens as f32) * 100.
                };
                (chart_color(index), pct)
            })
            .collect();
        column.add_child(render_segmented_bar(
            &segments,
            theme.outline().into_solid(),
        ));

        for (index, row) in rows.iter().enumerate() {
            column.add_child(self.render_model_usage_row(
                index,
                row,
                usage_display_unit,
                appearance,
            ));
        }

        column.finish()
    }

    /// Renders a per-model row. Every row is clickable (a trailing chevron
    /// indicates this) and toggles a breakdown subsection beneath it — an
    /// input/output/cache/web-search split when the conversation has charged
    /// usage for this model, or a fallback message when it doesn't (e.g. the
    /// server hasn't sent per-request charges for this conversation yet).
    fn render_model_usage_row(
        &self,
        index: usize,
        row: &ModelUsageRow,
        usage_display_unit: UsageDisplayUnit,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let font_size = appearance.ui_font_size();
        let color = chart_color(index);
        let expanded = self.expanded_model_ids.contains(&row.model_id);

        let badge = row.role.and_then(ModelRole::badge_label);
        let full_label = match badge {
            Some(badge) => format!("{} ({badge})", row.model_id),
            None => row.model_id.clone(),
        };
        let chevron_color = blended_colors::text_disabled(theme, background);
        let chevron_icon = if expanded {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };

        // Name and badge are separate `Text`s so they can use different
        // colors. `Shrinkable` keeps the name at its intrinsic width so the
        // badge follows it directly; `Expanded` would strand the badge at the
        // far right.
        let mut label_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        label_row.add_child(
            Shrinkable::new(
                1.,
                Text::new(row.model_id.clone(), appearance.ui_font_family(), font_size)
                    .with_color(blended_colors::text_main(theme, background))
                    .soft_wrap(false)
                    .with_clip(ClipConfig::ellipsis())
                    .finish(),
            )
            .finish(),
        );
        if let Some(badge) = badge {
            label_row.add_child(
                Text::new(
                    format!(" ({badge})"),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(blended_colors::text_sub(theme, background))
                .finish(),
            );
        }

        let label_with_tooltip = with_tooltip(
            self.hover_state_for(format!("label:model:{}", row.model_id)),
            label_row.finish(),
            full_label,
            appearance,
        );

        // `Expanded` bounds the label to the space left after the swatch, so a
        // long model name ellipsis-clips instead of pushing the value and
        // chevron off the popover's edge.
        let left = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(7.)
            .with_child(render_swatch(color))
            .with_child(Expanded::new(1., label_with_tooltip).finish());

        let value = Text::new(
            format_tokens_and_cost(Some(row.tokens), row.cost, usage_display_unit),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(blended_colors::text_main(theme, background))
        .finish();
        let value = self.maybe_with_tooltip(
            format!("value:model:{}", row.model_id),
            value,
            exact_token_count_tooltip(row.tokens),
            appearance,
        );
        let chevron =
            ConstrainedBox::new(chevron_icon.to_warpui_icon(chevron_color.into()).finish())
                .with_width(10.)
                .with_height(10.)
                .finish();
        let right = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.)
            .with_child(value)
            .with_child(chevron);

        let summary_row = space_between_row()
            .with_child(Expanded::new(1., left.finish()).finish())
            .with_child(Container::new(right.finish()).with_margin_left(8.).finish())
            .finish();

        // Only the summary row toggles; wrapping the whole column would make
        // any click inside the expanded breakdown collapse it again.
        let model_id = row.model_id.clone();
        let summary_row = EventHandler::new(summary_row)
            .on_left_mouse_down(move |ctx, _, _| {
                ctx.dispatch_typed_action(UsagePopoverAction::ToggleModelExpanded(
                    model_id.clone(),
                ));
                DispatchEventResult::StopPropagation
            })
            .finish();

        let mut column = Flex::column().with_spacing(6.).with_child(summary_row);
        if expanded {
            let breakdown = match row.charged_usage {
                Some(charged_usage) => self.render_charged_usage_breakdown(
                    &row.model_id,
                    &charged_usage,
                    usage_display_unit,
                    appearance,
                ),
                None => Text::new(
                    "No detailed breakdown available".to_string(),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(blended_colors::text_disabled(theme, background))
                .finish(),
            };
            // Extra right padding keeps the breakdown's values clear of the
            // model row's value, which stops short to make room for its
            // chevron.
            column.add_child(
                Container::new(breakdown)
                    .with_padding_left(15.)
                    .with_padding_right(20.)
                    .finish(),
            );
        }

        column.finish()
    }

    /// Renders a model's input/output/cache/web-search charged-usage
    /// breakdown, shown beneath a per-model row when expanded. Rows are
    /// omitted for categories the model didn't incur (e.g. cache tokens
    /// only apply to Anthropic models, and web searches are relatively
    /// rare). Each token-count row gets an "exact amount" tooltip when its
    /// value is large enough to be abbreviated, keyed by `model_id` plus a
    /// per-category suffix so each row has its own persistent hover state.
    fn render_charged_usage_breakdown(
        &self,
        model_id: &str,
        charged_usage: &ModelChargedUsage,
        usage_display_unit: UsageDisplayUnit,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut column = Flex::column().with_spacing(4.);
        if charged_usage.input_tokens > 0 {
            column.add_child(self.render_label_value_row_with_tooltip(
                format!("value:model:{model_id}:input"),
                "Input tokens",
                format_tokens_and_cost(
                    Some(u64::from(charged_usage.input_tokens)),
                    Some(charged_usage.input_cost()),
                    usage_display_unit,
                ),
                exact_token_count_tooltip(u64::from(charged_usage.input_tokens)),
                appearance,
            ));
        }
        if charged_usage.output_tokens > 0 {
            column.add_child(self.render_label_value_row_with_tooltip(
                format!("value:model:{model_id}:output"),
                "Output tokens",
                format_tokens_and_cost(
                    Some(u64::from(charged_usage.output_tokens)),
                    Some(charged_usage.output_cost()),
                    usage_display_unit,
                ),
                exact_token_count_tooltip(u64::from(charged_usage.output_tokens)),
                appearance,
            ));
        }
        if charged_usage.input_cache_read_tokens > 0 {
            column.add_child(self.render_label_value_row_with_tooltip(
                format!("value:model:{model_id}:cache_read"),
                "Cache read tokens",
                format_tokens_and_cost(
                    Some(u64::from(charged_usage.input_cache_read_tokens)),
                    Some(charged_usage.input_cache_read_cost()),
                    usage_display_unit,
                ),
                exact_token_count_tooltip(u64::from(charged_usage.input_cache_read_tokens)),
                appearance,
            ));
        }
        if charged_usage.input_cache_write_tokens > 0 {
            column.add_child(self.render_label_value_row_with_tooltip(
                format!("value:model:{model_id}:cache_write"),
                "Cache write tokens",
                format_tokens_and_cost(
                    Some(u64::from(charged_usage.input_cache_write_tokens)),
                    Some(charged_usage.input_cache_write_cost()),
                    usage_display_unit,
                ),
                exact_token_count_tooltip(u64::from(charged_usage.input_cache_write_tokens)),
                appearance,
            ));
        }
        if charged_usage.web_search_count > 0 {
            column.add_child(render_label_value_row(
                "Web searches",
                format_searches_and_cost(
                    charged_usage.web_search_count,
                    charged_usage.web_search_cost(),
                    usage_display_unit,
                ),
                appearance,
            ));
        }
        column.finish()
    }

    /// Like [`render_label_value_row`], but wraps the value in a hover
    /// tooltip (see [`Self::maybe_with_tooltip`]) when `tooltip_text` is
    /// present -- used for token-count rows whose displayed value may be
    /// abbreviated.
    fn render_label_value_row_with_tooltip(
        &self,
        key: String,
        label: &str,
        value: String,
        tooltip_text: Option<String>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let font_size = appearance.ui_font_size();
        let value_element = Text::new(value, appearance.ui_font_family(), font_size)
            .with_color(blended_colors::text_main(theme, background))
            .finish();
        let value_element = self.maybe_with_tooltip(key, value_element, tooltip_text, appearance);
        space_between_row()
            .with_child(
                Text::new(label.to_string(), appearance.ui_font_family(), font_size)
                    .with_color(blended_colors::text_sub(theme, background))
                    .finish(),
            )
            .with_child(value_element)
            .finish()
    }

    /// Fetches (lazily creating if needed) the persistent hover state
    /// backing a tooltip keyed by `key`. See `hover_states`' docs for why
    /// persistence (vs. a fresh `MouseStateHandle::default()` per render)
    /// matters.
    fn hover_state_for(&self, key: impl Into<String>) -> MouseStateHandle {
        self.hover_states
            .borrow_mut()
            .entry(key.into())
            .or_default()
            .clone()
    }

    /// Wraps `content` in a hover tooltip showing `tooltip_text`, when
    /// present -- e.g. the exact token count behind an abbreviated "9.6k
    /// tokens" figure (see [`exact_token_count_tooltip`]). Returns
    /// `content` unchanged when `tooltip_text` is `None` (nothing to
    /// disambiguate, so no tooltip is worth showing).
    fn maybe_with_tooltip(
        &self,
        key: String,
        content: Box<dyn Element>,
        tooltip_text: Option<String>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        match tooltip_text {
            Some(tooltip_text) => {
                with_tooltip(self.hover_state_for(key), content, tooltip_text, appearance)
            }
            None => content,
        }
    }

    fn render_tool_call_summary_section(
        &self,
        conversation: &AIConversation,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let tool_usage = conversation.tool_usage_metadata();
        if tool_usage.total_tool_calls() == 0 {
            return None;
        }
        let mut column = Flex::column().with_spacing(8.);
        column.add_child(self.render_section_header(
            "TOOL CALL SUMMARY",
            self.tool_call_summary_section_expanded,
            CollapsedSummary::new(format!("{} tool calls", tool_usage.total_tool_calls())),
            SectionToggle {
                mouse_state: self.tool_call_summary_toggle_mouse_state.clone(),
                action: UsagePopoverAction::ToggleToolCallSummarySection,
            },
            appearance,
        ));
        if !self.tool_call_summary_section_expanded {
            return Some(column.finish());
        }

        let mut inner = Flex::column().with_spacing(4.);
        inner.add_child(render_label_value_row(
            "Tool calls",
            format!("{}", tool_usage.total_tool_calls()),
            appearance,
        ));
        inner.add_child(render_label_value_row(
            "Files changed",
            format!("{}", tool_usage.apply_file_diff_stats.files_changed),
            appearance,
        ));
        inner.add_child(render_diffs_row(
            tool_usage.apply_file_diff_stats.lines_added,
            tool_usage.apply_file_diff_stats.lines_removed,
            appearance,
        ));
        inner.add_child(render_label_value_row(
            "Commands executed",
            format!("{}", tool_usage.run_command_stats.commands_executed),
            appearance,
        ));
        column.add_child(inner.finish());
        Some(column.finish())
    }

    /// Unlike every other section here, these figures cover only the exchanges
    /// since the most recent user query, so the header says so explicitly.
    fn render_response_time_section(
        &self,
        conversation: &AIConversation,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let ttft_ms = conversation.time_to_first_token_for_last_user_query_ms();
        let response_ms = conversation.total_agent_response_time_since_last_user_query_ms();
        let wall_ms = conversation.wall_to_wall_response_time_since_last_query();
        if ttft_ms == 0 && response_ms == 0 && wall_ms.unwrap_or(0) == 0 {
            return None;
        }

        // Prefer the wall-to-wall total (including tool call time) for the
        // collapsed summary, since that's the most representative single
        // "total time" figure; fall back to agent response time alone when
        // the wall-clock total isn't available.
        let total_time_ms = wall_ms.filter(|&ms| ms != 0).unwrap_or(response_ms);

        let mut column = Flex::column().with_spacing(8.);
        column.add_child(self.render_section_header(
            "LAST RESPONSE TIME",
            self.response_time_section_expanded,
            CollapsedSummary::new(format!("{:.1}s", total_time_ms as f64 / 1000.)),
            SectionToggle {
                mouse_state: self.response_time_toggle_mouse_state.clone(),
                action: UsagePopoverAction::ToggleResponseTimeSection,
            },
            appearance,
        ));
        if !self.response_time_section_expanded {
            return Some(column.finish());
        }

        let mut inner = Flex::column().with_spacing(4.);
        inner.add_child(render_label_value_row(
            "Time to first token",
            format!("{:.1} seconds", ttft_ms as f64 / 1000.),
            appearance,
        ));
        inner.add_child(render_label_value_row(
            "Total agent response time",
            format!("{:.1} seconds", response_ms as f64 / 1000.),
            appearance,
        ));
        if let Some(wall_ms) = wall_ms
            && wall_ms != 0
        {
            inner.add_child(render_label_value_row(
                "Total time (including tool calls)",
                format!("{:.1} seconds", wall_ms as f64 / 1000.),
                appearance,
            ));
        }
        column.add_child(inner.finish());
        Some(column.finish())
    }
}

impl View for UsagePopoverView {
    fn ui_name() -> &'static str {
        "UsagePopoverView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let usage_display_unit = AISettings::as_ref(app).usage_display_unit;
        let theme = appearance.theme();
        let history = BlocklistAIHistoryModel::as_ref(app);
        let Some(conversation_id) = self.conversation_id else {
            return Empty::new().finish();
        };
        let Some(conversation) = history.conversation(&conversation_id) else {
            return Empty::new().finish();
        };
        let llm_preferences = LLMPreferences::as_ref(app);
        let custom_endpoint_label =
            |config_key: &str| llm_preferences.custom_endpoint_usage_display_label(config_key);
        let charged_usage_by_key =
            sum_charged_usage_by_key(conversation.all_tasks().flat_map(|task| task.messages()));

        // Absent sections are skipped rather than rendered empty, so they don't
        // leave the column's inter-section spacing behind as a stray gap.
        let sections = [
            Some(self.render_header(conversation, usage_display_unit, appearance)),
            self.render_usage_breakdown_section(
                conversation,
                &charged_usage_by_key,
                custom_endpoint_label,
                usage_display_unit,
                appearance,
            ),
            self.render_platform_usage_section(conversation, usage_display_unit, appearance),
            self.render_tool_call_summary_section(conversation, appearance),
            self.render_response_time_section(conversation, appearance),
        ];
        let mut column = Flex::column().with_spacing(12.);
        for section in sections.into_iter().flatten() {
            column.add_child(section);
        }

        let content = Container::new(column.finish())
            .with_background(theme.surface_2())
            .with_border(Border::all(1.).with_border_color(theme.outline().into_solid()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .with_uniform_padding(12.)
            .finish();

        let popover = ConstrainedBox::new(content)
            .with_width(POPOVER_WIDTH)
            .finish();

        // Swallows any left-click that lands within the popover's own bounds
        // (even on inert content like labels/padding) before it ever reaches
        // `Dismiss`'s outside-click check below — otherwise every click inside
        // the popover that doesn't land on an interactive element (a link,
        // section header, etc.) would be treated as an "outside" click and
        // close the popover.
        let popover = EventHandler::new(popover)
            .on_left_mouse_down(|_, _, _| DispatchEventResult::StopPropagation)
            .finish();

        Dismiss::new(popover)
            .prevent_interaction_with_other_elements()
            .on_dismiss(|ctx, _app| {
                ctx.dispatch_typed_action(UsagePopoverAction::RequestClose);
            })
            .finish()
    }
}

impl Entity for UsagePopoverView {
    type Event = UsagePopoverEvent;
}

impl TypedActionView for UsagePopoverView {
    type Action = UsagePopoverAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            UsagePopoverAction::ToggleModelUsageSection => {
                self.model_usage_section_expanded = !self.model_usage_section_expanded;
                ctx.notify();
            }
            UsagePopoverAction::ToggleToolCallSummarySection => {
                self.tool_call_summary_section_expanded = !self.tool_call_summary_section_expanded;
                ctx.notify();
            }
            UsagePopoverAction::ToggleResponseTimeSection => {
                self.response_time_section_expanded = !self.response_time_section_expanded;
                ctx.notify();
            }
            UsagePopoverAction::RequestClose => {
                ctx.emit(UsagePopoverEvent::Close);
            }
            UsagePopoverAction::ToggleModelExpanded(model_id) => {
                if !self.expanded_model_ids.remove(model_id) {
                    self.expanded_model_ids.insert(model_id.clone());
                }
                ctx.notify();
            }
        }
    }
}

/// The role a model was used in, derived from its token-usage categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelRole {
    PrimaryAgent,
    FullTerminalUse,
}

impl ModelRole {
    /// `None` for [`Self::PrimaryAgent`], which is the default role and so is
    /// noise on every row.
    fn badge_label(self) -> Option<&'static str> {
        match self {
            Self::PrimaryAgent => None,
            Self::FullTerminalUse => Some("Full terminal use"),
        }
    }
}

/// One row of the per-model usage breakdown.
struct ModelUsageRow {
    model_id: String,
    role: Option<ModelRole>,
    tokens: u64,
    cost: Option<CostValue>,
    charged_usage: Option<ModelChargedUsage>,
}

/// A known cost, in both display units, so the credits/dollars unit can be
/// applied at format time.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CostValue {
    credits: f32,
    cost_in_cents: f32,
}

impl CostValue {
    fn new(credits: f32, cost_in_cents: f32) -> Self {
        Self {
            credits,
            cost_in_cents,
        }
    }

    fn add_assign(&mut self, rhs: Self) {
        self.credits += rhs.credits;
        self.cost_in_cents += rhs.cost_in_cents;
    }
}

/// Per-model charged usage, summed at render time from the conversation's
/// persisted per-request `RequestMetadata` charges: every category's nested
/// per-model inference usage is folded together across all requests.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ModelChargedUsage {
    input_tokens: u32,
    output_tokens: u32,
    input_cache_read_tokens: u32,
    input_cache_write_tokens: u32,
    input_cost_in_cents: f32,
    output_cost_in_cents: f32,
    input_cache_read_cost_in_cents: f32,
    input_cache_write_cost_in_cents: f32,
    input_cost_in_credits: f32,
    output_cost_in_credits: f32,
    input_cache_read_cost_in_credits: f32,
    input_cache_write_cost_in_credits: f32,
    web_search_count: u32,
    web_search_cost_in_cents: f32,
    web_search_cost_in_credits: f32,
}

impl ModelChargedUsage {
    /// Every token bucket the expanded breakdown itemizes.
    fn tokens(&self) -> u64 {
        u64::from(self.input_tokens)
            + u64::from(self.output_tokens)
            + u64::from(self.input_cache_read_tokens)
            + u64::from(self.input_cache_write_tokens)
    }

    fn input_cost(&self) -> CostValue {
        CostValue::new(self.input_cost_in_credits, self.input_cost_in_cents)
    }

    fn output_cost(&self) -> CostValue {
        CostValue::new(self.output_cost_in_credits, self.output_cost_in_cents)
    }

    fn input_cache_read_cost(&self) -> CostValue {
        CostValue::new(
            self.input_cache_read_cost_in_credits,
            self.input_cache_read_cost_in_cents,
        )
    }

    fn input_cache_write_cost(&self) -> CostValue {
        CostValue::new(
            self.input_cache_write_cost_in_credits,
            self.input_cache_write_cost_in_cents,
        )
    }

    fn web_search_cost(&self) -> CostValue {
        CostValue::new(
            self.web_search_cost_in_credits,
            self.web_search_cost_in_cents,
        )
    }

    /// Every cost bucket the expanded breakdown itemizes.
    fn cost(&self) -> CostValue {
        let mut cost = CostValue::new(0., 0.);
        cost.add_assign(self.input_cost());
        cost.add_assign(self.output_cost());
        cost.add_assign(self.input_cache_read_cost());
        cost.add_assign(self.input_cache_write_cost());
        cost.add_assign(self.web_search_cost());
        cost
    }

    /// Whether any charge this aggregates would be invisible as a zero-token,
    /// zero-cost row.
    fn has_activity(&self) -> bool {
        self.tokens() > 0 || self.web_search_count > 0 || self.cost() != CostValue::new(0., 0.)
    }

    fn add(&mut self, usage: &api::InferenceUsage) {
        if let Some(token_count) = usage.token_count.as_ref() {
            self.input_tokens = self.input_tokens.saturating_add(token_count.input);
            self.output_tokens = self.output_tokens.saturating_add(token_count.output);
            self.input_cache_read_tokens = self
                .input_cache_read_tokens
                .saturating_add(token_count.input_cache_read);
            self.input_cache_write_tokens = self
                .input_cache_write_tokens
                .saturating_add(token_count.input_cache_write);
        }
        if let Some(token_cost) = usage.token_cost.as_ref() {
            self.input_cost_in_cents += token_cost.input_cost_in_cents;
            self.output_cost_in_cents += token_cost.output_cost_in_cents;
            self.input_cache_read_cost_in_cents += token_cost.input_cache_read_cost_in_cents;
            self.input_cache_write_cost_in_cents += token_cost.input_cache_write_cost_in_cents;
            self.input_cost_in_credits += token_cost.input_cost_in_credits;
            self.output_cost_in_credits += token_cost.output_cost_in_credits;
            self.input_cache_read_cost_in_credits += token_cost.input_cache_read_cost_in_credits;
            self.input_cache_write_cost_in_credits += token_cost.input_cache_write_cost_in_credits;
        }
        self.web_search_count = self.web_search_count.saturating_add(usage.web_search_count);
        self.web_search_cost_in_cents += usage.web_search_cost_in_cents;
        self.web_search_cost_in_credits += usage.web_search_cost_in_credits;
    }
}

/// Identity of a charge source, deliberately distinct from the display label:
/// a custom endpoint's display label can collide with a standard model's id,
/// and merging them would double-count the charges on both rows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ModelChargeKey {
    /// Warp API-key and BYOK usage, keyed by the server-known model id.
    Standard(String),
    /// Custom-endpoint usage, keyed by the upstream `config_key`.
    CustomEndpoint(String),
}

impl ModelChargeKey {
    fn display_label(&self, custom_endpoint_label: impl Fn(&str) -> String) -> String {
        match self {
            Self::Standard(model_id) => model_id.clone(),
            Self::CustomEndpoint(config_key) => custom_endpoint_label(config_key),
        }
    }
}

/// Folds every persisted `Message.RequestMetadata` record's nested
/// per-category, per-model charges into one `ModelChargedUsage` per charge
/// identity.
fn sum_charged_usage_by_key<'a>(
    messages: impl Iterator<Item = &'a api::Message>,
) -> HashMap<ModelChargeKey, ModelChargedUsage> {
    let mut by_key: HashMap<ModelChargeKey, ModelChargedUsage> = HashMap::new();
    for message in messages {
        let Some(api::message::Message::RequestMetadata(metadata)) = message.message.as_ref()
        else {
            continue;
        };
        let Some(charges) = metadata.charges.as_ref() else {
            continue;
        };
        for charged in charges.usage_by_category.values() {
            for (model_id, usage) in charged
                .direct_api_inference_usage
                .iter()
                .chain(charged.byok_inference_usage.iter())
            {
                by_key
                    .entry(ModelChargeKey::Standard(model_id.clone()))
                    .or_default()
                    .add(usage);
            }
            for (config_key, usage) in &charged.custom_endpoint_inference_usage {
                by_key
                    .entry(ModelChargeKey::CustomEndpoint(config_key.clone()))
                    .or_default()
                    .add(usage);
            }
        }
    }
    by_key
}

/// Tokens and cost for a set of rendered rows.
#[derive(Clone, Copy)]
struct RowTotals {
    tokens: Option<u64>,
    cost: Option<CostValue>,
}

impl RowTotals {
    /// Sums the values the model rows actually display, so a section summary
    /// built from this always equals the rows beneath it.
    ///
    /// The cost is `None` only when no row has a known cost; a partially
    /// attributed set still reports the portion that is known, matching the
    /// visible rows.
    fn of_model_rows(rows: &[ModelUsageRow]) -> Self {
        let cost = rows
            .iter()
            .filter_map(|row| row.cost)
            .reduce(|mut acc, cost| {
                acc.add_assign(cost);
                acc
            });
        Self {
            tokens: Some(rows.iter().map(|row| row.tokens).sum()),
            cost,
        }
    }
}

/// Joins a standard token row with its charged usage, keyed by the server
/// model id, marking the charge consumed so it can never attach to a second
/// row.
fn charged_usage_for_standard_row(
    model_id: &str,
    charged_usage_by_key: &HashMap<ModelChargeKey, ModelChargedUsage>,
    consumed_keys: &mut HashSet<ModelChargeKey>,
) -> Option<ModelChargedUsage> {
    let key = ModelChargeKey::Standard(model_id.to_string());
    charged_usage_by_key.get(&key).map(|charged_usage| {
        consumed_keys.insert(key);
        *charged_usage
    })
}

/// Joins a custom-endpoint token row with its charged usage. Both are labeled
/// through the same display-label lookup, so the join is by label; the first
/// unconsumed matching charge wins, so each charge attaches to at most one
/// row even when labels collide.
fn charged_usage_for_custom_row(
    label: &str,
    charged_usage_by_key: &HashMap<ModelChargeKey, ModelChargedUsage>,
    custom_endpoint_label: impl Fn(&str) -> String,
    consumed_keys: &mut HashSet<ModelChargeKey>,
) -> Option<ModelChargedUsage> {
    charged_usage_by_key
        .iter()
        .find(|(key, _)| match key {
            ModelChargeKey::CustomEndpoint(config_key) => {
                !consumed_keys.contains(key) && custom_endpoint_label(config_key) == label
            }
            ModelChargeKey::Standard(_) => false,
        })
        .map(|(key, charged_usage)| {
            consumed_keys.insert(key.clone());
            *charged_usage
        })
}

/// Builds the sorted per-model row list. Rows are ordered primary-agent-first,
/// then alphabetically by model id.
fn model_usage_rows(
    models: &[ModelTokenUsage],
    charged_usage_by_key: &HashMap<ModelChargeKey, ModelChargedUsage>,
    custom_endpoint_label: impl Fn(&str) -> String,
) -> Vec<ModelUsageRow> {
    let mut consumed_keys: HashSet<ModelChargeKey> = HashSet::new();
    let mut rows: Vec<ModelUsageRow> = models
        .iter()
        .filter_map(|model| {
            let reported_tokens = model.warp_tokens as u64
                + model.byok_tokens as u64
                + model.custom_endpoint_tokens as u64;
            let charged_usage = if model.custom_endpoint_tokens > 0 {
                charged_usage_for_custom_row(
                    &model.model_id,
                    charged_usage_by_key,
                    &custom_endpoint_label,
                    &mut consumed_keys,
                )
            } else {
                charged_usage_for_standard_row(
                    &model.model_id,
                    charged_usage_by_key,
                    &mut consumed_keys,
                )
            };
            if reported_tokens == 0 && charged_usage.is_none() {
                return None;
            }
            Some(ModelUsageRow {
                model_id: model.model_id.clone(),
                role: role_for_model(model),
                tokens: charged_usage
                    .as_ref()
                    .map(ModelChargedUsage::tokens)
                    .unwrap_or(reported_tokens),
                cost: charged_usage.as_ref().map(ModelChargedUsage::cost),
                charged_usage,
            })
        })
        .collect();
    // Charges without a token row to join (e.g. a restore that dropped token
    // metadata) still rendered, as long as they would not be invisible zeros.
    for (key, charged_usage) in charged_usage_by_key {
        if consumed_keys.contains(key) || !charged_usage.has_activity() {
            continue;
        }
        rows.push(ModelUsageRow {
            model_id: key.display_label(&custom_endpoint_label),
            role: None,
            tokens: charged_usage.tokens(),
            cost: Some(charged_usage.cost()),
            charged_usage: Some(*charged_usage),
        });
    }
    rows.sort_by(|a, b| {
        let primary = |role| role == Some(ModelRole::PrimaryAgent);
        match (primary(a.role), primary(b.role)) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => a.model_id.cmp(&b.model_id),
        }
    });
    rows
}

/// Determines a model's role from which token-usage category buckets it has
/// non-zero tokens in.
fn role_for_model(model: &ModelTokenUsage) -> Option<ModelRole> {
    let categories = [
        &model.warp_token_usage_by_category,
        &model.byok_token_usage_by_category,
        &model.custom_endpoint_token_usage_by_category,
    ];
    let has_category = |category: &str| {
        categories
            .iter()
            .any(|map| map.get(category).is_some_and(|&tokens| tokens > 0))
    };
    if has_category(PRIMARY_AGENT_CATEGORY) {
        Some(ModelRole::PrimaryAgent)
    } else if has_category(FULL_TERMINAL_USE_CATEGORY) {
        Some(ModelRole::FullTerminalUse)
    } else {
        None
    }
}

/// Formats a raw token count with `k`/`M` abbreviations, e.g. `9.6k`, `1.6M`.
///
/// The thresholds compare the *rounded* value, so 999,999 reads as `1.0M`
/// rather than `1000.0k`.
fn format_token_count(tokens: u64) -> String {
    /// Smallest quotient that still rounds up to `1.0` at one decimal place.
    const ROUNDS_TO_ONE: f64 = 0.9995;

    let millions = tokens as f64 / 1_000_000.;
    if millions >= ROUNDS_TO_ONE {
        return format!("{millions:.1}M");
    }
    let thousands = tokens as f64 / 1000.;
    if thousands >= ROUNDS_TO_ONE {
        return format!("{thousands:.1}k");
    }
    tokens.to_string()
}

/// Returns the exact (unabbreviated, comma-separated) token count for a
/// tooltip, e.g. `"9,614 tokens"` -- `None` when `tokens` is small enough
/// that [`format_token_count`] wouldn't have abbreviated it in the first
/// place, since a tooltip repeating an already-exact "500 tokens" would be
/// redundant.
fn exact_token_count_tooltip(tokens: u64) -> Option<String> {
    (tokens >= 1000).then(|| format!("{} tokens", tokens.separate_with_commas()))
}

/// Wraps `content` in a hover tooltip showing `tooltip_text` below its
/// bottom-left corner, using the given (persistent, per-instance)
/// `hover_state` so the hover-in delay can actually fire -- see
/// `UsagePopoverView::hover_states`' docs for why persistence matters.
fn with_tooltip(
    hover_state: MouseStateHandle,
    content: Box<dyn Element>,
    tooltip_text: String,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Hoverable::new(hover_state, |state| {
        let mut stack = Stack::new().with_child(content);
        if state.is_hovered() {
            stack.add_positioned_overlay_child(
                render_tooltip_box(tooltip_text, appearance),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 4.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }
        stack.finish()
    })
    .finish()
}

/// Renders a small opaque tooltip box containing `text`.
///
/// The alpha is forced to 255 because theme surfaces can carry a reduced alpha
/// from the user's window-opacity setting, and neither `into_solid()` nor
/// `coloru_with_opacity()` forces opacity.
fn render_tooltip_box(text: String, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let surface = theme.surface_3().into_solid();
    let bg = ColorU::new(surface.r, surface.g, surface.b, 255);
    Container::new(
        Text::new(text, appearance.ui_font_family(), appearance.ui_font_size())
            .with_color(blended_colors::text_main(theme, bg))
            .with_selectable(false)
            .finish(),
    )
    .with_background_color(bg)
    .with_border(Border::all(1.).with_border_color(theme.outline().into_solid()))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
    .with_padding_left(8.)
    .with_padding_right(8.)
    .with_padding_top(4.)
    .with_padding_bottom(4.)
    .with_drop_shadow(
        DropShadow::new_with_standard_offset_and_spread(ColorU::new(0, 0, 0, 48))
            .with_offset(vec2f(0., 4.)),
    )
    .finish()
}

/// The conversation-level total from the usage metadata: cumulative charged
/// usage in credits mode, the server-seeded provider cost in dollars mode.
/// A conversation with no cost data renders an em dash rather than a fake
/// zero.
pub(crate) fn conversation_total_text(
    conversation: &AIConversation,
    usage_display_unit: UsageDisplayUnit,
) -> String {
    let usage_totals = conversation.usage_totals();
    match usage_display_unit {
        UsageDisplayUnit::Dollars => usage_totals
            .total_cost_in_cents()
            .map(format_dollars)
            .unwrap_or_else(|| EM_DASH.to_string()),
        UsageDisplayUnit::Credits => usage_totals
            .charged_usage
            .map(|charged_usage| format_credits(charged_usage.total_cost_in_credits()))
            .unwrap_or_else(|| EM_DASH.to_string()),
    }
}

/// Formats a US-cent amount as dollars. A non-zero amount that would round to
/// `$0.00` is shown as `<$0.01`, since rounding it to zero would misleadingly
/// suggest no cost was incurred.
fn format_dollars(cost_in_cents: f32) -> String {
    // Summing float costs can produce `-0.0`, which would print as `$-0.00`.
    let cost_in_cents = if cost_in_cents == 0.0 {
        0.0
    } else {
        cost_in_cents
    };
    let dollars = cost_in_cents / 100.;
    if cost_in_cents > 0.0 && dollars < 0.01 {
        "<$0.01".to_string()
    } else {
        format!("${dollars:.2}")
    }
}

/// Formats a token count alongside its charged cost, e.g. `"9.6k tokens /
/// $0.36"`, falling back to whichever figure is known and to an em dash when
/// neither is.
fn format_tokens_and_cost(
    tokens: Option<u64>,
    cost: Option<CostValue>,
    usage_display_unit: UsageDisplayUnit,
) -> String {
    let token_text = tokens.map(|tokens| format!("{} tokens", format_token_count(tokens)));
    let cost_text = cost.map(|cost| match usage_display_unit {
        UsageDisplayUnit::Credits => format_credits(cost.credits),
        UsageDisplayUnit::Dollars => format_dollars(cost.cost_in_cents),
    });
    match (token_text, cost_text) {
        (Some(tokens), Some(cost)) => format!("{tokens} / {cost}"),
        (Some(tokens), None) => tokens,
        (None, Some(cost)) => cost,
        (None, None) => EM_DASH.to_string(),
    }
}

/// Formats a web-search count alongside its charged cost, e.g.
/// `"3 searches / $0.02"`.
fn format_searches_and_cost(
    count: u32,
    cost: CostValue,
    usage_display_unit: UsageDisplayUnit,
) -> String {
    let cost = match usage_display_unit {
        UsageDisplayUnit::Credits => format_credits(cost.credits),
        UsageDisplayUnit::Dollars => format_dollars(cost.cost_in_cents),
    };
    format!("{count} searches / {cost}")
}

/// Renders a small rounded color swatch used to key a row to its bar
/// segment.
fn render_swatch(color: ColorU) -> Box<dyn Element> {
    Container::new(
        ConstrainedBox::new(Empty::new().finish())
            .with_width(SWATCH_SIZE)
            .with_height(SWATCH_SIZE)
            .finish(),
    )
    .with_background_color(color)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
    .finish()
}

/// Renders a full-width segmented bar. `segments` is a list of (color,
/// percentage) pairs; any remaining percentage up to 100 is filled with
/// `track_color`. The leading and trailing edges of the bar are rounded
/// (each visible segment's own edges stay square except at those two ends),
/// giving the overall bar a pill-like shape.
fn render_segmented_bar(segments: &[(ColorU, f32)], track_color: ColorU) -> Box<dyn Element> {
    let mut visible: Vec<(ColorU, f32)> = segments
        .iter()
        .copied()
        .filter(|(_, pct)| *pct > 0.)
        .collect();
    let used_pct: f32 = visible.iter().map(|(_, pct)| pct).sum();
    let remainder = (100. - used_pct).max(0.);
    if remainder > 0. {
        visible.push((track_color, remainder));
    }

    let end_radius = Radius::Pixels(BAR_HEIGHT / 2.);
    let last_index = visible.len().saturating_sub(1);
    let mut row = Flex::row();
    for (index, (color, pct)) in visible.iter().enumerate() {
        let mut corner_radius = CornerRadius::default();
        if index == 0 {
            corner_radius.merge(CornerRadius::with_left(end_radius));
        }
        if index == last_index {
            corner_radius.merge(CornerRadius::with_right(end_radius));
        }
        row.add_child(
            Expanded::new(
                *pct,
                Container::new(Empty::new().finish())
                    .with_background_color(*color)
                    .with_corner_radius(corner_radius)
                    .finish(),
            )
            .finish(),
        );
    }

    ConstrainedBox::new(row.finish())
        .with_height(BAR_HEIGHT)
        .finish()
}

fn render_label_value_row(label: &str, value: String, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let background = theme.surface_2();
    let font_size = appearance.ui_font_size();
    space_between_row()
        .with_child(
            Text::new(label.to_string(), appearance.ui_font_family(), font_size)
                .with_color(blended_colors::text_sub(theme, background))
                .finish(),
        )
        .with_child(
            Text::new(value, appearance.ui_font_family(), font_size)
                .with_color(blended_colors::text_main(theme, background))
                .finish(),
        )
        .finish()
}

fn render_diffs_row(
    lines_added: i32,
    lines_removed: i32,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let background = theme.surface_2();
    let font_size = appearance.ui_font_size();
    space_between_row()
        .with_child(
            Text::new(
                "Diffs applied".to_string(),
                appearance.ui_font_family(),
                font_size,
            )
            .with_color(blended_colors::text_sub(theme, background))
            .finish(),
        )
        .with_child(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Text::new(
                        format!("+{lines_added}"),
                        appearance.ui_font_family(),
                        font_size,
                    )
                    .with_color(theme.ansi_fg_green())
                    .finish(),
                )
                .with_child(
                    Container::new(
                        Text::new(
                            format!("-{lines_removed}"),
                            appearance.ui_font_family(),
                            font_size,
                        )
                        .with_color(theme.ansi_fg_red())
                        .finish(),
                    )
                    .with_margin_left(6.)
                    .finish(),
                )
                .finish(),
        )
        .finish()
}

#[cfg(test)]
#[path = "usage_popover_view_tests.rs"]
mod tests;
