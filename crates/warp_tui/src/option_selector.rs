//! [`TuiOptionSelector`]: a reusable single-select option list for TUI
//! permission prompts, rendered from a frontend-neutral
//! [`OptionSnapshot`]. One configuration page may show a header (title, "n of
//! m" position, question) above a selectable option list with viewport
//! scrolling, optional Loading/Failed/Empty status rows, and an optional
//! custom-text footer editor.
//!
//! Enter, Numpad Enter, arrows, viewport-relative digits, printable
//! characters, clicks, and wheel scrolling are handled at the element level
//! since the selector is only rendered while its host is the active blocking
//! interaction. Escape remains host policy, with an element-level fallback
//! through [`TuiOptionSelector::handle_back`].
use std::collections::{HashMap, HashSet};

use warp::tui_export::{OptionBadge, OptionFooter, OptionRow, OptionSnapshot, OptionSourceStatus};
use warp_search_core::inline_menu::InlineMenuSelection;
use warpui_core::elements::MouseStateHandle;
use warpui_core::elements::tui::{
    Modifier, TuiChildView, TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiFlex,
    TuiHoverable, TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiParentElement,
    TuiPresentationContext, TuiScreenPoint, TuiScreenPosition, TuiSize, TuiStyle, TuiText,
};
use warpui_core::keymap::EditableBinding;
use warpui_core::keymap::macros::*;
use warpui_core::{
    AppContext, BlurContext, Entity, EntityId, FocusContext, TuiView, TypedActionView, ViewContext,
    ViewHandle,
};

use crate::editor_view::{
    TuiEditorVerticalDirection, TuiEditorView, TuiEditorViewAction, TuiEditorViewEvent,
};
use crate::inline_menu::keep_selected_visible;
use crate::keybindings::TUI_BINDING_GROUP;
use crate::tui_builder::TuiUiBuilder;

/// Maximum option rows visible at once; longer lists scroll.
pub(crate) const MAX_VISIBLE_OPTION_ROWS: usize = 6;

/// Validation copy shown when the custom-text editor is submitted empty.
const CUSTOM_TEXT_EMPTY_ERROR: &str = "Enter a value to continue.";
const SELECTOR_NAVIGATION_ACTIVE: &str = "TuiOptionSelectorNavigationActive";

pub(crate) fn init(app: &mut AppContext) {
    let predicate = id!(TuiOptionSelector::ui_name()) & id!(SELECTOR_NAVIGATION_ACTIVE);
    app.register_editable_bindings([
        EditableBinding::new(
            "tui:option-selector:previous",
            "Select the previous option",
            TuiOptionSelectorAction::MoveUp,
        )
        .with_context_predicate(predicate.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("up"),
        EditableBinding::new(
            "tui:option-selector:next",
            "Select the next option",
            TuiOptionSelectorAction::MoveDown,
        )
        .with_context_predicate(predicate)
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("down"),
    ]);
}

/// Optional header rendered above a selector page.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OptionSelectorHeader {
    /// Short field label shown in the header row.
    pub(crate) field_label: String,
    /// One-based position in the current page sequence: `(current, total)`.
    pub(crate) position: (usize, usize),
    /// Full prompt shown above the available options.
    pub(crate) prompt: String,
}

/// Renderable fields for one selector page.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OptionSelectorPage {
    /// Header metadata, or `None` when the embedding view provides its own.
    pub(crate) header: Option<OptionSelectorHeader>,
    /// Options and catalog status rendered on this page.
    pub(crate) snapshot: OptionSnapshot,
    /// Whether this page offers label filtering.
    pub(crate) searchable: bool,
    /// Non-numeric shortcuts keyed by option row id.
    pub(crate) row_shortcuts: HashMap<String, char>,
}

impl Default for OptionSelectorPage {
    fn default() -> Self {
        Self {
            header: None,
            snapshot: OptionSnapshot {
                rows: Vec::new(),
                selected_id: None,
                status: OptionSourceStatus::Ready,
                footer: None,
            },
            searchable: false,
            row_shortcuts: HashMap::new(),
        }
    }
}

/// Events emitted to the embedding card view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TuiOptionSelectorEvent {
    /// An enabled option row was confirmed.
    Confirmed { id: String },
    /// The custom-text footer editor was submitted with a valid value.
    CustomTextSubmitted { value: String },
    /// A checked question-card Other value was selected again and cleared.
    CustomTextCleared,
    /// The question-card Other editor was opened.
    CustomTextOpened,
    /// The custom-text editor was left without submitting a new value.
    CustomTextClosed,
    /// The Retry affordance of a `Failed` catalog was activated.
    RetryRequested,
    /// The selector asked to be dismissed (element-level Escape fallback for
    /// hosts without their own Escape binding).
    Dismissed,
    /// The selector's intrinsic height changed. `ctx.notify()` rerenders this
    /// view, but the block list may reuse a stable-width cached rich-content
    /// height. The host forwards this event so the containing rich-content
    /// item is marked dirty and remeasured.
    LayoutInvalidated,
}

/// User interactions dispatched from the selector's element tree.
#[derive(Clone, Debug)]
pub(crate) enum TuiOptionSelectorAction {
    /// Confirm the currently selected item.
    ConfirmSelected,
    MoveUp,
    MoveDown,
    /// Selects an item at an absolute index without confirming it.
    SelectItemWithoutConfirm(usize),
    /// Select the viewport-relative item and confirm it when enabled.
    SelectNumberedOption(u8),
    /// Select the row assigned to a host-defined shortcut.
    SelectShortcut(char),
    /// Select the item at an absolute index and confirm it when enabled.
    /// Dispatched by row clicks.
    SelectItem(usize),
    /// Scroll the viewport by whole rows without moving the selection.
    ScrollBy(isize),
    /// Move focus from the option list to search and seed its query.
    FocusSearchAndInsert(char),
    /// Handle contextual Escape behavior, falling back to dismissal.
    HandleEscape,
}

/// One navigable entry in the selector, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectorItem {
    /// Index into `snapshot.rows`.
    Row(usize),
    /// The Retry affordance shown for a `Failed` catalog.
    Retry,
    /// The custom-text footer entry point.
    CustomText,
}

/// The selector zone that currently owns real WarpUI focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectorFocusZone {
    List,
    Search,
    CustomText,
    LeadingEditor,
    None,
}

/// Editing phase for the custom-text footer.
#[derive(Default)]
enum CustomTextEditingState {
    #[default]
    Inactive,
    Active {
        error_visible: bool,
    },
}

