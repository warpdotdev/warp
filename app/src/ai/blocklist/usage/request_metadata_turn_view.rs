//! A docked panel showing locally received usage and timing metadata for one agent turn.

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use thousands::Separable;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DropShadow, Empty, Flex,
    Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, Text,
};
use warpui::platform::Cursor;
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use super::render_context_window_usage_icon;
use crate::ai::agent::request_metadata::{
    LegacyCharges, RequestMetadataRecord, TurnPanelData, TurnSummary, summarize_turn,
};
use crate::ai::blocklist::view_util::format_credits;
use crate::appearance::Appearance;
use crate::features::FeatureFlag;
use crate::settings::UsageDisplayUnit;
use crate::settings::ai::{AISettings, AISettingsChangedEvent};
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;

/// A single label/value pair rendered as a row in the panel's shared label/value columns.
type LabelValueRow = (Box<dyn Element>, Box<dyn Element>);
/// The panel's two shared columns as parallel element lists: labels, then values.
type LabelValueColumns = (Vec<Box<dyn Element>>, Vec<Box<dyn Element>>);

/// Typed actions dispatched by widgets inside [`RequestMetadataTurnView`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestMetadataTurnViewAction {
    /// The user clicked the header's close ("X") button.
    Close,
    /// The user clicked a model row's label to expand/collapse its token breakdown. Carries
    /// the row's index into [`RequestMetadataRecord::model_charges`].
    ToggleModelExpanded(usize),
}

/// Emitted so the owning view (the terminal view) can remove this panel from the blocklist
/// when the user clicks the close button.
#[derive(Clone, Debug)]
pub enum RequestMetadataTurnViewEvent {
    CloseRequested,
}

struct ModelRowState {
    mouse: MouseStateHandle,
    expanded: bool,
}

/// The docked "Turn" panel view. See module docs for scope/behavior.
pub struct RequestMetadataTurnView {
    /// The turn's data, aggregated once at construction into `summary`.
    summary: TurnSummary,
    /// Charge data for a legacy turn (without server records): `None` on the records path.
    legacy_charges: Option<LegacyCharges>,
    close_button_mouse_state: MouseStateHandle,
    /// Per-model row UI state, indexed in lockstep with `summary.model_charges`.
    model_rows: Vec<ModelRowState>,
}

