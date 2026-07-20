//! Reusable active-menu routing and character-cell presentation for TUI inline menus.
use std::ops::Range;
use std::rc::Rc;

use string_offset::CharOffset;
use warp::tui_export::{
    AcceptSlashCommandOrSavedPrompt, AgentConversationEntryId, LLMId, TuiMcpAction,
};
use warp_search_core::inline_menu::{InlineMenuResultsUpdate, InlineMenuSelection};
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::elements::tui::{
    TuiConstrainedBox, TuiConstraint, TuiContainer, TuiElement, TuiEvent, TuiEventContext, TuiFlex,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiPresentationContext, TuiScreenPoint,
    TuiScreenPosition, TuiSize, TuiText,
};
use warpui_core::{AppContext, ModelHandle};

use crate::conversation_menu::TuiConversationMenuModel;
use crate::input_suggestions_mode::TuiInputSuggestionsMode;
use crate::mcp_menu::TuiMcpMenuModel;
use crate::model_menu::TuiModelMenuModel;
use crate::skills_menu::TuiSkillMenuModel;
use crate::slash_commands::TuiSlashCommandModel;
use crate::tui_builder::TuiUiBuilder;
use crate::tui_column_layout::{
    TuiTwoColumnConstraints, TuiTwoColumnLayout, format_tui_first_column, tui_two_column_layout,
};

const SLASH_COMMAND_COLUMN_CONSTRAINTS: TuiTwoColumnConstraints = TuiTwoColumnConstraints {
    preferred_first_columns: 29,
    minimum_first_columns: 8,
    minimum_second_columns: 12,
    preferred_maximum_second_columns: 21,
    gap_columns: 1,
};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TuiInlineMenuRowStyle {
    Default,
    InlineMenuItem,
}
pub(crate) fn active_inline_menu(
    inline_menus: &[TuiInlineMenu],
    mode: TuiInputSuggestionsMode,
    ctx: &AppContext,
) -> Option<TuiInlineMenu> {
    inline_menus
        .iter()
        .find(|menu| menu.mode() == mode && menu.is_open(ctx))
        .cloned()
}

impl TuiInlineMenuHandle for ModelHandle<TuiMcpMenuModel> {
    fn mode(&self) -> TuiInputSuggestionsMode {
        TuiInputSuggestionsMode::Mcp
    }
    fn is_open(&self, ctx: &AppContext) -> bool {
        self.as_ref(ctx).is_open(ctx)
    }

    fn input_highlight_range(&self, _ctx: &AppContext) -> Option<Range<CharOffset>> {
        None
    }

    fn input_argument_hint_text(&self, _ctx: &AppContext) -> Option<&'static str> {
        None
    }

    fn select_previous(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.select_previous(ctx));
    }

    fn select_next(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.select_next(ctx));
    }

    fn accept(&self, ctx: &mut AppContext) -> Option<TuiInlineMenuAccepted> {
        self.update(ctx, |model, ctx| model.accept_selected(ctx))
            .map(TuiInlineMenuAccepted::Mcp)
    }

    fn dismiss(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.dismiss(ctx));
    }

    fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        self.as_ref(ctx).snapshot(ctx)
    }
}

pub(crate) const MAX_INLINE_MENU_ROWS: u16 = 10;

/// A presentation-only row in a TUI inline menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TuiInlineMenuRow {
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) is_selectable: bool,
    pub(crate) style: TuiInlineMenuRowStyle,
}

/// A presentation-only tab in a TUI inline-menu header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TuiInlineMenuTab {
    pub(crate) label: String,
    pub(crate) is_selected: bool,
}

/// Optional header metadata rendered above menu rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TuiInlineMenuHeader {
    pub(crate) title: Option<String>,
    pub(crate) tabs: Vec<TuiInlineMenuTab>,
}

/// Empty-list presentation for an open inline menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TuiInlineMenuStatus {
    Loading(String),
    Empty(String),
}