impl CustomTextEditingState {
    /// Returns the active editor's validation state.
    fn error_visible(&self) -> Option<bool> {
        match self {
            Self::Inactive => None,
            Self::Active { error_visible } => Some(*error_visible),
        }
    }
}

/// Editor, committed value, and editing phase for a custom-text footer.
struct CustomTextState {
    editor: ViewHandle<TuiEditorView>,
    committed_value: Option<String>,
    editing: CustomTextEditingState,
}

impl CustomTextState {
    /// Creates inactive custom-text state around its editor view.
    fn new(editor: ViewHandle<TuiEditorView>) -> Self {
        Self {
            editor,
            committed_value: None,
            editing: CustomTextEditingState::Inactive,
        }
    }

    /// Whether the custom-text editor currently owns the interaction.
    fn is_editing(&self) -> bool {
        self.editing.error_visible().is_some()
    }

    /// Whether the active editor shows its validation error.
    fn error_is_visible(&self) -> bool {
        self.editing.error_visible() == Some(true)
    }

    /// Restores the committed value encoded by a snapshot.
    fn sync_committed_value(&mut self, snapshot: &OptionSnapshot) {
        self.committed_value = match (&snapshot.footer, &snapshot.selected_id) {
            (Some(OptionFooter::CustomText { .. }), Some(selected_id))
                if !snapshot.rows.iter().any(|row| row.id == *selected_id) =>
            {
                Some(selected_id.clone())
            }
            (
                Some(OptionFooter::CustomText { .. } | OptionFooter::CreateNewAuthSecret) | None,
                Some(_) | None,
            ) => None,
        };
    }

    /// Resets editing and synchronizes the editor with the committed value.
    fn reset_editor(&mut self, ctx: &mut ViewContext<TuiOptionSelector>) {
        self.editing = CustomTextEditingState::Inactive;
        let value = self.committed_value.clone().unwrap_or_default();
        self.editor
            .update(ctx, |editor, ctx| editor.set_text(value, ctx));
    }

    /// Activates the editor with the last committed value.
    fn begin_editing(&mut self, ctx: &mut ViewContext<TuiOptionSelector>) {
        let value = self.committed_value.clone().unwrap_or_default();
        self.editor
            .update(ctx, |editor, ctx| editor.set_text(value, ctx));
        self.editing = CustomTextEditingState::Active {
            error_visible: false,
        };
    }

    /// Cancels editing and returns whether a validation row was removed.
    fn cancel_editing(&mut self) -> Option<bool> {
        let error_visible = self.editing.error_visible()?;
        self.editing = CustomTextEditingState::Inactive;
        Some(error_visible)
    }

    /// Shows the validation error and returns whether layout changed.
    fn show_validation_error(&mut self) -> bool {
        if self.error_is_visible() {
            return false;
        };
        self.editing = CustomTextEditingState::Active {
            error_visible: true,
        };
        true
    }
    /// Clears a visible validation error and returns whether layout changed.
    fn clear_validation_error(&mut self) -> bool {
        if !self.error_is_visible() {
            return false;
        }
        self.editing = CustomTextEditingState::Active {
            error_visible: false,
        };
        true
    }

    /// Commits a value and returns whether a validation row was removed.
    fn commit(&mut self, value: String) -> bool {
        let error_visible = self.error_is_visible();
        self.editing = CustomTextEditingState::Inactive;
        self.committed_value = Some(value);
        error_visible
    }
}

/// Transient list and editor state reset when a page is replaced.
#[derive(Default)]
struct SelectorInteractionState {
    selection: InlineMenuSelection,
    scroll_offset: usize,
    search_query: String,
}

/// A reusable single-select option list view. See the module docs.
pub(crate) struct TuiOptionSelector {
    page: OptionSelectorPage,
    interaction: SelectorInteractionState,
    /// Optional host-owned editor rendered above the options.
    leading_editor: Option<ViewHandle<TuiEditorView>>,
    /// Validation copy rendered directly below the host-owned editor.
    leading_editor_error: Option<String>,
    search_field: Option<ViewHandle<TuiEditorView>>,
    custom_text: CustomTextState,
    /// Selected question option ids, independent of the keyboard highlight.
    selected_ids: HashSet<String>,
    /// Whether selected question options render checkmarks.
    show_selection_markers: bool,
    /// Whether this selector is embedded in an AskUserQuestion card.
    question_style: bool,
    /// Whether the selector itself (the list zone) is focused.
    focused: bool,
    /// Per-item mouse state, indexed like [`Self::items`]. Owned here (not
    /// created inline during render) so hover/click state survives
    /// element-tree rebuilds.
    item_mouse_states: Vec<MouseStateHandle>,
}

impl TuiOptionSelector {
    /// Creates an empty selector; hosts call [`Self::set_page`] before render.
    pub(crate) fn new(ctx: &mut ViewContext<Self>) -> Self {
        let custom_text_editor = ctx.add_typed_action_tui_view(TuiEditorView::single_line);
        ctx.subscribe_to_view(&custom_text_editor, |me, _, event, ctx| {
            let TuiEditorViewEvent::Changed(_) = event;
            if me.custom_text.clear_validation_error() {
                me.invalidate_layout(ctx);
            } else {
                ctx.notify();
            }
        });

        Self {
            page: OptionSelectorPage::default(),
            interaction: SelectorInteractionState::default(),
            leading_editor: None,
            leading_editor_error: None,
            search_field: None,
            custom_text: CustomTextState::new(custom_text_editor),
            selected_ids: HashSet::new(),
            show_selection_markers: false,
            question_style: false,
            focused: false,
            item_mouse_states: Vec::new(),
        }
    }

    /// Creates and subscribes to the optional search editor.
    fn add_search_field(ctx: &mut ViewContext<Self>) -> ViewHandle<TuiEditorView> {
        let search_field = ctx.add_typed_action_tui_view(TuiEditorView::single_line);
        ctx.subscribe_to_view(&search_field, |me, _, event, ctx| {
            let TuiEditorViewEvent::Changed(query) = event;
            me.interaction.search_query = query.clone();
            me.interaction.selection.clear();
            me.interaction.scroll_offset = 0;
            me.sync_after_items_changed();
            me.invalidate_layout(ctx);
        });
        search_field
    }

    /// Notifies this view and any host that caches its intrinsic height.
    fn invalidate_layout(&self, ctx: &mut ViewContext<Self>) {
        ctx.emit(TuiOptionSelectorEvent::LayoutInvalidated);
        ctx.notify();
    }

