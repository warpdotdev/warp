//! Searchable `/conversations` menu state for the TUI.
//!
//! The model reads normalized entries from `AgentConversationsModel`, applies the TUI
//! selection policy and shared query ranking, and preserves stable selection and scrolling
//! across live model updates. It owns menu lifecycle and presentation only; the terminal
//! session revalidates an accepted entry before restoring it.

use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{
    agent_conversations_cloud_metadata_load_failed, query_conversation_entries,
    AgentConversationEntryId, AgentConversationListEntryState, AgentConversationsModel,
    AgentConversationsModelEvent, AgentManagementFilters, ConversationSelectionHandle, Harness,
    HarnessFilter,
};
use warp_editor::model::CoreEditorModel;
use warp_search_core::inline_menu::InlineMenuSelection;
use warpui_core::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity, WindowId};

use crate::inline_menu::{
    keep_selected_visible, result_row_capacity, TuiInlineMenuHeader, TuiInlineMenuRow,
    TuiInlineMenuRowStyle, TuiInlineMenuSnapshot, TuiInlineMenuStatus, MAX_INLINE_MENU_ROWS,
};

const MAX_VISIBLE_ROWS: usize = result_row_capacity(MAX_INLINE_MENU_ROWS, true, false);

#[derive(Debug, Clone, PartialEq, Eq)]
struct TuiConversationMenuRow {
    id: AgentConversationEntryId,
    title: String,
}

#[derive(Debug, Clone, Default)]
enum TuiConversationMenuState {
    #[default]
    Closed,
    Open {
        rows: Vec<TuiConversationMenuRow>,
        selection: InlineMenuSelection,
        scroll_offset: usize,
        is_loading: bool,
    },
}

/// Events emitted by the TUI conversation menu.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TuiConversationMenuEvent {
    Updated,
    CloudMetadataUnavailable,
}

/// Query, selection, and model-subscription state for `/conversations`.
pub(crate) struct TuiConversationMenuModel {
    input_editor: ModelHandle<CodeEditorModel>,
    conversation_selection: ConversationSelectionHandle,
    window_id: WindowId,
    state: TuiConversationMenuState,
    cloud_warning_shown: bool,
}

