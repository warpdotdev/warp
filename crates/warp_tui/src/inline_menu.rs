//! Reusable active-menu routing and character-cell presentation for TUI inline menus.
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use string_offset::CharOffset;
use warp::tui_export::{
    AcceptSlashCommandOrSavedPrompt, AgentConversationEntryId, LLMId, ServerId, TuiMcpAction,
    TuiUpArrowHistoryItemKind,
};
use warp_search_core::inline_menu::{InlineMenuResultsUpdate, InlineMenuSelection};
use warpui_core::elements::tui::{
    Modifier, TuiConstrainedBox, TuiConstraint, TuiContainer, TuiElement, TuiEvent,
    TuiEventContext, TuiFlex, TuiHoverable, TuiLayoutContext, TuiPaintContext, TuiPaintSurface,
    TuiPresentationContext, TuiScreenPoint, TuiScreenPosition, TuiSize, TuiText,
};
use warpui_core::elements::{CrossAxisAlignment, MouseStateHandle};
use warpui_core::{AppContext, ModelHandle};

use crate::api_keys_menu::TuiApiKeysMenuModel;
use crate::completion_menu::TuiCompletionAcceptance;
use crate::conversation_menu::TuiConversationMenuModel;
use crate::input_suggestions_mode::TuiInputSuggestionsMode;
use crate::mcp_install_flow::{TuiMcpInstallFlowAction, TuiMcpInstallFlowModel};
use crate::mcp_menu::TuiMcpMenuModel;
use crate::model_menu::TuiModelMenuModel;
use crate::prompt_and_command_history_menu::TuiPromptAndCommandHistoryMenuModel;
use crate::skills_menu::TuiSkillMenuModel;
use crate::slash_commands::TuiSlashCommandModel;
use crate::team_menu::TuiTeamMenuModel;
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

pub(crate) const MAX_INLINE_MENU_ROWS: u16 = 10;
const MIN_REAL_ROWS_WITH_SCROLL_INDICATORS: usize = 3;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TuiInlineMenuRowStyle {
    Default,
    InlineMenuItem,
    StateWithDetail,
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
    fn open(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.open(ctx));
    }

    fn input_highlight_range(&self, _ctx: &AppContext) -> Option<Range<CharOffset>> {
        None
    }

    fn input_argument_hint_text(&self, ctx: &AppContext) -> Option<&'static str> {
        self.as_ref(ctx).input_hint_text(ctx)
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
            .map(TuiInlineMenuAccepted::Mcp)
    }
    fn accept_secondary(&self, ctx: &mut AppContext) -> Option<TuiInlineMenuAccepted> {
        self.as_ref(ctx)
            .logout_selected(ctx)
            .map(TuiInlineMenuAccepted::Mcp)
    }

    fn dismiss(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.dismiss(ctx));
    }

    fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        self.as_ref(ctx).snapshot(ctx)
    }

    fn select_by_snapshot_index(&self, index: usize, ctx: &mut AppContext) -> bool {
        self.update(ctx, |model, ctx| model.select_at_snapshot_index(index, ctx))
    }

    fn scroll_by_delta(&self, delta: isize, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.scroll_by_delta(delta, ctx));
    }
}

impl TuiInlineMenuHandle for ModelHandle<TuiMcpInstallFlowModel> {
    fn mode(&self) -> TuiInputSuggestionsMode {
        TuiInputSuggestionsMode::McpInstall
    }

    fn is_open(&self, ctx: &AppContext) -> bool {
        self.as_ref(ctx).is_open(ctx)
    }

    fn input_highlight_range(&self, _ctx: &AppContext) -> Option<Range<CharOffset>> {
        None
    }

    fn input_argument_hint_text(&self, ctx: &AppContext) -> Option<&'static str> {
        self.as_ref(ctx).input_hint_text(ctx)
    }

    fn select_previous(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.select_previous(ctx));
    }

    fn select_next(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.select_next(ctx));
    }

    fn accept(&self, ctx: &mut AppContext) -> Option<TuiInlineMenuAccepted> {
        self.as_ref(ctx)
            .accept(ctx)
            .map(TuiInlineMenuAccepted::McpInstall)
    }

    fn dismiss(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.dismiss(ctx));
    }

    fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        self.as_ref(ctx).snapshot(ctx)
    }

    fn select_by_snapshot_index(&self, index: usize, ctx: &mut AppContext) -> bool {
        self.update(ctx, |model, ctx| model.select_at_snapshot_index(index, ctx))
    }

    fn scroll_by_delta(&self, delta: isize, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.scroll_by_delta(delta, ctx));
    }
}