    /// Installs a host-owned editor above the option list.
    pub(crate) fn set_leading_editor(
        &mut self,
        editor: ViewHandle<TuiEditorView>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.leading_editor = Some(editor);
        self.invalidate_layout(ctx);
    }
    /// Updates validation copy shown below the host-owned editor.
    pub(crate) fn set_leading_editor_error(
        &mut self,
        error: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.leading_editor_error == error {
            return;
        }
        self.leading_editor_error = error;
        self.invalidate_layout(ctx);
    }

    /// Resolves all selector-owned focus predicates through one exhaustive zone.
    fn focus_zone(&self, ctx: &AppContext) -> SelectorFocusZone {
        if self.focused {
            SelectorFocusZone::List
        } else if self
            .search_field
            .as_ref()
            .is_some_and(|field| field.as_ref(ctx).is_focused())
        {
            SelectorFocusZone::Search
        } else if self.custom_text.editor.as_ref(ctx).is_focused() {
            SelectorFocusZone::CustomText
        } else if self
            .leading_editor
            .as_ref()
            .is_some_and(|editor| editor.as_ref(ctx).is_focused())
        {
            SelectorFocusZone::LeadingEditor
        } else {
            SelectorFocusZone::None
        }
    }

    /// The editor that participates in top-of-list focus cycling.
    fn top_editor(&self) -> Option<&ViewHandle<TuiEditorView>> {
        self.leading_editor.as_ref().or_else(|| {
            self.page
                .searchable
                .then_some(self.search_field.as_ref())
                .flatten()
        })
    }

    /// Moves focus from the option list to its top editor.
    pub(crate) fn focus_leading_editor(&self, ctx: &mut ViewContext<Self>) {
        if let Some(editor) = self.top_editor() {
            ctx.focus(editor);
            ctx.notify();
        }
    }

    /// Resets all transient interaction state for the newly installed page.
    fn reset_interaction_for_page(&mut self, ctx: &mut ViewContext<Self>) {
        let custom_text_was_focused = matches!(self.focus_zone(ctx), SelectorFocusZone::CustomText);
        self.interaction = SelectorInteractionState::default();
        if let Some(search_field) = self.search_field.as_ref() {
            search_field.update(ctx, |editor, ctx| editor.set_text("", ctx));
        }
        self.custom_text.reset_editor(ctx);
        self.select_id(self.page.snapshot.selected_id.clone());
        self.sync_after_items_changed();
        if custom_text_was_focused {
            ctx.focus_self();
        }
    }

    /// Replaces the current page and resets its transient interaction state.
    pub(crate) fn set_page(&mut self, page: OptionSelectorPage, ctx: &mut ViewContext<Self>) {
        if page.searchable && self.search_field.is_none() {
            self.search_field = Some(Self::add_search_field(ctx));
        }
        self.page = page;
        self.custom_text.sync_committed_value(&self.page.snapshot);
        self.reset_interaction_for_page(ctx);
        self.invalidate_layout(ctx);
    }

    /// Adapts this selector for an ask-question card. The questionnaire owns
    /// the committed selections; the selector retains its keyboard highlight.
    pub(crate) fn set_question_state(
        &mut self,
        selected_ids: HashSet<String>,
        show_selection_markers: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.question_style = true;
        self.selected_ids = selected_ids;
        self.show_selection_markers = show_selection_markers;
        ctx.notify();
    }

    /// The highlighted question-option index, including the trailing Other
    /// entry when present.
    pub(crate) fn highlighted_question_index(&self) -> Option<usize> {
        (!self.custom_text.is_editing())
            .then(|| self.interaction.selection.selected_index())
            .flatten()
    }

    /// Whether host shortcuts scoped to the bare option list should be active.
    pub(crate) fn list_is_focused(&self, ctx: &AppContext) -> bool {
        matches!(self.focus_zone(ctx), SelectorFocusZone::List)
    }

    /// Whether the host-owned leading editor currently owns focus.
    pub(crate) fn leading_editor_is_focused(&self, ctx: &AppContext) -> bool {
        matches!(self.focus_zone(ctx), SelectorFocusZone::LeadingEditor)
    }

    /// The highlighted item index, including a trailing custom-text entry.
    #[cfg(test)]
    pub(crate) fn highlighted_index(&self) -> Option<usize> {
        (!self.custom_text.is_editing())
            .then(|| self.interaction.selection.selected_index())
            .flatten()
    }

    #[cfg(test)]
    pub(crate) fn search_field_for_test(&self) -> Option<ViewHandle<TuiEditorView>> {
        self.search_field.clone()
    }

    /// Moves focus and highlight to an item without confirming it.
    pub(crate) fn select_item_without_confirm(
        &mut self,
        index: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        let items_len = self.items().len();
        if index >= items_len {
            return;
        }
        self.interaction
            .selection
            .select(index, items_len, |_| true);
        ctx.focus_self();
        self.scroll_to_keep_visible(items_len, index, ctx);
        ctx.notify();
    }

    /// The current inline Other buffer, trimmed for questionnaire transitions.
    pub(crate) fn active_custom_text(&self, ctx: &AppContext) -> Option<String> {
        self.custom_text.is_editing().then(|| {
            self.custom_text
                .editor
                .as_ref(ctx)
                .text(ctx)
                .trim()
                .to_owned()
        })
    }

    #[cfg(test)]
    pub(crate) fn set_active_custom_text_for_test(
        &mut self,
        text: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.custom_text.is_editing() {
            self.custom_text
                .editor
                .update(ctx, |editor, ctx| editor.set_text(text, ctx));
        }
    }

    /// Refreshes the snapshot in place after a live catalog change,
    /// preserving the active selection when it still exists and falling back
    /// to the snapshot's committed selection otherwise.
    pub(crate) fn refresh_snapshot(
        &mut self,
        snapshot: OptionSnapshot,
        ctx: &mut ViewContext<Self>,
    ) {
        let selected = self.selected_row_id();
        self.page.snapshot = snapshot;
        self.custom_text.sync_committed_value(&self.page.snapshot);
        let target = selected
            .filter(|id| self.page.snapshot.rows.iter().any(|row| &row.id == id))
            .or_else(|| self.page.snapshot.selected_id.clone());
        if matches!(self.focus_zone(ctx), SelectorFocusZone::Search) {
            self.interaction.selection.clear();
        } else {
            self.select_id(target);
        }
        self.sync_after_items_changed();
        // A refreshed catalog can change the row count and thus the height.
        self.invalidate_layout(ctx);
    }