impl TuiConversationMenuModel {
    /// Creates a closed conversation menu and subscribes it to input/model changes.
    pub(crate) fn new(
        input_editor: ModelHandle<CodeEditorModel>,
        conversation_selection: ConversationSelectionHandle,
        window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&input_editor, |model, _, event, ctx| {
            if model.is_open() && matches!(event, CodeEditorModelEvent::ContentChanged { .. }) {
                model.refresh_rows(ctx);
            }
        });
        ctx.subscribe_to_model(
            &AgentConversationsModel::handle(ctx),
            |model, _, _: &AgentConversationsModelEvent, ctx| {
                if model.is_open() {
                    model.refresh_rows(ctx);
                }
            },
        );
        Self {
            input_editor,
            conversation_selection,
            window_id,
            state: TuiConversationMenuState::Closed,
            cloud_warning_shown: false,
        }
    }

    /// Returns whether the conversation menu is currently open.
    pub(crate) fn is_open(&self) -> bool {
        matches!(self.state, TuiConversationMenuState::Open { .. })
    }

    /// Opens the menu and registers it as an active conversation-list consumer.
    pub(crate) fn open(&mut self, ctx: &mut ModelContext<Self>) {
        if self.is_open() {
            return;
        }
        self.state = TuiConversationMenuState::Open {
            rows: Vec::new(),
            selection: InlineMenuSelection::default(),
            scroll_offset: 0,
            is_loading: true,
        };
        self.cloud_warning_shown = false;
        let window_id = self.window_id;
        let model_id = ctx.model_id();
        AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
            model.register_view_open(window_id, model_id, ctx);
        });
        self.refresh_rows(ctx);
    }

    /// Closes the menu and clears its query buffer.
    pub(crate) fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.is_open() {
            return;
        }
        self.close(ctx);
        self.input_editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
    }

    /// Moves selection to the previous row and keeps it visible.
    pub(crate) fn select_previous(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiConversationMenuState::Open {
            rows,
            selection,
            scroll_offset,
            ..
        } = &mut self.state
        else {
            return;
        };
        if let Some(index) = selection.select_previous(rows.len(), |_| true) {
            keep_selected_visible(rows.len(), index, MAX_VISIBLE_ROWS, scroll_offset);
        }
        ctx.emit(TuiConversationMenuEvent::Updated);
    }

    /// Moves selection to the next row and keeps it visible.
    pub(crate) fn select_next(&mut self, ctx: &mut ModelContext<Self>) {
        let TuiConversationMenuState::Open {
            rows,
            selection,
            scroll_offset,
            ..
        } = &mut self.state
        else {
            return;
        };
        if let Some(index) = selection.select_next(rows.len(), |_| true) {
            keep_selected_visible(rows.len(), index, MAX_VISIBLE_ROWS, scroll_offset);
        }
        ctx.emit(TuiConversationMenuEvent::Updated);
    }

    /// Returns the stable ID of the selected row without closing the menu.
    pub(crate) fn accept_selected(
        &mut self,
        _ctx: &mut ModelContext<Self>,
    ) -> Option<AgentConversationEntryId> {
        let selected_id = match &self.state {
            TuiConversationMenuState::Open {
                rows, selection, ..
            } => selection
                .selected_index()
                .and_then(|index| rows.get(index))
                .map(|row| row.id),
            TuiConversationMenuState::Closed => None,
        };
        selected_id
    }

    /// Returns the render snapshot for the open menu.
    pub(crate) fn snapshot(&self) -> Option<TuiInlineMenuSnapshot> {
        let TuiConversationMenuState::Open {
            rows,
            selection,
            scroll_offset,
            is_loading,
        } = &self.state
        else {
            return None;
        };
        let status = if rows.is_empty() {
            Some(if *is_loading {
                TuiInlineMenuStatus::Loading("Loading conversations…".to_owned())
            } else {
                TuiInlineMenuStatus::Empty("No conversations found".to_owned())
            })
        } else {
            None
        };
        Some(TuiInlineMenuSnapshot {
            header: Some(TuiInlineMenuHeader {
                title: Some("Conversations".to_owned()),
                tabs: Vec::new(),
            }),
            rows: rows
                .iter()
                .map(|row| TuiInlineMenuRow {
                    title: row.title.clone(),
                    description: None,
                    is_selectable: true,
                    style: TuiInlineMenuRowStyle::Default,
                })
                .collect(),
            selected_index: selection.selected_index(),
            scroll_offset: *scroll_offset,
            max_visible_rows: MAX_VISIBLE_ROWS,
            status,
        })
    }

    /// Closes the menu and unregisters its conversation-list consumer.
    fn close(&mut self, ctx: &mut ModelContext<Self>) {
        self.state = TuiConversationMenuState::Closed;
        let window_id = self.window_id;
        let model_id = ctx.model_id();
        AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
            model.register_view_closed(window_id, model_id, ctx);
        });
        ctx.emit(TuiConversationMenuEvent::Updated);
    }

    /// Rebuilds rows from the current query while preserving stable selection.
    fn refresh_rows(&mut self, ctx: &mut ModelContext<Self>) {
        let previous_id = match &self.state {
            TuiConversationMenuState::Open {
                rows, selection, ..
            } => selection
                .selected_index()
                .and_then(|index| rows.get(index))
                .map(|row| row.id),
            TuiConversationMenuState::Closed => return,
        };
        let previous_index = match &self.state {
            TuiConversationMenuState::Open { selection, .. } => selection.selected_index(),
            TuiConversationMenuState::Closed => None,
        };
        let conversations_model = AgentConversationsModel::as_ref(ctx);
        let is_loading = conversations_model.is_loading();
        let cloud_metadata_load_failed = agent_conversations_cloud_metadata_load_failed(ctx);
        let rows = if is_loading {
            Vec::new()
        } else {
            let filters = AgentManagementFilters {
                harness: HarnessFilter::Specific(Harness::Oz),
                ..Default::default()
            };
            let policy = self.conversation_selection.as_ref(ctx);
            let entries = conversations_model
                .get_entries(&filters, ctx)
                .into_iter()
                .filter(|entry| {
                    policy.classify_entry(entry, ctx) == AgentConversationListEntryState::Available
                })
                .collect();
            query_conversation_entries(entries, &input_text(&self.input_editor, ctx))
                .into_iter()
                .map(|result| TuiConversationMenuRow {
                    id: result.entry.id,
                    title: result.entry.display.title,
                })
                .collect()
        };

        let selection = reconcile_selection(&rows, previous_id, previous_index);
        let mut scroll_offset = 0;
        if let Some(index) = selection.selected_index() {
            keep_selected_visible(rows.len(), index, MAX_VISIBLE_ROWS, &mut scroll_offset);
        }
        self.state = TuiConversationMenuState::Open {
            rows,
            selection,
            scroll_offset,
            is_loading,
        };
        if cloud_metadata_load_failed && !self.cloud_warning_shown {
            self.cloud_warning_shown = true;
            ctx.emit(TuiConversationMenuEvent::CloudMetadataUnavailable);
        }
        ctx.emit(TuiConversationMenuEvent::Updated);
    }
}

/// Preserves selection by ID, falling back to the nearest valid index.
fn reconcile_selection(
    rows: &[TuiConversationMenuRow],
    previous_id: Option<AgentConversationEntryId>,
    previous_index: Option<usize>,
) -> InlineMenuSelection {
    let mut selection = InlineMenuSelection::default();
    let index = previous_id
        .and_then(|id| rows.iter().position(|row| row.id == id))
        .or_else(|| {
            (!rows.is_empty()).then(|| {
                previous_index
                    .unwrap_or(rows.len().saturating_sub(1))
                    .min(rows.len().saturating_sub(1))
            })
        });
    if let Some(index) = index {
        selection.select(index, rows.len(), |_| true);
    }
    selection
}

impl Entity for TuiConversationMenuModel {
    type Event = TuiConversationMenuEvent;
}

/// Returns the input editor's current plain text.
fn input_text(editor: &ModelHandle<CodeEditorModel>, app: &AppContext) -> String {
    let model = editor.as_ref(app);
    let buffer = model.content().as_ref(app);
    if buffer.is_empty() {
        String::new()
    } else {
        buffer.text().into_string()
    }
}

#[cfg(test)]
#[path = "conversation_menu_tests.rs"]
mod tests;