impl RequestMetadataTurnView {
    pub fn new(data: TurnPanelData, ctx: &mut ViewContext<Self>) -> Self {
        // Every amount is rendered in the Credits/Dollars usage-display-unit setting, so the
        // panel must re-render when the user flips it — otherwise an already-open panel would
        // show stale units until closed and reopened.
        ctx.subscribe_to_model(&AISettings::handle(ctx), |_, _, event, ctx| {
            if matches!(event, AISettingsChangedEvent::UsageDisplayUnit { .. }) {
                ctx.notify();
            }
        });
        Self::build(data)
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(data: impl Into<TurnPanelData>) -> Self {
        Self::build(data.into())
    }

    fn build(data: TurnPanelData) -> Self {
        let (records, legacy_charges) = match data {
            TurnPanelData::Records(records) => (records, None),
            TurnPanelData::Legacy { records, charges } => (records, Some(charges)),
        };
        let mut summary = summarize_turn(&records);
        // Usage-ranked display order: most tokens first, then model id for stability.
        summary.model_charges.sort_by(|a, b| {
            b.tokens()
                .cmp(&a.tokens())
                .then_with(|| a.model_id.cmp(&b.model_id))
        });
        let model_rows = summary
            .model_charges
            .iter()
            .map(|_| ModelRowState {
                mouse: MouseStateHandle::default(),
                expanded: false,
            })
            .collect();
        Self {
            summary,
            legacy_charges,
            close_button_mouse_state: MouseStateHandle::default(),
            model_rows,
        }
    }

    #[cfg(test)]
    pub(crate) fn records(&self) -> &[RequestMetadataRecord] {
        &self.summary.records
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
            ctx.dispatch_typed_action(RequestMetadataTurnViewAction::Close);
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

    /// A section's small-caps label. Purely decorative: sections are always expanded.
    fn render_section_header(label: &str, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let header_font_size = appearance.overline_font_size() + 3.;
        Text::new(
            label.to_string(),
            appearance.overline_font_family(),
            header_font_size,
        )
        .with_color(blended_colors::text_disabled(theme, theme.surface_2()))
        .soft_wrap(false)
        .finish()
    }

    fn model_row(
        &self,
        index: usize,
        appearance: &Appearance,
        usage_display_unit: UsageDisplayUnit,
    ) -> Vec<LabelValueRow> {
        let charge = &self.summary.model_charges[index];
        let font_size = appearance.ui_font_size() + 1.;
        let theme = appearance.theme();
        let text_color = blended_colors::text_main(theme, theme.surface_2());
        let row_state = &self.model_rows[index];
        let expanded = row_state.expanded;

        let value = Text::new(
            format_tokens_with_cost(
                charge.tokens(),
                charge.cost_in_cents(),
                charge.cost_in_credits(),
                usage_display_unit,
            ),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(text_color)
        .finish();

        let font_family = appearance.ui_font_family();
        let model_id = charge.model_id.clone();
        let chevron_icon = if expanded {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };
        let label = Hoverable::new(row_state.mouse.clone(), move |_state| {
            let text_element = Text::new(model_id.clone(), font_family, font_size)
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
            ctx.dispatch_typed_action(RequestMetadataTurnViewAction::ToggleModelExpanded(index));
        })
        .finish();

        let mut rows = vec![(label, value)];
        if expanded {
            let breakdown_font_size = appearance.ui_font_size() - 1.;
            let mut push = |label: &str, tokens: u32, cents: f32, credits: f32| {
                rows.push((
                    render_indented_label_text(label, breakdown_font_size, appearance),
                    render_value_text(
                        format_tokens_with_cost(
                            u64::from(tokens),
                            cents,
                            credits,
                            usage_display_unit,
                        ),
                        breakdown_font_size,
                        appearance,
                    ),
                ));
            };
            push(
                "Input",
                charge.input_tokens,
                charge.input_cost_in_cents,
                charge.input_cost_in_credits,
            );
            push(
                "Output",
                charge.output_tokens,
                charge.output_cost_in_cents,
                charge.output_cost_in_credits,
            );
            if charge.cache_read_tokens > 0 {
                push(
                    "Cache read",
                    charge.cache_read_tokens,
                    charge.cache_read_cost_in_cents,
                    charge.cache_read_cost_in_credits,
                );
            }
            if charge.cache_write_tokens > 0 {
                push(
                    "Cache write",
                    charge.cache_write_tokens,
                    charge.cache_write_cost_in_cents,
                    charge.cache_write_cost_in_credits,
                );
            }
            if charge.web_search_count > 0 {
                rows.push((
                    render_indented_label_text("Web search", breakdown_font_size, appearance),
                    render_value_text(
                        format!(
                            "{}  /  {}",
                            format_web_searches(charge.web_search_count),
                            format_cost(
                                charge.web_search_cost_in_cents,
                                charge.web_search_cost_in_credits,
                                usage_display_unit,
                            )
                        ),
                        breakdown_font_size,
                        appearance,
                    ),
                ));
            }
        }
        rows
    }

    fn model_usage_rows(
        &self,
        appearance: &Appearance,
        usage_display_unit: UsageDisplayUnit,
    ) -> Vec<LabelValueRow> {
        (0..self.summary.model_charges.len())
            .flat_map(|index| self.model_row(index, appearance, usage_display_unit))
            .collect()
    }

    fn inference_usage_header_row(
        &self,
        appearance: &Appearance,
        usage_display_unit: UsageDisplayUnit,
    ) -> LabelValueRow {
        let header_font_size = appearance.overline_font_size() + 3.;
        let theme = appearance.theme();
        let value = Text::new(
            format_tokens_with_cost(
                self.summary.total_tokens(),
                self.summary.inference_cost_in_cents(),
                self.summary.inference_cost_in_credits(),
                usage_display_unit,
            ),
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

    /// The "PLATFORM USAGE" section: a header row with the platform charge in the value
    /// column. `None` when there was no platform charge, to avoid a noisy zero section.
    fn platform_usage_rows(
        &self,
        appearance: &Appearance,
        usage_display_unit: UsageDisplayUnit,
    ) -> Option<Vec<LabelValueRow>> {
        let platform_cents = self.summary.platform_cost_in_cents();
        let platform_credits = self.summary.platform_cost_in_credits();
        if platform_cents <= 0.0 && platform_credits <= 0.0 {
            return None;
        }
        let header_font_size = appearance.overline_font_size() + 3.;
        Some(vec![(
            Self::render_section_header("PLATFORM USAGE", appearance),
            render_value_text(
                format_cost(platform_cents, platform_credits, usage_display_unit),
                header_font_size,
                appearance,
            ),
        )])
    }

    fn context_window_usage_row(&self, appearance: &Appearance) -> Option<LabelValueRow> {
        let usage = self.summary.context_window_usage?;
        let font_size = appearance.ui_font_size() + 2.;
        let theme = appearance.theme();
        let text_color = blended_colors::text_main(theme, theme.surface_2());

        let label = Text::new(
            "Context window usage".to_string(),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(text_color)
        .finish();
        let fraction = usage.clamp(0., 1.);
        let percent = (fraction * 100.).round();
        let value = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(4.)
            .with_child(
                ConstrainedBox::new(render_context_window_usage_icon(fraction, theme, None))
                    .with_width(font_size)
                    .with_height(font_size)
                    .finish(),
            )
            .with_child(
                Text::new(
                    format!("{percent}%"),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(text_color)
                .finish(),
            )
            .finish();
        Some((label, value))
    }

    fn tool_call_summary_rows(&self, appearance: &Appearance) -> Option<Vec<LabelValueRow>> {
        let tool_calls = self.summary.tool_calls?;
        let font_size = appearance.ui_font_size() + 2.;
        let theme = appearance.theme();

        let diffs_value = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(8.)
            .with_child(
                Text::new(
                    format!("+{}", self.summary.lines_added.unwrap_or(0)),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(theme.ansi_fg_green())
                .finish(),
            )
            .with_child(
                Text::new(
                    format!("-{}", self.summary.lines_removed.unwrap_or(0)),
                    appearance.ui_font_family(),
                    font_size,
                )
                .with_color(theme.ansi_fg_red())
                .finish(),
            )
            .finish();

        Some(vec![
            (
                render_label_text("Tool calls", appearance),
                render_value_text(tool_calls.to_string(), font_size, appearance),
            ),
            (
                render_label_text("Files changed", appearance),
                render_value_text(
                    self.summary.files_changed.unwrap_or(0).to_string(),
                    font_size,
                    appearance,
                ),
            ),
            (render_label_text("Diffs applied", appearance), diffs_value),
            (
                render_label_text("Commands executed", appearance),
                render_value_text(
                    self.summary.commands_executed.unwrap_or(0).to_string(),
                    font_size,
                    appearance,
                ),
            ),
        ])
    }

    fn response_time_rows(&self, appearance: &Appearance) -> Option<Vec<LabelValueRow>> {
        let font_size = appearance.ui_font_size() + 2.;
        let mut rows = Vec::new();
        if let Some(ttft) = self.summary.time_to_first_token_ms() {
            rows.push((
                render_label_text("Time to first token", appearance),
                render_value_text(format_seconds(ttft), font_size, appearance),
            ));
        }
        // The agent's own processing time: the sum of the turn's per-request processing
        // windows, excluding the tool-execution gaps between them. Unknown unless every
        // record carries its timing.
        let per_request_total_ms: Option<i64> = self
            .summary
            .records
            .iter()
            .map(|record| record.request_duration_ms())
            .sum();
        if let Some(total) = per_request_total_ms {
            rows.push((
                render_label_text("Total agent response time", appearance),
                render_value_text(format_seconds(total), font_size, appearance),
            ));
        }
        // Earliest request start to latest request end, including tool execution between
        // requests.
        if let Some(wall_ms) = self.summary.request_duration_ms() {
            rows.push((
                render_label_text("Total time (including tool calls)", appearance),
                render_value_text(format_seconds(wall_ms), font_size, appearance),
            ));
        }
        (!rows.is_empty()).then_some(rows)
    }

    fn build_label_value_columns(
        &self,
        appearance: &Appearance,
        usage_display_unit: UsageDisplayUnit,
    ) -> LabelValueColumns {
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

        // A legacy turn without any charge data hides the inference section entirely:
        // its charges are unknown, and a zero header would read as "free".
        match &self.legacy_charges {
            Some(LegacyCharges::Unknown) => (),
            Some(LegacyCharges::CreditsOnly(credits)) => {
                // Only a credits total is known: no token/cost rows anywhere.
                let header_font_size = appearance.overline_font_size() + 3.;
                push_row(
                    Self::render_section_header("INFERENCE USAGE", appearance),
                    render_value_text(
                        format_credits_amount(*credits),
                        header_font_size,
                        appearance,
                    ),
                    8.,
                );
            }
            Some(LegacyCharges::Breakdown(_)) | None => {
                let (inference_label, inference_value) =
                    self.inference_usage_header_row(appearance, usage_display_unit);
                push_row(inference_label, inference_value, 8.);
                push_section_rows(
                    self.model_usage_rows(appearance, usage_display_unit),
                    &mut push_row,
                );
            }
        }

        if let Some(rows) = self.platform_usage_rows(appearance, usage_display_unit) {
            push_section_rows(rows, &mut push_row);
        }

        if let Some((label, value)) = self.context_window_usage_row(appearance) {
            push_row(label, value, ROW_MARGIN_BOTTOM + SECTION_END_EXTRA_MARGIN);
        }

        if let Some(rows) = self.tool_call_summary_rows(appearance) {
            push_row(
                Self::render_section_header("TOOL CALL SUMMARY", appearance),
                Self::render_section_header("", appearance),
                8.,
            );
            push_section_rows(rows, &mut push_row);
        }

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

impl View for RequestMetadataTurnView {
    fn ui_name() -> &'static str {
        "RequestMetadataTurnView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        // The panel is a pricing-transparency surface. If the flag turned off while the panel
        // was open, render nothing until the owning view removes it.
        if !FeatureFlag::PricingTransparency.is_enabled() {
            return Empty::new().finish();
        }
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
            );

        Container::new(content.finish())
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

impl Entity for RequestMetadataTurnView {
    type Event = RequestMetadataTurnViewEvent;
}

impl TypedActionView for RequestMetadataTurnView {
    type Action = RequestMetadataTurnViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            RequestMetadataTurnViewAction::Close => {
                ctx.emit(RequestMetadataTurnViewEvent::CloseRequested);
            }
            RequestMetadataTurnViewAction::ToggleModelExpanded(index) => {
                if let Some(row) = self.model_rows.get_mut(*index) {
                    row.expanded = !row.expanded;
                }
                ctx.notify();
            }
        }
    }
}

pub(crate) fn turn_panel_tooltip_text_for_data(
    data: &TurnPanelData,
    usage_display_unit: UsageDisplayUnit,
) -> String {
    match data {
        TurnPanelData::Legacy {
            charges: LegacyCharges::CreditsOnly(credits),
            ..
        } => format!("Turn: {}", format_credits_amount(*credits)),
        _ => turn_panel_tooltip_text(data.records(), usage_display_unit),
    }
}

/// The trigger icon's hover tooltip: the turn's charge, honoring the user's credits/dollars
/// display-unit setting. Stays quiet ("Turn") rather than fabricating a total when neither
/// figure is known.
pub(crate) fn turn_panel_tooltip_text(
    records: &[RequestMetadataRecord],
    usage_display_unit: UsageDisplayUnit,
) -> String {
    let total_cost_in_cents: f32 = records
        .iter()
        .map(|record| record.total_cost_in_cents())
        .sum();
    let total_credits: f32 = records
        .iter()
        .map(|record| record.total_cost_in_credits())
        .sum();
    let turn_cost = Some(total_cost_in_cents).filter(|&cost| cost > 0.0);
    let credits = Some(total_credits).filter(|&credits| credits > 0.0);
    let value = match (usage_display_unit, turn_cost, credits) {
        (UsageDisplayUnit::Dollars, Some(cost), _) => format_dollars(cost),
        (UsageDisplayUnit::Dollars | UsageDisplayUnit::Credits, _, Some(credits)) => {
            format_credits_amount(credits)
        }
        (UsageDisplayUnit::Dollars | UsageDisplayUnit::Credits, _, None) => {
            return "Turn".to_string();
        }
    };
    format!("Turn: {value}")
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
    format!(
        "{} token{}",
        tokens.separate_with_commas(),
        if tokens == 1 { "" } else { "s" }
    )
}

fn format_tokens_with_cost(
    tokens: u64,
    cost_in_cents: f32,
    cost_in_credits: f32,
    usage_display_unit: UsageDisplayUnit,
) -> String {
    format!(
        "{}  /  {}",
        format_tokens(tokens),
        format_cost(cost_in_cents, cost_in_credits, usage_display_unit)
    )
}

/// Formats a charge in the user's display unit.
fn format_cost(
    cost_in_cents: f32,
    cost_in_credits: f32,
    usage_display_unit: UsageDisplayUnit,
) -> String {
    match usage_display_unit {
        UsageDisplayUnit::Dollars => format_dollars(cost_in_cents),
        UsageDisplayUnit::Credits => format_credits_amount(cost_in_credits),
    }
}

/// Formats a credit amount with thousands separators. A non-zero amount that would round to
/// `0 credits` is shown as `<0.1 credits`, since rounding it to zero would misleadingly suggest
/// no cost was incurred.
fn format_credits_amount(credits: f32) -> String {
    if credits > 0.0 && credits < 0.1 {
        return "<0.1 credits".to_string();
    }
    let text = format_credits(credits);
    let Some((amount, unit)) = text.split_once(' ') else {
        return text;
    };
    let (whole, fraction) = amount.split_once('.').unwrap_or((amount, ""));
    let whole = whole
        .parse::<i64>()
        .map_or_else(|_| whole.to_string(), |whole| whole.separate_with_commas());
    if fraction.is_empty() {
        format!("{whole} {unit}")
    } else {
        format!("{whole}.{fraction} {unit}")
    }
}

pub(crate) fn format_web_searches(count: u32) -> String {
    format!("{count} search{}", if count == 1 { "" } else { "es" })
}

/// Formats a US-cent amount as dollars. A non-zero amount that would round to `$0.00` is shown
/// as `<$0.01`, since rounding it to zero would misleadingly suggest no cost was incurred.
pub(crate) fn format_dollars(cost_in_cents: f32) -> String {
    // Summing an empty charge list yields `-0.0`, which would print as `$-0.00`.
    let cost_in_cents = if cost_in_cents == 0.0 {
        0.0
    } else {
        cost_in_cents
    };
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
#[path = "request_metadata_turn_view_tests.rs"]
mod tests;