    /// Scrolls to keep `selected` visible, announcing the scroll change (it
    /// toggles overflow markers, so the height may change) to the host.
    fn scroll_to_keep_visible(
        &mut self,
        items_len: usize,
        selected: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        let before = self.interaction.scroll_offset;
        keep_selected_visible(
            items_len,
            selected,
            MAX_VISIBLE_OPTION_ROWS,
            &mut self.interaction.scroll_offset,
        );
        if self.interaction.scroll_offset != before {
            ctx.emit(TuiOptionSelectorEvent::LayoutInvalidated);
        }
    }

    /// Confirms the selected item (Enter): enabled rows emit
    /// [`TuiOptionSelectorEvent::Confirmed`]; disabled rows are kept
    /// selected so their reason stays visible. While the
    /// custom-text editor is active, Enter validates and submits it instead
    ///.
    pub(crate) fn confirm_selected(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        if self.custom_text.is_editing() {
            return self.submit_custom_text(ctx);
        }
        if matches!(self.focus_zone(ctx), SelectorFocusZone::Search) {
            if let Some(index) = self.items().iter().position(|item| {
                matches!(item, SelectorItem::Row(_)) && self.item_is_confirmable(*item)
            }) {
                return self.confirm_item(index, ctx);
            }
            return false;
        }
        let Some(index) = self.interaction.selection.selected_index() else {
            return false;
        };
        self.confirm_item(index, ctx)
    }

    /// Cancels active custom-text editing, returning whether it consumed Back.
    fn cancel_custom_text_editing(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let Some(layout_changed) = self.custom_text.cancel_editing() else {
            return false;
        };

        ctx.focus_self();
        ctx.emit(TuiOptionSelectorEvent::CustomTextClosed);
        if layout_changed {
            self.invalidate_layout(ctx);
        } else {
            ctx.notify();
        }
        true
    }

    /// Activates the custom editor with the last committed value.
    fn begin_custom_text_editing(&mut self, ctx: &mut ViewContext<Self>) {
        self.custom_text.begin_editing(ctx);
        if self.question_style {
            ctx.emit(TuiOptionSelectorEvent::CustomTextOpened);
        }
        ctx.focus(&self.custom_text.editor);
        ctx.notify();
    }

    /// Shows the custom-text validation error when it is not already visible.
    fn show_custom_text_validation_error(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.custom_text.show_validation_error() {
            ctx.notify();
            return;
        }
        self.invalidate_layout(ctx);
    }

    /// Commits a validated custom-text value and exits its editor.
    fn commit_custom_text(&mut self, value: String, ctx: &mut ViewContext<Self>) {
        let layout_changed = self.custom_text.commit(value.clone());
        self.page.snapshot.selected_id = Some(value.clone());
        ctx.focus_self();
        ctx.emit(TuiOptionSelectorEvent::CustomTextSubmitted { value });
        if layout_changed {
            self.invalidate_layout(ctx);
        } else {
            ctx.notify();
        }
    }

    /// Clears a committed question-card Other value without opening its editor.
    fn clear_custom_text(&mut self, ctx: &mut ViewContext<Self>) {
        self.custom_text.committed_value = None;
        self.custom_text.editing = CustomTextEditingState::Inactive;
        self.custom_text
            .editor
            .update(ctx, |editor, ctx| editor.set_text("", ctx));
        self.page.snapshot.selected_id = self
            .page
            .snapshot
            .rows
            .iter()
            .find(|row| self.selected_ids.contains(&row.id))
            .map(|row| row.id.clone());
        ctx.focus_self();
        ctx.emit(TuiOptionSelectorEvent::CustomTextCleared);
        ctx.notify();
    }

    /// Clears focused search text, returning whether it consumed Back.
    fn clear_focused_search(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        if !matches!(self.focus_zone(ctx), SelectorFocusZone::Search)
            || self.interaction.search_query.is_empty()
        {
            return false;
        }

        self.interaction.search_query.clear();
        if let Some(search_field) = self.search_field.as_ref() {
            search_field.update(ctx, |field, ctx| field.set_text("", ctx));
        }
        self.interaction.scroll_offset = 0;
        self.sync_after_items_changed();
        self.invalidate_layout(ctx);
        true
    }

    /// Handles Escape from the embedding card, consuming it when the selector
    /// has an active editor interaction to unwind.
    pub(crate) fn handle_back(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        self.cancel_custom_text_editing(ctx) || self.clear_focused_search(ctx)
    }

    /// The navigable entries, in display order.
    fn items(&self) -> Vec<SelectorItem> {
        let query = self.interaction.search_query.to_lowercase();
        let mut items: Vec<SelectorItem> = (0..self.page.snapshot.rows.len())
            .filter(|index| {
                query.is_empty()
                    || self.page.snapshot.rows[*index]
                        .label
                        .to_lowercase()
                        .contains(&query)
            })
            .map(SelectorItem::Row)
            .collect();
        if matches!(self.page.snapshot.status, OptionSourceStatus::Failed { .. }) {
            items.push(SelectorItem::Retry);
        }
        match &self.page.snapshot.footer {
            Some(OptionFooter::CustomText { .. }) => items.push(SelectorItem::CustomText),
            // Resource creation is out of scope in the TUI.
            Some(OptionFooter::CreateNewAuthSecret) | None => {}
        }
        items
    }

    /// Whether the item can be confirmed. Disabled rows stay selectable
    /// but unconfirmable.
    fn item_is_confirmable(&self, item: SelectorItem) -> bool {
        match item {
            SelectorItem::Row(index) => self
                .page
                .snapshot
                .rows
                .get(index)
                .is_some_and(|row| row.disabled_reason.is_none()),
            SelectorItem::Retry | SelectorItem::CustomText => true,
        }
    }

    /// The row id currently selected, when the selection is on a row.
    fn selected_row_id(&self) -> Option<String> {
        let items = self.items();
        match self
            .interaction
            .selection
            .selected_index()
            .and_then(|i| items.get(i))
        {
            Some(SelectorItem::Row(index)) => self
                .page
                .snapshot
                .rows
                .get(*index)
                .map(|row| row.id.clone()),
            Some(SelectorItem::Retry) | Some(SelectorItem::CustomText) | None => None,
        }
    }

