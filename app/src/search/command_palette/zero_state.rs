mod items;
use std::collections::HashMap;

pub use items::Items;
use warp_core::context_flag::ContextFlag;
use warp_core::features::FeatureFlag;
use warpui::elements::{Container, Flex, MouseStateHandle, ParentElement, Shrinkable, Wrap};
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    WindowId,
};

use crate::appearance::Appearance;
use crate::drive::settings::WarpDriveSettings;
use crate::search::command_palette::FilterChipRenderer;
use crate::search::QueryFilter;
use crate::settings::AISettings;
use crate::workspace::Workspace;

/// A zero-state view for the command palette.
pub struct ZeroState {
    filter_chip_to_mouse_state_handle: HashMap<QueryFilter, MouseStateHandle>,
    items: ModelHandle<Items>,
    // Store the window this view belongs to so we don't rely on the global active window
    window_id: WindowId,
}

#[derive(Debug)]
pub enum Action {
    FilterChipClicked { filter: QueryFilter },
}

#[derive(Debug)]
pub enum Event {
    FilterChipSelected { filter: QueryFilter },
}

impl ZeroState {
    pub fn new(results_model: ModelHandle<Items>, ctx: &mut ViewContext<Self>) -> Self {
        ctx.observe(&results_model, |_, _, ctx| {
            ctx.notify();
        });
        Self {
            filter_chip_to_mouse_state_handle: QueryFilter::all()
                .map(|filter| (filter, MouseStateHandle::default()))
                .collect(),

            items: results_model,
            window_id: ctx.window_id(),
        }
    }

    /// Renders a clickable chip for each valid query filter. When a chip is
    /// clicked, the filter is emitted in a [`Event::FilterChipSelected`] event.
    fn render_filter_chips(
        &self,
        appearance: &Appearance,
        valid_filters: impl IntoIterator<Item = QueryFilter>,
    ) -> Box<dyn Element> {
        let wrap = Wrap::row()
            .with_run_spacing(styles::FILTER_CHIP_MARGIN)
            .with_children(valid_filters.into_iter().map(|filter| {
                Container::new(filter.render_filter_chip(
                    self.filter_chip_to_mouse_state_handle[&filter].clone(),
                    appearance,
                    |event_ctx, filter| {
                        event_ctx.dispatch_typed_action(Action::FilterChipClicked { filter })
                    },
                ))
                .with_margin_right(styles::FILTER_CHIP_MARGIN)
                .finish()
            }));

        Container::new(wrap.finish())
            .with_margin_bottom(styles::FILTER_CHIPS_MARGIN_BOTTOM)
            .finish()
    }

    /// Returns the set of valid query filters for this zero state view.
    fn valid_query_filters(
        app: &AppContext,
        window_id: WindowId,
    ) -> impl Iterator<Item = QueryFilter> {
        let show_warp_drive = WarpDriveSettings::is_warp_drive_enabled(app);

        let mut valid_filters = vec![];
        if show_warp_drive {
            valid_filters.push(QueryFilter::Workflows);
            if FeatureFlag::AgentModeWorkflows.is_enabled()
                && AISettings::as_ref(app).is_any_ai_enabled(app)
            {
                valid_filters.push(QueryFilter::AgentModeWorkflows);
            }
            valid_filters.push(QueryFilter::Notebooks);

            valid_filters.push(QueryFilter::EnvironmentVariables);
        }

        // Don't show Files filter if the user is a viewer of a shared session
        if FeatureFlag::CommandPaletteFileSearch.is_enabled() {
            let is_shared_session_viewer_focused = app
                .views_of_type::<Workspace>(window_id)
                .and_then(|workspaces| workspaces.first().cloned())
                .is_some_and(|workspace| {
                    workspace.as_ref(app).is_shared_session_viewer_focused(app)
                });
            if !is_shared_session_viewer_focused {
                valid_filters.push(QueryFilter::Files);
            }
        }

        if show_warp_drive {
            valid_filters.push(QueryFilter::Drive);
        }
        valid_filters.extend([QueryFilter::Actions, QueryFilter::Sessions]);

        if ContextFlag::LaunchConfigurations.is_enabled() {
            valid_filters.push(QueryFilter::LaunchConfigurations);
        }

        if AISettings::as_ref(app).is_any_ai_enabled(app) {
            valid_filters.push(QueryFilter::Conversations);
        }

        valid_filters.into_iter()
    }
}

impl Entity for ZeroState {
    type Event = Event;
}

impl View for ZeroState {
    fn ui_name() -> &'static str {
        "CommandPaletteZeroState"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut flex = Flex::column().with_child(
            self.render_filter_chips(appearance, Self::valid_query_filters(app, self.window_id)),
        );

        let zero_state_items = self.items.as_ref(app).render(app);
        flex.add_child(Shrinkable::new(1., zero_state_items).finish());

        Container::new(flex.finish())
            .with_vertical_padding(styles::PADDING_VERTICAL)
            .with_horizontal_padding(styles::PADDING_HORIZONTAL)
            .finish()
    }
}

impl TypedActionView for ZeroState {
    type Action = Action;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            Action::FilterChipClicked { filter } => {
                ctx.emit(Event::FilterChipSelected { filter: *filter })
            }
        }
    }
}

mod styles {
    pub const FILTER_CHIP_MARGIN: f32 = 8.;
    pub const FILTER_CHIPS_MARGIN_BOTTOM: f32 = 16.;

    /// Horizontal padding around all inner content within the view.
    pub const PADDING_HORIZONTAL: f32 = 24.;

    /// Vertical padding around all inner content within the view.
    pub const PADDING_VERTICAL: f32 = 8.;
}
