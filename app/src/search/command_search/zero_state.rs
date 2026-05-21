use std::collections::HashMap;

use lazy_static::lazy_static;
use warp_core::features::FeatureFlag;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Container, CornerRadius, Flex, Hoverable, MouseStateHandle, ParentElement, Radius, Text, Wrap,
};
use warpui::platform::Cursor;
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::appearance::Appearance;
use crate::drive::settings::{WarpDriveSettings, WarpDriveSettingsChangedEvent};
use crate::search::{FilterChipRenderer, QueryFilter};
use crate::settings::{AISettings, AISettingsChangedEvent};

lazy_static! {
    /// Map of sample queries to the [`QueryFilter`]s they employ.
    ///
    /// These are rendered as clickable 'chips' in the zero state.
    static ref SAMPLE_QUERY_TO_FILTER: HashMap<&'static str, QueryFilter> = HashMap::from([
        ("history: git checkout", QueryFilter::History),
        ("workflows: run dev server", QueryFilter::Workflows),
        (
            "# find \"foo\" in files",
            QueryFilter::NaturalLanguage
        ),
        (
            "notebooks: deploy production server",
            QueryFilter::Notebooks
        ),
    ]);
}

pub enum CommandSearchZeroStateEvent {
    /// A filter chip was selected by the user in the zero state panel. The contained
    /// [`QueryFilter`] should be applied to the [`CommandSearchView`].
    FilterChipSelected(QueryFilter),

    /// A sample filter prefix query was selected by the user in the zero state panel. The
    /// contained [`QueryFilter`] was included in the sample query and should be applied to the
    /// [`CommandSearchView`].
    SampleQuerySelected(QueryFilter),
}

pub struct CommandSearchZeroStateView {
    filter_chip_to_mouse_state_handle: HashMap<QueryFilter, MouseStateHandle>,
    sample_query_to_mouse_state_handle: HashMap<&'static str, MouseStateHandle>,
}

impl CommandSearchZeroStateView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&AISettings::handle(ctx), |_, _, event, ctx| {
            if let AISettingsChangedEvent::IsAnyAIEnabled { .. } = event {
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(&WarpDriveSettings::handle(ctx), |_, _, event, ctx| {
            if let WarpDriveSettingsChangedEvent::EnableWarpDrive { .. } = event {
                ctx.notify();
            }
        });

        Self {
            filter_chip_to_mouse_state_handle: QueryFilter::all()
                .map(|filter| (filter, MouseStateHandle::default()))
                .collect(),
            sample_query_to_mouse_state_handle: SAMPLE_QUERY_TO_FILTER
                .keys()
                .map(|query| (*query, MouseStateHandle::default()))
                .collect(),
        }
    }

    /// Renders sample queries as a row of chips, wrapping around to as many lines
    /// as necessary. Each sample query is only shown if its query filter is
    /// enabled in `valid_filters`.
    fn render_sample_queries(
        &self,
        appearance: &Appearance,
        valid_filters: &[QueryFilter],
    ) -> Box<dyn Element> {
        let mut row = Wrap::row().with_run_spacing(styles::SAMPLE_QUERY_MARGIN);

        for (sample_query, filter) in SAMPLE_QUERY_TO_FILTER.iter() {
            if valid_filters.contains(filter) {
                row.add_child(
                    Container::new(self.render_sample_query(
                        sample_query.to_string(),
                        *filter,
                        appearance,
                    ))
                    .with_margin_right(styles::SAMPLE_QUERY_MARGIN)
                    .finish(),
                );
            }
        }

        Container::new(row.finish())
            .with_margin_bottom(styles::SAMPLE_QUERY_MARGIN)
            .finish()
    }

    /// Renders a sample query as a clickable 'chip'.
    ///
    /// When the sample query chip is clicked, the associated filter is emitted in a
    /// [`SampleQuerySelected`] event.
    fn render_sample_query(
        &self,
        sample_query: String,
        filter: QueryFilter,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        Hoverable::new(
            self.sample_query_to_mouse_state_handle[sample_query.as_str()].clone(),
            |mouse_state| {
                Container::new(
                    Text::new_inline(
                        sample_query.clone(),
                        appearance.monospace_font_family(),
                        appearance.monospace_font_size(),
                    )
                    .with_color(
                        appearance
                            .theme()
                            .main_text_color(appearance.theme().surface_2())
                            .into_solid(),
                    )
                    .finish(),
                )
                .with_padding_top(6.)
                .with_padding_bottom(6.)
                .with_padding_left(8.)
                .with_padding_right(8.)
                .with_background(if mouse_state.is_hovered() {
                    theme.accent_overlay()
                } else {
                    internal_colors::neutral_3(theme).into()
                })
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .finish()
            },
        )
        .with_cursor(Cursor::PointingHand)
        .on_click(move |event_ctx, _, _| {
            event_ctx
                .dispatch_typed_action(CommandSearchZeroStateAction::SampleQueryClicked(filter))
        })
        .finish()
    }