/// Render-friendly, domain-neutral state for a TUI inline menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TuiInlineMenuSnapshot {
    pub(crate) header: Option<TuiInlineMenuHeader>,
    pub(crate) rows: Vec<TuiInlineMenuRow>,
    pub(crate) selected_index: Option<usize>,
    pub(crate) scroll_offset: usize,
    pub(crate) max_visible_rows: usize,
    pub(crate) status: Option<TuiInlineMenuStatus>,
}
/// Reusable list mechanics shared by the slash-command, conversation, and model menus.
#[derive(Debug, Clone)]
pub(crate) struct TuiInlineMenuListState<Row> {
    rows: Vec<Row>,
    selection: InlineMenuSelection,
    is_loading: bool,
    scroll_offset: usize,
}

impl<Row> Default for TuiInlineMenuListState<Row> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            selection: InlineMenuSelection::default(),
            is_loading: false,
            scroll_offset: 0,
        }
    }
}

impl<Row> TuiInlineMenuListState<Row> {
    pub(crate) fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub(crate) fn is_loading(&self) -> bool {
        self.is_loading
    }

    pub(crate) fn set_loading(&mut self, is_loading: bool) {
        self.is_loading = is_loading;
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.selection.selected_index()
    }

    pub(crate) fn selected_row(&self) -> Option<&Row> {
        self.selected_index().and_then(|index| self.rows.get(index))
    }

    pub(crate) fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Replaces the current rows and applies a caller-selected preferred row.
    pub(crate) fn replace_rows(
        &mut self,
        rows: Vec<Row>,
        is_loading: bool,
        preferred_index: Option<usize>,
        max_visible_rows: usize,
        mut is_selectable: impl FnMut(&Row) -> bool,
    ) {
        self.rows = rows;
        self.is_loading = is_loading;
        self.selection.clear();
        if let Some(index) = preferred_index {
            self.selection.select(index, self.rows.len(), |index| {
                self.rows.get(index).is_some_and(&mut is_selectable)
            });
        }
        self.keep_selected_visible(max_visible_rows);
    }

    /// Reconciles mixer-ordered rows, preserving the previous results while loading.
    pub(crate) fn reconcile_mixer_rows(
        &mut self,
        rows: Vec<Row>,
        is_loading: bool,
        max_visible_rows: usize,
        mut is_selectable: impl FnMut(&Row) -> bool,
    ) -> InlineMenuResultsUpdate {
        self.is_loading = is_loading;
        let update = self
            .selection
            .reconcile_results(is_loading, rows.len(), |index| {
                rows.get(index).is_some_and(&mut is_selectable)
            });
        if !matches!(update, InlineMenuResultsUpdate::Loading) {
            self.rows = rows;
            self.keep_selected_visible(max_visible_rows);
        }
        update
    }

    pub(crate) fn select_next(
        &mut self,
        max_visible_rows: usize,
        mut is_selectable: impl FnMut(&Row) -> bool,
    ) {
        self.selection.select_next(self.rows.len(), |index| {
            self.rows.get(index).is_some_and(&mut is_selectable)
        });
        self.keep_selected_visible(max_visible_rows);
    }

    pub(crate) fn select_previous(
        &mut self,
        max_visible_rows: usize,
        mut is_selectable: impl FnMut(&Row) -> bool,
    ) {
        self.selection.select_previous(self.rows.len(), |index| {
            self.rows.get(index).is_some_and(&mut is_selectable)
        });
        self.keep_selected_visible(max_visible_rows);
    }

    fn keep_selected_visible(&mut self, max_visible_rows: usize) {
        if let Some(selected_index) = self.selection.selected_index() {
            keep_selected_visible(
                self.rows.len(),
                selected_index,
                max_visible_rows,
                &mut self.scroll_offset,
            );
        } else {
            self.scroll_offset = self
                .scroll_offset
                .min(self.rows.len().saturating_sub(max_visible_rows));
        }
    }
}

/// Domain action produced by accepting the selected item in an active menu.
#[derive(Debug, Clone)]
pub(crate) enum TuiInlineMenuAccepted {
    SlashCommand(AcceptSlashCommandOrSavedPrompt),
    Conversation(AgentConversationEntryId),
    Model(LLMId),
    Mcp(TuiMcpAction),
}

