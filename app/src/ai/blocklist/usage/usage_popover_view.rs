//! The "Conversation" usage popover, anchored to the footer's usage icon.
//!
//! Two invariants hold across every figure shown here, since users add these
//! columns up:
//! * A section's summary equals the sum of the rows beneath it.
//! * A model row's value equals the sum of its expanded breakdown rows.
//!
//! Both are maintained by deriving section summaries from the same row list
//! that gets rendered, rather than from a separately-sourced aggregate.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use thousands::Separable;
use warp_core::ui::Icon;
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
use crate::ai::blocklist::agent_view::orchestration_pill_bar::{
    render_agent_avatar_disc, render_orchestrator_avatar_disc,
};
use crate::ai::blocklist::usage::colors::{ORCHESTRATOR_COLOR, chart_color};
use crate::ai::blocklist::usage::rollup::{
    AgentAvatar, OrchestrationCreditRollup, PerAgentCreditEntry, ROLLUP_TRUNCATION_CAP,
    compute_orchestration_rollup, truncate_rollup_rows,
};
use crate::appearance::Appearance;
use crate::persistence::model::{
    FULL_TERMINAL_USE_CATEGORY, ModelTokenUsage, PRIMARY_AGENT_CATEGORY, PersistedModelTokenCost,
};
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
    ShowAllRollupAgents,
    ShowFewerRollupAgents,
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

/// Floating "Conversation" usage popover. Holds only section-expand UI
/// state; all usage data is read live from [`BlocklistAIHistoryModel`] at
/// render time. The footer owns a single long-lived instance and calls
/// [`Self::reset_for_conversation`] each time the popover opens, so
/// section-collapse state resets to its default on reopen.
pub struct UsagePopoverView {
    /// `None` until the footer first opens the popover and points it at the
    /// active conversation.
    conversation_id: Option<AIConversationId>,
    model_usage_section_expanded: bool,
    tool_call_summary_section_expanded: bool,
    response_time_section_expanded: bool,
    rollup_show_all: bool,
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
    show_more_mouse_state: MouseStateHandle,
    show_fewer_mouse_state: MouseStateHandle,
    view_account_usage_mouse_state: MouseStateHandle,
}

impl UsagePopoverView {
    pub fn new(conversation_id: Option<AIConversationId>) -> Self {
        Self {
            conversation_id,
            model_usage_section_expanded: true,
            tool_call_summary_section_expanded: true,
            response_time_section_expanded: true,
            rollup_show_all: false,
            expanded_model_ids: HashSet::new(),
            hover_states: RefCell::new(HashMap::new()),
            model_usage_toggle_mouse_state: MouseStateHandle::default(),
            tool_call_summary_toggle_mouse_state: MouseStateHandle::default(),
            response_time_toggle_mouse_state: MouseStateHandle::default(),
            show_more_mouse_state: MouseStateHandle::default(),
            show_fewer_mouse_state: MouseStateHandle::default(),
            view_account_usage_mouse_state: MouseStateHandle::default(),
        }
    }

    /// Points this (reused) popover at `conversation_id` and resets all
    /// section-collapse state back to [`Self::new`]'s defaults.
    pub fn reset_for_conversation(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        *self = Self::new(Some(conversation_id));
        ctx.notify();
    }

