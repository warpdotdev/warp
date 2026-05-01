use std::collections::HashSet;

use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CornerRadius, DropShadow, Radius, Text,
};
use warpui::{
    AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity, View, ViewContext,
    ViewHandle,
};

use crate::ai::blocklist::agent_view::AgentViewController;
use crate::search::data_source::QueryFilter;
use crate::terminal::input::buffer_model::InputBufferModel;
use crate::terminal::input::inline_history::{
    AcceptHistoryItem, HistoryTab, InlineHistoryMenuEvent, InlineHistoryMenuView,
};
use crate::terminal::input::inline_menu::styles as inline_menu_styles;
use crate::terminal::input::inline_menu::{InlineMenuPositioner, InlineMenuTabConfig};
use crate::terminal::input::suggestions_mode_model::InputSuggestionsModeModel;
use crate::terminal::model::session::active_session::ActiveSession;

const MENU_MAX_HEIGHT: f32 = 168.;

const MENU_VERTICAL_PADDING: f32 = 4.;

const MENU_CORNER_RADIUS: f32 = 6.;

const DROP_SHADOW_COLOR: ColorU = ColorU {
    r: 0,
    g: 0,
    b: 0,
    a: 77,
};

pub struct CloudModeV2HistoryMenuView {
    inner: ViewHandle<InlineHistoryMenuView>,
}

impl CloudModeV2HistoryMenuView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        terminal_view_id: EntityId,
        active_session: ModelHandle<ActiveSession>,
        input_suggestions_model: &ModelHandle<InputSuggestionsModeModel>,
        agent_view_controller: ModelHandle<AgentViewController>,
        positioner: &ModelHandle<InlineMenuPositioner>,
        buffer_model: ModelHandle<InputBufferModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let tab_configs = vec![InlineMenuTabConfig {
            id: HistoryTab::Prompts,
            label: "Prompts".to_string(),
            filters: HashSet::from([QueryFilter::PromptHistory]),
        }];
        let inner = ctx.add_view(|ctx| {
            InlineHistoryMenuView::new_with_tab_configs(
                terminal_view_id,
                active_session,
                input_suggestions_model,
                agent_view_controller,
                positioner,
                buffer_model,
                tab_configs,
                ctx,
            )
        });

        ctx.subscribe_to_view(&inner, |_, _, event, ctx| {
            ctx.emit(event.clone());
            ctx.notify();
        });

        Self { inner }
    }

    pub fn select_up(&self, ctx: &mut ViewContext<Self>) {
        self.inner.update(ctx, |v, ctx| v.select_up(ctx));
    }

    pub fn arm_initial_buffer_sync(&self, ctx: &mut ViewContext<Self>) {
        self.inner.update(ctx, |v, _| v.arm_initial_buffer_sync());
    }

    pub fn select_down(&self, ctx: &mut ViewContext<Self>) {
        self.inner.update(ctx, |v, ctx| v.select_down(ctx));
    }

    pub fn accept_selected(&self, ctx: &mut ViewContext<Self>) {
        self.inner.update(ctx, |v, ctx| v.accept_selected_item(ctx));
    }

    pub fn has_selection(&self, app: &AppContext) -> bool {
        self.inner
            .as_ref(app)
            .model()
            .as_ref(app)
            .selected_item()
            .is_some()
    }

    /// Returns the currently selected AI prompt's query text, if any.
    ///
    /// The cloud-mode V2 menu is restricted to `AcceptHistoryItem::AIPrompt`
    /// items via its tab filters, so we only ever expect prompt selections.
    pub fn selected_query_text(&self, app: &AppContext) -> Option<String> {
        match self.inner.as_ref(app).model().as_ref(app).selected_item()? {
            AcceptHistoryItem::AIPrompt { query_text } => Some(query_text.clone()),
            AcceptHistoryItem::Command { .. } | AcceptHistoryItem::Conversation { .. } => None,
        }
    }
}

impl Entity for CloudModeV2HistoryMenuView {
    type Event = InlineHistoryMenuEvent;
}

impl View for CloudModeV2HistoryMenuView {
    fn ui_name() -> &'static str {
        "CloudModeV2HistoryMenuView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let row_count = self.inner.as_ref(app).result_count(app);
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let border_color = internal_colors::neutral_4(theme);
        let background = internal_colors::neutral_1(theme);

        let item_height = appearance.monospace_font_size() + 8.;
        let visible_row_count = row_count.max(1) as f32;
        let content_height = (item_height * visible_row_count
            + 2. * inline_menu_styles::CONTENT_VERTICAL_PADDING)
            .min(MENU_MAX_HEIGHT);

        let content: Box<dyn Element> = if row_count == 0 {
            let no_results_text = Text::new(
                "No results".to_string(),
                appearance.ui_font_family(),
                inline_menu_styles::font_size(appearance),
            )
            .with_color(
                theme
                    .disabled_text_color(Fill::Solid(background))
                    .into_solid(),
            )
            .finish();
            Align::new(no_results_text).finish()
        } else {
            self.inner.as_ref(app).render_results_only(app)
        };

        let constrained = ConstrainedBox::new(content)
            .with_height(content_height)
            .finish();

        let padded = Container::new(constrained)
            .with_padding_top(MENU_VERTICAL_PADDING)
            .with_padding_bottom(MENU_VERTICAL_PADDING)
            .finish();

        Container::new(padded)
            .with_background(background)
            .with_border(Border::all(1.).with_border_color(border_color))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(MENU_CORNER_RADIUS)))
            .with_drop_shadow(DropShadow::new_with_standard_offset_and_spread(
                DROP_SHADOW_COLOR,
            ))
            .finish()
    }
}