/// Type-erased operations shared by TUI inline-menu model handles.
pub(crate) trait TuiInlineMenuHandle {
    /// Returns the input-suggestions mode represented by this menu.
    fn mode(&self) -> TuiInputSuggestionsMode;
    /// Returns whether this menu is open.
    fn is_open(&self, ctx: &AppContext) -> bool;
    /// Opens the menu when it supports explicit opening.
    fn open(&self, _ctx: &mut AppContext) {}
    /// Returns the input range highlighted by this menu.
    fn input_highlight_range(&self, ctx: &AppContext) -> Option<Range<CharOffset>>;
    /// Returns the input argument hint shown by this menu.
    fn input_argument_hint_text(&self, ctx: &AppContext) -> Option<&'static str>;
    /// Moves selection to the previous row.
    fn select_previous(&self, ctx: &mut AppContext);
    /// Moves selection to the next row.
    fn select_next(&self, ctx: &mut AppContext);
    /// Accepts the selected row.
    fn accept(&self, ctx: &mut AppContext) -> Option<TuiInlineMenuAccepted>;
    /// Dismisses the menu.
    fn dismiss(&self, ctx: &mut AppContext);
    /// Returns the menu's presentation snapshot.
    fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot>;
}

/// Cloneable type-erased handle for one TUI inline menu.
#[derive(Clone)]
pub(crate) struct TuiInlineMenu(Rc<dyn TuiInlineMenuHandle>);

impl TuiInlineMenu {
    /// Erases a concrete menu-model handle behind the shared routing interface.
    pub(crate) fn new(handle: impl TuiInlineMenuHandle + 'static) -> Self {
        Self(Rc::new(handle))
    }
    pub(crate) fn is_open(&self, ctx: &AppContext) -> bool {
        self.0.is_open(ctx)
    }
    pub(crate) fn open(&self, ctx: &mut AppContext) {
        self.0.open(ctx);
    }

    pub(crate) fn mode(&self) -> TuiInputSuggestionsMode {
        self.0.mode()
    }

    pub(crate) fn render(&self, ctx: &AppContext) -> Option<Box<dyn TuiElement>> {
        self.snapshot(ctx)
            .map(|snapshot| render_inline_menu(&snapshot, &TuiUiBuilder::from_app(ctx)))
    }
    pub(crate) fn input_highlight_range(&self, ctx: &AppContext) -> Option<Range<CharOffset>> {
        self.0.input_highlight_range(ctx)
    }

    pub(crate) fn input_argument_hint_text(&self, ctx: &AppContext) -> Option<&'static str> {
        self.0.input_argument_hint_text(ctx)
    }

    pub(crate) fn select_previous(&self, ctx: &mut AppContext) {
        self.0.select_previous(ctx);
    }

    pub(crate) fn select_next(&self, ctx: &mut AppContext) {
        self.0.select_next(ctx);
    }

    pub(crate) fn accept(&self, ctx: &mut AppContext) -> Option<TuiInlineMenuAccepted> {
        self.0.accept(ctx)
    }

    pub(crate) fn dismiss(&self, ctx: &mut AppContext) {
        self.0.dismiss(ctx);
    }

    fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        self.0.snapshot(ctx)
    }
}

impl TuiInlineMenuHandle for ModelHandle<TuiSlashCommandModel> {
    fn mode(&self) -> TuiInputSuggestionsMode {
        TuiInputSuggestionsMode::SlashCommands
    }
    fn is_open(&self, ctx: &AppContext) -> bool {
        self.as_ref(ctx).is_open(ctx)
    }
    fn input_highlight_range(&self, ctx: &AppContext) -> Option<Range<CharOffset>> {
        self.as_ref(ctx).highlighted_prefix_range()
    }