    /// The total is deliberately not re-derived from the sections below, which
    /// under-report when the server hasn't attributed every charge to a model
    /// yet. It matches what the footer icon's tooltip shows.
    fn render_header(
        &self,
        conversation: &AIConversation,
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
            format_cost_only(conversation.usage_totals().total_cost_in_cents()),
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
    /// on the right, no click handling. Used for sections (e.g. Platform
    /// Usage) that have no expand/collapse state or separate content rows.
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

    /// Renders either the per-model breakdown (default) or, when an
    /// orchestration rollup applies, the per-agent breakdown in its place.
    ///
    /// Returns `None` when there is nothing to break down, so the caller can
    /// omit the section rather than emit an empty element that still consumes
    /// the parent column's spacing.
    fn render_usage_breakdown_section(
        &self,
        conversation: &AIConversation,
        rollup: Option<&OrchestrationCreditRollup>,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let (label, summary, rows) = match rollup {
            Some(rollup) => (
                "AGENT USAGE",
                RowTotals {
                    tokens: rollup.total_tokens.map(u64::from),
                    cost_in_cents: rollup.total_cost_in_cents,
                },
                self.render_agent_rollup_rows(rollup, appearance),
            ),
            None => {
                let rows = model_usage_rows(
                    conversation.token_usage(),
                    conversation.charged_usage_by_model(),
                );
                if rows.is_empty() {
                    return None;
                }
                (
                    "INFERENCE USAGE",
                    RowTotals::of_model_rows(&rows),
                    self.render_model_usage_rows(&rows, appearance),
                )
            }
        };

        let mut column = Flex::column().with_spacing(8.);
        column.add_child(
            self.render_section_header(
                label,
                self.model_usage_section_expanded,
                CollapsedSummary::new(format_tokens_and_cost(
                    summary.tokens,
                    summary.cost_in_cents,
                ))
                .with_tooltip(summary.tokens.and_then(exact_token_count_tooltip)),
                SectionToggle {
                    mouse_state: self.model_usage_toggle_mouse_state.clone(),
                    action: UsagePopoverAction::ToggleModelUsageSection,
                },
                appearance,
            ),
        );
        if self.model_usage_section_expanded {
            column.add_child(rows);
        }
        Some(column.finish())
    }

    /// Renders the non-collapsible "PLATFORM USAGE" section: Warp's platform
    /// fee, which unlike inference cost isn't attributable to any single
    /// model.
    ///
    /// Omitted entirely when the server sent no charged usage at all. A charged
    /// usage whose platform fee is zero renders `$0.00`, matching how the rest
    /// of the popover distinguishes a known zero from an unknown figure.
    fn render_platform_usage_section(
        &self,
        conversation: &AIConversation,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let platform_cost_in_cents = conversation
            .usage_totals()
            .charged_usage
            .map(|charged_usage| charged_usage.platform_cost_in_cents)?;

        Some(self.render_static_section_header_with_value(
            "PLATFORM USAGE",
            format_cost_only(Some(platform_cost_in_cents)),
            appearance,
        ))
    }

    fn render_model_usage_rows(
        &self,
        rows: &[ModelUsageRow],
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let totals = RowTotals::of_model_rows(rows);
        let total_tokens = totals.tokens.unwrap_or(0);
        let theme = appearance.theme();
        let background = theme.surface_2();
        let font_size = appearance.ui_font_size();

        let mut column = Flex::column().with_spacing(6.);

        let all_models_value = Text::new(
            format_tokens_and_cost(totals.tokens, totals.cost_in_cents),
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
            column.add_child(self.render_model_usage_row(index, row, appearance));
        }

        column.finish()
    }

    /// Renders a per-model row. Every row is clickable (a trailing chevron
    /// indicates this) and toggles a breakdown subsection beneath it — an
    /// input/output/cache/web-search split when the model has a known
    /// charged-usage breakdown, or a fallback message when it doesn't (e.g.
    /// the server hasn't sent per-model charges for this conversation yet).
    fn render_model_usage_row(
        &self,
        index: usize,
        row: &ModelUsageRow,
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
            format_tokens_and_cost(Some(row.tokens), row.cost_in_cents),
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
                Some(charged_usage) => {
                    self.render_charged_usage_breakdown(&row.model_id, &charged_usage, appearance)
                }
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
        charged_usage: &PersistedModelTokenCost,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut column = Flex::column().with_spacing(4.);
        if charged_usage.total_input > 0 {
            column.add_child(self.render_label_value_row_with_tooltip(
                format!("value:model:{model_id}:input"),
                "Input tokens",
                format_tokens_and_cost(
                    Some(charged_usage.total_input),
                    Some(charged_usage.input_cost_in_cents),
                ),
                exact_token_count_tooltip(charged_usage.total_input),
                appearance,
            ));
        }
        if charged_usage.output > 0 {
            column.add_child(self.render_label_value_row_with_tooltip(
                format!("value:model:{model_id}:output"),
                "Output tokens",
                format_tokens_and_cost(
                    Some(charged_usage.output),
                    Some(charged_usage.output_cost_in_cents),
                ),
                exact_token_count_tooltip(charged_usage.output),
                appearance,
            ));
        }
        if charged_usage.input_cache_read > 0 {
            column.add_child(self.render_label_value_row_with_tooltip(
                format!("value:model:{model_id}:cache_read"),
                "Cache read tokens",
                format_tokens_and_cost(
                    Some(charged_usage.input_cache_read),
                    Some(charged_usage.input_cache_read_cost_in_cents),
                ),
                exact_token_count_tooltip(charged_usage.input_cache_read),
                appearance,
            ));
        }
        if charged_usage.input_cache_write > 0 {
            column.add_child(self.render_label_value_row_with_tooltip(
                format!("value:model:{model_id}:cache_write"),
                "Cache write tokens",
                format_tokens_and_cost(
                    Some(charged_usage.input_cache_write),
                    Some(charged_usage.input_cache_write_cost_in_cents),
                ),
                exact_token_count_tooltip(charged_usage.input_cache_write),
                appearance,
            ));
        }
        if charged_usage.web_search_count > 0 {
            column.add_child(render_label_value_row(
                "Web searches",
                format_searches_and_cost(
                    charged_usage.web_search_count as u32,
                    charged_usage.web_search_cost_in_cents,
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

    /// Per-agent breakdown, using the same stacked-bar + swatch treatment as
    /// the per-model breakdown.
    fn render_agent_rollup_rows(
        &self,
        rollup: &OrchestrationCreditRollup,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let font_size = appearance.ui_font_size();

        let mut column = Flex::column().with_spacing(6.);
        let all_agents_tokens = rollup.total_tokens.map(u64::from);
        let all_agents_value = Text::new(
            format_tokens_and_cost(all_agents_tokens, rollup.total_cost_in_cents),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(blended_colors::text_main(theme, background))
        .finish();
        let all_agents_value = self.maybe_with_tooltip(
            "value:all_agents".to_string(),
            all_agents_value,
            all_agents_tokens.and_then(exact_token_count_tooltip),
            appearance,
        );
        column.add_child(
            space_between_row()
                .with_child(
                    Text::new(
                        "All agents".to_string(),
                        appearance.ui_font_family(),
                        font_size,
                    )
                    .with_color(blended_colors::text_sub(theme, background))
                    .finish(),
                )
                .with_child(all_agents_value)
                .finish(),
        );

        // Proportioned by the cost the rows actually display rather than by
        // credits, which are never shown; credits would make the segment
        // widths disagree with the numbers beside them. Falls back to credits
        // only when no contributor has a known dollar figure.
        let cost_total = rollup.total_cost_in_cents.filter(|total| *total > 0.);
        let segments: Vec<(ColorU, f32)> = rollup
            .per_agent
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let pct = match cost_total {
                    Some(total) => (entry.cost_in_cents.unwrap_or(0.) / total) * 100.,
                    None if rollup.total_credits > 0. => {
                        (entry.credits_spent / rollup.total_credits) * 100.
                    }
                    None => 0.,
                };
                (agent_row_color(entry, index), pct)
            })
            .collect();
        column.add_child(render_segmented_bar(
            &segments,
            theme.outline().into_solid(),
        ));

        let (shown, hidden_count) = truncate_rollup_rows(&rollup.per_agent, self.rollup_show_all);
        for (index, entry) in shown.iter().enumerate() {
            column.add_child(self.render_agent_rollup_row(index, entry, appearance));
        }
        if hidden_count > 0 {
            column.add_child(self.render_show_more_link(hidden_count, appearance));
        } else if self.rollup_show_all && rollup.per_agent.len() > ROLLUP_TRUNCATION_CAP {
            column.add_child(self.render_show_fewer_link(appearance));
        }

        column.finish()
    }

    fn render_agent_rollup_row(
        &self,
        index: usize,
        entry: &PerAgentCreditEntry,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let font_size = appearance.ui_font_size();
        const ROW_AVATAR_SIZE: f32 = 16.;
        let swatch = render_swatch(agent_row_color(entry, index));
        let avatar = match entry.avatar {
            AgentAvatar::Orchestrator => {
                render_orchestrator_avatar_disc(ROW_AVATAR_SIZE, theme, appearance)
            }
            AgentAvatar::Child => {
                render_agent_avatar_disc(&entry.display_name, ROW_AVATAR_SIZE, theme, appearance)
            }
        };
        let name = Text::new(
            entry.display_name.clone(),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(blended_colors::text_main(theme, background))
        .soft_wrap(false)
        .with_clip(ClipConfig::ellipsis())
        .finish();
        let entry_tokens = entry.tokens.map(u64::from);
        let value = Text::new(
            format_tokens_and_cost(entry_tokens, entry.cost_in_cents),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(blended_colors::text_sub(theme, background))
        .finish();
        let value = self.maybe_with_tooltip(
            format!("value:agent:{}", entry.conversation_id),
            value,
            entry_tokens.and_then(exact_token_count_tooltip),
            appearance,
        );

        space_between_row()
            .with_child(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_spacing(8.)
                    .with_child(swatch)
                    .with_child(avatar)
                    .with_child(name)
                    .finish(),
            )
            .with_child(value)
            .finish()
    }

    fn render_show_more_link(
        &self,
        hidden_count: usize,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        render_text_link(
            format!("Show {hidden_count} more"),
            self.show_more_mouse_state.clone(),
            UsagePopoverAction::ShowAllRollupAgents,
            appearance,
        )
    }

    /// A way back to the truncated view without collapsing and reopening the
    /// whole section.
    fn render_show_fewer_link(&self, appearance: &Appearance) -> Box<dyn Element> {
        render_text_link(
            "Show fewer".to_string(),
            self.show_fewer_mouse_state.clone(),
            UsagePopoverAction::ShowFewerRollupAgents,
            appearance,
        )
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
        let theme = appearance.theme();
        let history = BlocklistAIHistoryModel::as_ref(app);
        let Some(conversation_id) = self.conversation_id else {
            return Empty::new().finish();
        };
        let Some(conversation) = history.conversation(&conversation_id) else {
            return Empty::new().finish();
        };
        let rollup = compute_orchestration_rollup(conversation_id, history);

        // Absent sections are skipped rather than rendered empty, so they don't
        // leave the column's inter-section spacing behind as a stray gap.
        let sections = [
            Some(self.render_header(conversation, appearance)),
            self.render_usage_breakdown_section(conversation, rollup.as_ref(), appearance),
            self.render_platform_usage_section(conversation, appearance),
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
            UsagePopoverAction::ShowAllRollupAgents => {
                self.rollup_show_all = true;
                ctx.notify();
            }
            UsagePopoverAction::ShowFewerRollupAgents => {
                self.rollup_show_all = false;
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
    cost_in_cents: Option<f32>,
    charged_usage: Option<PersistedModelTokenCost>,
}

/// Every token bucket the expanded breakdown itemizes.
///
/// Deliberately not [`PersistedModelTokenCost::tokens`], which excludes cache
/// tokens: the breakdown shows cache rows, so excluding them here would leave
/// the rows summing past their own row total.
fn charged_usage_tokens(usage: &PersistedModelTokenCost) -> u64 {
    usage.total_input + usage.output + usage.input_cache_read + usage.input_cache_write
}

/// Every cost bucket the expanded breakdown itemizes, including web search
/// (which [`PersistedModelTokenCost::cost_in_cents`] omits).
fn charged_usage_cost_in_cents(usage: &PersistedModelTokenCost) -> f32 {
    usage.cost_in_cents() + usage.web_search_cost_in_cents
}

/// Tokens and cost for a set of rendered rows.
#[derive(Clone, Copy)]
struct RowTotals {
    tokens: Option<u64>,
    cost_in_cents: Option<f32>,
}

impl RowTotals {
    /// Sums the values the model rows actually display, so a section summary
    /// built from this always equals the rows beneath it.
    ///
    /// The cost is `None` only when no row has a known cost; a partially
    /// attributed set still reports the portion that is known, matching the
    /// visible rows.
    fn of_model_rows(rows: &[ModelUsageRow]) -> Self {
        let cost_in_cents = rows
            .iter()
            .filter_map(|row| row.cost_in_cents)
            .reduce(|acc, cost| acc + cost);
        Self {
            tokens: Some(rows.iter().map(|row| row.tokens).sum()),
            cost_in_cents,
        }
    }
}

/// Builds the sorted per-model row list. Rows are ordered primary-agent-first,
/// then alphabetically by model id.
///
/// Tokens and cost come from the per-model charged-usage breakdown when the
/// server has attributed charges to this model, so the row total matches the
/// breakdown rows shown when it's expanded. Models without attributed charges
/// fall back to the raw token counts and render "No detailed breakdown
/// available" when expanded, so there's nothing to disagree with.
fn model_usage_rows(
    models: &[ModelTokenUsage],
    charged_usage_by_model: &HashMap<String, PersistedModelTokenCost>,
) -> Vec<ModelUsageRow> {
    let mut rows: Vec<ModelUsageRow> = models
        .iter()
        .filter_map(|model| {
            let reported_tokens = model.warp_tokens as u64
                + model.byok_tokens as u64
                + model.custom_endpoint_tokens as u64;
            if reported_tokens == 0 {
                return None;
            }
            let charged_usage = charged_usage_by_model.get(&model.model_id).copied();
            Some(ModelUsageRow {
                model_id: model.model_id.clone(),
                role: role_for_model(model),
                tokens: charged_usage
                    .as_ref()
                    .map(charged_usage_tokens)
                    .unwrap_or(reported_tokens),
                cost_in_cents: charged_usage.as_ref().map(charged_usage_cost_in_cents),
                charged_usage,
            })
        })
        .collect();
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

/// The orchestrator keeps its fixed identity color; children take chart
/// colors by position, matching the per-model bar.
fn agent_row_color(entry: &PerAgentCreditEntry, index: usize) -> ColorU {
    match entry.avatar {
        AgentAvatar::Orchestrator => ORCHESTRATOR_COLOR,
        AgentAvatar::Child => chart_color(index),
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

/// Formats a token count alongside its dollar cost, e.g. `"9.6k tokens /
/// $0.36"`, falling back to whichever figure is known and to an em dash when
/// neither is.
pub(crate) fn format_tokens_and_cost(tokens: Option<u64>, cost_in_cents: Option<f32>) -> String {
    let token_text = tokens.map(|tokens| format!("{} tokens", format_token_count(tokens)));
    let cost_text = cost_in_cents.map(format_cents);
    match (token_text, cost_text) {
        (Some(tokens), Some(cost)) => format!("{tokens} / {cost}"),
        (Some(tokens), None) => tokens,
        (None, Some(cost)) => cost,
        (None, None) => EM_DASH.to_string(),
    }
}

/// Formats a web-search count alongside its dollar cost, e.g.
/// `"3 searches / $0.02"`.
fn format_searches_and_cost(count: u32, cost_in_cents: f32) -> String {
    format!("{count} searches / {}", format_cents(cost_in_cents))
}

/// Formats a bare dollar cost, e.g. `"$0.36"`, or an em dash when unknown.
pub(crate) fn format_cost_only(cost_in_cents: Option<f32>) -> String {
    cost_in_cents
        .map(format_cents)
        .unwrap_or(EM_DASH.to_string())
}

fn format_cents(cost_in_cents: f32) -> String {
    format!("${:.2}", cost_in_cents / 100.)
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

/// Renders a hyperlink-styled, non-chevron text link (used for "Show N
/// more" / "Show fewer").
fn render_text_link(
    label: String,
    mouse_state: MouseStateHandle,
    action: UsagePopoverAction,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let link_color = theme.ansi_fg_blue();
    let font_size = appearance.ui_font_size();
    let font_family = appearance.ui_font_family();
    Hoverable::new(mouse_state, move |_state| {
        Text::new(label.clone(), font_family, font_size)
            .with_color(link_color)
            .with_selectable(false)
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(action.clone());
    })
    .finish()
}

#[cfg(test)]
#[path = "usage_popover_view_tests.rs"]
mod tests;