/// A presentation-only row in a TUI inline menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TuiInlineMenuRow {
    pub(crate) title: String,
    pub(crate) prefix: Option<TuiInlineMenuRowPrefix>,
    pub(crate) description: Option<String>,
    pub(crate) state_suffix: Option<String>,
    pub(crate) promotional_suffix: Option<String>,
    pub(crate) is_selectable: bool,
    pub(crate) style: TuiInlineMenuRowStyle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TuiInlineMenuRowPrefix {
    pub(crate) text: String,
    pub(crate) style: TuiInlineMenuRowPrefixStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TuiInlineMenuRowPrefixStyle {
    ShellCommand,
}
/// Returns a single-line menu title while leaving the source text unchanged.
pub(crate) fn single_line_menu_title(text: &str) -> String {
    let Some((first_line, _)) = text.split_once('\n') else {
        return text.to_owned();
    };
    format!("{}...", first_line.strip_suffix('\r').unwrap_or(first_line))
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
/// Controls whether rendering follows the selection or an explicit wheel offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TuiInlineMenuScrollAnchor {
    Selection,
    ScrollOffset,
}

/// Render-friendly, domain-neutral state for a TUI inline menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TuiInlineMenuSnapshot {
    pub(crate) header: Option<TuiInlineMenuHeader>,
    pub(crate) rows: Vec<TuiInlineMenuRow>,
    pub(crate) selected_index: Option<usize>,
    pub(crate) scroll_offset: usize,
    pub(crate) scroll_anchor: TuiInlineMenuScrollAnchor,
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
    scroll_anchor: TuiInlineMenuScrollAnchor,
}

impl<Row> Default for TuiInlineMenuListState<Row> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            selection: InlineMenuSelection::default(),
            is_loading: false,
            scroll_offset: 0,
            scroll_anchor: TuiInlineMenuScrollAnchor::Selection,
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
    pub(crate) fn scroll_anchor(&self) -> TuiInlineMenuScrollAnchor {
        self.scroll_anchor
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
        self.scroll_anchor = TuiInlineMenuScrollAnchor::Selection;
        self.scroll_offset = inline_menu_viewport(
            self.rows.len(),
            self.selection.selected_index(),
            self.scroll_offset,
            max_visible_rows,
            self.scroll_anchor,
        )
        .rows
        .start;
    }

    /// Selects the row at `index` directly (for mouse-click targeting) and
    /// scrolls to keep it visible. Only moves the selection when
    /// `is_selectable` returns true for the target row, matching the
    /// behaviour of `select_next` / `select_previous` / `replace_rows`.
    /// Returns `true` when the selection was actually moved to `index`, or
    /// `false` when the index was out of bounds or the row is not selectable.
    pub(crate) fn select_absolute(
        &mut self,
        index: usize,
        max_visible_rows: usize,
        mut is_selectable: impl FnMut(&Row) -> bool,
    ) -> bool {
        let rows_len = self.rows.len();
        if index < rows_len && is_selectable(&self.rows[index]) {
            self.selection.select(index, rows_len, |i| {
                self.rows.get(i).is_some_and(&mut is_selectable)
            });
            self.keep_selected_visible(max_visible_rows);
            true
        } else {
            false
        }
    }

    /// Scrolls the viewport by `delta` rows without changing the selection.
    pub(crate) fn scroll_by(&mut self, delta: isize, max_visible_rows: usize) {
        let max_offset = max_inline_menu_scroll_offset(self.rows.len(), max_visible_rows);
        let scroll_offset = self
            .scroll_offset
            .saturating_add_signed(delta)
            .min(max_offset);
        if scroll_offset != self.scroll_offset {
            self.scroll_offset = scroll_offset;
            self.scroll_anchor = TuiInlineMenuScrollAnchor::ScrollOffset;
        }
    }
}

/// Domain action produced by accepting the selected item in an active menu.
#[derive(Debug, Clone)]
pub(crate) enum TuiInlineMenuAccepted {
    SlashCommand(AcceptSlashCommandOrSavedPrompt),
    Conversation(AgentConversationEntryId),
    Model(LLMId),
    Team(ServerId),
    Mcp(TuiMcpAction),
    McpInstall(TuiMcpInstallFlowAction),
    PromptAndCommandHistory {
        text: String,
        kind: TuiUpArrowHistoryItemKind,
    },
    /// A shell completion and the exact input span it replaces.
    Completion(TuiCompletionAcceptance),
}

/// Who owns the shared TUI editor while an inline menu is active.
///
/// The variants are exhaustive so ownership and masking cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TuiInlineMenuInputOwnership {
    /// The composer owns editor behavior; the menu only observes input and handles menu actions.
    Composer,
    /// The inline menu owns ordinary plaintext editor behavior.
    InlineMenuPlainText,
    /// The inline menu owns editor behavior, with masked rendering and clipboard export disabled.
    InlineMenuMasked,
}