    fn input_argument_hint_text(&self, ctx: &AppContext) -> Option<&'static str> {
        self.as_ref(ctx).argument_hint_text()
    }

    fn select_previous(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.select_previous(ctx));
    }

    fn select_next(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.select_next(ctx));
    }

    fn accept(&self, ctx: &mut AppContext) -> Option<TuiInlineMenuAccepted> {
        self.update(ctx, |model, ctx| model.accept_selected(ctx))
            .map(TuiInlineMenuAccepted::SlashCommand)
    }

    fn dismiss(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.dismiss(ctx));
    }

    fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        self.as_ref(ctx).snapshot(ctx)
    }
}

impl TuiInlineMenuHandle for ModelHandle<TuiConversationMenuModel> {
    fn mode(&self) -> TuiInputSuggestionsMode {
        TuiInputSuggestionsMode::ConversationMenu
    }
    fn is_open(&self, ctx: &AppContext) -> bool {
        self.as_ref(ctx).is_open(ctx)
    }
    fn open(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.open(ctx));
    }

    fn input_highlight_range(&self, _ctx: &AppContext) -> Option<Range<CharOffset>> {
        None
    }

    fn input_argument_hint_text(&self, _ctx: &AppContext) -> Option<&'static str> {
        None
    }

    fn select_previous(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.select_previous(ctx));
    }

    fn select_next(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.select_next(ctx));
    }

    fn accept(&self, ctx: &mut AppContext) -> Option<TuiInlineMenuAccepted> {
        self.update(ctx, |model, ctx| model.accept_selected(ctx))
            .map(TuiInlineMenuAccepted::Conversation)
    }

    fn dismiss(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.dismiss(ctx));
    }

    fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        self.as_ref(ctx).snapshot(ctx)
    }
}

impl TuiInlineMenuHandle for ModelHandle<TuiModelMenuModel> {
    fn mode(&self) -> TuiInputSuggestionsMode {
        TuiInputSuggestionsMode::ModelSelector
    }
    fn is_open(&self, ctx: &AppContext) -> bool {
        self.as_ref(ctx).is_open(ctx)
    }
    fn input_highlight_range(&self, _ctx: &AppContext) -> Option<Range<CharOffset>> {
        None
    }

    fn input_argument_hint_text(&self, _ctx: &AppContext) -> Option<&'static str> {
        None
    }

    fn select_previous(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.select_previous(ctx));
    }

    fn select_next(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.select_next(ctx));
    }

    fn accept(&self, ctx: &mut AppContext) -> Option<TuiInlineMenuAccepted> {
        self.as_ref(ctx)
            .accept_selected(ctx)
            .map(TuiInlineMenuAccepted::Model)
    }

    fn dismiss(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.dismiss(ctx));
    }

    fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        self.as_ref(ctx).snapshot(ctx)
    }
}

impl TuiInlineMenuHandle for ModelHandle<TuiSkillMenuModel> {
    fn mode(&self) -> TuiInputSuggestionsMode {
        TuiInputSuggestionsMode::SkillMenu
    }
    fn is_open(&self, ctx: &AppContext) -> bool {
        self.as_ref(ctx).is_open(ctx)
    }

    fn input_highlight_range(&self, _ctx: &AppContext) -> Option<Range<CharOffset>> {
        None
    }

    fn input_argument_hint_text(&self, _ctx: &AppContext) -> Option<&'static str> {
        None
    }

    fn select_previous(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.select_previous(ctx));
    }

    fn select_next(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.select_next(ctx));
    }

    fn accept(&self, ctx: &mut AppContext) -> Option<TuiInlineMenuAccepted> {
        self.update(ctx, |model, ctx| model.accept_selected(ctx))
            .map(|skill| {
                TuiInlineMenuAccepted::SlashCommand(AcceptSlashCommandOrSavedPrompt::Skill {
                    reference: skill.skill_reference,
                    name: skill.skill_name,
                })
            })
    }

    fn dismiss(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.dismiss(ctx));
    }

    fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        self.as_ref(ctx).snapshot(ctx)
    }
}

pub(crate) fn render_inline_menu(
    snapshot: &TuiInlineMenuSnapshot,
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    Box::new(TuiInlineMenuElement {
        snapshot: snapshot.clone(),
        builder: builder.clone(),
        content: None,
    })
}

