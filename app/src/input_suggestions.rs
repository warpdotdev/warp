use crate::ai::blocklist::{render_ai_agent_mode_icon, AIQueryHistory, AIQueryHistoryOutputStatus};
use crate::terminal::model::session::SessionId;
use crate::ui_components::icons::Icon as UIComponentsIcon;
use async_channel::Sender;
use chrono::{DateTime, Local};
use fuzzy_match::match_indices;
use itertools::Itertools;
use pathfinder_geometry::vector::vec2f;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::{cmp, ops::Range, vec};
use warp_command_signatures::IconType;
use warp_completer::completer::{
    MatchType, PathSeparators, Suggestion, SuggestionResults, SuggestionType,
};
use warp_core::features::FeatureFlag;
use warp_core::ui::theme::AnsiColorIdentifier;
use warpui::elements::{
    ChildAnchor, DispatchEventResult, Expanded, Hoverable, MouseStateHandle, ParentAnchor,
    ParentOffsetBounds, ScrollbarWidth,
};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    accessibility::{AccessibilityContent, WarpA11yRole},
    elements::{
        Align, AnchorPair, Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
        DropShadow, Element, Empty, EventHandler, Flex, Highlight, Icon, OffsetPositioning,
        OffsetType, ParentElement, PositionedElementOffsetBounds, PositioningAxis, Radius,
        SavePosition, ScrollStateHandle, Scrollable, ScrollableElement, Shrinkable,
        SizeConstraintCondition, SizeConstraintSwitch, Stack, Text, UniformList, UniformListState,
        XAxisAnchor, YAxisAnchor,
    },
    fonts::{Cache, Properties, Weight},
    AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, WeakViewHandle,
};

use crate::appearance::Appearance;
use crate::terminal::history::LinkedWorkflowData;
use crate::terminal::rich_history::{render_ai_query_rich_history, render_rich_history};
use crate::terminal::HistoryEntry;
use crate::util::time_format::format_approx_duration_from_now;

/// This enum allows the parent view to indicate which type of details panel is shown.
#[derive(Clone, Debug)]
pub enum DetailContent {
    /// The details panel for a rich history item. Boxed to prevent a size imbalance between
    /// variants.
    RichHistory(Box<HistoryEntry>),
    /// A details panel for a simple string.
    Description(String),
    AIQueryHistory(Box<AIQueryHistoryEntryDetails>),
}

impl From<HistoryEntry> for DetailContent {
    fn from(entry: HistoryEntry) -> Self {
        DetailContent::RichHistory(Box::new(entry))
    }
}

impl From<String> for DetailContent {
    fn from(description: String) -> Self {
        DetailContent::Description(description)
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    /// Underlying text to be sent back to parent. This is also used as the replacement text.
    text: String,
    // Text to be displayed in the UI. Defaults to displaying text if None.
    display: Option<String>,
    /// Contents of the details panel.
    details: Option<DetailContent>,
    /// Vector of byte indexes to be highlighted in the UI.
    matches: Option<Vec<usize>>,
    /// The icon to show to the left of the item text.
    icon_type: Option<ItemIconType>,
    /// How precisely this items matches the search term.
    match_type: MatchType,
    /// True if this Item represents a query sent to an AI model.
    is_ai_query: bool,
    /// True if this Item represents a history item.
    is_history_item: bool,
}

impl Item {
    #[cfg(test)]
    pub fn from_text(text: String) -> Self {
        Self {
            text,
            display: None,
            details: None,
            matches: None,
            icon_type: None,
            match_type: MatchType::Prefix {
                is_case_sensitive: true,
            },
            is_ai_query: false,
            is_history_item: false,
        }
    }

    #[cfg(test)]
    pub fn matches(&self) -> Option<&Vec<usize>> {
        self.matches.as_ref()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn match_type(&self) -> MatchType {
        self.match_type
    }

    /// Returns LinkedWorkflowData for this `Item`, if the `Item` is a history command that was
    /// created using a workflow.
    pub fn linked_workflow_data(&self) -> Option<LinkedWorkflowData> {
        match self.details.as_ref() {
            Some(DetailContent::RichHistory(history_entry)) => history_entry.linked_workflow_data(),
            _ => None,
        }
    }

    pub fn is_ai_query(&self) -> bool {
        self.is_ai_query
    }
}

#[derive(Clone, Debug)]
pub enum ItemIconType {
    Option,
    Argument,
    SubCommand,
    File,
    Folder,
    GitBranch,
    AIQuery,
}

impl ItemIconType {
    pub fn icon_path(&self) -> &'static str {
        match self {
            ItemIconType::SubCommand => SUBCOMMAND_ICON_PATH,
            ItemIconType::Option => OPTION_ICON_PATH,
            ItemIconType::Argument => ARGUMENT_ICON_PATH,
            ItemIconType::File => FILE_ICON_PATH,
            ItemIconType::Folder => FOLDER_ICON_PATH,
            ItemIconType::GitBranch => GIT_BRANCH_ICON_PATH,
            ItemIconType::AIQuery => UIComponentsIcon::AgentMode.into(),
        }
    }