impl TuiInlineMenuInputOwnership {
    pub(crate) fn inline_menu_owns_input(self) -> bool {
        matches!(self, Self::InlineMenuPlainText | Self::InlineMenuMasked)
    }

    pub(crate) fn is_masked(self) -> bool {
        matches!(self, Self::InlineMenuMasked)
    }
}

/// Type alias for mouse-interaction callbacks stored in the element tree.
type InlineMenuAcceptFn = dyn Fn(usize, &mut TuiEventContext<'_>, &AppContext);
type InlineMenuScrollFn = dyn Fn(isize, &mut TuiEventContext<'_>, &AppContext);
fn reset_hover_states(states: &RefCell<Vec<MouseStateHandle>>) {
    for state in states.borrow().iter() {
        state.lock().unwrap().reset_hover_state();
    }
}

/// Type-erased operations shared by TUI inline-menu model handles.
pub(crate) trait TuiInlineMenuHandle {
    /// Returns the input-suggestions mode represented by this menu.
    fn mode(&self) -> TuiInputSuggestionsMode;
    /// Returns whether this menu is open.
    fn is_open(&self, ctx: &AppContext) -> bool;
    /// Returns who owns the shared editor while this menu is active.
    fn input_ownership(&self, _ctx: &AppContext) -> TuiInlineMenuInputOwnership {
        TuiInlineMenuInputOwnership::Composer
    }
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
    /// Accepts the selected row's optional secondary action.
    fn accept_secondary(&self, _ctx: &mut AppContext) -> Option<TuiInlineMenuAccepted> {
        None
    }
    /// Dismisses the menu.
    fn dismiss(&self, ctx: &mut AppContext);
    /// Returns the menu's presentation snapshot.
    fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot>;
    /// Selects the row at the given absolute snapshot index (for mouse-click
    /// targeting). Returns `true` when the selection was (or could have been)
    /// made so that the caller can safely fire `accept`. Returns `false` by
    /// default so that a menu that forgets to override this method does not
    /// silently accept whatever row happened to be keyboard-selected.
    fn select_by_snapshot_index(&self, _index: usize, _ctx: &mut AppContext) -> bool {
        false
    }
    /// Scrolls the menu viewport by `delta` rows without changing the
    /// selection. No-op by default; models that support it override this.
    fn scroll_by_delta(&self, _delta: isize, _ctx: &mut AppContext) {}
    /// Returns whether the menu's selected row supports the contextual clear action.
    fn can_clear_selected(&self, _ctx: &AppContext) -> bool {
        false
    }
    /// Clears the menu's selected row when supported.
    fn clear_selected(&self, _ctx: &mut AppContext) {}
}

/// Cloneable type-erased handle for one TUI inline menu, with retained
/// per-row mouse state for hover and click interactions.
#[derive(Clone)]
pub(crate) struct TuiInlineMenu {
    handle: Rc<dyn TuiInlineMenuHandle>,
    /// Per-row mouse state handles, grown on demand to match the snapshot's
    /// row count. Shared across `Clone`s so both the session view and the
    /// input view see the same hover/click state.
    item_mouse_states: Rc<RefCell<Vec<MouseStateHandle>>>,
}

impl TuiInlineMenu {
    /// Erases a concrete menu-model handle behind the shared routing interface.
    pub(crate) fn new(handle: impl TuiInlineMenuHandle + 'static) -> Self {
        Self {
            handle: Rc::new(handle),
            item_mouse_states: Rc::new(RefCell::new(Vec::new())),
        }
    }
    pub(crate) fn is_open(&self, ctx: &AppContext) -> bool {
        self.handle.is_open(ctx)
    }
    pub(crate) fn open(&self, ctx: &mut AppContext) {
        self.handle.open(ctx);
    }

    pub(crate) fn mode(&self) -> TuiInputSuggestionsMode {
        self.handle.mode()
    }
    pub(crate) fn input_ownership(&self, ctx: &AppContext) -> TuiInlineMenuInputOwnership {
        self.handle.input_ownership(ctx)
    }

    /// Renders the menu without mouse interactions (used in tests and other
    /// non-interactive contexts).
    #[cfg(test)]
    pub(crate) fn render(&self, ctx: &AppContext) -> Option<Box<dyn TuiElement>> {
        self.snapshot(ctx)
            .map(|snapshot| render_inline_menu(&snapshot, &TuiUiBuilder::from_app(ctx)))
    }

    /// Renders the menu with hover/click and scroll-wheel interactions.
    /// `on_accept` is called with the clicked row's absolute snapshot index.
    /// `on_scroll` is called with the scroll-wheel row delta.
    pub(crate) fn render_with_interaction(
        &self,
        ctx: &AppContext,
        on_accept: impl Fn(usize, &mut TuiEventContext<'_>, &AppContext) + 'static,
        on_scroll: impl Fn(isize, &mut TuiEventContext<'_>, &AppContext) + 'static,
    ) -> Option<Box<dyn TuiElement>> {
        // Mouse-state growth happens in TuiInlineMenuElement::layout every
        // frame, so there is no need to pre-grow the vec here.
        self.snapshot(ctx).map(|snapshot| {
            let on_accept: Rc<InlineMenuAcceptFn> = Rc::new(on_accept);
            let on_scroll: Box<InlineMenuScrollFn> = Box::new(on_scroll);
            TuiInlineMenuElement {
                snapshot,
                builder: TuiUiBuilder::from_app(ctx),
                content: None,
                item_mouse_states: Rc::clone(&self.item_mouse_states),
                on_accept: Some(on_accept),
                on_scroll: Some(on_scroll),
            }
            .finish()
        })
    }

    pub(crate) fn select_by_snapshot_index(&self, index: usize, ctx: &mut AppContext) -> bool {
        self.handle.select_by_snapshot_index(index, ctx)
    }

    pub(crate) fn scroll_by_delta(&self, delta: isize, ctx: &mut AppContext) {
        self.handle.scroll_by_delta(delta, ctx);
    }

    pub(crate) fn can_clear_selected(&self, ctx: &AppContext) -> bool {
        self.handle.can_clear_selected(ctx)
    }

    pub(crate) fn clear_selected(&self, ctx: &mut AppContext) {
        self.handle.clear_selected(ctx);
    }

    pub(crate) fn input_highlight_range(&self, ctx: &AppContext) -> Option<Range<CharOffset>> {
        self.handle.input_highlight_range(ctx)
    }

    pub(crate) fn input_argument_hint_text(&self, ctx: &AppContext) -> Option<&'static str> {
        self.handle.input_argument_hint_text(ctx)
    }

    pub(crate) fn select_previous(&self, ctx: &mut AppContext) {
        self.handle.select_previous(ctx);
    }

    pub(crate) fn select_next(&self, ctx: &mut AppContext) {
        self.handle.select_next(ctx);
    }

    pub(crate) fn accept(&self, ctx: &mut AppContext) -> Option<TuiInlineMenuAccepted> {
        let result = self.handle.accept(ctx);
        if result.is_some() {
            reset_hover_states(&self.item_mouse_states);
        }
        result
    }
    pub(crate) fn accept_secondary(&self, ctx: &mut AppContext) -> Option<TuiInlineMenuAccepted> {
        let result = self.handle.accept_secondary(ctx);
        if result.is_some() {
            reset_hover_states(&self.item_mouse_states);
        }
        result
    }

    pub(crate) fn dismiss(&self, ctx: &mut AppContext) {
        self.handle.dismiss(ctx);
        reset_hover_states(&self.item_mouse_states);
    }

    fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        self.handle.snapshot(ctx)
    }
}

impl TuiInlineMenuHandle for ModelHandle<TuiApiKeysMenuModel> {
    fn mode(&self) -> TuiInputSuggestionsMode {
        TuiInputSuggestionsMode::ApiKeys
    }

    fn is_open(&self, ctx: &AppContext) -> bool {
        self.as_ref(ctx).is_open(ctx)
    }

    fn input_ownership(&self, ctx: &AppContext) -> TuiInlineMenuInputOwnership {
        self.as_ref(ctx).input_ownership(ctx)
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
        self.update(ctx, |model, ctx| model.accept_selected(ctx));
        None
    }

    fn dismiss(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.dismiss(ctx));
    }

    fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        self.as_ref(ctx).snapshot(ctx)
    }

    fn select_by_snapshot_index(&self, index: usize, ctx: &mut AppContext) -> bool {
        self.update(ctx, |model, ctx| model.select_at_snapshot_index(index, ctx))
    }

    fn scroll_by_delta(&self, delta: isize, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.scroll_by_delta(delta, ctx));
    }

    fn can_clear_selected(&self, ctx: &AppContext) -> bool {
        self.as_ref(ctx).can_clear_selected(ctx)
    }

    fn clear_selected(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.clear_selected(ctx));
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

    fn select_by_snapshot_index(&self, index: usize, ctx: &mut AppContext) -> bool {
        self.update(ctx, |model, ctx| model.select_at_snapshot_index(index, ctx))
    }

    fn scroll_by_delta(&self, delta: isize, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.scroll_by_delta(delta, ctx));
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

    fn select_by_snapshot_index(&self, index: usize, ctx: &mut AppContext) -> bool {
        self.update(ctx, |model, ctx| model.select_at_snapshot_index(index, ctx))
    }

    fn scroll_by_delta(&self, delta: isize, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.scroll_by_delta(delta, ctx));
    }
}

