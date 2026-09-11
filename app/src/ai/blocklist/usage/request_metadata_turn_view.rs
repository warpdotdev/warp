//! A docked panel showing locally received usage and timing metadata for one agent turn.

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DropShadow, Empty, Flex,
    Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, Text,
};
use warpui::platform::Cursor;
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use super::render_context_window_usage_icon;
use crate::ai::agent::request_metadata::{RequestMetadataRecord, TurnSummary, summarize_turn};
use crate::ai::blocklist::view_util::{format_credits, format_usage};
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
    /// The user clicked the "RAW RECORD" header to show/hide the records as JSON.
    ToggleRawRecord,
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
    /// The exchange's records, aggregated once at construction into `summary`.
    summary: TurnSummary,
    /// The records pretty-printed (as a JSON array) once at construction; they never change
    /// afterwards.
    raw_json: String,
    close_button_mouse_state: MouseStateHandle,
    raw_record_toggle_mouse_state: MouseStateHandle,
    /// Per-model row UI state, indexed in lockstep with `summary.model_charges`.
    model_rows: Vec<ModelRowState>,
    raw_record_expanded: bool,
}

impl RequestMetadataTurnView {
    pub fn new(records: Vec<RequestMetadataRecord>, ctx: &mut ViewContext<Self>) -> Self {
        // The "Credits" rows' visibility depends on the Credits/Dollars usage-display-unit
        // setting, so the panel must re-render when the user flips it — otherwise an
        // already-open panel would show a stale section state until closed and reopened.
        ctx.subscribe_to_model(&AISettings::handle(ctx), |_, _, event, ctx| {
            if matches!(event, AISettingsChangedEvent::UsageDisplayUnit { .. }) {
                ctx.notify();
            }
        });
        Self::build(records)
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(records: Vec<RequestMetadataRecord>) -> Self {
        Self::build(records)
    }

    fn build(records: Vec<RequestMetadataRecord>) -> Self {
        let mut summary = summarize_turn(&records);
        // Usage-ranked display order: most tokens first, then model id for stability.
        summary.model_charges.sort_by(|a, b| {
            b.tokens()
                .cmp(&a.tokens())
                .then_with(|| a.model_id.cmp(&b.model_id))
        });
        let raw_json = serde_json::to_string_pretty(
            &records
                .iter()
                .map(|record| record.to_json())
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default();
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
            raw_json,
            close_button_mouse_state: MouseStateHandle::default(),
            raw_record_toggle_mouse_state: MouseStateHandle::default(),
            model_rows,
            raw_record_expanded: false,
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

        let outcome = self.summary.outcome;
        let outcome_color = if outcome.is_interrupted() {
            theme.ansi_fg_yellow()
        } else {
            blended_colors::text_sub(theme, background)
        };
        let outcome_badge = Container::new(
            Text::new(
                outcome.label().to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(outcome_color)
            .soft_wrap(false)
            .finish(),
        )
        .with_margin_left(8.)
        .with_horizontal_padding(6.)
        .with_vertical_padding(1.)
        .with_border(Border::all(1.).with_border_fill(outcome_color))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
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

        // When the turn spans several requests, say so in the header so the summed numbers
        // below are self-explanatory.
        let mut title_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(title);
        if self.summary.request_count > 1 {
            title_row = title_row.with_child(
                Text::new(
                    format!("{} requests", self.summary.request_count),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(blended_colors::text_sub(theme, background))
                .soft_wrap(false)
                .finish(),
            );
        }
        title_row = title_row.with_child(outcome_badge);

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(title_row.finish())
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

    fn model_row(&self, index: usize, appearance: &Appearance) -> Vec<LabelValueRow> {
        let charge = &self.summary.model_charges[index];
        let font_size = appearance.ui_font_size() + 1.;
        let theme = appearance.theme();
        let text_color = blended_colors::text_main(theme, theme.surface_2());
        let row_state = &self.model_rows[index];
        let expanded = row_state.expanded;

        let value = Text::new(
            format_tokens_with_cost(charge.tokens(), charge.cost_in_cents()),
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
            let mut push = |label: &str, tokens: u32, cents: f32| {
                rows.push((
                    render_indented_label_text(label, breakdown_font_size, appearance),
                    render_value_text(
                        format_tokens_with_cost(u64::from(tokens), cents),
                        breakdown_font_size,
                        appearance,
                    ),
                ));
            };
            push("Input", charge.input_tokens, charge.input_cost_in_cents);
            push("Output", charge.output_tokens, charge.output_cost_in_cents);
            if charge.cache_read_tokens > 0 {
                push(
                    "Cache read",
                    charge.cache_read_tokens,
                    charge.cache_read_cost_in_cents,
                );
            }
            if charge.cache_write_tokens > 0 {
                push(
                    "Cache write",
                    charge.cache_write_tokens,
                    charge.cache_write_cost_in_cents,
                );
            }
            if charge.web_search_count > 0 {
                rows.push((
                    render_indented_label_text("Web search", breakdown_font_size, appearance),
                    render_value_text(
                        format!(
                            "{}  /  {}",
                            format_web_searches(charge.web_search_count),
                            format_dollars(charge.web_search_cost_in_cents)
                        ),
                        breakdown_font_size,
                        appearance,
                    ),
                ));
            }
        }
        rows
    }

    /// The per-model rows, plus (in Credits mode) a trailing "Credits" row for the turn's
    /// inference-only credit total.
    fn model_usage_rows(
        &self,
        appearance: &Appearance,
        usage_display_unit: UsageDisplayUnit,
    ) -> Vec<LabelValueRow> {
        let mut rows: Vec<LabelValueRow> = (0..self.summary.model_charges.len())
            .flat_map(|index| self.model_row(index, appearance))
            .collect();
        if usage_display_unit == UsageDisplayUnit::Credits {
            rows.push((
                render_label_text("Credits", appearance),
                render_value_text(
                    format_credits(self.summary.inference_cost_in_credits()),
                    appearance.ui_font_size() + 2.,
                    appearance,
                ),
            ));
        }
        rows
    }

    fn inference_usage_header_row(&self, appearance: &Appearance) -> LabelValueRow {
        let header_font_size = appearance.overline_font_size() + 3.;
        let theme = appearance.theme();
        let value = Text::new(
            format_tokens_with_cost(
                self.summary.total_tokens(),
                self.summary.inference_cost_in_cents(),
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

    /// The "PLATFORM USAGE" section: a header row with the dollar amount in the value column,
    /// plus (in Credits mode, if any) a trailing "Credits" row for the platform-only credit
    /// total. `None` when there was no platform charge, to avoid a noisy `$0.00` section.
    fn platform_usage_rows(
        &self,
        appearance: &Appearance,
        usage_display_unit: UsageDisplayUnit,
    ) -> Option<Vec<LabelValueRow>> {
        let platform_cents = self.summary.platform_cost_in_cents();
        if platform_cents <= 0.0 {
            return None;
        }
        let header_font_size = appearance.overline_font_size() + 3.;
        let mut rows = vec![(
            Self::render_section_header("PLATFORM USAGE", appearance),
            render_value_text(format_dollars(platform_cents), header_font_size, appearance),
        )];
        if usage_display_unit == UsageDisplayUnit::Credits {
            let credits = self.summary.platform_cost_in_credits();
            if credits > 0.0 {
                rows.push((
                    render_label_text("Credits", appearance),
                    render_value_text(
                        format_credits(credits),
                        appearance.ui_font_size() + 2.,
                        appearance,
                    ),
                ));
            }
        }
        Some(rows)
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
        // The record carries a 0-100 percentage; the ring icon reads a 0-1 fraction.
        let fraction = (usage / 100.).clamp(0., 1.);
        let percent = usage.round();
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
        // Earliest request start to latest request end. Includes tool execution, so it only
        // adds information (and is only shown) when tools ran between requests.
        if let Some(wall_ms) = self.summary.request_duration_ms()
            && per_request_total_ms.is_none_or(|total| wall_ms > total)
        {
            rows.push((
                render_label_text("Total time (including tool calls)", appearance),
                render_value_text(format_seconds(wall_ms), font_size, appearance),
            ));
        }
        (!rows.is_empty()).then_some(rows)
    }

    fn request_rows(&self, appearance: &Appearance) -> Vec<LabelValueRow> {
        let font_size = appearance.ui_font_size();
        let mut rows = vec![(
            render_label_text("Outcome", appearance),
            render_value_text(
                self.summary.outcome.label().to_string(),
                font_size + 2.,
                appearance,
            ),
        )];
        // A multi-request turn that partially failed says so explicitly, rather than letting the
        // single worst-outcome badge imply every request failed. Name the interrupted count as
        // such: it is not the worst-outcome count (one Errored + one Canceled is 2 of 2
        // interrupted, not "2 of 2 Errored").
        if self.summary.interrupted_count > 0 && self.summary.request_count > 1 {
            rows.push((
                render_label_text("Interrupted", appearance),
                render_value_text(
                    format!(
                        "{} of {} requests interrupted",
                        self.summary.interrupted_count, self.summary.request_count
                    ),
                    font_size,
                    appearance,
                ),
            ));
        }
        if let Some(recorded_at) = self.summary.recorded_at {
            rows.push((
                render_label_text("Recorded at", appearance),
                render_value_text(
                    recorded_at.format("%-m/%-d/%Y %H:%M:%S").to_string(),
                    font_size,
                    appearance,
                ),
            ));
        }
        let request_id = if self.summary.request_count == 1 {
            self.summary
                .records
                .first()
                .map(|record| record.request_id.clone())
                .unwrap_or_default()
        } else {
            format!("{} requests", self.summary.request_count)
        };
        rows.push((
            render_label_text("Request ID", appearance),
            render_value_text(request_id, font_size - 1., appearance),
        ));
        rows
    }

    fn raw_record_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let header_font_size = appearance.overline_font_size() + 3.;
        let color = blended_colors::text_disabled(theme, theme.surface_2());
        let chevron_icon = if self.raw_record_expanded {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };
        let font_family = appearance.overline_font_family();
        Hoverable::new(self.raw_record_toggle_mouse_state.clone(), move |_| {
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(4.)
                .with_child(
                    ConstrainedBox::new(chevron_icon.to_warpui_icon(color.into()).finish())
                        .with_width(header_font_size)
                        .with_height(header_font_size)
                        .finish(),
                )
                .with_child(
                    Text::new("RAW RECORD".to_string(), font_family, header_font_size)
                        .with_color(color)
                        .soft_wrap(false)
                        .finish(),
                )
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(RequestMetadataTurnViewAction::ToggleRawRecord);
        })
        .finish()
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

        let (inference_label, inference_value) = self.inference_usage_header_row(appearance);
        push_row(inference_label, inference_value, 8.);
        push_section_rows(
            self.model_usage_rows(appearance, usage_display_unit),
            &mut push_row,
        );

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

        push_row(
            Self::render_section_header("REQUEST", appearance),
            Self::render_section_header("", appearance),
            8.,
        );
        push_section_rows(self.request_rows(appearance), &mut push_row);

        (labels, values)
    }

    fn render_raw_record(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Container::new(
            Text::new(
                self.raw_json.clone(),
                appearance.monospace_font_family(),
                appearance.monospace_font_size() - 2.,
            )
            .with_color(blended_colors::text_main(theme, theme.surface_2()))
            .with_selectable(true)
            .finish(),
        )
        .with_uniform_padding(8.)
        .with_background(theme.surface_1())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .finish()
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

        let mut content = Flex::column()
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
            .with_child(
                Container::new(self.raw_record_header(appearance))
                    .with_margin_bottom(6.)
                    .finish(),
            );
        if self.raw_record_expanded {
            content.add_child(self.render_raw_record(appearance));
        }

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
            RequestMetadataTurnViewAction::ToggleRawRecord => {
                self.raw_record_expanded = !self.raw_record_expanded;
                ctx.notify();
            }
        }
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
    match (credits, turn_cost) {
        (Some(credits), _) => format!(
            "Turn: {}",
            format_usage(credits, None, turn_cost, usage_display_unit)
        ),
        (None, Some(cost)) if usage_display_unit == UsageDisplayUnit::Dollars => {
            format!("Turn: ${:.2}", cost / 100.0)
        }
        (None, _) => "Turn".to_string(),
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

fn format_tokens_with_cost(tokens: u64, cost_in_cents: f32) -> String {
    format!(
        "{}  /  {}",
        format_tokens(tokens),
        format_dollars(cost_in_cents)
    )
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
