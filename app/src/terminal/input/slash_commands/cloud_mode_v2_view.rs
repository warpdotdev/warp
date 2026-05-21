use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Border, ChildAnchor, Clipped, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, CrossAxisAlignment, DispatchEventResult, DropShadow, EventHandler,
    Flex, Hoverable, MainAxisSize, MouseInBehavior, MouseStateHandle, OffsetPositioning,
    ParentElement, PositionedElementAnchor, PositionedElementOffsetBounds, Radius, SavePosition,
    ScrollTarget, ScrollToPositionMode, ScrollbarWidth, Stack, Text,
};
use warpui::platform::Cursor;
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::search::data_source::QueryFilter;
use crate::search::item::SearchItemDetail;
use crate::search::mixer::{AddAsyncSourceOptions, SearchMixer, SearchMixerEvent};
use crate::search::result_renderer::{QueryResultRenderer, QueryResultRendererStyles};
use crate::search::slash_command_menu::static_commands::commands::COMMAND_REGISTRY;
use crate::terminal::input::buffer_model::{InputBufferModel, InputBufferUpdateEvent};
use crate::terminal::input::inline_menu::styles as inline_styles;
use crate::terminal::input::slash_command_model::{SlashCommandEntryState, SlashCommandModel};
use crate::terminal::input::slash_commands::view::{slash_command_query, CloseReason};
use crate::terminal::input::slash_commands::{
    saved_prompts_data_source, AcceptSlashCommandOrSavedPrompt, SlashCommandDataSource,
    SlashCommandsEvent, UpdatedActiveCommands, ZeroStateDataSource,
};
use crate::terminal::input::suggestions_mode_model::{
    InputSuggestionsModeEvent, InputSuggestionsModeModel,
};

const MENU_WIDTH: f32 = 320.;

const MENU_MAX_HEIGHT: f32 = 400.;

const ITEMS_PER_SECTION_COLLAPSED: usize = 3;

const SECTION_HEADER_FONT_SIZE: f32 = 12.;

const ITEM_FONT_SIZE: f32 = 14.;

const MENU_HORIZONTAL_PADDING: f32 = 16.;

const MENU_VERTICAL_PADDING: f32 = 4.;

const MENU_CORNER_RADIUS: f32 = 6.;

const ROW_VERTICAL_PADDING: f32 = 4.;

const ICON_SIZE: f32 = 16.;

const DIVIDER_HEIGHT: f32 = 1.;

const DIVIDER_VERTICAL_PADDING: f32 = 0.;

const SIDECAR_WIDTH: f32 = MENU_WIDTH;

const SIDECAR_MAX_HEIGHT: f32 = 240.;

const SIDECAR_GAP: f32 = 2.;

const SIDECAR_DESCRIPTION_FONT_SIZE: f32 = 12.;

const SIDECAR_TITLE_TO_DESCRIPTION_GAP: f32 = 4.;

const NAME_DESCRIPTION_GAP_PX: f32 = 8.;

fn row_position_id(visible_idx: usize) -> String {
    format!("cloud_mode_v2_slash_row_{visible_idx}")
}

fn item_is_truncated_in_row(detail: &SearchItemDetail, app: &AppContext) -> bool {
    let appearance = Appearance::as_ref(app);
    let font_size = inline_styles::font_size(appearance);
    let font_cache = app.font_cache();
    let name_em = font_cache.em_width(detail.title_font_family, font_size);
    let name_px = name_em * detail.title.chars().count() as f32;
    let row_chrome_px = MENU_HORIZONTAL_PADDING * 2. + ICON_SIZE + inline_styles::ICON_MARGIN;
    let available = MENU_WIDTH - row_chrome_px;
    match &detail.description {
        Some(description) => {
            let description_em = font_cache.em_width(appearance.ui_font_family(), font_size);
            let description_px = description_em * description.chars().count() as f32;
            (name_px + NAME_DESCRIPTION_GAP_PX + description_px) > available
        }
        None => name_px > available,
    }
}

static QUERY_RESULT_RENDERER_STYLES: LazyLock<QueryResultRendererStyles> =
    LazyLock::new(|| QueryResultRendererStyles {
        result_item_height_fn: |appearance| appearance.monospace_font_size() + 8.,
        panel_corner_radius: CornerRadius::with_all(Radius::Pixels(0.)),
        result_vertical_padding: ROW_VERTICAL_PADDING,
        ..Default::default()
    });

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Section {
    Commands,
    Skills,
    Prompts,
}

impl Section {
    const RENDER_ORDER: [Self; 3] = [Self::Commands, Self::Skills, Self::Prompts];