    /// Moves the selection to the row with `id`, else the first item.
    fn select_id(&mut self, id: Option<String>) {
        let items = self.items();
        let target = id
            .and_then(|id| {
                items.iter().position(|item| match item {
                    SelectorItem::Row(index) => self
                        .page
                        .snapshot
                        .rows
                        .get(*index)
                        .is_some_and(|row| row.id == id),
                    SelectorItem::CustomText => {
                        self.custom_text.committed_value.as_ref() == Some(&id)
                    }
                    SelectorItem::Retry => false,
                })
            })
            .or(if items.is_empty() { None } else { Some(0) });
        self.interaction.selection.clear();
        if let Some(target) = target {
            self.interaction
                .selection
                .select(target, items.len(), |_| true);
        }
    }

    /// Clamps scroll state and mouse-handle storage to the current items.
    fn sync_after_items_changed(&mut self) {
        let items_len = self.items().len();
        self.interaction.scroll_offset = self
            .interaction
            .scroll_offset
            .min(items_len.saturating_sub(MAX_VISIBLE_OPTION_ROWS));
        if let Some(selected) = self.interaction.selection.selected_index() {
            keep_selected_visible(
                items_len,
                selected,
                MAX_VISIBLE_OPTION_ROWS,
                &mut self.interaction.scroll_offset,
            );
        }
        // Handles are stable per item index across renders; grow as needed.
        while self.item_mouse_states.len() < items_len {
            self.item_mouse_states.push(MouseStateHandle::default());
        }
    }

    /// Moves the selection one step, scrolling to keep it visible.
    fn move_selection(&mut self, forward: bool, ctx: &mut ViewContext<Self>) {
        let items_len = self.items().len();
        match self.focus_zone(ctx) {
            SelectorFocusZone::CustomText => {
                self.cancel_custom_text_editing(ctx);
            }
            SelectorFocusZone::Search => {
                self.focus_list_boundary(forward, items_len, ctx);
                return;
            }
            SelectorFocusZone::LeadingEditor => {
                let direction = if forward {
                    TuiEditorVerticalDirection::Down
                } else {
                    TuiEditorVerticalDirection::Up
                };
                let moved_within_editor = self.leading_editor.as_ref().is_some_and(|editor| {
                    if !editor.as_ref(ctx).can_move_vertically(direction, ctx) {
                        return false;
                    }
                    editor.update(ctx, |editor, ctx| {
                        editor.handle_action(&TuiEditorViewAction::Command(direction.into()), ctx);
                    });
                    true
                });
                if moved_within_editor {
                    return;
                }
                self.focus_list_boundary(forward, items_len, ctx);
                return;
            }
            SelectorFocusZone::List | SelectorFocusZone::None => {}
        }
        let move_to_editor = self.top_editor().is_some()
            && match (forward, self.interaction.selection.selected_index()) {
                (false, None | Some(0)) => true,
                (true, Some(index)) => index + 1 >= items_len,
                (true, None) | (false, Some(_)) => false,
            };
        if move_to_editor {
            self.interaction.selection.clear();
            self.interaction.scroll_offset = 0;
            self.focus_leading_editor(ctx);
            ctx.notify();
            return;
        }
        if forward {
            self.interaction.selection.select_next(items_len, |_| true);
        } else {
            self.interaction
                .selection
                .select_previous(items_len, |_| true);
        }
        if let Some(selected) = self.interaction.selection.selected_index() {
            self.scroll_to_keep_visible(items_len, selected, ctx);
        }
        ctx.notify();
    }

    /// Focuses the first or last item when leaving an editor above the list.
    fn focus_list_boundary(
        &mut self,
        forward: bool,
        items_len: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        if items_len > 0 {
            let target = if forward { 0 } else { items_len - 1 };
            self.interaction
                .selection
                .select(target, items_len, |_| true);
            ctx.focus_self();
            self.scroll_to_keep_visible(items_len, target, ctx);
        }
        ctx.notify();
    }

    /// Confirms the item at `index` when enabled; otherwise selects it so
    /// its disabled reason is surfaced.
    fn confirm_item(&mut self, index: usize, ctx: &mut ViewContext<Self>) -> bool {
        let items = self.items();
        let Some(item) = items.get(index).copied() else {
            return false;
        };
        self.interaction
            .selection
            .select(index, items.len(), |_| true);
        self.scroll_to_keep_visible(items.len(), index, ctx);
        if !self.item_is_confirmable(item) {
            ctx.notify();
            return false;
        }
        match item {
            SelectorItem::Row(row_index) => {
                if let Some(row) = self.page.snapshot.rows.get(row_index) {
                    ctx.emit(TuiOptionSelectorEvent::Confirmed { id: row.id.clone() });
                }
            }
            SelectorItem::Retry => ctx.emit(TuiOptionSelectorEvent::RetryRequested),
            SelectorItem::CustomText => {
                if self.show_selection_markers && self.custom_text.committed_value.is_some() {
                    self.clear_custom_text(ctx);
                    return true;
                }
                self.begin_custom_text_editing(ctx);
                return true;
            }
        }
        ctx.notify();
        true
    }