    // The multiplication factor we need to apply on font size to get icon width.
    pub fn width_font_size_multiplication_factor(&self) -> f32 {
        match self {
            ItemIconType::File | ItemIconType::Folder => 1.05,
            _ => 1.2,
        }
    }

    pub fn right_padding(&self) -> f32 {
        match self {
            ItemIconType::File | ItemIconType::Folder => FILE_FOLDER_ICON_PADDING_RIGHT,
            _ => ICON_PADDING_RIGHT,
        }
    }

    pub fn left_padding(&self) -> f32 {
        match self {
            // We want to center the file and folder icon.
            ItemIconType::File | ItemIconType::Folder => {
                FILE_FOLDER_ICON_PADDING_RIGHT - ICON_PADDING_RIGHT
            }
            _ => 0.,
        }
    }
}

fn icon_type(suggestion: &Suggestion) -> ItemIconType {
    suggestion
        .override_icon
        .and_then(|icon_type| match icon_type {
            IconType::File => Some(ItemIconType::File),
            IconType::Folder => Some(ItemIconType::Folder),
            IconType::GitBranch => Some(ItemIconType::GitBranch),
            _ => None,
        })
        .unwrap_or(match suggestion.suggestion_type {
            SuggestionType::Command(_) | SuggestionType::Subcommand => ItemIconType::SubCommand,
            SuggestionType::Option(..) => ItemIconType::Option,
            SuggestionType::Argument | SuggestionType::Variable => ItemIconType::Argument,
        })
}

pub struct InputSuggestions {
    handle: WeakViewHandle<Self>,
    items: Vec<Item>,
    list_state: UniformListState,
    scroll_state: ScrollStateHandle,
    selected_index: Option<usize>,
    /// Which characters to use as path separators. This can deviate from
    /// [`std::path::MAIN_SEPARATOR`], e.g. for a WSL session on Windows.
    path_separators: PathSeparators,
    /// Flag for whether we want to cycle through the items or not.
    cycle: bool,

    /// The range of the items that are currently visible.
    visible_items: Option<Range<usize>>,

    visible_items_tx: Sender<Range<usize>>,

    /// Mouse state handles for X buttons, one per history item
    ignore_button_handles: Vec<MouseStateHandle>,

    /// Mouse state handles for row hover state, one per item
    row_mouse_state_handles: Vec<MouseStateHandle>,
}

pub enum Event {
    ConfirmAndExecuteSuggestion {
        suggestion: String,
        match_type: MatchType,
    },
    ConfirmSuggestion {
        suggestion: String,
        match_type: MatchType,
    },
    CloseSuggestion {
        should_restore_buffer_before_history_up: bool,
    },
    Select(Item),
    IgnoreItem {
        item: Item,
    },
}

const SUGGESTIONS_LIST_POSITION_ID: &str = "InputSuggestionsList";

pub const LABEL_PADDING: f32 = 6.;
pub const DESCRIPTION_PADDING: f32 = 10.;
const SCROLLBAR_WIDTH: ScrollbarWidth = ScrollbarWidth::Auto;

const ICON_PADDING_RIGHT: f32 = 8.;
const FILE_FOLDER_ICON_PADDING_RIGHT: f32 = 10.5;

const ARGUMENT_ICON_PATH: &str = "bundled/svg/completion-argument.svg";
const SUBCOMMAND_ICON_PATH: &str = "bundled/svg/completion-subcommand.svg";
const OPTION_ICON_PATH: &str = "bundled/svg/completion-flag.svg";
const FILE_ICON_PATH: &str = "bundled/svg/completion-file.svg";
const FOLDER_ICON_PATH: &str = "bundled/svg/completion-folder.svg";
const GIT_BRANCH_ICON_PATH: &str = "bundled/svg/completion-gitbranch.svg";

const DETAILS_MIN_WIDTH: f32 = 180.;
const DESCRIPTION_PANEL_WIDTH: f32 = 300.;
pub const HISTORY_DETAILS_PANEL_WIDTH: f32 = 256.;
pub const DETAILS_PANEL_PADDING: f32 = 12.;
pub const DETAILS_PANEL_MARGIN: f32 = 4.;

#[derive(Copy, Clone, Debug)]
pub enum SelectAction {
    Prev,
    Next,
    Index(usize),
}

#[derive(Debug)]
pub enum InputSuggestionsAction {
    SelectAndConfirm(usize),
    IgnoreItem(usize),
}

fn filter_tab_suggestions(
    suggestions: &SuggestionResults,
    query: &str,
    path_separators: &[char],
) -> Vec<Item> {
    suggestions
        .filter_by_query(query, path_separators)
        .map(|suggestion| Item {
            // TODO(vorporeal): Consider changing the type of `text` and `display` here to be `SmolStr`.
            text: suggestion.suggestion.replacement.to_string(),
            display: Some(suggestion.suggestion.display.to_string()),
            details: suggestion
                .suggestion
                .description
                .as_ref()
                .map(|desc| desc.clone().into()),
            matches: Some(suggestion.matching_indices),
            icon_type: Some(icon_type(suggestion.suggestion)),
            match_type: suggestion.match_type,
            is_ai_query: false,
            is_history_item: false,
        })
        .collect::<Vec<_>>()
}

/// Controls which (if any) item should be pre-selected
/// when setting tab completion results.
pub enum TabCompletionsPreselectOption {
    /// The first suggestion should be pre-selected.
    First,
    /// No selections should be selected.
    Unselected,
    /// Don't change the existing selection.
    Unchanged,
}

impl InputSuggestions {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let (visible_items_tx, visible_items_rx) = async_channel::unbounded();