    fn header(self) -> &'static str {
        match self {
            Self::Commands => "Commands",
            Self::Skills => "Skills",
            Self::Prompts => "Prompts",
        }
    }

    fn for_action(action: &AcceptSlashCommandOrSavedPrompt) -> Self {
        match action {
            AcceptSlashCommandOrSavedPrompt::SlashCommand { .. } => Self::Commands,
            AcceptSlashCommandOrSavedPrompt::Skill { .. } => Self::Skills,
            AcceptSlashCommandOrSavedPrompt::SavedPrompt { .. } => Self::Prompts,
        }
    }
}

struct RenderedSection {
    section: Section,
    items: Vec<QueryResultRenderer<AcceptSlashCommandOrSavedPrompt>>,
}

#[derive(Clone, Copy)]
enum NoSearchActiveRow {
    SectionHeader(Section),
    Item {
        section: Section,
        item_idx: usize,
    },
    ShowMore {
        section: Section,
        hidden_count: usize,
    },
    Divider,
}

impl NoSearchActiveRow {
    fn is_selectable(self) -> bool {
        matches!(
            self,
            NoSearchActiveRow::Item { .. } | NoSearchActiveRow::ShowMore { .. }
        )
    }
}

enum MenuState {
    NoSearchActive {
        sections: Vec<RenderedSection>,
        expanded_sections: HashSet<Section>,
        selected_idx: Option<usize>,
        show_more_mouse_states: HashMap<Section, MouseStateHandle>,
    },
    SearchActive {
        results: Vec<QueryResultRenderer<AcceptSlashCommandOrSavedPrompt>>,
        selected_idx: Option<usize>,
    },
}