    /// Confirms the visible row assigned to `shortcut`.
    fn confirm_shortcut(&mut self, shortcut: char, ctx: &mut ViewContext<Self>) -> bool {
        let items = self.items();
        let visible_end =
            (self.interaction.scroll_offset + MAX_VISIBLE_OPTION_ROWS).min(items.len());
        let Some(index) = (self.interaction.scroll_offset..visible_end).find(|index| {
            let SelectorItem::Row(row_index) = items[*index] else {
                return false;
            };
            self.page
                .snapshot
                .rows
                .get(row_index)
                .and_then(|row| self.page.row_shortcuts.get(&row.id))
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(&shortcut))
        }) else {
            return false;
        };
        self.confirm_item(index, ctx)
    }

    /// Validates and submits the custom-text editor: the value
    /// is trimmed; empty input stays editable with a concise error.
    fn submit_custom_text(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        if !self.custom_text.is_editing() {
            return false;
        }
        let value = self
            .custom_text
            .editor
            .as_ref(ctx)
            .text(ctx)
            .trim()
            .to_string();
        if value.is_empty() {
            self.show_custom_text_validation_error(ctx);
            false
        } else {
            self.commit_custom_text(value, ctx);
            true
        }
    }

    /// Scrolls the viewport by `rows` without moving the selection
    ///.
    fn scroll_by(&mut self, rows: isize, ctx: &mut ViewContext<Self>) {
        let items_len = self.items().len();
        let max_offset = items_len.saturating_sub(MAX_VISIBLE_OPTION_ROWS);
        let before = self.interaction.scroll_offset;
        self.interaction.scroll_offset = self
            .interaction
            .scroll_offset
            .saturating_add_signed(rows)
            .min(max_offset);
        if self.interaction.scroll_offset != before {
            self.invalidate_layout(ctx);
        } else {
            ctx.notify();
        }
    }

    // ── Rendering ───────────────────────────────────────────────────

    /// One header block: field label + position, then the page prompt.
    fn render_header(header: &OptionSelectorHeader, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
        let (current, total) = header.position;
        let title = TuiText::new(header.field_label.clone())
            .with_style(builder.primary_text_style())
            .truncate()
            .finish();
        let previous_style = if current > 1 {
            builder.primary_text_style()
        } else {
            builder.muted_text_style()
        };
        let next_style = if current < total {
            builder.primary_text_style()
        } else {
            builder.muted_text_style()
        };
        let position = TuiText::from_spans([
            ("←".to_string(), previous_style),
            (format!(" {current} "), builder.primary_text_style()),
            (format!("of {total} "), builder.muted_text_style()),
            ("→".to_string(), next_style),
        ])
        .truncate()
        .finish();
        let title_row = TuiFlex::row()
            .child(title)
            .flex_child(TuiFlex::row().finish())
            .child(position)
            .finish();
        TuiFlex::column()
            .child(title_row)
            .child(TuiText::new(" ").finish())
            .child(
                TuiText::new(header.prompt.clone())
                    .with_style(builder.primary_text_style().add_modifier(Modifier::BOLD))
                    .finish(),
            )
            .finish()
    }

    /// One option row: shortcut, label, badge, and disabled
    /// reason, with the current selection rendered in bold magenta.
    fn render_row(
        &self,
        row: &OptionRow,
        shortcut: Option<char>,
        is_highlighted: bool,
        builder: &TuiUiBuilder,
    ) -> Box<dyn TuiElement> {
        let disabled = row.disabled_reason.is_some();
        let is_selected = if self.question_style {
            self.selected_ids.contains(&row.id)
        } else {
            is_highlighted
        };
        let selected_style = if self.question_style {
            builder.question_option_selected_style()
        } else {
            builder.option_selector_selected_style()
        };
        let label_style = if is_highlighted || is_selected {
            selected_style
        } else if disabled {
            builder.dim_text_style()
        } else {
            builder.primary_text_style()
        };
        let detail_style = if is_highlighted {
            selected_style
        } else if disabled {
            builder.dim_text_style()
        } else {
            builder.muted_text_style()
        };
        let shortcut_prefix = match shortcut {
            Some(shortcut) => format!("({shortcut}) "),
            None => "    ".to_string(),
        };
        let mut spans = vec![(shortcut_prefix, detail_style)];
        if self.show_selection_markers {
            spans.push((
                if is_selected { "[✓] " } else { "[ ] " }.to_string(),
                if is_selected {
                    builder.success_glyph_style()
                } else {
                    builder.primary_text_style()
                },
            ));
        }
        spans.push((row.label.clone(), label_style));
        let badge = match row.badge {
            Some(OptionBadge::Default) => Some("default"),
            Some(OptionBadge::Recent) => Some("recent"),
            Some(OptionBadge::Connected) => Some("connected"),
            None => None,
        };
        if let Some(badge) = badge {
            spans.push((format!("  ({badge})"), detail_style));
        }
        if let Some(reason) = &row.disabled_reason {
            spans.push((format!(" — {reason}"), detail_style));
        }
        TuiText::from_spans(spans).truncate().finish()
    }

    /// A generic single-span selectable virtual row (Retry / custom text).
    fn render_virtual_row(
        &self,
        text: String,
        digit: Option<usize>,
        is_highlighted: bool,
        selection_marker: Option<bool>,
        style: TuiStyle,
        builder: &TuiUiBuilder,
    ) -> Box<dyn TuiElement> {
        let is_selected = selection_marker == Some(true);

        let selected_style = if self.question_style {
            builder.question_option_selected_style()
        } else {
            builder.option_selector_selected_style()
        };

        let label_style = if is_highlighted || is_selected {
            selected_style
        } else {
            style
        };

        let detail_style = if is_highlighted {
            selected_style
        } else if selection_marker.is_some() {
            builder.muted_text_style()
        } else {
            style
        };

        let digit_prefix = match digit {
            Some(digit) => format!("({digit}) "),
            None => "    ".to_string(),
        };
        let mut spans = vec![(digit_prefix, detail_style)];
        if let Some(is_selected) = selection_marker {
            spans.push((
                if is_selected { "[✓] " } else { "[ ] " }.to_string(),
                if is_selected {
                    builder.success_glyph_style()
                } else {
                    builder.primary_text_style()
                },
            ));
        }
        spans.push((text, label_style));
        TuiText::from_spans(spans).truncate().finish()
    }

    /// Renders selector-owned label/error chrome around a generic editor view.
    fn render_editor_field(
        &self,
        prefix: String,
        label: &str,
        editor: &ViewHandle<TuiEditorView>,
        error: Option<&str>,
        selection_marker: Option<bool>,
        builder: &TuiUiBuilder,
    ) -> Box<dyn TuiElement> {
        let mut spans = vec![(prefix, builder.muted_text_style())];
        if let Some(is_selected) = selection_marker {
            spans.push((
                if is_selected { "[✓] " } else { "[ ] " }.to_string(),
                if is_selected {
                    builder.success_glyph_style()
                } else {
                    builder.primary_text_style()
                },
            ));
        }
        spans.push((format!("{label}: "), builder.muted_text_style()));
        let label = TuiText::from_spans(spans).truncate().finish();
        let row = TuiFlex::row()
            .child(label)
            .flex_child(TuiChildView::new(editor).finish())
            .finish();
        let mut content = TuiFlex::column().child(row);
        if let Some(error) = error {
            content.add_child(
                TuiText::new(error.to_string())
                    .with_style(builder.error_text_style())
                    .truncate()
                    .finish(),
            );
        }
        content.finish()
    }

    /// The option list: visible window of items with digit prefixes, plus
    /// non-selectable status rows for Loading/Failed/Empty.
    fn render_list(&self, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
        let items = self.items();
        let mut column = TuiFlex::column();

        let visible_end =
            (self.interaction.scroll_offset + MAX_VISIBLE_OPTION_ROWS).min(items.len());
        let visible = self.interaction.scroll_offset..visible_end;
        if self.page.searchable
            && !self.interaction.search_query.is_empty()
            && !items
                .iter()
                .any(|item| matches!(item, SelectorItem::Row(_)))
        {
            column.add_child(
                TuiText::new("No matches")
                    .with_style(builder.dim_text_style())
                    .truncate()
                    .finish(),
            );
        }
        if self.interaction.scroll_offset > 0 {
            column.add_child(
                TuiText::new("↑")
                    .with_style(builder.dim_text_style())
                    .truncate()
                    .finish(),
            );
        }
        for (position, index) in visible.clone().enumerate() {
            let item = items[index];
            let digit = (position < 9).then_some(position + 1);
            let is_selected = !self.custom_text.is_editing()
                && self.interaction.selection.selected_index() == Some(index);
            let element = match item {
                SelectorItem::Row(row_index) => {
                    let Some(row) = self.page.snapshot.rows.get(row_index) else {
                        continue;
                    };
                    let shortcut = self
                        .page
                        .row_shortcuts
                        .get(&row.id)
                        .copied()
                        .or_else(|| digit.and_then(|digit| char::from_digit(digit as u32, 10)));
                    self.render_row(row, shortcut, is_selected, builder)
                }
                SelectorItem::Retry => self.render_virtual_row(
                    "↻ Retry".to_string(),
                    digit,
                    is_selected,
                    None,
                    builder.error_text_style(),
                    builder,
                ),
                SelectorItem::CustomText => {
                    match (&self.page.snapshot.footer, self.custom_text.is_editing()) {
                        (Some(OptionFooter::CustomText { label }), true) => self
                            .render_editor_field(
                                digit.map_or_else(
                                    || "    ".to_string(),
                                    |digit| format!("({digit}) "),
                                ),
                                label,
                                &self.custom_text.editor,
                                self.custom_text
                                    .error_is_visible()
                                    .then_some(CUSTOM_TEXT_EMPTY_ERROR),
                                self.show_selection_markers
                                    .then_some(self.custom_text.committed_value.is_some()),
                                builder,
                            ),
                        (Some(OptionFooter::CustomText { label }), false) => {
                            let custom_text_selected = self.custom_text.committed_value.is_some();
                            self.render_virtual_row(
                                self.custom_text
                                    .committed_value
                                    .clone()
                                    .unwrap_or_else(|| label.clone()),
                                digit,
                                is_selected,
                                self.show_selection_markers.then_some(custom_text_selected),
                                builder.primary_text_style(),
                                builder,
                            )
                        }
                        (Some(OptionFooter::CreateNewAuthSecret) | None, _) => continue,
                    }
                }
            };
            // Each visible row is clickable through its own persistent
            // mouse-state handle.
            let element = match self.item_mouse_states.get(index) {
                Some(mouse_state) => TuiHoverable::new(mouse_state.clone(), element)
                    .on_click(move |event_ctx, _| {
                        event_ctx.dispatch_typed_action(TuiOptionSelectorAction::SelectItem(index));
                    })
                    .finish(),
                None => element,
            };
            column.add_child(element);
        }
        if visible_end < items.len() {
            column.add_child(
                TuiText::new("↓")
                    .with_style(builder.dim_text_style())
                    .truncate()
                    .finish(),
            );
        }

        match &self.page.snapshot.status {
            OptionSourceStatus::Ready => {}
            OptionSourceStatus::Loading => {
                column.add_child(
                    TuiText::new("Loading…")
                        .with_style(builder.dim_text_style())
                        .truncate()
                        .finish(),
                );
            }
            OptionSourceStatus::Failed { message } => {
                column.add_child(
                    TuiText::new(message.clone())
                        .with_style(builder.error_text_style())
                        .truncate()
                        .finish(),
                );
            }
            OptionSourceStatus::Empty { message } => {
                column.add_child(
                    TuiText::new(message.clone())
                        .with_style(builder.dim_text_style())
                        .truncate()
                        .finish(),
                );
            }
        }
        column.finish()
    }
}