struct TuiInlineMenuElement {
    snapshot: TuiInlineMenuSnapshot,
    builder: TuiUiBuilder,
    content: Option<Box<dyn TuiElement>>,
}

impl TuiElement for TuiInlineMenuElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let mut content = build_inline_menu(
            &self.snapshot,
            &self.builder,
            constraint.max.width,
            constraint.max.height,
        );
        let size = content.layout(constraint, ctx, app);
        self.content = Some(content);
        size
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        if let Some(content) = self.content.as_mut() {
            content.render(origin, surface, ctx);
        }
    }

    /// Returns the laid-out content size.
    fn size(&self) -> Option<TuiSize> {
        self.content.as_ref()?.size()
    }

    /// Returns the painted content origin.
    fn origin(&self) -> Option<TuiScreenPoint> {
        self.content.as_ref()?.origin()
    }

    /// Delegates child-view presentation to the laid-out content.
    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        if let Some(content) = self.content.as_mut() {
            content.present(ctx);
        }
    }

    /// Delegates event dispatch to the laid-out content.
    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        app: &AppContext,
    ) -> bool {
        self.content
            .as_mut()
            .is_some_and(|content| content.dispatch_event(event, event_ctx, app))
    }
}

/// Returns the result rows available after reserving header chrome.
pub(crate) const fn result_row_capacity(
    allocated_height: u16,
    has_title: bool,
    has_tabs: bool,
) -> usize {
    let title_rows = if has_title { 1 } else { 0 };
    let tab_rows = if has_tabs { 1 } else { 0 };
    (allocated_height as usize).saturating_sub(title_rows + tab_rows)
}

fn visible_result_capacity(snapshot: &TuiInlineMenuSnapshot, allocated_height: u16) -> usize {
    let has_title = snapshot
        .header
        .as_ref()
        .is_some_and(|header| header.title.is_some());
    let has_tabs = snapshot
        .header
        .as_ref()
        .is_some_and(|header| !header.tabs.is_empty());
    result_row_capacity(allocated_height, has_title, has_tabs).min(snapshot.max_visible_rows)
}