impl TuiInlineMenuHandle for ModelHandle<TuiPromptAndCommandHistoryMenuModel> {
    fn mode(&self) -> TuiInputSuggestionsMode {
        TuiInputSuggestionsMode::PromptAndCommandHistory
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
            .map(|row| TuiInlineMenuAccepted::PromptAndCommandHistory {
                text: row.text,
                kind: row.kind,
            })
    }

    fn dismiss(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.dismiss(ctx));
    }

    fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        self.as_ref(ctx).snapshot(ctx)
    }

    fn select_by_snapshot_index(&self, index: usize, ctx: &mut AppContext) -> bool {
        self.update(ctx, |model, ctx| model.select_at_snapshot_index(index, ctx))
    }

    fn scroll_by_delta(&self, delta: isize, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.scroll_by_delta(delta, ctx));
    }
}

impl TuiInlineMenuHandle for ModelHandle<TuiModelMenuModel> {
    fn mode(&self) -> TuiInputSuggestionsMode {
        TuiInputSuggestionsMode::ModelSelector
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

    fn select_by_snapshot_index(&self, index: usize, ctx: &mut AppContext) -> bool {
        self.update(ctx, |model, ctx| model.select_at_snapshot_index(index, ctx))
    }

    fn scroll_by_delta(&self, delta: isize, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.scroll_by_delta(delta, ctx));
    }
}