        let _ = ctx.spawn_stream_local(visible_items_rx, Self::on_visible_items, |_, _| {});

        Self {
            handle: ctx.handle(),
            items: vec![],
            list_state: UniformListState::new(),
            scroll_state: Default::default(),
            selected_index: None,
            // Before bootstrap, we don't know what separators to use for sure. Start with the
            // platform default.
            path_separators: PathSeparators::for_os(),
            cycle: false,
            visible_items: None,
            visible_items_tx,
            ignore_button_handles: vec![],
            row_mouse_state_handles: vec![],
        }
    }

    fn on_visible_items(&mut self, new_visible_items: Range<usize>, ctx: &mut ViewContext<Self>) {
        if let Some(visible_items) = &self.visible_items {
            if visible_items == &new_visible_items {
                return;
            }
        }

        self.visible_items = Some(new_visible_items);
        ctx.notify();
    }

    pub fn position_id_at_index(index: usize) -> String {
        format!("input_suggestions:index_{index}")
    }

    pub fn is_empty(&self) -> bool {
        self.items.len() == 0
    }

    #[cfg(test)]
    pub fn item_text(&self, index: usize) -> &String {
        &self.items[index].text
    }

    pub fn items(&self) -> &Vec<Item> {
        &self.items
    }

    /// Filters down the set of options based on the given query,
    /// and preselects an item in the menu based on the preselect option.
    pub fn prefix_search_for_tab_completion(
        &mut self,
        query: &str,
        options: &SuggestionResults,
        preselect_option: TabCompletionsPreselectOption,
        ctx: &mut ViewContext<Self>,
    ) {
        let results = filter_tab_suggestions(options, query, self.path_separators.all);
        self.set_items(results);

        if self.items.is_empty() {
            return;
        }

        match preselect_option {
            TabCompletionsPreselectOption::First => self.select_first_item(ctx),
            TabCompletionsPreselectOption::Unselected => self.selected_index = None,
            TabCompletionsPreselectOption::Unchanged => {}
        }

        self.cycle = true;
        ctx.notify();
    }

    pub fn fuzzy_substring_search(
        &mut self,
        query: String,
        options: Vec<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        let trimmed_query = query.trim();

        self.set_items(
            options
                .into_iter()
                .filter_map(|text| {
                    let trimmed_text = text.trim();
                    match_indices(trimmed_text, trimmed_query).map(|result| {
                        (
                            result.score,
                            Item {
                                text: trimmed_text.to_string(),
                                display: None,
                                details: None,
                                matches: Some(result.matched_indices),
                                icon_type: None,
                                match_type: MatchType::Fuzzy,
                                is_ai_query: false,
                                is_history_item: false,
                            },
                        )
                    })
                })
                .sorted_by(|(score1, _), (score2, _)| score1.cmp(score2))
                .map(|(_, item)| item)
                .collect(),
        );

        self.select_last_item(ctx);
        self.cycle = false;
        ctx.notify();
    }

    /// Given a list of matched items, set the items and ensure the first one is selected.
    pub fn set_enum_variants(&mut self, variants: Vec<String>, ctx: &mut ViewContext<Self>) {
        let items = variants
            .iter()
            .map(|text| Item {
                text: text.clone(),
                display: None,
                details: None,
                matches: None,
                icon_type: None,
                match_type: MatchType::Other,
                is_ai_query: false,
                is_history_item: false,
            })
            .collect();

        self.set_items(items);

        // Select the first item with side effects of notifying
        // view context, to ensure the buffer gets populated
        self.select_first_item(ctx);
        self.cycle = true;
        ctx.notify();
    }