fn build_inline_menu(
    snapshot: &TuiInlineMenuSnapshot,
    builder: &TuiUiBuilder,
    allocated_width: u16,
    allocated_height: u16,
) -> Box<dyn TuiElement> {
    let slash_command_columns = tui_two_column_layout(
        usize::from(allocated_width),
        snapshot.rows.iter().filter_map(|row| {
            if row.style != TuiInlineMenuRowStyle::InlineMenuItem {
                return None;
            }
            Some((row.title.as_str(), row.description.as_deref()?))
        }),
        SLASH_COMMAND_COLUMN_CONSTRAINTS,
    );
    let mut column = TuiFlex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    if let Some(header) = &snapshot.header {
        if let Some(title) = &header.title {
            column = column.child(menu_header_row(title, builder));
        }
        if !header.tabs.is_empty() {
            let labels = header
                .tabs
                .iter()
                .map(|tab| {
                    if tab.is_selected {
                        format!("[{}]", tab.label)
                    } else {
                        tab.label.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join("  ");
            column = column.child(menu_header_row(&labels, builder));
        }
    }

    if snapshot.rows.is_empty() {
        if let Some(status) = &snapshot.status {
            let label = match status {
                TuiInlineMenuStatus::Loading(label) | TuiInlineMenuStatus::Empty(label) => label,
            };
            column = column.child(menu_status_row(label, builder));
        }
    } else {
        let visible_rows = visible_result_capacity(snapshot, allocated_height);
        let mut scroll_offset = snapshot.scroll_offset;
        if let Some(selected_index) = snapshot.selected_index {
            keep_selected_visible(
                snapshot.rows.len(),
                selected_index,
                visible_rows,
                &mut scroll_offset,
            );
        } else {
            scroll_offset = scroll_offset.min(snapshot.rows.len().saturating_sub(visible_rows));
        }
        for (index, row) in snapshot
            .rows
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_rows)
        {
            column = column.child(menu_result_row(
                row,
                snapshot.selected_index == Some(index),
                slash_command_columns,
                builder,
            ));
        }
    }

    column.finish()
}

fn menu_header_row(label: &str, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
    TuiText::new(label)
        .with_style(builder.dim_text_style())
        .truncate()
        .finish()
}

/// Clamps stale scroll offsets and moves the viewport only as far as needed to
/// keep the selected row within a window of `visible_rows`.
pub(crate) fn keep_selected_visible(
    rows_len: usize,
    selected_index: usize,
    visible_rows: usize,
    scroll_offset: &mut usize,
) {
    if rows_len == 0 || visible_rows == 0 {
        *scroll_offset = 0;
        return;
    }

    let max_scroll_offset = rows_len.saturating_sub(visible_rows);
    *scroll_offset = (*scroll_offset).min(max_scroll_offset);
    if selected_index < *scroll_offset {
        *scroll_offset = selected_index;
    } else if selected_index >= *scroll_offset + visible_rows {
        *scroll_offset = selected_index + 1 - visible_rows;
    }
}

fn menu_status_row(label: &str, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
    TuiContainer::new(
        TuiText::new(label.to_owned())
            .with_style(builder.dim_text_style())
            .truncate()
            .finish(),
    )
    .with_padding_left(1)
    .with_padding_right(1)
    .finish()
}

fn menu_result_row(
    row: &TuiInlineMenuRow,
    is_selected: bool,
    slash_command_columns: TuiTwoColumnLayout,
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    let title_style = if is_selected {
        builder.slash_command_selection_text_style()
    } else {
        match (row.is_selectable, row.style) {
            (true, TuiInlineMenuRowStyle::InlineMenuItem) => builder.slash_command_text_style(),
            (true, TuiInlineMenuRowStyle::Default) => builder.primary_text_style(),
            (false, TuiInlineMenuRowStyle::Default | TuiInlineMenuRowStyle::InlineMenuItem) => {
                builder.dim_text_style()
            }
        }
    };
    let show_description = match row.style {
        TuiInlineMenuRowStyle::Default => row.description.is_some(),
        TuiInlineMenuRowStyle::InlineMenuItem => {
            slash_command_columns.show_second && row.description.is_some()
        }
    };
    let title_columns = if show_description {
        slash_command_columns.first_columns
    } else {
        slash_command_columns.available_columns
    };
    let title = match row.style {
        TuiInlineMenuRowStyle::Default => row.title.clone(),
        TuiInlineMenuRowStyle::InlineMenuItem => format_tui_first_column(
            &row.title,
            slash_command_columns.with_second_visible(show_description),
        ),
    };
    let title = TuiText::new(title)
        .with_style(title_style)
        .truncate()
        .finish();
    let description_style = if is_selected {
        builder.slash_command_selection_text_style()
    } else {
        match row.style {
            TuiInlineMenuRowStyle::Default => builder.muted_text_style(),
            TuiInlineMenuRowStyle::InlineMenuItem => builder.primary_text_style(),
        }
    };

    let mut content = TuiFlex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .child(match row.style {
            TuiInlineMenuRowStyle::Default => title,
            TuiInlineMenuRowStyle::InlineMenuItem => TuiConstrainedBox::new(title)
                .with_max_cols(
                    u16::try_from(title_columns)
                        .expect("title columns come from the u16 width constraint"),
                )
                .finish(),
        });
    if let Some(description) = row.description.as_ref().filter(|_| show_description) {
        let description = match row.style {
            TuiInlineMenuRowStyle::Default => format!("  {description}"),
            TuiInlineMenuRowStyle::InlineMenuItem => description.clone(),
        };
        content = content.child(
            TuiText::new(description)
                .with_style(description_style)
                .truncate()
                .finish(),
        );
    }
    let mut container = TuiContainer::new(content.finish());
    if is_selected {
        container = container.with_background(builder.slash_command_selection_background());
    }
    container.finish()
}

#[cfg(test)]
#[path = "inline_menu_tests.rs"]
mod tests;