impl TuiInlineMenuHandle for ModelHandle<TuiTeamMenuModel> {
    fn mode(&self) -> TuiInputSuggestionsMode {
        TuiInputSuggestionsMode::TeamSelector
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
        self.as_ref(ctx)
            .accept_selected(ctx)
            .map(TuiInlineMenuAccepted::Team)
    }

    fn dismiss(&self, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.dismiss(ctx));
    }

    fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        self.as_ref(ctx).snapshot(ctx)
    }

    fn select_by_snapshot_index(&self, index: usize, ctx: &mut AppContext) -> bool {
        self.update(ctx, |model, ctx| model.select_at_snapshot_index(index, ctx))
    }

    fn scroll_by_delta(&self, delta: isize, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.scroll_by_delta(delta, ctx));
    }
}

impl TuiInlineMenuHandle for ModelHandle<TuiSkillMenuModel> {
    fn mode(&self) -> TuiInputSuggestionsMode {
        TuiInputSuggestionsMode::SkillMenu
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

    fn select_by_snapshot_index(&self, index: usize, ctx: &mut AppContext) -> bool {
        self.update(ctx, |model, ctx| model.select_at_snapshot_index(index, ctx))
    }

    fn scroll_by_delta(&self, delta: isize, ctx: &mut AppContext) {
        self.update(ctx, |model, ctx| model.scroll_by_delta(delta, ctx));
    }
}

#[cfg(test)]
pub(crate) fn render_inline_menu(
    snapshot: &TuiInlineMenuSnapshot,
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    TuiInlineMenuElement {
        snapshot: snapshot.clone(),
        builder: builder.clone(),
        content: None,
        item_mouse_states: Rc::new(RefCell::new(Vec::new())),
        on_accept: None,
        on_scroll: None,
    }
    .finish()
}

struct TuiInlineMenuElement {
    snapshot: TuiInlineMenuSnapshot,
    builder: TuiUiBuilder,
    content: Option<Box<dyn TuiElement>>,
    /// Retained per-row mouse handles shared with the owning `TuiInlineMenu`.
    item_mouse_states: Rc<RefCell<Vec<MouseStateHandle>>>,
    /// Called with the absolute snapshot index when a row is clicked.
    on_accept: Option<Rc<InlineMenuAcceptFn>>,
    /// Called with the scroll-wheel row delta.
    on_scroll: Option<Box<InlineMenuScrollFn>>,
}

impl TuiElement for TuiInlineMenuElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        {
            let mut states = self.item_mouse_states.borrow_mut();
            while states.len() < self.snapshot.rows.len() {
                states.push(MouseStateHandle::default());
            }
        }
        let mouse_states = self.item_mouse_states.borrow();
        let mut content = build_inline_menu(
            &self.snapshot,
            &self.builder,
            constraint.max.width,
            constraint.max.height,
            &mouse_states,
            self.on_accept.clone(),
        );
        drop(mouse_states);
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