impl Entity for TuiOptionSelector {
    type Event = TuiOptionSelectorEvent;
}

impl TuiView for TuiOptionSelector {
    fn ui_name() -> &'static str {
        "TuiOptionSelector"
    }

    fn keymap_context(&self, app: &AppContext) -> warpui_core::keymap::Context {
        let mut context = Self::default_keymap_context();
        if matches!(
            self.focus_zone(app),
            SelectorFocusZone::List
                | SelectorFocusZone::Search
                | SelectorFocusZone::CustomText
                | SelectorFocusZone::LeadingEditor
        ) {
            context.set.insert(SELECTOR_NAVIGATION_ACTIVE);
        }
        context
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);
        let mut content = TuiFlex::column();
        if let Some(header) = &self.page.header {
            content.add_child(Self::render_header(header, &builder));
        }
        if let Some(leading_editor) = self.leading_editor.as_ref() {
            content.add_child(TuiChildView::new(leading_editor).finish());
            if let Some(error) = self.leading_editor_error.as_ref() {
                content.add_child(
                    TuiText::new(error.clone())
                        .with_style(builder.error_text_style())
                        .finish(),
                );
            }
            content.add_child(TuiText::new(" ").finish());
        } else if let Some(search_field) = self
            .page
            .searchable
            .then_some(self.search_field.as_ref())
            .flatten()
        {
            content.add_child(self.render_editor_field(
                String::new(),
                "Search",
                search_field,
                None,
                None,
                &builder,
            ));
        }
        content.add_child(self.render_list(&builder));
        SelectorInputElement {
            child: content.finish(),
            list_focused: matches!(self.focus_zone(app), SelectorFocusZone::List),
            searchable: self.page.searchable,
            row_shortcuts: self.page.row_shortcuts.values().copied().collect(),
        }
        .finish()
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<EntityId> {
        let mut ids = vec![self.custom_text.editor.id()];
        ids.extend(self.leading_editor.iter().map(ViewHandle::id));
        if self.page.searchable {
            ids.extend(self.search_field.iter().map(ViewHandle::id));
        }
        ids
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        match focus_ctx {
            FocusContext::SelfFocused => self.focused = true,
            FocusContext::DescendentFocused(view_id) => {
                self.focused = false;
                if self
                    .leading_editor
                    .as_ref()
                    .is_some_and(|editor| *view_id == editor.id())
                    || self
                        .search_field
                        .as_ref()
                        .is_some_and(|search_field| *view_id == search_field.id())
                {
                    self.interaction.selection.clear();
                }
            }
        }
        ctx.notify();
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            self.focused = false;
            ctx.notify();
        }
    }
}

