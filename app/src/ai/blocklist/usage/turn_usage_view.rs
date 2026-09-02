//! The docked, closeable "Turn" panel. Unlike
//! [`super::conversation_usage_view::ConversationUsageView`] (which shows
//! conversation-cumulative totals, optionally alongside a "last response"
//! annotation), the tool-call/diff/command/token/cost values in this view
//! are scoped to a single agent turn ("block") -- see
//! `AIConversation::current_turn_usage`/`turn_usage_snapshot_for_exchange`.
//! `context_window_usage` is the one exception: it is inherently
//! conversation-level, captured here as the conversation's cumulative value
//! as of that turn (see [`TurnUsageInfo::context_window_usage`]).
//!
//! This panel is triggered independently from (and has no cross-navigation
//! link to) the "Conversation" popover; has no per-section collapse/expand
//! affordance (all sections are always fully expanded); aligns the value
//! column across all sections, not just within each section; and is
//! dismissed via a standard "X" close button in the header.

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DropShadow, Flex,
    Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, Text,
};
use warpui::platform::Cursor;
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use super::conversation_usage_view::TimingInfo;
use super::render_context_window_usage_icon;
use crate::ai::blocklist::view_util::format_credits;
use crate::appearance::Appearance;
use crate::persistence::model::PersistedModelTokenCost;
use crate::settings::{AISettings, AISettingsChangedEvent, UsageDisplayUnit};
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;

/// A single label/value pair rendered as a row in the panel's shared
/// label/value columns (see [`TurnUsageView::render`]).
type LabelValueRow = (Box<dyn Element>, Box<dyn Element>);

/// The panel's two shared columns, as parallel `(labels, values)` vectors.
/// See [`TurnUsageView::build_label_value_columns`].
type LabelValueColumns = (Vec<Box<dyn Element>>, Vec<Box<dyn Element>>);

/// Turn-scoped token/cost usage for a single model. A turn can involve
/// multiple models (e.g. if the user or router switched models mid-turn).
pub struct TurnModelUsage {
    /// The model's display identifier (e.g. `auto (cost-efficient)`).
    pub model_id: String,
    /// Token/cost usage accrued by this model during this turn.
    pub usage: PersistedModelTokenCost,
}

/// Turn-scoped usage data backing the "INFERENCE USAGE" section. All fields
/// are scoped to a single agent turn (block), not the whole conversation.
pub struct TurnUsageInfo {
    /// Per-model token/cost usage for this turn. One row is rendered per
    /// entry.
    pub models: Vec<TurnModelUsage>,
    /// The conversation's cumulative context window usage (0.0-1.0) as of
    /// this turn. Context window usage is inherently conversation-level --
    /// it cannot be scoped to a single turn -- so this is a point-in-time
    /// value (the conversation's running total as of this turn) rather than
    /// a per-turn delta.
    pub context_window_usage: f32,
    /// Platform usage charged (in US cents) over this turn. Rendered as its
    /// own "PLATFORM USAGE" section by [`TurnUsageView::platform_usage_rows`].
    pub platform_usage_in_cents: Option<f32>,
    /// Inference-only credits spent over this turn (the combined
    /// inference + platform legacy credits total, minus the platform
    /// portion). Rendered as a normal "Credits" row under INFERENCE USAGE,
    /// shown only when the user's usage-display-unit setting is `Credits`.
    /// `None` if no cost data has landed for this turn yet.
    pub inference_credits_spent_for_last_block: Option<f32>,
    /// Platform-only credits spent over this turn. Rendered as a normal
    /// "Credits" row under PLATFORM USAGE (alongside its dollar figure),
    /// shown only when the usage-display-unit setting is `Credits` and
    /// there's a nonzero value to show.
    pub platform_credits_spent_for_last_block: Option<f32>,
    pub tool_calls: i32,
    pub files_changed: i32,
    pub lines_added: i32,
    pub lines_removed: i32,
    pub commands_executed: i32,
}

/// Typed actions dispatched by widgets inside [`TurnUsageView`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnUsageViewAction {
    /// The user clicked the header's close ("X") button.
    Close,
    /// The user clicked a model row's label to expand/collapse its
    /// input/output/cache token breakdown. Carries the row's index into
    /// [`TurnUsageInfo::models`].
    ToggleModelExpanded(usize),
}