    /// Handles scroll-wheel events over the menu bounds, then delegates
    /// remaining events to the laid-out content (which forwards hover/click
    /// through `TuiHoverable`).
    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        app: &AppContext,
    ) -> bool {
        if let TuiEvent::ScrollWheel {
            position, delta, ..
        } = event
            && let Some(on_scroll) = &self.on_scroll
            && let Some((origin, size)) = self.origin().zip(self.size())
            && event_ctx.hit_test(origin, size, *position)
            && delta.1 != 0
        {
            // Positive wheel delta scrolls toward the start of the list,
            // matching `option_selector` and the transcript scrollable.
            reset_hover_states(&self.item_mouse_states);
            on_scroll(-delta.1, event_ctx, app);
            return true;
        }
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
    mouse_states: &[MouseStateHandle],
    on_accept: Option<Rc<InlineMenuAcceptFn>>,
) -> Box<dyn TuiElement> {
    let slash_command_row_text = snapshot
        .rows
        .iter()
        .filter(|row| row.style == TuiInlineMenuRowStyle::InlineMenuItem)
        .filter_map(|row| {
            let mut description = row.description.clone()?;
            if let Some(suffix) = &row.state_suffix {
                description.push(' ');
                description.push_str(suffix);
            }
            Some((row.title.clone(), description))
        })
        .collect::<Vec<_>>();

    let slash_command_columns = tui_two_column_layout(
        usize::from(allocated_width),
        slash_command_row_text
            .iter()
            .map(|(title, description)| (title.as_str(), description.as_str())),
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
        let viewport = inline_menu_viewport(
            snapshot.rows.len(),
            snapshot.selected_index,
            snapshot.scroll_offset,
            visible_rows,
            snapshot.scroll_anchor,
        );

        if viewport.has_more_above {
            column = column.child(menu_scroll_indicator_row("↑", builder));
        }

        for (index, row) in snapshot
            .rows
            .iter()
            .enumerate()
            .skip(viewport.rows.start)
            .take(viewport.rows.len())
        {
            // Check hover state from the retained mouse-state handle.
            let is_hovered = mouse_states
                .get(index)
                .is_some_and(|s| s.lock().unwrap().is_hovered());
            let base_element = menu_result_row(
                row,
                snapshot.selected_index == Some(index),
                is_hovered,
                slash_command_columns,
                builder,
            );
            // Wrap the row in a hoverable with a click callback when mouse
            // interaction is enabled and the row is selectable.
            let element = match (mouse_states.get(index), on_accept.as_ref()) {
                (Some(mouse_state), Some(on_accept_fn)) if row.is_selectable => {
                    let on_accept_clone = Rc::clone(on_accept_fn);
                    TuiHoverable::new(mouse_state.clone(), base_element)
                        .on_click(move |event_ctx, app| {
                            on_accept_clone(index, event_ctx, app);
                        })
                        .finish()
                }
                _ => base_element,
            };
            column = column.child(element);
        }

        if viewport.has_more_below {
            column = column.child(menu_scroll_indicator_row("↓", builder));
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct TuiInlineMenuViewport {
    rows: Range<usize>,
    has_more_above: bool,
    has_more_below: bool,
}

fn inline_menu_viewport(
    rows_len: usize,
    selected_index: Option<usize>,
    scroll_offset: usize,
    visible_rows: usize,
    scroll_anchor: TuiInlineMenuScrollAnchor,
) -> TuiInlineMenuViewport {
    if rows_len == 0 || visible_rows == 0 {
        return TuiInlineMenuViewport {
            rows: 0..0,
            has_more_above: false,
            has_more_below: false,
        };
    }
    if rows_len <= visible_rows {
        return TuiInlineMenuViewport {
            rows: 0..rows_len,
            has_more_above: false,
            has_more_below: false,
        };
    }
    if visible_rows <= MIN_REAL_ROWS_WITH_SCROLL_INDICATORS {
        return inline_menu_viewport_without_indicators(
            rows_len,
            selected_index,
            scroll_offset,
            visible_rows,
            scroll_anchor,
        );
    }

    let bottom_start = rows_len.saturating_sub(visible_rows - 1);
    let mut start = scroll_offset.min(bottom_start);
    let mut end = inline_menu_viewport_end(rows_len, start, visible_rows);
    if let Some(selected_index) = selected_index
        .filter(|_| matches!(scroll_anchor, TuiInlineMenuScrollAnchor::Selection))
        .filter(|index| *index < rows_len)
    {
        if selected_index < start {
            start = selected_index;
        } else if selected_index >= end {
            start = (selected_index + 1 - (visible_rows - 2)).min(bottom_start);
        }
        end = inline_menu_viewport_end(rows_len, start, visible_rows);
    }

    let viewport = TuiInlineMenuViewport {
        rows: start..end,
        has_more_above: start > 0,
        has_more_below: end < rows_len,
    };
    if viewport.rows.len() < MIN_REAL_ROWS_WITH_SCROLL_INDICATORS {
        return inline_menu_viewport_without_indicators(
            rows_len,
            selected_index,
            scroll_offset,
            visible_rows,
            scroll_anchor,
        );
    }
    viewport
}

fn inline_menu_viewport_without_indicators(
    rows_len: usize,
    selected_index: Option<usize>,
    scroll_offset: usize,
    visible_rows: usize,
    scroll_anchor: TuiInlineMenuScrollAnchor,
) -> TuiInlineMenuViewport {
    let mut start = scroll_offset.min(rows_len.saturating_sub(visible_rows));
    if let Some(selected_index) = selected_index
        .filter(|_| matches!(scroll_anchor, TuiInlineMenuScrollAnchor::Selection))
        .filter(|index| *index < rows_len)
    {
        if selected_index < start {
            start = selected_index;
        } else if selected_index >= start + visible_rows {
            start = selected_index + 1 - visible_rows;
        }
    }
    TuiInlineMenuViewport {
        rows: start..(start + visible_rows).min(rows_len),
        has_more_above: false,
        has_more_below: false,
    }
}

fn max_inline_menu_scroll_offset(rows_len: usize, visible_rows: usize) -> usize {
    if visible_rows > MIN_REAL_ROWS_WITH_SCROLL_INDICATORS {
        rows_len.saturating_sub(visible_rows - 1)
    } else {
        rows_len.saturating_sub(visible_rows)
    }
}
fn inline_menu_viewport_end(rows_len: usize, start: usize, visible_rows: usize) -> usize {
    let upper_indicator_rows = usize::from(start > 0);
    let rows_without_lower_indicator = visible_rows - upper_indicator_rows;
    let reaches_end = start.saturating_add(rows_without_lower_indicator) >= rows_len;
    let lower_indicator_rows = usize::from(!reaches_end);
    start
        .saturating_add(visible_rows - upper_indicator_rows - lower_indicator_rows)
        .min(rows_len)
}

/// Clamps stale scroll offsets and moves the viewport only as far as needed to
/// keep the selected row within a window of `visible_rows` result rows.
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
    TuiText::new(label.to_owned())
        .with_style(builder.dim_text_style())
        .truncate()
        .finish()
}

fn menu_scroll_indicator_row(label: &str, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
    TuiText::new(label)
        .with_style(builder.dim_text_style())
        .finish()
}

fn menu_result_row(
    row: &TuiInlineMenuRow,
    is_selected: bool,
    is_hovered: bool,
    slash_command_columns: TuiTwoColumnLayout,
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    if row.style == TuiInlineMenuRowStyle::StateWithDetail {
        return menu_state_with_detail_row(row, is_selected, is_hovered, builder);
    }
    let title_style = if is_selected {
        builder.slash_command_selection_text_style()
    } else if is_hovered && row.is_selectable {
        // Hover: bold the text to indicate the row is interactive.
        match row.style {
            TuiInlineMenuRowStyle::InlineMenuItem => builder
                .slash_command_text_style()
                .add_modifier(Modifier::BOLD),
            TuiInlineMenuRowStyle::Default => {
                builder.primary_text_style().add_modifier(Modifier::BOLD)
            }
            TuiInlineMenuRowStyle::StateWithDetail => unreachable!(),
        }
    } else {
        match (row.is_selectable, row.style) {
            (true, TuiInlineMenuRowStyle::InlineMenuItem) => builder.slash_command_text_style(),
            (true, TuiInlineMenuRowStyle::Default) => builder.primary_text_style(),
            (false, TuiInlineMenuRowStyle::Default | TuiInlineMenuRowStyle::InlineMenuItem) => {
                builder.dim_text_style()
            }
            (_, TuiInlineMenuRowStyle::StateWithDetail) => unreachable!(),
        }
    };
    let show_description = match row.style {
        TuiInlineMenuRowStyle::Default => {
            row.description.is_some()
                || row.state_suffix.is_some()
                || row.promotional_suffix.is_some()
        }
        TuiInlineMenuRowStyle::InlineMenuItem => {
            slash_command_columns.show_second && row.description.is_some()
        }
        TuiInlineMenuRowStyle::StateWithDetail => unreachable!(),
    };
    let title_columns = if show_description {
        slash_command_columns.first_columns
    } else {
        slash_command_columns.available_columns
    };
    let single_line_title = single_line_menu_title(&row.title);
    let title = match row.style {
        TuiInlineMenuRowStyle::Default => single_line_title,
        TuiInlineMenuRowStyle::InlineMenuItem => format_tui_first_column(
            &single_line_title,
            slash_command_columns.with_second_visible(show_description),
        ),
        TuiInlineMenuRowStyle::StateWithDetail => unreachable!(),
    };
    let title = if let Some(prefix) = &row.prefix {
        let prefix_style = if is_selected {
            title_style
        } else {
            match prefix.style {
                TuiInlineMenuRowPrefixStyle::ShellCommand => builder.shell_command_prefix_style(),
            }
        };
        TuiText::from_spans([(prefix.text.clone(), prefix_style), (title, title_style)])
            .truncate_with_ellipsis()
            .finish()
    } else {
        TuiText::new(title)
            .with_style(title_style)
            .truncate_with_ellipsis()
            .finish()
    };
    let description_style = if is_selected {
        builder.slash_command_selection_text_style()
    } else if is_hovered && row.is_selectable {
        match row.style {
            TuiInlineMenuRowStyle::Default => {
                builder.muted_text_style().add_modifier(Modifier::BOLD)
            }
            TuiInlineMenuRowStyle::InlineMenuItem => {
                builder.primary_text_style().add_modifier(Modifier::BOLD)
            }
            TuiInlineMenuRowStyle::StateWithDetail => unreachable!(),
        }
    } else {
        match row.style {
            TuiInlineMenuRowStyle::Default => builder.muted_text_style(),
            TuiInlineMenuRowStyle::InlineMenuItem => builder.primary_text_style(),
            TuiInlineMenuRowStyle::StateWithDetail => unreachable!(),
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
            TuiInlineMenuRowStyle::StateWithDetail => unreachable!(),
        });
    if show_description {
        let mut description_spans = Vec::new();
        if let Some(description) = row.description.as_ref() {
            let description_prefix = match row.style {
                TuiInlineMenuRowStyle::Default => format!("  {description}"),
                TuiInlineMenuRowStyle::InlineMenuItem => description.clone(),
                TuiInlineMenuRowStyle::StateWithDetail => unreachable!(),
            };
            description_spans.push((description_prefix, description_style));
        }
        if let Some(suffix) = &row.state_suffix {
            let suffix_style = match row.style {
                TuiInlineMenuRowStyle::Default => builder.key_connected_suffix_style(),
                TuiInlineMenuRowStyle::InlineMenuItem if is_selected => {
                    builder.slash_command_selection_state_suffix_style()
                }
                TuiInlineMenuRowStyle::InlineMenuItem => builder.success_glyph_style(),
                TuiInlineMenuRowStyle::StateWithDetail => unreachable!(),
            };
            description_spans.push((format!(" {suffix}"), suffix_style));
        }
        if let Some(suffix) = &row.promotional_suffix {
            let suffix_style = if is_selected {
                builder.selection_promotional_suffix_style()
            } else {
                builder.promotional_suffix_style()
            };
            description_spans.push((format!(" {suffix}"), suffix_style));
        }
        if !description_spans.is_empty() {
            content = content.child(TuiText::from_spans(description_spans).truncate().finish());
        }
    }
    let mut container = TuiContainer::new(content.finish());
    if is_selected {
        container = container.with_background(builder.slash_command_selection_background());
    }
    container.finish()
}

fn menu_state_with_detail_row(
    row: &TuiInlineMenuRow,
    is_selected: bool,
    is_hovered: bool,
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    let title_style = if is_selected {
        builder.slash_command_selection_text_style()
    } else if is_hovered {
        builder
            .slash_command_text_style()
            .add_modifier(Modifier::BOLD)
    } else {
        builder.slash_command_text_style()
    };
    let suffix_style = if is_selected {
        builder.slash_command_selection_state_suffix_style()
    } else {
        builder.success_glyph_style()
    };
    let description_style = if is_selected {
        builder.slash_command_selection_text_style()
    } else {
        builder.muted_text_style()
    };
    let label = TuiText::from_spans([
        (row.title.clone(), title_style),
        (
            row.state_suffix
                .as_ref()
                .map(|suffix| format!(" {suffix}"))
                .unwrap_or_default(),
            suffix_style,
        ),
        (" – ".to_owned(), description_style),
    ])
    .finish();
    let description = TuiText::new(row.description.clone().unwrap_or_default())
        .with_style(description_style)
        .finish();
    let mut container = TuiContainer::new(
        TuiFlex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .child(label)
            .flex_child(description)
            .finish(),
    );
    if is_selected {
        container = container.with_background(builder.slash_command_selection_background());
    }
    container.finish()
}

#[cfg(test)]
#[path = "inline_menu_tests.rs"]
mod tests;