impl TypedActionView for TuiOptionSelector {
    fn handle_action(&mut self, action: &TuiOptionSelectorAction, ctx: &mut ViewContext<Self>) {
        match action {
            TuiOptionSelectorAction::ConfirmSelected => {
                self.confirm_selected(ctx);
            }
            TuiOptionSelectorAction::MoveUp => self.move_selection(false, ctx),
            TuiOptionSelectorAction::MoveDown => self.move_selection(true, ctx),
            TuiOptionSelectorAction::SelectItemWithoutConfirm(index) => {
                self.select_item_without_confirm(*index, ctx);
            }
            TuiOptionSelectorAction::SelectNumberedOption(digit) => {
                let index = self.interaction.scroll_offset + usize::from(*digit) - 1;
                let item_has_custom_shortcut = self.items().get(index).is_some_and(|item| {
                    let SelectorItem::Row(row_index) = item else {
                        return false;
                    };
                    self.page
                        .snapshot
                        .rows
                        .get(*row_index)
                        .is_some_and(|row| self.page.row_shortcuts.contains_key(&row.id))
                });
                if !item_has_custom_shortcut {
                    self.confirm_item(index, ctx);
                }
            }
            TuiOptionSelectorAction::SelectShortcut(shortcut) => {
                self.confirm_shortcut(*shortcut, ctx);
            }
            TuiOptionSelectorAction::SelectItem(index) => {
                self.confirm_item(*index, ctx);
            }
            TuiOptionSelectorAction::ScrollBy(rows) => self.scroll_by(*rows, ctx),
            TuiOptionSelectorAction::FocusSearchAndInsert(c) => {
                if let Some(search_field) =
                    self.search_field.clone().filter(|_| self.page.searchable)
                {
                    self.interaction.search_query.push(*c);
                    let query = self.interaction.search_query.clone();
                    search_field.update(ctx, |field, ctx| field.set_text(query, ctx));
                    self.interaction.selection.clear();
                    self.interaction.scroll_offset = 0;
                    self.sync_after_items_changed();
                    ctx.focus(&search_field);
                    self.invalidate_layout(ctx);
                }
            }
            TuiOptionSelectorAction::HandleEscape => {
                if !self.handle_back(ctx) {
                    ctx.emit(TuiOptionSelectorEvent::Dismissed);
                }
            }
        }
    }

    type Action = TuiOptionSelectorAction;
}

/// Wraps the selector's rendered content and translates element-level input
/// (confirmation, arrows, digits, custom-text characters, wheel scrolling) into
/// [`TuiOptionSelectorAction`]s.
struct SelectorInputElement {
    child: Box<dyn TuiElement>,
    list_focused: bool,
    searchable: bool,
    row_shortcuts: Vec<char>,
}

impl TuiElement for SelectorInputElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        self.child.layout(constraint, ctx, app)
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.child.render(origin, surface, ctx);
    }

    fn size(&self) -> Option<TuiSize> {
        self.child.size()
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.child.origin()
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        app: &AppContext,
    ) -> bool {
        if self.child.dispatch_event(event, event_ctx, app) {
            return true;
        }
        match event {
            TuiEvent::KeyDown {
                keystroke, chars, ..
            } => {
                if keystroke.ctrl || keystroke.alt || keystroke.cmd || keystroke.meta {
                    return false;
                }
                match keystroke.key.as_str() {
                    "enter" | "numpadenter" if self.list_focused => {
                        event_ctx.dispatch_typed_action(TuiOptionSelectorAction::ConfirmSelected);
                        true
                    }
                    "escape" if self.list_focused => {
                        // Escape fallback for hosts without their own
                        // Escape keymap binding; the embedding card's
                        // `escape` binding normally consumes the key first.
                        event_ctx.dispatch_typed_action(TuiOptionSelectorAction::HandleEscape);
                        true
                    }
                    "up" if self.list_focused => {
                        event_ctx.dispatch_typed_action(TuiOptionSelectorAction::MoveUp);
                        true
                    }
                    "down" if self.list_focused => {
                        event_ctx.dispatch_typed_action(TuiOptionSelectorAction::MoveDown);
                        true
                    }
                    key if self.list_focused
                        && key.chars().next().is_some_and(|candidate| {
                            key.chars().count() == 1
                                && self
                                    .row_shortcuts
                                    .iter()
                                    .any(|shortcut| shortcut.eq_ignore_ascii_case(&candidate))
                        }) =>
                    {
                        let shortcut = key.chars().next().expect("checked one-character key");
                        event_ctx.dispatch_typed_action(TuiOptionSelectorAction::SelectShortcut(
                            shortcut,
                        ));
                        true
                    }
                    key if self.list_focused => match key.parse::<u8>() {
                        Ok(digit @ 1..=9) => {
                            event_ctx.dispatch_typed_action(
                                TuiOptionSelectorAction::SelectNumberedOption(digit),
                            );
                            true
                        }
                        Ok(_) => false,
                        Err(_) => {
                            let Some(c) = chars.chars().next().filter(|c| !c.is_control()) else {
                                return false;
                            };
                            if self.searchable {
                                event_ctx.dispatch_typed_action(
                                    TuiOptionSelectorAction::FocusSearchAndInsert(c),
                                );
                                true
                            } else {
                                false
                            }
                        }
                    },
                    _ => false,
                }
            }
            TuiEvent::ScrollWheel {
                position, delta, ..
            } => {
                let Some((origin, size)) = self.origin().zip(self.size()) else {
                    return false;
                };
                if !event_ctx.hit_test(origin, size, *position) {
                    return false;
                }
                let (_, rows) = *delta;
                if rows == 0 {
                    return false;
                }
                // Positive wheel delta scrolls the content up (toward the
                // start of the list), matching the transcript's scrollable.
                event_ctx.dispatch_typed_action(TuiOptionSelectorAction::ScrollBy(-rows));
                true
            }
            TuiEvent::Paste { .. } => false,
            TuiEvent::LeftMouseDown { .. }
            | TuiEvent::LeftMouseUp { .. }
            | TuiEvent::LeftMouseDragged { .. }
            | TuiEvent::MiddleMouseDown { .. }
            | TuiEvent::RightMouseDown { .. }
            | TuiEvent::MouseMoved { .. } => false,
        }
    }
}

#[cfg(test)]
#[path = "option_selector_tests.rs"]
mod tests;