impl MenuState {
    fn empty() -> Self {
        MenuState::NoSearchActive {
            sections: Vec::new(),
            expanded_sections: HashSet::new(),
            selected_idx: None,
            show_more_mouse_states: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum CloudModeV2SlashCommandAction {
    Accept {
        item: AcceptSlashCommandOrSavedPrompt,
        cmd_or_ctrl_enter: bool,
    },
    HoverIdx(usize),
    ToggleSection(Section),
    Dismiss,
}

pub struct CloudModeV2SlashCommandView {
    mixer: ModelHandle<SearchMixer<AcceptSlashCommandOrSavedPrompt>>,
    suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
    input_buffer_model: ModelHandle<InputBufferModel>,
    scroll_state: ClippedScrollStateHandle,
    menu_state: MenuState,
    section_filter: Option<Section>,
}

impl CloudModeV2SlashCommandView {
    pub fn new(
        slash_command_model: &ModelHandle<SlashCommandModel>,
        slash_commands_source: ModelHandle<SlashCommandDataSource>,
        suggestions_mode_model: ModelHandle<InputSuggestionsModeModel>,
        input_buffer_model: ModelHandle<InputBufferModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(
            &slash_commands_source,
            |me, _, _: &UpdatedActiveCommands, ctx| {
                me.mixer.update(ctx, |mixer, ctx| {
                    if let Some(query) = mixer.current_query().cloned() {
                        mixer.run_query(query, ctx);
                    }
                });
            },
        );

        let zero_state_source =
            ctx.add_model(|_| ZeroStateDataSource::new(&slash_commands_source, true));
        let saved_prompts_source = saved_prompts_data_source();

        let mixer = ctx.add_model(|ctx| {
            let mut mixer = SearchMixer::<AcceptSlashCommandOrSavedPrompt>::new();
            mixer.add_sync_source(
                slash_commands_source.clone(),
                [QueryFilter::StaticSlashCommands],
            );
            mixer.add_async_source(
                saved_prompts_source,
                [QueryFilter::StaticSlashCommands],
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: false,
                },
                ctx,
            );
            mixer.add_sync_source(
                zero_state_source.clone(),
                [QueryFilter::StaticSlashCommands],
            );
            mixer.run_query(slash_command_query(""), ctx);
            mixer
        });

        ctx.subscribe_to_model(&mixer, |me, _, event, ctx| match event {
            SearchMixerEvent::ResultsChanged => {
                if me.mixer.as_ref(ctx).is_loading() {
                    return;
                }
                me.rebuild_from_results(ctx);
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(slash_command_model, |me, model, _, ctx| {
            if !me.suggestions_mode_model.as_ref(ctx).is_slash_commands() {
                return;
            }
            match model.as_ref(ctx).state().clone() {
                SlashCommandEntryState::None
                | SlashCommandEntryState::Composing { .. }
                | SlashCommandEntryState::SlashCommand(_) => {
                    me.run_query_for_current_slash_filter(ctx);
                }
                _ => (),
            }
        });

        ctx.subscribe_to_model(
            &input_buffer_model,
            |me, _, _: &InputBufferUpdateEvent, ctx| {
                if !me.suggestions_mode_model.as_ref(ctx).is_slash_commands() {
                    return;
                }
                me.run_query_for_current_slash_filter(ctx);
            },
        );

        ctx.subscribe_to_model(&suggestions_mode_model, |me, _, event, ctx| {
            let InputSuggestionsModeEvent::ModeChanged { .. } = event;
            if me.suggestions_mode_model.as_ref(ctx).is_closed() {
                me.mixer.update(ctx, |mixer, ctx| {
                    mixer.reset_results(ctx);
                });
                me.menu_state = MenuState::empty();
                me.section_filter = None;
                return;
            }
            if me.suggestions_mode_model.as_ref(ctx).is_slash_commands() {
                me.run_query_for_current_slash_filter(ctx);
            }
        });

        Self {
            mixer,
            suggestions_mode_model,
            input_buffer_model,
            scroll_state: Default::default(),
            menu_state: MenuState::empty(),
            section_filter: None,
        }
    }

    pub fn set_section_filter(&mut self, filter: Option<Section>, ctx: &mut ViewContext<Self>) {
        let previous = self.section_filter;
        self.section_filter = filter;
        if let MenuState::NoSearchActive {
            sections,
            expanded_sections,
            selected_idx,
            ..
        } = &mut self.menu_state
        {
            let rows = browsing_rows_filtered(sections, expanded_sections, self.section_filter);
            *selected_idx = match (filter, previous) {
                (None, Some(prev)) => rows
                    .iter()
                    .position(|r| matches_originating_command(r, sections, prev))
                    .or_else(|| rows.iter().position(|r| r.is_selectable())),
                _ => rows.iter().position(|r| r.is_selectable()),
            };
        }
        ctx.notify();
    }

    pub fn has_section_filter(&self) -> bool {
        self.section_filter.is_some()
    }

    pub fn select_up(&mut self, ctx: &mut ViewContext<Self>) {
        self.move_selection(SelectionDirection::Up, ctx);
    }

    pub fn select_down(&mut self, ctx: &mut ViewContext<Self>) {
        self.move_selection(SelectionDirection::Down, ctx);
    }

    fn scroll_selected_into_view(&self, visible_idx: usize) {
        self.scroll_state.scroll_to_position(ScrollTarget {
            position_id: row_position_id(visible_idx),
            mode: ScrollToPositionMode::FullyIntoView,
        });
    }

    pub fn accept_selected_item(&mut self, cmd_or_ctrl_enter: bool, ctx: &mut ViewContext<Self>) {
        match &self.menu_state {
            MenuState::NoSearchActive {
                sections,
                expanded_sections,
                selected_idx,
                ..
            } => {
                let Some(selected_idx) = *selected_idx else {
                    return;
                };
                let rows = browsing_rows_filtered(sections, expanded_sections, self.section_filter);
                let Some(row) = rows.get(selected_idx) else {
                    return;
                };
                match *row {
                    NoSearchActiveRow::Item { section, item_idx } => {
                        let Some(rendered) = sections.iter().find(|s| s.section == section) else {
                            return;
                        };
                        let Some(item) = rendered.items.get(item_idx) else {
                            return;
                        };
                        if item.search_result.is_disabled() {
                            return;
                        }
                        let action = item.search_result.accept_result();
                        self.emit_selection(&action, cmd_or_ctrl_enter, ctx);
                    }
                    NoSearchActiveRow::ShowMore { section, .. } => {
                        self.toggle_section(section, ctx);
                    }
                    NoSearchActiveRow::SectionHeader(_) | NoSearchActiveRow::Divider => {}
                }
            }
            MenuState::SearchActive {
                results,
                selected_idx,
            } => {
                let Some(idx) = *selected_idx else {
                    return;
                };
                let Some(item) = results.get(idx) else {
                    return;
                };
                if item.search_result.is_disabled() {
                    return;
                }
                let action = item.search_result.accept_result();
                self.emit_selection(&action, cmd_or_ctrl_enter, ctx);
            }
        }
    }

    pub fn dismiss(&mut self, ctx: &mut ViewContext<Self>) {
        if self.section_filter.is_some() {
            self.set_section_filter(None, ctx);
            return;
        }
        ctx.emit(SlashCommandsEvent::Close(CloseReason::ManualDismissal));
    }

    pub fn result_count(&self, app: &AppContext) -> usize {
        self.mixer.as_ref(app).results().len()
    }

    fn current_query_text(&self, app: &AppContext) -> String {
        self.input_buffer_model
            .as_ref(app)
            .current_value()
            .strip_prefix('/')
            .map(ToOwned::to_owned)
            .unwrap_or_default()
    }

    fn run_query_for_current_slash_filter(&mut self, ctx: &mut ViewContext<Self>) {
        let filter = self.current_query_text(ctx);
        self.mixer.update(ctx, move |mixer, ctx| {
            if mixer.current_query().is_some_and(|q| q.text == filter) {
                return;
            }
            mixer.run_query(slash_command_query(&filter), ctx);
        });
    }

    fn rebuild_from_results(&mut self, ctx: &mut ViewContext<Self>) {
        let on_click_fn = |_idx: usize,
                           item: AcceptSlashCommandOrSavedPrompt,
                           evt_ctx: &mut warpui::EventContext| {
            evt_ctx.dispatch_typed_action(CloudModeV2SlashCommandAction::Accept {
                item,
                cmd_or_ctrl_enter: false,
            });
        };

        let renderers: Vec<QueryResultRenderer<AcceptSlashCommandOrSavedPrompt>> = self
            .mixer
            .as_ref(ctx)
            .results()
            .iter()
            .rev()
            .enumerate()
            .map(|(idx, result)| {
                QueryResultRenderer::new(
                    result.clone(),
                    format!("v2_slash:{idx}"),
                    on_click_fn,
                    *QUERY_RESULT_RENDERER_STYLES,
                )
            })
            .collect();

        let query_is_empty = self.current_query_text(ctx).is_empty();

        self.menu_state = if query_is_empty {
            let mut by_section: HashMap<Section, Vec<QueryResultRenderer<_>>> = HashMap::new();
            for renderer in renderers {
                let section = Section::for_action(&renderer.search_result.accept_result());
                by_section.entry(section).or_default().push(renderer);
            }
            let sections: Vec<RenderedSection> = Section::RENDER_ORDER
                .into_iter()
                .map(|s| RenderedSection {
                    section: s,
                    items: by_section.remove(&s).unwrap_or_default(),
                })
                .collect();

            let expanded_sections = match &self.menu_state {
                MenuState::NoSearchActive {
                    expanded_sections, ..
                } => expanded_sections.clone(),
                MenuState::SearchActive { .. } => HashSet::new(),
            };

            let mut show_more_mouse_states = match &self.menu_state {
                MenuState::NoSearchActive {
                    show_more_mouse_states,
                    ..
                } => show_more_mouse_states.clone(),
                MenuState::SearchActive { .. } => HashMap::new(),
            };
            for section in Section::RENDER_ORDER {
                show_more_mouse_states
                    .entry(section)
                    .or_insert_with(MouseStateHandle::default);
            }

            let mut state = MenuState::NoSearchActive {
                sections,
                expanded_sections,
                selected_idx: None,
                show_more_mouse_states,
            };
            initialize_browsing_selection(&mut state);
            state
        } else {
            self.section_filter = None;
            let mut state = MenuState::SearchActive {
                results: renderers,
                selected_idx: None,
            };
            initialize_search_selection(&mut state);
            state
        };
    }

    fn toggle_section(&mut self, section: Section, ctx: &mut ViewContext<Self>) {
        if let MenuState::NoSearchActive {
            expanded_sections, ..
        } = &mut self.menu_state
        {
            if !expanded_sections.insert(section) {
                expanded_sections.remove(&section);
            }
        }
        clamp_browsing_selection(&mut self.menu_state);
        ctx.notify();
    }

    fn emit_selection(
        &self,
        action: &AcceptSlashCommandOrSavedPrompt,
        cmd_or_ctrl_enter: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            AcceptSlashCommandOrSavedPrompt::SlashCommand { id } => {
                ctx.emit(SlashCommandsEvent::SelectedStaticCommand {
                    id: *id,
                    cmd_or_ctrl_enter,
                });
            }
            AcceptSlashCommandOrSavedPrompt::SavedPrompt { id } => {
                ctx.emit(SlashCommandsEvent::SelectedSavedPrompt { id: *id });
            }
            AcceptSlashCommandOrSavedPrompt::Skill { name, reference } => {
                ctx.emit(SlashCommandsEvent::SelectedSkill {
                    reference: reference.clone(),
                    name: name.clone(),
                });
            }
        }
    }

    fn move_selection(&mut self, direction: SelectionDirection, ctx: &mut ViewContext<Self>) {
        let section_filter = self.section_filter;
        let next_idx: Option<usize> = match &mut self.menu_state {
            MenuState::NoSearchActive {
                sections,
                expanded_sections,
                selected_idx,
                ..
            } => {
                let rows = browsing_rows_filtered(sections, expanded_sections, section_filter);
                if rows.is_empty() {
                    return;
                }
                let next =
                    next_selectable_idx(&rows, *selected_idx, direction, |r| r.is_selectable());
                if let Some(next) = next {
                    *selected_idx = Some(next);
                }
                next
            }
            MenuState::SearchActive {
                results,
                selected_idx,
            } => {
                if results.is_empty() {
                    return;
                }
                let next = next_selectable_idx(results, *selected_idx, direction, |r| {
                    !r.search_result.is_disabled()
                });
                if let Some(next) = next {
                    *selected_idx = Some(next);
                }
                next
            }
        };
        if let Some(idx) = next_idx {
            self.scroll_selected_into_view(idx);
        }
        ctx.notify();
    }

    fn set_browsing_selection(&mut self, idx: usize, ctx: &mut ViewContext<Self>) {
        let section_filter = self.section_filter;
        if let MenuState::NoSearchActive {
            sections,
            expanded_sections,
            selected_idx,
            ..
        } = &mut self.menu_state
        {
            let rows = browsing_rows_filtered(sections, expanded_sections, section_filter);
            if rows.get(idx).is_some_and(|r| r.is_selectable()) {
                *selected_idx = Some(idx);
                self.scroll_selected_into_view(idx);
                ctx.notify();
            }
        }
    }

    fn set_search_selection(&mut self, idx: usize, ctx: &mut ViewContext<Self>) {
        if let MenuState::SearchActive {
            results,
            selected_idx,
        } = &mut self.menu_state
        {
            if let Some(item) = results.get(idx) {
                if !item.search_result.is_disabled() {
                    *selected_idx = Some(idx);
                    self.scroll_selected_into_view(idx);
                    ctx.notify();
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum SelectionDirection {
    Up,
    Down,
}

fn browsing_rows_filtered(
    sections: &[RenderedSection],
    expanded_sections: &HashSet<Section>,
    filter: Option<Section>,
) -> Vec<NoSearchActiveRow> {
    let mut rows = Vec::new();
    let non_empty_sections: Vec<&RenderedSection> = sections
        .iter()
        .filter(|s| !s.items.is_empty())
        .filter(|s| filter.is_none_or(|f| f == s.section))
        .collect();

    for (idx, rendered) in non_empty_sections.iter().enumerate() {
        rows.push(NoSearchActiveRow::SectionHeader(rendered.section));
        let is_filtered_section = filter == Some(rendered.section);
        let visible_count = if is_filtered_section || expanded_sections.contains(&rendered.section)
        {
            rendered.items.len()
        } else {
            rendered.items.len().min(ITEMS_PER_SECTION_COLLAPSED)
        };
        for item_idx in 0..visible_count {
            rows.push(NoSearchActiveRow::Item {
                section: rendered.section,
                item_idx,
            });
        }
        let hidden_count = rendered.items.len().saturating_sub(visible_count);
        if hidden_count > 0 && !is_filtered_section {
            rows.push(NoSearchActiveRow::ShowMore {
                section: rendered.section,
                hidden_count,
            });
        }
        let is_last = idx + 1 == non_empty_sections.len();
        if !is_last {
            rows.push(NoSearchActiveRow::Divider);
        }
    }
    rows
}

fn browsing_rows(
    sections: &[RenderedSection],
    expanded_sections: &HashSet<Section>,
) -> Vec<NoSearchActiveRow> {
    browsing_rows_filtered(sections, expanded_sections, None)
}

fn initialize_browsing_selection(state: &mut MenuState) {
    if let MenuState::NoSearchActive {
        sections,
        expanded_sections,
        selected_idx,
        ..
    } = state
    {
        let rows = browsing_rows(sections, expanded_sections);
        *selected_idx = rows.iter().position(|r| r.is_selectable());
    }
}

fn matches_originating_command(
    row: &NoSearchActiveRow,
    sections: &[RenderedSection],
    previous_filter: Section,
) -> bool {
    let target_name = match previous_filter {
        Section::Prompts => "/prompts",
        Section::Skills => "/skills",
        Section::Commands => return false,
    };
    let NoSearchActiveRow::Item { section, item_idx } = *row else {
        return false;
    };
    if section != Section::Commands {
        return false;
    }
    let Some(item) = sections
        .iter()
        .find(|s| s.section == Section::Commands)
        .and_then(|s| s.items.get(item_idx))
    else {
        return false;
    };
    let AcceptSlashCommandOrSavedPrompt::SlashCommand { id } = item.search_result.accept_result()
    else {
        return false;
    };
    COMMAND_REGISTRY
        .get_command(&id)
        .is_some_and(|cmd| cmd.name == target_name)
}

fn initialize_search_selection(state: &mut MenuState) {
    if let MenuState::SearchActive {
        results,
        selected_idx,
    } = state
    {
        *selected_idx = results.iter().position(|r| !r.search_result.is_disabled());
    }
}

fn clamp_browsing_selection(state: &mut MenuState) {
    if let MenuState::NoSearchActive {
        sections,
        expanded_sections,
        selected_idx,
        ..
    } = state
    {
        let rows = browsing_rows(sections, expanded_sections);
        let in_range = selected_idx
            .as_ref()
            .is_some_and(|idx| rows.get(*idx).is_some_and(|r| r.is_selectable()));
        if !in_range {
            *selected_idx = rows.iter().position(|r| r.is_selectable());
        }
    }
}

fn next_selectable_idx<T>(
    items: &[T],
    current: Option<usize>,
    direction: SelectionDirection,
    is_selectable: impl Fn(&T) -> bool,
) -> Option<usize> {
    if items.is_empty() {
        return None;
    }
    let count = items.len();
    let start = match (current, direction) {
        (Some(idx), SelectionDirection::Down) if idx + 1 < count => idx + 1,
        (Some(_), SelectionDirection::Down) => 0,
        (Some(idx), SelectionDirection::Up) if idx > 0 => idx - 1,
        (Some(_), SelectionDirection::Up) => count - 1,
        (None, SelectionDirection::Down) => 0,
        (None, SelectionDirection::Up) => count - 1,
    };
    for offset in 0..count {
        let candidate = match direction {
            SelectionDirection::Down => (start + offset) % count,
            SelectionDirection::Up => (start + count - offset) % count,
        };
        if is_selectable(&items[candidate]) {
            return Some(candidate);
        }
    }
    None
}

impl CloudModeV2SlashCommandView {
    fn render_no_search_active(
        &self,
        sections: &[RenderedSection],
        expanded_sections: &HashSet<Section>,
        selected_idx: Option<usize>,
        show_more_mouse_states: &HashMap<Section, MouseStateHandle>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let rows = browsing_rows_filtered(sections, expanded_sections, self.section_filter);
        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min);

        for (visible_idx, row) in rows.iter().enumerate() {
            let is_selected = selected_idx == Some(visible_idx);
            match *row {
                NoSearchActiveRow::SectionHeader(section) => {
                    column.add_child(render_section_header(section, app));
                }
                NoSearchActiveRow::Item { section, item_idx } => {
                    let Some(rendered) = sections.iter().find(|s| s.section == section) else {
                        continue;
                    };
                    let Some(renderer) = rendered.items.get(item_idx) else {
                        continue;
                    };
                    let row_element = self.wrap_with_hover(
                        renderer.render_inline(visible_idx, is_selected, app),
                        visible_idx,
                        true,
                    );
                    column.add_child(
                        SavePosition::new(row_element, &row_position_id(visible_idx)).finish(),
                    );
                }
                NoSearchActiveRow::ShowMore {
                    section,
                    hidden_count,
                } => {
                    let mouse_state = show_more_mouse_states
                        .get(&section)
                        .cloned()
                        .unwrap_or_default();
                    let row_element = render_show_more_row(
                        section,
                        hidden_count,
                        is_selected,
                        mouse_state,
                        visible_idx,
                        app,
                    );
                    column.add_child(
                        SavePosition::new(row_element, &row_position_id(visible_idx)).finish(),
                    );
                }
                NoSearchActiveRow::Divider => {
                    column.add_child(render_divider(app));
                }
            }
        }

        column.finish()
    }

    fn render_search_active(
        &self,
        results: &[QueryResultRenderer<AcceptSlashCommandOrSavedPrompt>],
        selected_idx: Option<usize>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min);
        for (idx, renderer) in results.iter().enumerate() {
            let is_selected = selected_idx == Some(idx);
            let row_element =
                self.wrap_with_hover(renderer.render_inline(idx, is_selected, app), idx, false);
            column.add_child(SavePosition::new(row_element, &row_position_id(idx)).finish());
        }
        column.finish()
    }

    fn wrap_with_hover(
        &self,
        element: Box<dyn Element>,
        idx: usize,
        _is_browsing: bool,
    ) -> Box<dyn Element> {
        EventHandler::new(element)
            .on_mouse_in(
                move |ctx, _, _| {
                    ctx.dispatch_typed_action(CloudModeV2SlashCommandAction::HoverIdx(idx));
                    DispatchEventResult::PropagateToParent
                },
                Some(MouseInBehavior {
                    fire_on_synthetic_events: false,
                    fire_when_covered: true,
                }),
            )
            .finish()
    }

    fn render_no_results_state(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let menu_bg = inline_styles::menu_background_color(app);
        let label = if self.mixer.as_ref(app).is_loading() {
            "Loading..."
        } else {
            "No results"
        };
        Container::new(
            Text::new(
                label.to_owned(),
                appearance.ui_font_family(),
                ITEM_FONT_SIZE,
            )
            .with_color(theme.disabled_text_color(Fill::Solid(menu_bg)).into_solid())
            .finish(),
        )
        .with_horizontal_padding(MENU_HORIZONTAL_PADDING)
        .with_vertical_padding(ROW_VERTICAL_PADDING)
        .finish()
    }

    fn selected_detail_data(&self) -> Option<SearchItemDetail> {
        match &self.menu_state {
            MenuState::NoSearchActive {
                sections,
                expanded_sections,
                selected_idx,
                ..
            } => {
                let idx = (*selected_idx)?;
                let row = browsing_rows(sections, expanded_sections)
                    .get(idx)
                    .copied()?;
                match row {
                    NoSearchActiveRow::Item { section, item_idx } => {
                        let rendered = sections.iter().find(|s| s.section == section)?;
                        let renderer = rendered.items.get(item_idx)?;
                        renderer.search_result.detail_data()
                    }
                    _ => None,
                }
            }
            MenuState::SearchActive {
                results,
                selected_idx,
            } => {
                let idx = (*selected_idx)?;
                let renderer = results.get(idx)?;
                renderer.search_result.detail_data()
            }
        }
    }

    fn selected_visible_idx(&self) -> Option<usize> {
        match &self.menu_state {
            MenuState::NoSearchActive { selected_idx, .. } => *selected_idx,
            MenuState::SearchActive { selected_idx, .. } => *selected_idx,
        }
    }

    fn render_sidecar_if_eligible(&self, app: &AppContext) -> Option<(String, Box<dyn Element>)> {
        let detail = self.selected_detail_data()?;
        if !item_is_truncated_in_row(&detail, app) {
            return None;
        }
        let visible_idx = self.selected_visible_idx()?;
        Some((
            row_position_id(visible_idx),
            self.render_sidecar_panel(&detail, app),
        ))
    }

    fn render_sidecar_panel(
        &self,
        detail: &SearchItemDetail,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let menu_bg = inline_styles::menu_background_color(app);
        let primary = inline_styles::primary_text_color(theme, menu_bg.into());
        let secondary = inline_styles::secondary_text_color(theme, menu_bg.into());

        let title = Text::new_inline(
            detail.title.clone(),
            detail.title_font_family,
            inline_styles::font_size(appearance),
        )
        .with_color(primary.into())
        .finish();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(title);

        if let Some(description_text) = detail.description.clone() {
            let description = Text::new(
                description_text,
                appearance.ui_font_family(),
                SIDECAR_DESCRIPTION_FONT_SIZE,
            )
            .with_color(secondary.into())
            .finish();
            column = column.with_child(
                Container::new(description)
                    .with_margin_top(SIDECAR_TITLE_TO_DESCRIPTION_GAP)
                    .finish(),
            );
        }

        Container::new(
            ConstrainedBox::new(
                Clipped::new(
                    Container::new(column.finish())
                        .with_horizontal_padding(MENU_HORIZONTAL_PADDING)
                        .with_vertical_padding(ROW_VERTICAL_PADDING)
                        .finish(),
                )
                .finish(),
            )
            .with_max_width(SIDECAR_WIDTH)
            .with_max_height(SIDECAR_MAX_HEIGHT)
            .finish(),
        )
        .with_background(Fill::Solid(menu_bg))
        .with_border(Border::all(1.).with_border_fill(Fill::Solid(theme.outline().into_solid())))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(MENU_CORNER_RADIUS)))
        .with_padding_top(MENU_VERTICAL_PADDING)
        .with_padding_bottom(MENU_VERTICAL_PADDING)
        .with_drop_shadow(DropShadow::default())
        .finish()
    }

    fn render_menu_panel(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let menu_bg = inline_styles::menu_background_color(app);

        let content: Box<dyn Element> = match &self.menu_state {
            MenuState::NoSearchActive {
                sections,
                expanded_sections,
                selected_idx,
                show_more_mouse_states,
            } => {
                let any_items = sections.iter().any(|s| !s.items.is_empty());
                if !any_items {
                    self.render_no_results_state(app)
                } else {
                    self.render_no_search_active(
                        sections,
                        expanded_sections,
                        *selected_idx,
                        show_more_mouse_states,
                        app,
                    )
                }
            }
            MenuState::SearchActive {
                results,
                selected_idx,
            } => {
                if results.is_empty() {
                    self.render_no_results_state(app)
                } else {
                    self.render_search_active(results, *selected_idx, app)
                }
            }
        };

        let scrollable = ClippedScrollable::vertical(
            self.scroll_state.clone(),
            content,
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        let menu = Container::new(
            ConstrainedBox::new(scrollable)
                .with_max_height(MENU_MAX_HEIGHT)
                .with_max_width(MENU_WIDTH)
                .finish(),
        )
        .with_background(Fill::Solid(menu_bg))
        .with_border(Border::all(1.).with_border_fill(Fill::Solid(theme.outline().into_solid())))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(MENU_CORNER_RADIUS)))
        .with_padding_top(MENU_VERTICAL_PADDING)
        .with_padding_bottom(MENU_VERTICAL_PADDING)
        .with_drop_shadow(DropShadow::default())
        .finish();

        EventHandler::new(menu)
            .on_left_mouse_down(|_, _, _| DispatchEventResult::StopPropagation)
            .on_left_mouse_up(|_, _, _| DispatchEventResult::StopPropagation)
            .finish()
    }
}

impl Entity for CloudModeV2SlashCommandView {
    type Event = SlashCommandsEvent;
}

impl TypedActionView for CloudModeV2SlashCommandView {
    type Action = CloudModeV2SlashCommandAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CloudModeV2SlashCommandAction::Accept {
                item,
                cmd_or_ctrl_enter,
            } => {
                self.emit_selection(item, *cmd_or_ctrl_enter, ctx);
            }
            CloudModeV2SlashCommandAction::HoverIdx(idx) => {
                let idx = *idx;
                match &self.menu_state {
                    MenuState::NoSearchActive { .. } => self.set_browsing_selection(idx, ctx),
                    MenuState::SearchActive { .. } => self.set_search_selection(idx, ctx),
                }
            }
            CloudModeV2SlashCommandAction::ToggleSection(section) => {
                self.toggle_section(*section, ctx);
            }
            CloudModeV2SlashCommandAction::Dismiss => {
                self.dismiss(ctx);
            }
        }
    }
}

impl View for CloudModeV2SlashCommandView {
    fn ui_name() -> &'static str {
        "CloudModeV2SlashCommandView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let menu_panel = self.render_menu_panel(app);
        let Some((row_position_id, sidecar)) = self.render_sidecar_if_eligible(app) else {
            return menu_panel;
        };
        let mut stack = Stack::new();
        stack.add_child(menu_panel);
        stack.add_positioned_overlay_child(
            sidecar,
            OffsetPositioning::offset_from_save_position_element(
                row_position_id,
                vec2f(SIDECAR_GAP, 0.),
                PositionedElementOffsetBounds::WindowByPosition,
                PositionedElementAnchor::TopRight,
                ChildAnchor::TopLeft,
            ),
        );
        stack.finish()
    }
}

fn render_section_header(section: Section, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let menu_bg = inline_styles::menu_background_color(app);
    let header_color = theme.sub_text_color(Fill::Solid(menu_bg)).into_solid();

    Container::new(
        Text::new(
            section.header().to_owned(),
            appearance.ui_font_family(),
            SECTION_HEADER_FONT_SIZE,
        )
        .with_color(header_color)
        .finish(),
    )
    .with_horizontal_padding(MENU_HORIZONTAL_PADDING)
    .with_vertical_padding(ROW_VERTICAL_PADDING)
    .finish()
}

fn render_show_more_row(
    section: Section,
    hidden_count: usize,
    is_selected: bool,
    mouse_state: MouseStateHandle,
    _row_idx: usize,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let menu_bg = inline_styles::menu_background_color(app);
    let secondary_color = theme.sub_text_color(Fill::Solid(menu_bg)).into_solid();

    let label = format!("Show {hidden_count} more");

    let row = Hoverable::new(mouse_state, move |mouse_state| {
        let bg = if is_selected || mouse_state.is_hovered() {
            Some(theme.surface_overlay_2())
        } else {
            None
        };
        let mut container = Container::new(
            Text::new(label.clone(), appearance.ui_font_family(), ITEM_FONT_SIZE)
                .with_color(secondary_color)
                .finish(),
        )
        .with_horizontal_padding(MENU_HORIZONTAL_PADDING)
        .with_vertical_padding(ROW_VERTICAL_PADDING);
        if let Some(bg) = bg {
            container = container.with_background(bg);
        }
        container.finish()
    })
    .with_cursor(Cursor::PointingHand)
    .finish();

    EventHandler::new(row)
        .on_left_mouse_down(|_, _, _| DispatchEventResult::StopPropagation)
        .on_left_mouse_up(move |evt_ctx, _, _| {
            evt_ctx.dispatch_typed_action(CloudModeV2SlashCommandAction::ToggleSection(section));
            DispatchEventResult::StopPropagation
        })
        .finish()
}

fn render_divider(app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    Container::new(
        ConstrainedBox::new(
            Container::new(warpui::elements::Empty::new().finish())
                .with_background(theme.surface_overlay_2())
                .finish(),
        )
        .with_height(DIVIDER_HEIGHT)
        .finish(),
    )
    .with_vertical_padding(DIVIDER_VERTICAL_PADDING)
    .finish()
}