    /// Filters down the set of options to those that have the given prefix. If prefix is only
    /// whitespace, then the input suggestions are simply all the options.
    pub(crate) fn history_prefix_search<'a, I: IntoIterator<Item = HistoryInputSuggestion<'a>>>(
        prefix: &str,
        options: I,
    ) -> Vec<Item> {
        let trimmed_prefix = prefix.trim();
        options
            .into_iter()
            .filter_map(|entry| {
                if entry.text().starts_with(trimmed_prefix) {
                    Some(Item {
                        text: entry.text().trim().to_string(),
                        display: None,
                        details: entry.details(),
                        matches: Some((0..trimmed_prefix.len()).collect()),
                        icon_type: entry.icon_type(),
                        match_type: MatchType::Prefix {
                            is_case_sensitive: true,
                        },
                        is_ai_query: entry.is_ai_query(),
                        is_history_item: true,
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    /// Given a filtered list of matched items, set the items and ensure the last one is selected.
    pub fn set_history_matches(&mut self, matches: Vec<Item>, ctx: &mut ViewContext<Self>) {
        self.set_items(matches);

        // Select the last item with side effects of notifying
        // view context, to ensure the buffer gets populated
        self.select_last_item(ctx);
        self.cycle = false;
        ctx.notify();
    }

    /// Sets the items within the suggestions list
    pub fn set_items(&mut self, items: Vec<Item>) {
        // Reset state after items have been replaced.
        self.list_state = UniformListState::default();
        self.items = items;

        self.ignore_button_handles = (0..self.items.len())
            .map(|_| MouseStateHandle::new(Default::default()))
            .collect();

        self.row_mouse_state_handles = (0..self.items.len())
            .map(|_| MouseStateHandle::new(Default::default()))
            .collect();
    }

    fn select_first_item(&mut self, ctx: &mut ViewContext<Self>) {
        self.select(&SelectAction::Index(0), ctx);
    }

    fn select_last_item(&mut self, ctx: &mut ViewContext<Self>) {
        self.select(
            &SelectAction::Index(self.items.len().saturating_sub(1)),
            ctx,
        );
    }

    pub fn select_prev(&mut self, ctx: &mut ViewContext<Self>) {
        self.select(&SelectAction::Prev, ctx);
    }

    pub fn select_next(&mut self, ctx: &mut ViewContext<Self>) {
        self.select(&SelectAction::Next, ctx);
    }

    fn get_item(&self, index: usize) -> Option<&Item> {
        self.items.get(index)
    }

    pub fn get_selected_item(&self) -> Option<&Item> {
        self.get_item(self.selected_index?)
    }

    pub fn get_selected_item_text(&self) -> Option<&str> {
        self.get_selected_item().map(|item| item.text.as_str())
    }

    fn get_selected_item_a11y_description(&self) -> Option<String> {
        self.get_selected_item()
            .and_then(|item| item.details.as_ref())
            .and_then(|details| match details {
                DetailContent::RichHistory(entry) => entry
                    .start_ts
                    .map(|ts| format!("Last ran {}", format_approx_duration_from_now(ts))),
                DetailContent::Description(desc) => Some(desc.clone()),
                DetailContent::AIQueryHistory(entry) => Some(format!(
                    "Last ran {}",
                    format_approx_duration_from_now(entry.start_time)
                )),
            })
    }

    pub fn select(&mut self, action: &SelectAction, ctx: &mut ViewContext<Self>) {
        if self.items.is_empty() {
            return;
        }

        let new_selected_index = match (self.selected_index, action) {
            (Some(selected_index), SelectAction::Next) => {
                if self.cycle {
                    (selected_index + 1) % self.items.len()
                } else if selected_index + 1 >= self.items.len() {
                    // If user hits down at the bottom of the history menu, it should be closed.
                    self.exit(true, ctx);
                    return;
                } else {
                    selected_index + 1
                }
            }
            (Some(selected_index), SelectAction::Prev) => {
                if self.cycle {
                    (selected_index + self.items.len().saturating_sub(1)) % self.items.len()
                } else {
                    selected_index.saturating_sub(1)
                }
            }
            (None, SelectAction::Next | SelectAction::Prev) => 0,
            (_, SelectAction::Index(index)) => *index,
        };
        self.selected_index = Some(new_selected_index);

        self.list_state.scroll_to(new_selected_index);

        if let Some(item) = self.items.get(new_selected_index) {
            ctx.emit(Event::Select(item.clone()));
        }
        match (
            self.get_selected_item_text(),
            self.get_selected_item_a11y_description(),
        ) {
            (Some(text), Some(desc)) => {
                ctx.emit_a11y_content(AccessibilityContent::new(
                    format!("Suggestion: {text}.\n"),
                    desc,
                    WarpA11yRole::MenuItemRole,
                ));
            }
            (Some(text), None) => {
                ctx.emit_a11y_content(AccessibilityContent::new_without_help(
                    format!("Suggestion: {text}.\n"),
                    WarpA11yRole::MenuItemRole,
                ));
            }
            _ => {}
        }
        ctx.notify();
    }

    pub fn confirm(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(item) = self.get_selected_item() {
            ctx.emit(Event::ConfirmSuggestion {
                suggestion: item.text.to_owned(),
                match_type: item.match_type,
            });
        } else {
            ctx.emit(Event::CloseSuggestion {
                should_restore_buffer_before_history_up: true,
            });
            return;
        }

        if let Some(text) = self.get_selected_item_text() {
            ctx.emit_a11y_content(AccessibilityContent::new_without_help(
                format!("Selected: {text}"),
                WarpA11yRole::MenuItemRole,
            ));
        }
    }

    pub fn confirm_and_execute(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(item) = self.get_selected_item() {
            ctx.emit(Event::ConfirmAndExecuteSuggestion {
                suggestion: item.text.to_owned(),
                match_type: item.match_type,
            });
        } else {
            ctx.emit(Event::CloseSuggestion {
                should_restore_buffer_before_history_up: true,
            });
        }
    }

    pub fn exit(
        &mut self,
        should_restore_buffer_before_history_up: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit_a11y_content(AccessibilityContent::new_without_help(
            "Closed suggestions.",
            WarpA11yRole::UserAction,
        ));
        ctx.emit(Event::CloseSuggestion {
            should_restore_buffer_before_history_up,
        });
    }

    pub fn em_width(&self, font_cache: &Cache, appearance: &Appearance) -> f32 {
        font_cache.em_width(
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
    }

    /// Renders the details of the item at index, if it is visible and has details.
    fn render_visible_item_details(
        &self,
        index: usize,
        appearance: &Appearance,
        ctx: &AppContext,
    ) -> Option<Box<dyn Element>> {
        if !self.visible_items.as_ref()?.contains(&index) {
            return None;
        }

        let item = self.get_item(index)?;
        let details = match item.details.as_ref()? {
            DetailContent::RichHistory(entry) => {
                ConstrainedBox::new(render_rich_history(entry, ctx))
                    .with_max_width(HISTORY_DETAILS_PANEL_WIDTH)
                    .finish()
            }
            DetailContent::Description(description) => {
                self.render_descriptions_box(item.text.clone(), description.clone(), appearance)
            }
            DetailContent::AIQueryHistory(entry) => {
                ConstrainedBox::new(render_ai_query_rich_history(entry, ctx))
                    .with_max_width(HISTORY_DETAILS_PANEL_WIDTH)
                    .finish()
            }
        };

        Some(details)
    }

    fn render_items(&self, ctx: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(ctx);
        let theme = appearance.theme();
        if self.items.is_empty() {
            // Render the "no suggestions"
            return ConstrainedBox::new(
                Container::new(
                    Align::new(
                        Container::new(
                            Text::new_inline(
                                String::from("No suggestions"),
                                appearance.monospace_font_family(),
                                appearance.monospace_font_size(),
                            )
                            .with_style(
                                Properties::default().weight(appearance.monospace_font_weight()),
                            )
                            .with_color(theme.sub_text_color(theme.surface_2()).into())
                            .finish(),
                        )
                        .with_uniform_padding(5.)
                        .finish(),
                    )
                    .top_center()
                    .finish(),
                )
                .finish(),
            )
            .with_max_height(appearance.monospace_font_size() + 15.)
            .finish();
        }
        let handle = self.handle.clone();
        let em_width = self.em_width(ctx.font_cache(), appearance);

        let list = UniformList::new(
            self.list_state.clone(),
            self.items.len(),
            move |mut range, app| {
                let input_suggestions = handle.upgrade(app).unwrap().as_ref(app);
                let appearance = Appearance::as_ref(app);
                let theme = appearance.theme();

                range.end = cmp::min(range.end, input_suggestions.items.len());
                range
                    .map(|index| {
                        let item = &input_suggestions.items[index];
                        let display_text_str = if let Some(display) = &item.display {
                            display
                        } else {
                            &item.text
                        };
                        let display_text_str = String::from(display_text_str);

                        let is_selected = Some(index) == input_suggestions.selected_index;
                        let background_color = if is_selected {
                            theme.accent()
                        } else {
                            theme.surface_2()
                        };
                        let highlight_text = theme.main_text_color(background_color).into_solid();
                        let main_text = theme.sub_text_color(background_color).into_solid();
                        let description_text = theme.hint_text_color(background_color).into_solid();
                        let font_size = appearance.monospace_font_size() - 1.;

                        let row_mouse_state =
                            input_suggestions.row_mouse_state_handles[index].clone();

                        let row_element = Hoverable::new(row_mouse_state.clone(), |mouse_state| {
                            let mut row =
                                Flex::row().with_cross_axis_alignment(CrossAxisAlignment::End);

                            if let Some(icon_type) = item.icon_type.as_ref() {
                                let icon_container = if let ItemIconType::AIQuery = icon_type {
                                    Container::new(render_ai_agent_mode_icon(
                                        app,
                                        if is_selected {
                                            theme.background()
                                        } else {
                                            AnsiColorIdentifier::Yellow
                                                .to_ansi_color(&theme.terminal_colors().normal)
                                                .into()
                                        },
                                    ))
                                    .with_padding_right(6. * (em_width / 6.))
                                    .with_padding_left(icon_type.left_padding())
                                    .finish()
                                } else {
                                    let icon_width = font_size
                                        * icon_type.width_font_size_multiplication_factor();
                                    Container::new(
                                        ConstrainedBox::new(
                                            Icon::new(
                                                icon_type.icon_path(),
                                                theme.main_text_color(background_color),
                                            )
                                            .finish(),
                                        )
                                        .with_width(icon_width)
                                        .with_height(icon_width)
                                        .finish(),
                                    )
                                    .with_padding_left(icon_type.left_padding())
                                    .with_padding_right(icon_type.right_padding())
                                    .finish()
                                };

                                row.add_child(icon_container);
                            }

                            row.add_child({
                                let mut display_text = Text::new_inline(
                                    display_text_str,
                                    appearance.monospace_font_family(),
                                    font_size,
                                )
                                .with_style(
                                    Properties::default()
                                        .weight(appearance.monospace_font_weight()),
                                )
                                .autosize_text(warp_core::ui::builder::MIN_FONT_SIZE)
                                .with_color(main_text);

                                let matches = item.matches.clone();
                                if let Some(matches) = matches {
                                    let highlight = Highlight::new()
                                        .with_properties(Properties::default().weight(Weight::Bold))
                                        .with_foreground_color(highlight_text);

                                    display_text =
                                        display_text.with_single_highlight(highlight, matches);
                                }
                                display_text.finish()
                            });

                            if let Some(DetailContent::Description(desc)) = &item.details {
                                row.add_child(
                                    Shrinkable::new(
                                        1.,
                                        Container::new(
                                            Text::new_inline(
                                                desc.clone(),
                                                appearance.ui_font_family(),
                                                font_size,
                                            )
                                            .with_style(
                                                Properties::default()
                                                    .weight(appearance.monospace_font_weight()),
                                            )
                                            .with_color(description_text)
                                            .finish(),
                                        )
                                        .with_padding_left(DESCRIPTION_PADDING)
                                        .finish(),
                                    )
                                    .finish(),
                                );
                            }

                            if FeatureFlag::AllowIgnoringInputSuggestions.is_enabled()
                                && item.is_history_item
                                && (mouse_state.is_hovered() || is_selected)
                            {
                                // Add an empty spacer so that the actual ignore button appears
                                // right justified.
                                row.add_child(Expanded::new(1., Empty::new().finish()).finish());

                                let ignore_button_mouse_state =
                                    input_suggestions.ignore_button_handles[index].clone();

                                // This matches the behavior of `render_ai_agent_mode_icon`; the heights of both the AM
                                // icon and the X button need to be the same to avoid jittering in the history menu.
                                let line_height = app.font_cache().line_height(
                                    appearance.monospace_font_size(),
                                    appearance.line_height_ratio(),
                                );

                                let ignore_button = appearance
                                    .ui_builder()
                                    .close_button(line_height, ignore_button_mouse_state.clone())
                                    .build()
                                    .on_click(move |ctx, _, _| {
                                        ctx.dispatch_typed_action(
                                            InputSuggestionsAction::IgnoreItem(index),
                                        );
                                    })
                                    .finish();

                                let ignore_button_with_tooltip =
                                    Hoverable::new(ignore_button_mouse_state, |state| {
                                        if state.is_hovered() {
                                            let mut stack = Stack::new().with_child(ignore_button);

                                            let tooltip_element = appearance
                                                .ui_builder()
                                                .tool_tip("Ignore this suggestion".to_string())
                                                .build()
                                                .finish();

                                            stack.add_positioned_overlay_child(
                                                tooltip_element,
                                                OffsetPositioning::offset_from_parent(
                                                    vec2f(0., -4.),
                                                    ParentOffsetBounds::WindowByPosition,
                                                    ParentAnchor::TopRight,
                                                    ChildAnchor::BottomRight,
                                                ),
                                            );
                                            stack.finish()
                                        } else {
                                            ignore_button
                                        }
                                    })
                                    .finish();

                                row.add_child(
                                    Container::new(ignore_button_with_tooltip)
                                        .with_margin_left(8.)
                                        .finish(),
                                );
                            }

                            let mut container =
                                Container::new(row.finish()).with_uniform_padding(LABEL_PADDING);

                            if is_selected || index < input_suggestions.items.len() - 1 {
                                container = container.with_border(Border::bottom(1.0));
                            }

                            if is_selected {
                                container = container.with_background(background_color);
                            }

                            EventHandler::new(container.finish())
                                .on_left_mouse_down(move |ctx, _, _| {
                                    ctx.dispatch_typed_action(
                                        InputSuggestionsAction::SelectAndConfirm(index),
                                    );
                                    DispatchEventResult::StopPropagation
                                })
                                .finish()
                        });

                        SavePosition::new(
                            row_element.finish(),
                            &InputSuggestions::position_id_at_index(index),
                        )
                        .finish()
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
            },
        )
        .notify_visible_items(self.visible_items_tx.clone());

        let input_suggestions_list = Container::new(
            Scrollable::vertical(
                self.scroll_state.clone(),
                list.finish_scrollable(),
                SCROLLBAR_WIDTH,
                theme.nonactive_ui_detail().into(),
                theme.active_ui_detail().into(),
                theme.surface_2().into(),
            )
            .finish(),
        )
        .with_margin_top(6.0)
        .with_margin_bottom(6.0)
        .finish();

        let mut stack = Stack::new();
        stack.add_child(
            SavePosition::new(input_suggestions_list, SUGGESTIONS_LIST_POSITION_ID).finish(),
        );

        // Render the overflow detail panel if the there is a visible, selected item with details.
        if let Some(selected_index) = self.selected_index {
            if let Some(details_box) =
                self.render_visible_item_details(selected_index, appearance, ctx)
            {
                stack.add_positioned_child(
                    SizeConstraintSwitch::new(
                        Container::new(details_box)
                            .with_uniform_padding(DETAILS_PANEL_PADDING)
                            .with_background(theme.surface_2())
                            .with_border(Border::all(1.0).with_border_fill(theme.outline()))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                            .with_margin_right(DETAILS_PANEL_MARGIN)
                            .with_drop_shadow(DropShadow::default())
                            .finish(),
                        vec![(
                            SizeConstraintCondition::WidthLessThan(DETAILS_MIN_WIDTH),
                            Empty::new().finish(),
                        )],
                    )
                    .finish(),
                    OffsetPositioning::from_axes(
                        PositioningAxis::relative_to_stack_child(
                            SUGGESTIONS_LIST_POSITION_ID,
                            PositionedElementOffsetBounds::WindowBySize,
                            OffsetType::Pixel(DETAILS_PANEL_MARGIN),
                            AnchorPair::new(XAxisAnchor::Right, XAxisAnchor::Left),
                        ),
                        PositioningAxis::relative_to_stack_child(
                            InputSuggestions::position_id_at_index(selected_index),
                            PositionedElementOffsetBounds::ParentByPosition,
                            OffsetType::Pixel(0.),
                            AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
                        ),
                    ),
                );
            }
        }

        stack.finish()
    }

    fn render_descriptions_box(
        &self,
        text: String,
        description: String,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let ui_builder = appearance.ui_builder();
        let title_text = ui_builder
            .paragraph(text)
            .with_style(UiComponentStyles {
                font_family_id: Some(appearance.monospace_font_family()),
                font_size: Some(appearance.monospace_font_size()),
                font_color: Some(theme.main_text_color(theme.surface_2()).into()),
                font_weight: Some(Weight::Bold),
                ..Default::default()
            })
            .build()
            .finish();

        let subtitle_text = ui_builder
            .paragraph(description)
            .with_style(UiComponentStyles {
                font_color: Some(theme.sub_text_color(theme.surface_2()).into()),
                margin: Some(Coords::uniform(0.).top(6.)),
                font_weight: Some(appearance.monospace_font_weight()),
                ..Default::default()
            })
            .build()
            .finish();

        let flex = Flex::column().with_children([title_text, subtitle_text]);

        ConstrainedBox::new(flex.finish())
            .with_width(DESCRIPTION_PANEL_WIDTH)
            .finish()
    }

    pub fn set_path_separators(&mut self, path_separators: PathSeparators) {
        self.path_separators = path_separators;
    }
}

impl Entity for InputSuggestions {
    type Event = Event;
}

impl TypedActionView for InputSuggestions {
    type Action = InputSuggestionsAction;

    fn handle_action(&mut self, action: &InputSuggestionsAction, ctx: &mut ViewContext<Self>) {
        match action {
            InputSuggestionsAction::SelectAndConfirm(index) => {
                self.select(&SelectAction::Index(*index), ctx);
                self.confirm(ctx);
            }
            InputSuggestionsAction::IgnoreItem(index) => {
                if let Some(item) = self.items.get(*index).cloned() {
                    ctx.emit(Event::IgnoreItem { item });
                }
            }
        }
    }
}

impl View for InputSuggestions {
    fn ui_name() -> &'static str {
        "InputSuggestions"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        Flex::column()
            .with_child(Shrinkable::new(1.0, self.render_items(ctx)).finish())
            .finish()
    }

    fn accessibility_contents(&self, _: &AppContext) -> Option<AccessibilityContent> {
        Some(AccessibilityContent::new(
            "Command suggestions.",
            // TODO use bindings from user settings
            "Navigate with tab and shift-tab, and confirm with enter. Execute selected command \
                with command + enter. Esc leaves the suggestions menu.",
            WarpA11yRole::MenuRole,
        ))
    }
}

/// Ordering of history items.
#[derive(Eq, PartialEq, Debug, Clone, Copy)]
pub enum HistoryOrder {
    /// The item is from a different session (either live or past).
    DifferentSession,
    /// The item is from the current session (including restored blocks).
    CurrentSession,
}

impl HistoryOrder {
    /// DifferentSession < CurrentSession
    fn ordering_value(&self) -> u8 {
        match self {
            HistoryOrder::DifferentSession => 0,
            HistoryOrder::CurrentSession => 1,
        }
    }
}

impl Ord for HistoryOrder {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.ordering_value().cmp(&other.ordering_value())
    }
}

impl PartialOrd for HistoryOrder {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Types of input that can be suggested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HistoryInputSuggestion<'a> {
    Command { entry: &'a HistoryEntry },
    AIQuery { entry: AIQueryHistory },
}

impl HistoryInputSuggestion<'_> {
    /// The timestamp this history entry was created. Useful for sorting.
    pub fn start_time(&self) -> DateTime<Local> {
        match self {
            HistoryInputSuggestion::Command { entry } => {
                entry.start_ts.unwrap_or(DateTime::default())
            }
            HistoryInputSuggestion::AIQuery { entry } => entry.start_time,
        }
    }

    /// Text to display for the suggestion.
    pub fn text(&self) -> &str {
        match self {
            HistoryInputSuggestion::Command { entry } => entry.command.as_str(),
            HistoryInputSuggestion::AIQuery { entry } => &entry.query_text,
        }
    }

    /// Which type of detail panel to show for this suggestion, if any.
    fn details(&self) -> Option<DetailContent> {
        match self {
            HistoryInputSuggestion::Command { entry } => {
                entry.has_metadata().then(|| ((*entry).clone()).into())
            }
            HistoryInputSuggestion::AIQuery { entry } => Some(DetailContent::AIQueryHistory(
                Box::new(AIQueryHistoryEntryDetails::from(entry)),
            )),
        }
    }

    /// Which input suggestion icon to use for this suggestion, if any.
    fn icon_type(&self) -> Option<ItemIconType> {
        match self {
            HistoryInputSuggestion::Command { .. } => None,
            HistoryInputSuggestion::AIQuery { .. } => Some(ItemIconType::AIQuery),
        }
    }

    /// True if this history item is for an AI query.
    pub(crate) fn is_ai_query(&self) -> bool {
        match self {
            HistoryInputSuggestion::Command { .. } => false,
            HistoryInputSuggestion::AIQuery { .. } => true,
        }
    }

    pub fn cmp(
        &self,
        other: &Self,
        current_session_id: Option<SessionId>,
        all_live_session_ids: &HashSet<SessionId>,
    ) -> Ordering {
        let ordering = self
            .history_order(current_session_id, all_live_session_ids)
            .cmp(&other.history_order(current_session_id, all_live_session_ids));
        if ordering == Ordering::Equal {
            self.start_time().cmp(&other.start_time())
        } else {
            ordering
        }
    }

    pub fn history_order(
        &self,
        current_session_id: Option<SessionId>,
        _all_live_session_ids: &HashSet<SessionId>,
    ) -> HistoryOrder {
        match self {
            HistoryInputSuggestion::Command { entry } => {
                // Restored blocks are always treated as CurrentSession
                if entry.is_for_restored_block {
                    return HistoryOrder::CurrentSession;
                }
                // Check if this entry belongs to the current session
                if let (Some(entry_session_id), Some(current_session_id)) =
                    (entry.session_id, current_session_id)
                {
                    if entry_session_id == current_session_id {
                        return HistoryOrder::CurrentSession;
                    }
                }
                // Other live session, or past session
                HistoryOrder::DifferentSession
            }
            HistoryInputSuggestion::AIQuery { entry } => entry.history_order,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AIQueryHistoryEntryDetails {
    /// The time the input was sent.
    pub(crate) start_time: DateTime<Local>,

    /// The status of the output streaming from the AI API.
    pub(crate) output_status: AIQueryHistoryOutputStatus,

    /// The working directory when the AI query was submitted.
    pub(crate) working_directory: Option<String>,
}

impl From<&AIQueryHistory> for AIQueryHistoryEntryDetails {
    fn from(value: &AIQueryHistory) -> Self {
        Self {
            start_time: value.start_time,
            output_status: value.output_status.clone(),
            working_directory: value.working_directory.clone(),
        }
    }
}

#[cfg(test)]
#[path = "input_suggestions_tests.rs"]
mod tests;
