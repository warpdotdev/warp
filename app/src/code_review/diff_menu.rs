//! Overlay menu for the code review diff selector: pinned search input and
//! a filtered list of label-only rows with a left check slot.
use std::cmp;

use fuzzy_match::{match_indices_case_insensitive, FuzzyMatchResult};
use warp_core::ui::theme::Fill;
use warp_editor::editor::NavigationKey;
use warpui::{
    color::ColorU,
    elements::{
        Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss,
        DispatchEventResult, DropShadow, Element, Empty, EventHandler, Flex, Highlight,
        MainAxisSize, MouseInBehavior, ParentElement, Radius, ScrollStateHandle, Scrollable,
        ScrollableElement, ScrollbarWidth, Text, UniformList, UniformListState,
    },
    fonts::{Properties, Weight},
    id,
    keymap::FixedBinding,
    ui_components::components::{Coords, UiComponent, UiComponentStyles},
    AppContext, Entity, FocusContext, SingletonEntity as _, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::{
    appearance::Appearance,
    code_review::{diff_selector::DiffTarget, diff_state::DiffMode},
    editor::{
        EditorOptions, EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys,
        TextOptions,
    },
    i18n::{self, I18nKey},
    ui_components::icons::Icon,
};

const MENU_WIDTH: f32 = 280.;
const MENU_MAX_LIST_HEIGHT: f32 = 200.;
const MENU_CORNER_RADIUS: f32 = 6.;
const ROW_HORIZONTAL_PADDING: f32 = 14.;
const ROW_VERTICAL_PADDING: f32 = 5.;
const LIST_BOTTOM_PADDING: f32 = 9.;
const SEARCH_INPUT_HORIZONTAL_PADDING: f32 = 8.;
const CHECK_GAP: f32 = 8.;
const CHECK_SLOT_SIZE: f32 = 12.;

#[derive(Clone, Debug)]
pub enum CodeReviewDiffMenuEvent {
    Select(DiffMode),
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CodeReviewDiffMenuAction {
    ClickRow { index: usize },
    HoverRow { index: usize },
    SelectUp,
    SelectDown,
    SelectEnter,
    Close,
}

pub fn init(app: &mut AppContext) {
    app.register_fixed_bindings([
        FixedBinding::new(
            "up",
            CodeReviewDiffMenuAction::SelectUp,
            id!(CodeReviewDiffMenu::ui_name()),
        ),
        FixedBinding::new(
            "down",
            CodeReviewDiffMenuAction::SelectDown,
            id!(CodeReviewDiffMenu::ui_name()),
        ),
        FixedBinding::new(
            "enter",
            CodeReviewDiffMenuAction::SelectEnter,
            id!(CodeReviewDiffMenu::ui_name()),
        ),
        FixedBinding::new(
            "escape",
            CodeReviewDiffMenuAction::Close,
            id!(CodeReviewDiffMenu::ui_name()),
        ),
    ]);
}

pub struct CodeReviewDiffMenu {
    targets: Vec<DiffTarget>,
    /// (target index, optional match result) pairs for rows that pass the
    /// current filter. Original target order is preserved; the match result
    /// carries indices for bolding matched characters in the label.
    filtered: Vec<(usize, Option<FuzzyMatchResult>)>,
    /// Index into `filtered` of the keyboard-focused row.
    selected_index: Option<usize>,
    search_input: ViewHandle<EditorView>,
    search_query: String,
    list_state: UniformListState,
    scroll_state: ScrollStateHandle,
}

impl CodeReviewDiffMenu {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let search_input = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::handle(ctx).as_ref(ctx);
            let ui_font_family = appearance.ui_font_family();

            let mut text_options = TextOptions::ui_font_size(appearance);
            text_options.font_family_override = Some(ui_font_family);

            let options = EditorOptions {
                autogrow: false,
                soft_wrap: false,
                single_line: true,
                text: text_options,
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::new(options, ctx);
            editor.set_placeholder_text(i18n::tr(ctx, I18nKey::CodeSearchDiffSetsPlaceholder), ctx);
            editor
        });

        ctx.subscribe_to_view(&search_input, |menu, _, event, ctx| match event {
            EditorEvent::Edited(_) => {
                let new_query = menu
                    .search_input
                    .read(ctx, |editor, ctx| editor.buffer_text(ctx).to_string());
                if new_query != menu.search_query {
                    menu.set_search_query(new_query, ctx);
                }
            }
            EditorEvent::Escape => menu.emit_close(ctx),
            EditorEvent::Navigate(NavigationKey::Up) => menu.select_prev(ctx),
            EditorEvent::Navigate(NavigationKey::Down) => menu.select_next(ctx),
            EditorEvent::Enter => menu.select_enter(ctx),
            _ => {}
        });

        Self {
            targets: Vec::new(),
            filtered: Vec::new(),
            selected_index: None,
            search_input,
            search_query: String::new(),
            list_state: Default::default(),
            scroll_state: Default::default(),
        }
    }

    /// Replace the row set and reset filter/scroll to the top.
    pub fn set_targets(&mut self, targets: Vec<DiffTarget>, ctx: &mut ViewContext<Self>) {
        self.targets = targets;
        self.refresh_filtered();
        self.scroll_list_to_top();
        ctx.notify();
    }

    /// Reset the menu to a fresh state: empty query, selection/scroll back
    /// to the top, and rows re-computed against the current targets.
    pub fn reset(&mut self, ctx: &mut ViewContext<Self>) {
        self.search_query.clear();
        self.search_input.update(ctx, |editor, ctx| {
            editor.clear_buffer(ctx);
        });
        self.refresh_filtered();
        self.scroll_list_to_top();
        ctx.notify();
    }

    fn set_search_query(&mut self, query: String, ctx: &mut ViewContext<Self>) {
        self.search_query = query;
        self.refresh_filtered();
        self.scroll_list_to_top();
        ctx.notify();
    }

    fn scroll_list_to_top(&mut self) {
        if !self.filtered.is_empty() {
            self.list_state.scroll_to(0);
        }
    }

    fn refresh_filtered(&mut self) {
        if self.search_query.is_empty() {
            self.filtered = (0..self.targets.len()).map(|i| (i, None)).collect();
        } else {
            // Use fuzzy matching for membership only; preserve original order
            // so the list never reorders under the user as they type.
            self.filtered = self
                .targets
                .iter()
                .enumerate()
                .filter_map(|(i, target)| {
                    match_indices_case_insensitive(&target.label, &self.search_query)
                        .map(|m| (i, Some(m)))
                })
                .collect();
        }
        self.selected_index = if self.filtered.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    fn select_prev(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(current) = self.selected_index else {
            return;
        };
        if current == 0 {
            return;
        }
        self.selected_index = Some(current - 1);
        self.list_state.scroll_to(current - 1);
        ctx.notify();
    }

    fn select_next(&mut self, ctx: &mut ViewContext<Self>) {
        if self.filtered.is_empty() {
            return;
        }
        let next = match self.selected_index {
            Some(i) if i + 1 < self.filtered.len() => i + 1,
            Some(i) => i,
            None => 0,
        };
        self.selected_index = Some(next);
        self.list_state.scroll_to(next);
        ctx.notify();
    }

    fn select_enter(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(selected) = self.selected_index else {
            return;
        };
        self.select_filtered_index(selected, ctx);
    }

    fn select_filtered_index(&mut self, filtered_index: usize, ctx: &mut ViewContext<Self>) {
        let Some((target_index, _)) = self.filtered.get(filtered_index).cloned() else {
            return;
        };
        let Some(target) = self.targets.get(target_index) else {
            return;
        };
        let mode = target.mode.clone();
        ctx.emit(CodeReviewDiffMenuEvent::Select(mode));
        ctx.notify();
    }

    fn emit_close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(CodeReviewDiffMenuEvent::Close);
        ctx.notify();
    }

    fn render_search_input(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let search_input = appearance
            .ui_builder()
            .text_input(self.search_input.clone())
            .with_style(UiComponentStyles {
                background: Some(Fill::Solid(ColorU::new(0, 0, 0, 0)).into()),
                border_color: None,
                border_width: Some(0.),
                border_radius: None,
                width: Some(MENU_WIDTH - (SEARCH_INPUT_HORIZONTAL_PADDING * 2.)),
                padding: Some(Coords::uniform(4.)),
                ..Default::default()
            })
            .build()
            .finish();

        // Search bar inherits the menu card surface; a hairline bottom
        // border separates it from the row list below.
        Container::new(search_input)
            .with_horizontal_padding(SEARCH_INPUT_HORIZONTAL_PADDING)
            .with_vertical_padding(2.)
            .with_border(Border::bottom(1.0).with_border_color(theme.outline().into()))
            .finish()
    }

    fn render_empty_state(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Container::new(
            Text::new(
                "No matches",
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(theme.sub_text_color(theme.surface_2()).into_solid())
            .finish(),
        )
        .with_horizontal_padding(ROW_HORIZONTAL_PADDING)
        .with_vertical_padding(ROW_VERTICAL_PADDING * 2.0)
        .finish()
    }

    fn render_rows(&self, ctx: &AppContext) -> Box<dyn Element> {
        if self.filtered.is_empty() {
            if !self.search_query.is_empty() {
                return self.render_empty_state(Appearance::as_ref(ctx));
            }
            return Empty::new().finish();
        }

        let appearance = Appearance::as_ref(ctx);
        let theme = appearance.theme();
        let selected = self.selected_index;
        let filtered_snapshot: Vec<(DiffTarget, Option<FuzzyMatchResult>)> = self
            .filtered
            .iter()
            .filter_map(|(i, m)| self.targets.get(*i).cloned().map(|t| (t, m.clone())))
            .collect();
        let filtered_len = filtered_snapshot.len();

        let list = UniformList::new(
            self.list_state.clone(),
            filtered_len,
            move |mut range, app| {
                let appearance = Appearance::as_ref(app);
                let theme = appearance.theme();
                range.end = cmp::min(range.end, filtered_len);
                range
                    .map(|row_index| {
                        let (target, match_result) = &filtered_snapshot[row_index];
                        let is_focused = selected == Some(row_index);
                        let font_size = appearance.ui_font_size();

                        let (text_color, bg) = if is_focused {
                            let bg = theme.accent();
                            (theme.main_text_color(bg).into_solid(), Some(bg))
                        } else {
                            (theme.main_text_color(theme.surface_2()).into_solid(), None)
                        };

                        let check_slot: Box<dyn Element> = if target.is_selected {
                            ConstrainedBox::new(
                                Icon::Check.to_warpui_icon(Fill::Solid(text_color)).finish(),
                            )
                            .with_width(CHECK_SLOT_SIZE)
                            .with_height(CHECK_SLOT_SIZE)
                            .finish()
                        } else {
                            ConstrainedBox::new(Empty::new().finish())
                                .with_width(CHECK_SLOT_SIZE)
                                .with_height(CHECK_SLOT_SIZE)
                                .finish()
                        };

                        let label_text = Text::new_inline(
                            target.label.clone(),
                            appearance.ui_font_family(),
                            font_size,
                        )
                        .with_color(text_color);
                        let label = match match_result {
                            Some(m) => label_text.with_single_highlight(
                                Highlight::new()
                                    .with_properties(Properties::default().weight(Weight::Bold))
                                    .with_foreground_color(text_color),
                                m.matched_indices.clone(),
                            ),
                            None => label_text,
                        }
                        .finish();

                        let row = Flex::row()
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_child(
                                Container::new(check_slot)
                                    .with_margin_right(CHECK_GAP)
                                    .finish(),
                            )
                            .with_child(label)
                            .finish();

                        let mut container = Container::new(row)
                            .with_horizontal_padding(ROW_HORIZONTAL_PADDING)
                            .with_vertical_padding(ROW_VERTICAL_PADDING);

                        if let Some(bg) = bg {
                            container = container.with_background(bg);
                        }

                        EventHandler::new(container.finish())
                            .on_left_mouse_down(move |ctx, _, _| {
                                ctx.dispatch_typed_action(CodeReviewDiffMenuAction::ClickRow {
                                    index: row_index,
                                });
                                DispatchEventResult::StopPropagation
                            })
                            .on_mouse_in(
                                move |ctx, _, _| {
                                    ctx.dispatch_typed_action(CodeReviewDiffMenuAction::HoverRow {
                                        index: row_index,
                                    });
                                    ctx.notify();
                                    DispatchEventResult::StopPropagation
                                },
                                Some(MouseInBehavior {
                                    fire_on_synthetic_events: false,
                                    fire_when_covered: false,
                                }),
                            )
                            .finish()
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
            },
        );

        let scrollable = Scrollable::vertical(
            self.scroll_state.clone(),
            list.finish_scrollable(),
            ScrollbarWidth::None,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .with_padding_end(0.)
        .with_padding_start(0.);

        ConstrainedBox::new(scrollable.finish())
            .with_width(MENU_WIDTH)
            .with_max_height(MENU_MAX_LIST_HEIGHT)
            .finish()
    }
}

impl Entity for CodeReviewDiffMenu {
    type Event = CodeReviewDiffMenuEvent;
}

impl TypedActionView for CodeReviewDiffMenu {
    type Action = CodeReviewDiffMenuAction;

    fn handle_action(&mut self, action: &CodeReviewDiffMenuAction, ctx: &mut ViewContext<Self>) {
        match action {
            CodeReviewDiffMenuAction::ClickRow { index } => {
                // Sync keyboard selection with the clicked row.
                if *index < self.filtered.len() {
                    self.selected_index = Some(*index);
                    self.select_filtered_index(*index, ctx);
                }
            }
            CodeReviewDiffMenuAction::HoverRow { index } => {
                if *index < self.filtered.len() && self.selected_index != Some(*index) {
                    self.selected_index = Some(*index);
                    ctx.notify();
                }
            }
            CodeReviewDiffMenuAction::SelectUp => self.select_prev(ctx),
            CodeReviewDiffMenuAction::SelectDown => self.select_next(ctx),
            CodeReviewDiffMenuAction::SelectEnter => self.select_enter(ctx),
            CodeReviewDiffMenuAction::Close => self.emit_close(ctx),
        }
    }
}

impl View for CodeReviewDiffMenu {
    fn ui_name() -> &'static str {
        "CodeReviewDiffMenu"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.search_input);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut column = Flex::column().with_child(self.render_search_input(appearance));
        if !self.targets.is_empty() || !self.search_query.is_empty() {
            column.add_child(
                Container::new(self.render_rows(app))
                    .with_padding_bottom(LIST_BOTTOM_PADDING)
                    .finish(),
            );
        }

        let menu_card = ConstrainedBox::new(
            Container::new(column.finish())
                .with_background(theme.surface_2())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(MENU_CORNER_RADIUS)))
                .with_drop_shadow(DropShadow::default())
                .finish(),
        )
        .with_width(MENU_WIDTH)
        .finish();

        Dismiss::new(menu_card)
            .on_dismiss(|ctx, _app| {
                ctx.dispatch_typed_action(CodeReviewDiffMenuAction::Close);
            })
            .prevent_interaction_with_other_elements()
            .finish()
    }
}