    /// Renders a clickable chip for each valid query filter. When a chip is
    /// clicked, the filter is emitted in a [`CommandSearchZeroStateEvent::FilterChipSelected`] event.
    fn render_filter_chips(
        &self,
        appearance: &Appearance,
        valid_filters: &[QueryFilter],
    ) -> Box<dyn Element> {
        let mut row = Wrap::row().with_run_spacing(styles::FILTER_CHIP_MARGIN);

        for filter in valid_filters.iter() {
            row.add_child(
                Container::new(filter.render_filter_chip(
                    self.filter_chip_to_mouse_state_handle[filter].clone(),
                    appearance,
                    |event_ctx, filter| {
                        event_ctx.dispatch_typed_action(
                            CommandSearchZeroStateAction::FilterChipClicked(filter),
                        )
                    },
                ))
                .with_margin_right(styles::FILTER_CHIP_MARGIN)
                .finish(),
            );
        }

        Container::new(row.finish())
            .with_margin_bottom(styles::FILTER_CHIPS_MARGIN_BOTTOM)
            .finish()
    }
}

impl View for CommandSearchZeroStateView {
    fn ui_name() -> &'static str {
        "CommandSearchZeroStateView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn warpui::Element> {
        let appearance = Appearance::as_ref(app);

        let command_search_text = Container::new(
            Text::new_inline(
                "Command Search",
                appearance.ui_font_family(),
                styles::header_text_font_size(appearance),
            )
            .with_color(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().surface_2())
                    .into_solid(),
            )
            .finish(),
        )
        .with_margin_bottom(styles::COMMAND_SEARCH_TEXT_MARGIN_BOTTOM)
        .finish();

        let valid_filters = valid_query_filters(app);

        let column = Flex::column()
            .with_child(command_search_text)
            .with_child(
                Container::new(
                    Text::new_inline(
                        "I'm looking for...",
                        appearance.ui_font_family(),
                        styles::subheader_text_font_size(appearance),
                    )
                    .with_color(
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().surface_2())
                            .into_solid(),
                    )
                    .finish(),
                )
                .with_margin_bottom(styles::FILTER_PREFIX_TEXT_MARGIN_BOTTOM)
                .finish(),
            )
            .with_child(self.render_filter_chips(appearance, &valid_filters))
            .with_child(
                Container::new(
                    Text::new_inline(
                        "Example queries",
                        appearance.ui_font_family(),
                        styles::subheader_text_font_size(appearance),
                    )
                    .with_color(
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().surface_2())
                            .into_solid(),
                    )
                    .finish(),
                )
                .with_margin_bottom(styles::FILTER_PREFIX_TEXT_MARGIN_BOTTOM)
                .finish(),
            )
            .with_child(self.render_sample_queries(appearance, &valid_filters));

        Container::new(column.finish())
            .with_uniform_padding(8.)
            .with_padding_right(12.)
            .with_padding_left(12.)
            .finish()
    }
}

impl Entity for CommandSearchZeroStateView {
    type Event = CommandSearchZeroStateEvent;
}

#[derive(Debug)]
pub enum CommandSearchZeroStateAction {
    FilterChipClicked(QueryFilter),
    SampleQueryClicked(QueryFilter),
}

impl TypedActionView for CommandSearchZeroStateView {
    type Action = CommandSearchZeroStateAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CommandSearchZeroStateAction::FilterChipClicked(filter) => {
                ctx.emit(CommandSearchZeroStateEvent::FilterChipSelected(*filter))
            }
            CommandSearchZeroStateAction::SampleQueryClicked(filter) => {
                ctx.emit(CommandSearchZeroStateEvent::SampleQuerySelected(*filter))
            }
        }
    }
}

/// Returns list of valid query filters that may be applied. This does not include notebooks if the
/// notebooks feature flag is disabled.
fn valid_query_filters(app: &AppContext) -> Vec<QueryFilter> {
    let mut filters = vec![QueryFilter::History];

    if FeatureFlag::AgentMode.is_enabled() && AISettings::as_ref(app).is_any_ai_enabled(app) {
        if FeatureFlag::AgentModeWorkflows.is_enabled() {
            filters.push(QueryFilter::AgentModeWorkflows);
        }
        filters.push(QueryFilter::PromptHistory);
    }

    if WarpDriveSettings::is_warp_drive_enabled(app) {
        filters.extend([QueryFilter::Workflows, QueryFilter::Notebooks]);

        filters.push(QueryFilter::EnvironmentVariables);
    }

    filters
}

mod styles {
    use crate::appearance::Appearance;

    pub const FILTER_CHIP_MARGIN: f32 = 8.;
    pub const FILTER_CHIPS_MARGIN_BOTTOM: f32 = 16.;

    pub const COMMAND_SEARCH_TEXT_MARGIN_BOTTOM: f32 = 12.;
    pub const FILTER_PREFIX_TEXT_MARGIN_BOTTOM: f32 = 8.;
    pub const SAMPLE_QUERY_MARGIN: f32 = 8.;

    pub fn header_text_font_size(appearance: &Appearance) -> f32 {
        appearance.monospace_font_size() + 6.
    }

    pub fn subheader_text_font_size(appearance: &Appearance) -> f32 {
        appearance.monospace_font_size()
    }
}