/// Emitted so the owning view (the terminal view) can remove this panel
/// from the blocklist when the user clicks the close button.
#[derive(Clone, Debug)]
pub enum TurnUsageViewEvent {
    CloseRequested,
}

/// Hover/click state and expanded-breakdown flag for a single row in
/// [`TurnUsageInfo::models`], indexed in lockstep with it so there is
/// structurally only one length to keep in sync.
struct ModelRowState {
    mouse: MouseStateHandle,
    expanded: bool,
}

/// The docked "Turn" panel view. See module docs for scope/behavior.
pub struct TurnUsageView {
    usage_info: TurnUsageInfo,
    pub timing_info: Option<TimingInfo>,
    close_button_mouse_state: MouseStateHandle,
    /// Per-model row UI state, indexed in lockstep with `usage_info.models`.
    model_rows: Vec<ModelRowState>,
}

impl TurnUsageView {
    pub fn new(
        usage_info: TurnUsageInfo,
        timing_info: Option<TimingInfo>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        // The "CREDITS" section's visibility depends on the Credits/Dollars
        // usage-display-unit setting, so the panel must re-render when the
        // user flips it -- otherwise an already-open panel would show a
        // stale section state until closed and reopened.
        ctx.subscribe_to_model(&AISettings::handle(ctx), |_, _, event, ctx| {
            if matches!(event, AISettingsChangedEvent::UsageDisplayUnit { .. }) {
                ctx.notify();
            }
        });

        let model_rows = usage_info
            .models
            .iter()
            .map(|_| ModelRowState {
                mouse: MouseStateHandle::default(),
                expanded: false,
            })
            .collect();
        Self {
            usage_info,
            timing_info,
            close_button_mouse_state: MouseStateHandle::default(),
            model_rows,
        }
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let font_size = appearance.ui_font_size() + 2.;

        let title = Text::new("Turn".to_string(), appearance.ui_font_family(), font_size)
            .with_style(warpui::fonts::Properties {
                weight: warpui::fonts::Weight::Bold,
                ..Default::default()
            })
            .with_color(blended_colors::text_main(theme, background))
            .finish();

        let close_icon_size = font_size;
        let close_button = Hoverable::new(self.close_button_mouse_state.clone(), {
            let icon_color = blended_colors::text_sub(theme, background);
            move |state| {
                let mut container = Container::new(
                    ConstrainedBox::new(Icon::X.to_warpui_icon(icon_color.into()).finish())
                        .with_width(close_icon_size)
                        .with_height(close_icon_size)
                        .finish(),
                )
                .with_uniform_padding(2.);
                if state.is_hovered() {
                    container = container
                        .with_background(blended_colors::neutral_4(appearance.theme()))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
                }
                container.finish()
            }
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(TurnUsageViewAction::Close);
        })
        .finish();

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(title)
            .with_child(close_button)
            .finish()
    }

    /// Renders a section's small-caps label as a standalone row, to be
    /// followed by that section's data rows in the shared label/value
    /// columns. Purely decorative (no chevron, not clickable): all sections
    /// are always fully expanded.
    fn render_section_header(label: &str, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = theme.surface_2();
        // A few points larger than the base overline size so the
        // section headers read clearly against the smaller body text.
        let header_font_size = appearance.overline_font_size() + 3.;
        Text::new(
            label.to_string(),
            appearance.overline_font_family(),
            header_font_size,
        )
        .with_color(blended_colors::text_disabled(theme, background))
        .soft_wrap(false)
        .finish()
    }

    /// Builds the clickable label/value row for a single model, plus (when
    /// that row is expanded) the indented input/output/cache breakdown rows
    /// immediately following it.
    fn model_row(&self, index: usize, appearance: &Appearance) -> Vec<LabelValueRow> {
        let model = &self.usage_info.models[index];
        let font_size = appearance.ui_font_size() + 1.;
        let theme = appearance.theme();
        let background = theme.surface_2();
        let text_color = blended_colors::text_main(theme, background);
        let row_state = &self.model_rows[index];
        let expanded = row_state.expanded;

        let value = Text::new(
            format_tokens_with_cost(model.usage.tokens(), model.usage.cost_in_cents()),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(text_color)
        .finish();

        let font_family = appearance.ui_font_family();
        let model_name = model.model_id.clone();
        // Collapsed points right (the row can be expanded further); expanded
        // points down (the breakdown is already showing below).
        let chevron_icon = if expanded {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };
        let label = Hoverable::new(row_state.mouse.clone(), move |_state| {
            let text_element = Text::new(model_name.clone(), font_family, font_size)
                .with_style(warpui::fonts::Properties {
                    weight: warpui::fonts::Weight::Medium,
                    ..Default::default()
                })
                .with_color(text_color)
                .finish();
            let icon_element =
                ConstrainedBox::new(chevron_icon.to_warpui_icon(text_color.into()).finish())
                    .with_width(font_size)
                    .with_height(font_size)
                    .finish();
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(4.)
                .with_child(icon_element)
                .with_child(text_element)
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(TurnUsageViewAction::ToggleModelExpanded(index));
        })
        .finish();

        let mut rows = vec![(label, value)];
        if expanded {
            // Smaller than the rest of the panel's body text, to visually
            // distinguish the breakdown as a nested detail.
            let breakdown_font_size = appearance.ui_font_size() - 1.;
            let usage = &model.usage;

            rows.push((
                render_indented_label_text("Input", breakdown_font_size, appearance),
                render_value_text(
                    format_tokens_with_cost(usage.total_input, usage.input_cost_in_cents),
                    breakdown_font_size,
                    appearance,
                ),
            ));
            rows.push((
                render_indented_label_text("Output", breakdown_font_size, appearance),
                render_value_text(
                    format_tokens_with_cost(usage.output, usage.output_cost_in_cents),
                    breakdown_font_size,
                    appearance,
                ),
            ));
            if usage.input_cache_read > 0 {
                rows.push((
                    render_indented_label_text("Cache read", breakdown_font_size, appearance),
                    render_value_text(
                        format_tokens_with_cost(
                            usage.input_cache_read,
                            usage.input_cache_read_cost_in_cents,
                        ),
                        breakdown_font_size,
                        appearance,
                    ),
                ));
            }
            if usage.input_cache_write > 0 {
                rows.push((
                    render_indented_label_text("Cache write", breakdown_font_size, appearance),
                    render_value_text(
                        format_tokens_with_cost(
                            usage.input_cache_write,
                            usage.input_cache_write_cost_in_cents,
                        ),
                        breakdown_font_size,
                        appearance,
                    ),
                ));
            }
            if usage.web_search_count > 0 {
                rows.push((
                    render_indented_label_text("Web search", breakdown_font_size, appearance),
                    render_value_text(
                        format!(
                            "{}  /  {}",
                            format_web_searches(usage.web_search_count),
                            format_dollars(usage.web_search_cost_in_cents)
                        ),
                        breakdown_font_size,
                        appearance,
                    ),
                ));
            }
        }
        rows
    }

    /// Label/value rows for the per-model usage entries, plus (in Credits
    /// mode) a trailing "Credits" row for the turn's inference-only credit
    /// total. Context window usage and platform usage are rendered as
    /// their own items in [`Self::build_label_value_columns`] rather than
    /// folded into this list, so their spacing (and, for platform usage,
    /// its section-header styling) can be controlled independently.
    fn model_usage_rows(
        &self,
        appearance: &Appearance,
        usage_display_unit: UsageDisplayUnit,
    ) -> Vec<LabelValueRow> {
        let mut rows: Vec<LabelValueRow> = (0..self.usage_info.models.len())
            .flat_map(|index| self.model_row(index, appearance))
            .collect();
        if usage_display_unit == UsageDisplayUnit::Credits
            && let Some(credits) = self.usage_info.inference_credits_spent_for_last_block
        {
            rows.push((
                render_label_text("Credits", appearance),
                render_value_text(
                    format_credits(credits),
                    appearance.ui_font_size() + 2.,
                    appearance,
                ),
            ));
        }
        rows
    }

    /// The "INFERENCE USAGE" section header, with the turn's total tokens
    /// and cost in the value column instead of an empty placeholder, so the
    /// total lines up with the other numeric amounts in the value column.
    fn inference_usage_header_row(&self, appearance: &Appearance) -> LabelValueRow {
        let header_font_size = appearance.overline_font_size() + 3.;
        let total_tokens: u64 = self
            .usage_info
            .models
            .iter()
            .map(|m| m.usage.tokens())
            .sum();
        let total_cost_in_cents: f32 = self
            .usage_info
            .models
            .iter()
            .map(|m| m.usage.cost_in_cents())
            .sum();
        let theme = appearance.theme();
        let value = Text::new(
            format_tokens_with_cost(total_tokens, total_cost_in_cents),
            appearance.ui_font_family(),
            header_font_size,
        )
        .with_style(warpui::fonts::Properties {
            weight: warpui::fonts::Weight::Bold,
            ..Default::default()
        })
        .with_color(blended_colors::text_main(theme, theme.surface_2()))
        .finish();

        (
            Self::render_section_header("INFERENCE USAGE", appearance),
            value,
        )
    }

    /// The "Context window usage" row, shown beneath the per-model rows and
    /// (when present) the "PLATFORM USAGE" section.
    fn context_window_usage_row(&self, appearance: &Appearance) -> LabelValueRow {
        let font_size = appearance.ui_font_size() + 2.;
        let theme = appearance.theme();
        // Matches the model row labels' color (rather than the dimmer
        // `text_sub` used by other section labels) since context window
        // usage is displayed alongside model usage in the same section.
        let text_color = blended_colors::text_main(theme, theme.surface_2());

        let label = Text::new(
            "Context window usage".to_string(),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(text_color)
        .finish();
        let context_usage_pct = (self.usage_info.context_window_usage * 100.).round();
        let value = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(4.)
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
            .with_child(
                Text::new(
                    format!("{context_usage_pct}%"),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(text_color)
                .finish(),
            )
            .finish();

        (label, value)
    }

    /// The "PLATFORM USAGE" section: a section-header row with the dollar
    /// amount in the value column, plus (in Credits mode, if any) a
    /// trailing "Credits" row for the turn's platform-only credit total.
    /// `None` when there's no charge data yet or the charge is truly zero
    /// (to avoid a noisy `$0.00` section).
    fn platform_usage_rows(
        &self,
        appearance: &Appearance,
        usage_display_unit: UsageDisplayUnit,
    ) -> Option<Vec<LabelValueRow>> {
        let platform_usage_in_cents = self.usage_info.platform_usage_in_cents?;
        if platform_usage_in_cents <= 0.0 {
            return None;
        }
        // Matches the header's font size (rather than the body row size)
        // so this row's label/value heights agree, per the same reasoning
        // as the empty header/value companions in `build_label_value_columns`.
        let header_font_size = appearance.overline_font_size() + 3.;
        let mut rows = vec![(
            Self::render_section_header("PLATFORM USAGE", appearance),
            render_value_text(
                format_dollars(platform_usage_in_cents),
                header_font_size,
                appearance,
            ),
        )];
        if usage_display_unit == UsageDisplayUnit::Credits
            && let Some(credits) = self
                .usage_info
                .platform_credits_spent_for_last_block
                .filter(|&credits| credits > 0.0)
        {
            rows.push((
                render_label_text("Credits", appearance),
                render_value_text(
                    format_credits(credits),
                    appearance.ui_font_size() + 2.,
                    appearance,
                ),
            ));
        }
        Some(rows)
    }

    fn tool_call_summary_rows(&self, appearance: &Appearance) -> Vec<LabelValueRow> {
        let font_size = appearance.ui_font_size() + 2.;
        let theme = appearance.theme();

        let diffs_value = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(8.)
            .with_child(
                Text::new(
                    format!("+{}", self.usage_info.lines_added),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(theme.ansi_fg_green())
                .finish(),
            )
            .with_child(
                Text::new(
                    format!("-{}", self.usage_info.lines_removed),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(theme.ansi_fg_red())
                .finish(),
            )
            .finish();

        vec![
            (
                render_label_text("Tool calls", appearance),
                render_value_text(
                    self.usage_info.tool_calls.to_string(),
                    font_size,
                    appearance,
                ),
            ),
            (
                render_label_text("Files changed", appearance),
                render_value_text(
                    self.usage_info.files_changed.to_string(),
                    font_size,
                    appearance,
                ),
            ),
            (render_label_text("Diffs applied", appearance), diffs_value),
            (
                render_label_text("Commands executed", appearance),
                render_value_text(
                    self.usage_info.commands_executed.to_string(),
                    font_size,
                    appearance,
                ),
            ),
        ]
    }

    fn response_time_rows(&self, appearance: &Appearance) -> Option<Vec<LabelValueRow>> {
        let timing = self.timing_info.as_ref()?;
        let font_size = appearance.ui_font_size() + 2.;

        let mut rows = vec![
            (
                render_label_text("Time to first token", appearance),
                render_value_text(
                    format_seconds(timing.time_to_first_token_ms),
                    font_size,
                    appearance,
                ),
            ),
            (
                render_label_text("Total agent response time", appearance),
                render_value_text(
                    format_seconds(timing.total_agent_response_time_ms),
                    font_size,
                    appearance,
                ),
            ),
        ];
        if let Some(wall_ms) = timing.wall_to_wall_response_time_ms {
            rows.push((
                render_label_text("Total time (including tool calls)", appearance),
                render_value_text(format_seconds(wall_ms), font_size, appearance),
            ));
        }

        Some(rows)
    }
}

impl TurnUsageView {
    /// Builds the panel's two shared label/value columns as flat, parallel
    /// vectors (row `i` in `labels` always corresponds to row `i` in
    /// `values`).
    ///
    /// Section headers pair with a value-column companion built via the
    /// same [`Self::render_section_header`] helper (passed an empty label)
    /// rather than an `Empty` placeholder, since `Empty` has zero layout
    /// height and would shift every later value row out of alignment with
    /// its label.
    fn build_label_value_columns(
        &self,
        appearance: &Appearance,
        usage_display_unit: UsageDisplayUnit,
    ) -> LabelValueColumns {
        // Row spacing within a section. The last row of each section gets
        // extra bottom margin (on top of this) to visually separate
        // top-level sections from one another.
        const ROW_MARGIN_BOTTOM: f32 = 6.;
        const SECTION_END_EXTRA_MARGIN: f32 = 8.;

        let mut labels: Vec<Box<dyn Element>> = Vec::new();
        let mut values: Vec<Box<dyn Element>> = Vec::new();
        let mut push_row =
            |label: Box<dyn Element>, value: Box<dyn Element>, margin_bottom: f32| {
                labels.push(
                    Container::new(label)
                        .with_margin_bottom(margin_bottom)
                        .finish(),
                );
                values.push(
                    Container::new(value)
                        .with_margin_bottom(margin_bottom)
                        .finish(),
                );
            };
        let push_section_rows =
            |rows: Vec<LabelValueRow>,
             push_row: &mut dyn FnMut(Box<dyn Element>, Box<dyn Element>, f32)| {
                let last_index = rows.len().checked_sub(1);
                for (i, (label, value)) in rows.into_iter().enumerate() {
                    let margin_bottom = if Some(i) == last_index {
                        ROW_MARGIN_BOTTOM + SECTION_END_EXTRA_MARGIN
                    } else {
                        ROW_MARGIN_BOTTOM
                    };
                    push_row(label, value, margin_bottom);
                }
            };

        let (inference_usage_label, inference_usage_value) =
            self.inference_usage_header_row(appearance);
        push_row(inference_usage_label, inference_usage_value, 8.);
        push_section_rows(
            self.model_usage_rows(appearance, usage_display_unit),
            &mut push_row,
        );

        if let Some(rows) = self.platform_usage_rows(appearance, usage_display_unit) {
            push_section_rows(rows, &mut push_row);
        }

        let (context_window_label, context_window_value) =
            self.context_window_usage_row(appearance);
        push_row(
            context_window_label,
            context_window_value,
            ROW_MARGIN_BOTTOM + SECTION_END_EXTRA_MARGIN,
        );

        push_row(
            Self::render_section_header("TOOL CALL SUMMARY", appearance),
            Self::render_section_header("", appearance),
            8.,
        );
        push_section_rows(self.tool_call_summary_rows(appearance), &mut push_row);

        if let Some(rows) = self.response_time_rows(appearance) {
            push_row(
                Self::render_section_header("RESPONSE TIME", appearance),
                Self::render_section_header("", appearance),
                8.,
            );
            push_section_rows(rows, &mut push_row);
        }

        (labels, values)
    }
}

impl View for TurnUsageView {
    fn ui_name() -> &'static str {
        "TurnUsageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let usage_display_unit = AISettings::as_ref(app).usage_display_unit;

        let (labels, values) = self.build_label_value_columns(appearance, usage_display_unit);

        let content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(self.render_header(appearance))
                    .with_margin_bottom(12.)
                    .finish(),
            )
            .with_child(
                Flex::row()
                    .with_spacing(16.)
                    .with_child(Flex::column().with_children(labels).finish())
                    .with_child(Flex::column().with_children(values).finish())
                    .finish(),
            )
            .finish();

        Container::new(content)
            .with_uniform_padding(12.)
            .with_background(theme.surface_2())
            .with_border(Border::all(1.0).with_border_fill(theme.outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .with_uniform_margin(16.)
            .with_drop_shadow(
                DropShadow::new_with_standard_offset_and_spread(ColorU::new(0, 0, 0, 32))
                    .with_offset(vec2f(0., 2.)),
            )
            .finish()
    }
}

impl Entity for TurnUsageView {
    type Event = TurnUsageViewEvent;
}

impl TypedActionView for TurnUsageView {
    type Action = TurnUsageViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TurnUsageViewAction::Close => {
                ctx.emit(TurnUsageViewEvent::CloseRequested);
            }
            TurnUsageViewAction::ToggleModelExpanded(index) => {
                if let Some(row) = self.model_rows.get_mut(*index) {
                    row.expanded = !row.expanded;
                }
                ctx.notify();
            }
        }
    }
}

fn render_label_text(text: &str, appearance: &Appearance) -> Box<dyn Element> {
    render_label_text_sized(text, appearance.ui_font_size() + 2., appearance)
}

fn render_label_text_sized(
    text: &str,
    font_size: f32,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    Text::new(text.to_string(), appearance.ui_font_family(), font_size)
        .with_color(blended_colors::text_sub(theme, theme.surface_2()))
        .finish()
}

/// Like [`render_label_text`], but indented to sit under an expanded model
/// row's breakdown, at the given font size.
fn render_indented_label_text(
    text: &str,
    font_size: f32,
    appearance: &Appearance,
) -> Box<dyn Element> {
    Container::new(render_label_text_sized(text, font_size, appearance))
        .with_margin_left(16.)
        .finish()
}

fn render_value_text(text: String, font_size: f32, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    Text::new(text, appearance.ui_font_family(), font_size)
        .with_color(blended_colors::text_main(theme, theme.surface_2()))
        .finish()
}

pub(crate) fn format_tokens(tokens: u64) -> String {
    format!("{tokens} token{}", if tokens == 1 { "" } else { "s" })
}

/// Formats a token count together with its dollar cost.
fn format_tokens_with_cost(tokens: u64, cost_in_cents: f32) -> String {
    format!(
        "{}  /  {}",
        format_tokens(tokens),
        format_dollars(cost_in_cents)
    )
}

pub(crate) fn format_web_searches(count: u64) -> String {
    format!("{count} search{}", if count == 1 { "" } else { "es" })
}

/// Formats a US-cent amount as a dollar string. A non-zero amount that
/// would otherwise round down to `$0.00` (e.g. a fraction of a cent) is
/// shown as `<$0.01` instead, since rounding it to zero would misleadingly
/// suggest no cost was incurred.
pub(crate) fn format_dollars(cost_in_cents: f32) -> String {
    let dollars = cost_in_cents / 100.0;
    if cost_in_cents > 0.0 && dollars < 0.01 {
        "<$0.01".to_string()
    } else {
        format!("${dollars:.2}")
    }
}

fn format_seconds(ms: i64) -> String {
    format!("{:.1} seconds", ms as f64 / 1000.0)
}

#[cfg(test)]
#[path = "turn_usage_view_tests.rs"]
mod tests;
