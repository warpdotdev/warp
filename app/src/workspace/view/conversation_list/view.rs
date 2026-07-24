use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::{Arc, Mutex};

use pathfinder_geometry::vector::Vector2F;
use warp_core::features::FeatureFlag;
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::Icon;
use warp_editor::editor::NavigationKey;
use warpui::elements::{
    Border, ChildAnchor, ChildView, Clipped, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Element, Fill, Flex, FormattedTextElement, Hoverable, MainAxisAlignment,
    MainAxisSize, MouseStateHandle, OffsetPositioning, Padding, ParentAnchor, ParentElement,
    ParentOffsetBounds, Radius, SavePosition, ScrollStateHandle, Scrollable, ScrollableElement,
    ScrollbarWidth, Shrinkable, Stack, Text, UniformList, UniformListState,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::FixedBinding;
use warpui::keymap::macros::*;
use warpui::platform::Cursor;
use warpui::text_layout::TextAlignment;
use warpui::{
    AppContext, BlurContext, Entity, EntityId, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle, WindowId,
};

use super::view_model::{ConversationEntry, ConversationListViewModel};
use crate::ai::active_agent_views_model::{ActiveAgentViewsModel, ConversationOrTaskId};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_conversations_model::{
    AgentConversationEntryId, AgentConversationNavigationSubject, AgentConversationsModel,
};
use crate::ai::agent_management::telemetry::{AgentManagementTelemetryEvent, OpenedFrom};
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;
use crate::ai::conversation_rename::rename_conversation;
use crate::appearance::Appearance;
use crate::drive::sharing::ShareableObject;
use crate::drive::sharing::dialog::SharingDialog;
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys,
    PropagateHorizontalNavigationKeys, SingleLineEditorOptions, TextOptions,
};
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields};
use crate::server::telemetry::SharingDialogSource;
use crate::view_components::DismissibleToast;
use crate::view_components::action_button::{ActionButton, ButtonSize, SecondaryTheme};
use crate::workspace::global_actions::ForkedConversationDestination;
use crate::workspace::header_toolbar_item::HeaderToolbarItemKind;
use crate::workspace::tab_settings::TabSettings;
use crate::workspace::view::conversation_list::item::{
    ItemProps, ItemState, OverflowMenuDisplay, STATIC_ITEM_MIN_HEIGHT, StaticItemProps,
    render_item, render_static_item,
};
use crate::workspace::{ToastStack, WorkspaceAction};

const VIEW_ALL_LABEL: &str = "View all";
/// Maximum number of past items to show before the user toggles "view all".
const INITIAL_MAX_PAST_ITEMS: usize = 10;

/// State handles for tracking UI state (hover, scroll, list selection, etc.).
struct StateHandles {
    list_state: UniformListState,
    scroll_state: ScrollStateHandle,
    item_states: HashMap<AgentConversationEntryId, ItemState>,
    start_new_conversation_item: ItemState,
    list_hover: MouseStateHandle,
    zero_state_button: MouseStateHandle,
    active_header: MouseStateHandle,
    past_header: MouseStateHandle,
}

impl Default for StateHandles {
    fn default() -> Self {
        Self {
            list_state: UniformListState::new(),
            scroll_state: Arc::new(Mutex::new(Default::default())),
            item_states: HashMap::new(),
            start_new_conversation_item: ItemState::default(),
            list_hover: MouseStateHandle::default(),
            zero_state_button: MouseStateHandle::default(),
            active_header: MouseStateHandle::default(),
            past_header: MouseStateHandle::default(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ConversationSection {
    Active,
    Past,
}

/// Represents an item in the uniform list - either a section header or a conversation.
#[derive(Clone, Debug)]
enum ListItem {
    SectionHeader(ConversationSection),
    Conversation {
        entry: ConversationEntry,
        /// The section the conversation is rendered under.
        section: ConversationSection,
    },
    /// The "+ New conversation" item at the end of the active section.
    StartNewConversation,
    ToggleViewAllButton,
}

#[derive(Clone, Copy)]
struct OverflowMenuState {
    conversation_id: AgentConversationEntryId,
    /// When `Some`, the menu was opened via right-click and should be
    /// positioned at the cursor location rather than the kebab button.
    position: Option<Vector2F>,
}

#[derive(Clone, Debug)]
pub enum ConversationListViewAction {
    DeleteConversation {
        conversation_id: AIConversationId,
        terminal_view_id: Option<EntityId>,
    },
    ToggleOverflowMenu {
        conversation_id: AgentConversationEntryId,
        /// When `Some`, the menu was opened via right-click and should be
        /// positioned where the right click took place.
        position: Option<Vector2F>,
    },
    OpenShareDialog {
        conversation_id: AgentConversationEntryId,
    },
    DeleteFromOverflowMenu {
        conversation_id: AgentConversationEntryId,
    },
    OpenItem {
        id: AgentConversationEntryId,
    },
    ArrowUp,
    ArrowDown,
    Enter,
    SetSelectedIndex(usize),
    ClearSelectedIndex,
    NewConversationInNewTab,
    ToggleSection(ConversationSection),
    ToggleViewAll,
    ForkConversation {
        conversation_id: AgentConversationEntryId,
        destination: ForkedConversationDestination,
    },
    StartRename {
        id: AgentConversationEntryId,
    },
    FinishRename,
    CancelRename,
}

pub enum Event {
    NewConversationInNewTab,
    ShowDeleteConfirmationDialog {
        conversation_id: AIConversationId,
        conversation_title: String,
        terminal_view_id: Option<EntityId>,
    },
}

pub struct ConversationListView {
    window_id: WindowId,
    view_id: EntityId,
    view_model: ModelHandle<ConversationListViewModel>,
    query_editor: ViewHandle<EditorView>,
    toggle_view_all_button: ViewHandle<ActionButton>,
    item_overflow_menu: ViewHandle<Menu<ConversationListViewAction>>,
    /// Tracks the overflow menu state (which item it's open for and where to position it).
    overflow_menu_state: Option<OverflowMenuState>,
    /// Sharing dialog for conversations.
    sharing_dialog: ViewHandle<SharingDialog>,
    rename_editor: ViewHandle<EditorView>,
    renaming_conversation_id: Option<AIConversationId>,
    /// Track which conversation the share dialog is open for.
    share_dialog_open_for: Option<AgentConversationEntryId>,
    selected_index: Option<usize>,
    collapsed_sections: HashSet<ConversationSection>,
    /// Cached flat list of items (headers + conversations) for rendering and navigation.
    /// Rebuilt when model data changes or collapse state changes.
    list_items: Arc<Vec<ListItem>>,
    /// Whether to show all past items or truncate to INITIAL_MAX_PAST_ITEMS.
    view_all: bool,
    /// Total number of past items before truncation
    /// (we use this to decide whether or not to show the view all button).
    total_past_items: usize,
    state_handles: StateHandles,
}

pub fn register_conversation_list_view_bindings(app: &mut AppContext) {
    app.register_fixed_bindings([
        FixedBinding::new(
            "up",
            ConversationListViewAction::ArrowUp,
            id!(ConversationListView::ui_name()),
        ),
        FixedBinding::new(
            "down",
            ConversationListViewAction::ArrowDown,
            id!(ConversationListView::ui_name()),
        ),
        FixedBinding::new(
            "enter",
            ConversationListViewAction::Enter,
            id!(ConversationListView::ui_name()),
        ),
    ]);
}

impl ConversationListView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let view_model = ctx.add_model(ConversationListViewModel::new);

        ctx.subscribe_to_model(&view_model, |me, _, _, ctx| {
            me.sync_list_items(ctx);
        });

        let active_agent_views_model = ActiveAgentViewsModel::handle(ctx);
        ctx.subscribe_to_model(&active_agent_views_model, |me, _, _, ctx| {
            me.sync_list_items(ctx);
        });

        // Editor for the search query.
        let query_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(14.), appearance),
                    select_all_on_focus: true,
                    clear_selections_on_blur: true,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    propagate_horizontal_navigation_keys: PropagateHorizontalNavigationKeys::Always,
                    ..Default::default()
                },
                ctx,
            );

            editor.set_placeholder_text("Search", ctx);
            editor
        });
        ctx.subscribe_to_view(&query_editor, |me, _handle, event, ctx| {
            me.handle_query_editor_event(event, ctx);
        });
        let rename_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(appearance.ui_font_size() + 2.), appearance),
                    select_all_on_focus: true,
                    clear_selections_on_blur: true,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    propagate_horizontal_navigation_keys: PropagateHorizontalNavigationKeys::Always,
                    ..Default::default()
                },
                ctx,
            )
        });
        ctx.subscribe_to_view(&rename_editor, |me, _, event, ctx| match event {
            EditorEvent::Blurred | EditorEvent::Enter => {
                me.finish_rename(ctx);
            }
            EditorEvent::Escape => {
                me.cancel_rename(ctx);
            }
            _ => {}
        });

        // We use this as both the "view all" and "show less" button
        // (switching out the text on-toggle).
        let toggle_view_all_button = ctx.add_typed_action_view(|_| {
            ActionButton::new(VIEW_ALL_LABEL, SecondaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(ConversationListViewAction::ToggleViewAll);
                })
        });

        let item_overflow_menu = ctx.add_typed_action_view(|_| {
            Menu::new()
                .prevent_interaction_with_other_elements()
                .with_width(160.)
        });
        ctx.subscribe_to_view(&item_overflow_menu, |me, _, event, ctx| match event {
            MenuEvent::Close { .. } => {
                me.overflow_menu_state = None;

                // On click-out of the menu, we should clear the selected index so that
                // the item hover state doesn't stick around.
                me.selected_index = None;
                ctx.notify();
            }
            MenuEvent::ItemSelected | MenuEvent::ItemHovered => {}
        });

        let sharing_dialog = ctx.add_typed_action_view(|ctx| SharingDialog::new(None, ctx));
        ctx.subscribe_to_view(&sharing_dialog, move |me, _, _event, ctx| {
            // SharingDialogEvent::Close is the only event currently
            me.share_dialog_open_for = None;
            ctx.notify();
        });

        let mut view = Self {
            window_id: ctx.window_id(),
            view_id: ctx.view_id(),
            view_model,
            query_editor,
            toggle_view_all_button,
            item_overflow_menu,
            overflow_menu_state: None,
            sharing_dialog,
            rename_editor,
            renaming_conversation_id: None,
            share_dialog_open_for: None,
            selected_index: None,
            collapsed_sections: HashSet::new(),
            list_items: Arc::new(Vec::new()),
            view_all: false,
            total_past_items: 0,
            state_handles: StateHandles::default(),
        };
        view.sync_list_items(ctx);
        view
    }

    /// Rebuilds the flat list of items based on sections and collapse state.
    fn rebuild_list_items(&mut self, ctx: &mut ViewContext<Self>) {
        let active_views_model = ActiveAgentViewsModel::as_ref(ctx);
        let active_ids: HashSet<_> =
            if FeatureFlag::ActiveConversationRequiresInteraction.is_enabled() {
                active_views_model.get_all_active_conversation_ids(ctx)
            } else {
                active_views_model.get_all_open_conversation_ids(ctx)
            }
            .into_iter()
            .map(AgentConversationEntryId::from)
            .collect();

        let focused_new_conversation =
            active_views_model.maybe_get_focused_new_conversation(ctx.window_id(), ctx);
        let model = self.view_model.as_ref(ctx);

        // Sort entries into active and past lists.
        let mut active_items = Vec::new();
        let mut past_items = Vec::new();
        for entry in model.filtered_items() {
            let local_conversation_entry_id = model
                .get_item_by_id(&entry.id, ctx)
                .and_then(|entry| entry.identity.local_conversation_id)
                .map(AgentConversationEntryId::Conversation);
            let is_active = active_ids.contains(&entry.id)
                || local_conversation_entry_id.is_some_and(|id| active_ids.contains(&id));
            if is_active {
                active_items.push(ListItem::Conversation {
                    entry: entry.clone(),
                    section: ConversationSection::Active,
                });
            } else {
                past_items.push(ListItem::Conversation {
                    entry: entry.clone(),
                    section: ConversationSection::Past,
                });
            }
        }

        // If the focused conversation is a new/empty conversation that's not already in the list,
        // add it as a regular conversation entry so it participates in the sort.
        if let Some(new_conv_id) = focused_new_conversation {
            let conv_id = AgentConversationEntryId::Conversation(new_conv_id);
            let already_in_list = active_items.iter().any(
                |item| matches!(item, ListItem::Conversation { entry, .. } if entry.id == conv_id),
            );
            if !already_in_list {
                active_items.push(ListItem::Conversation {
                    entry: ConversationEntry {
                        id: conv_id,
                        highlight_indices: vec![],
                    },
                    section: ConversationSection::Active,
                });
            }
        }

        // Sort active items by last opened time (most recently opened first).
        active_items.sort_by(|a, b| {
            let get_time = |item: &ListItem| match item {
                ListItem::Conversation { entry, .. } => {
                    let entry_time = active_views_model
                        .get_last_opened_time(&ConversationOrTaskId::from(entry.id));
                    let local_time = model
                        .get_item_by_id(&entry.id, ctx)
                        .and_then(|item| item.identity.local_conversation_id)
                        .and_then(|id| {
                            active_views_model
                                .get_last_opened_time(&ConversationOrTaskId::ConversationId(id))
                        });
                    entry_time.max(local_time)
                }
                _ => None,
            };
            get_time(b).cmp(&get_time(a))
        });

        let mut items = Vec::new();
        let has_content = !active_items.is_empty() || !past_items.is_empty();

        // If the section is not empty, add the section header + items.
        if !active_items.is_empty() {
            items.push(ListItem::SectionHeader(ConversationSection::Active));
            if !self
                .collapsed_sections
                .contains(&ConversationSection::Active)
            {
                items.extend(active_items);
            }
        }

        // Insert new conversation button between active and past sections if there are items
        // (otherwise we show the "no matching conversations" state).
        if has_content {
            items.push(ListItem::StartNewConversation);
        }

        // We truncate the past section to INITIAL_MAX_PAST_ITEMS if the user has not selected "view all".
        self.total_past_items = past_items.len();
        if !past_items.is_empty() {
            items.push(ListItem::SectionHeader(ConversationSection::Past));
            if !self.collapsed_sections.contains(&ConversationSection::Past) {
                if !self.view_all {
                    items.extend(past_items.into_iter().take(INITIAL_MAX_PAST_ITEMS));
                } else {
                    items.extend(past_items);
                }
            }
        }

        // Add toggle button if there are more past items than the limit
        // (and we're actually showing the past section).
        if self.total_past_items > INITIAL_MAX_PAST_ITEMS
            && !self.collapsed_sections.contains(&ConversationSection::Past)
        {
            items.push(ListItem::ToggleViewAllButton);
        }

        self.list_items = Arc::new(items);
    }

    fn item_count(&self) -> usize {
        self.list_items.len()
    }

    fn get_list_item(&self, index: usize) -> Option<&ListItem> {
        self.list_items.get(index)
    }

    fn get_index_of_conversation_id(
        &self,
        conversation_id: AgentConversationEntryId,
    ) -> Option<usize> {
        self.list_items.iter().position(|item| match item {
            ListItem::Conversation { entry, .. } => entry.id == conversation_id,
            ListItem::SectionHeader(_)
            | ListItem::StartNewConversation
            | ListItem::ToggleViewAllButton => false,
        })
    }

    /// Whether the given entry is currently shown in the Active section.
    fn is_in_active_section(&self, conversation_id: AgentConversationEntryId) -> bool {
        self.list_items.iter().any(|item| {
            matches!(
                item,
                ListItem::Conversation {
                    entry,
                    section: ConversationSection::Active,
                } if entry.id == conversation_id
            )
        })
    }

    pub fn on_left_panel_focused(&mut self, ctx: &mut ViewContext<Self>) {
        // Focus the search bar when the panel is opened.
        ctx.focus(&self.query_editor);

        // Select the focused conversation if there is one.
        let focused_conversation =
            ActiveAgentViewsModel::as_ref(ctx).get_focused_conversation(ctx.window_id());
        self.selected_index = focused_conversation
            .map(AgentConversationEntryId::from)
            .and_then(|id| self.get_index_of_conversation_id(id));

        if let Some(index) = self.selected_index {
            self.state_handles.list_state.scroll_to(index);
        }
        ctx.notify();
    }

    fn handle_query_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_) => {
                let new_query = self.query_editor.as_ref(ctx).buffer_text(ctx);
                self.view_model.update(ctx, |model, ctx| {
                    model.set_search_query(new_query, ctx);
                });
            }
            EditorEvent::Navigate(NavigationKey::Down) => {
                self.move_selection_down(ctx);
            }
            EditorEvent::Navigate(NavigationKey::Up) => {
                self.move_selection_up(ctx);
            }
            EditorEvent::Enter => {
                self.activate_selected_item(ctx);
            }
            _ => {}
        }
    }

    fn focus_query_editor(&mut self, ctx: &mut ViewContext<Self>) {
        self.selected_index = None;
        ctx.focus(&self.query_editor);
        ctx.notify();
    }

    fn is_selectable(&self, index: usize) -> bool {
        self.get_list_item(index).is_some_and(|item| match item {
            ListItem::Conversation { .. } | ListItem::StartNewConversation => true,
            ListItem::SectionHeader(_) | ListItem::ToggleViewAllButton => false,
        })
    }

    // Find the last item in the list that is selectable (i.e. is a conversation item).
    fn find_last_selectable_index(&self) -> Option<usize> {
        (0..self.item_count())
            .rev()
            .find(|&i| self.is_selectable(i))
    }

    fn move_selection_up(&mut self, ctx: &mut ViewContext<Self>) {
        // If the overflow menu is open for a list item, we ignore selection changes
        // (to avoid weirdness where the overflow menu is open for an item, but a different item
        // is selected and has a hover effect).
        if self.overflow_menu_state.is_some() {
            return;
        }

        let item_count = self.item_count();
        if item_count == 0 {
            return;
        }

        // Determine where to start searching backwards from.
        let start = match self.selected_index {
            // Search bar focused: wrap around to end of list.
            None => item_count,
            // Already at the top: go to search bar.
            Some(0) => {
                self.focus_query_editor(ctx);
                return;
            }
            // Start searching from current position.
            Some(index) => index,
        };

        // Search backwards for the first selectable item.
        for new_index in (0..start).rev() {
            if self.is_selectable(new_index) {
                self.selected_index = Some(new_index);
                self.state_handles.list_state.scroll_to(new_index);
                ctx.notify();
                return;
            }
        }

        // No selectable item found (e.g., all sections collapsed): go to search bar.
        self.focus_query_editor(ctx);
    }

    fn move_selection_down(&mut self, ctx: &mut ViewContext<Self>) {
        // If the overflow menu is open for a list item, we ignore selection changes
        // (to avoid weirdness where the overflow menu is open for an item, but a different item
        // is selected and has a hover effect).
        if self.overflow_menu_state.is_some() {
            return;
        }

        let item_count = self.item_count();
        if item_count == 0 {
            return;
        }

        // Determine where to start searching forwards from.
        // If search bar is focused, start at 0; otherwise start after current selection.
        let start = self.selected_index.map(|i| i + 1).unwrap_or(0);

        // Search forwards for the first selectable item.
        for new_index in start..item_count {
            if self.is_selectable(new_index) {
                // If coming from search bar, focus the list view.
                if self.selected_index.is_none() {
                    ctx.focus_self();
                }
                self.selected_index = Some(new_index);
                self.state_handles.list_state.scroll_to(new_index);
                ctx.notify();
                return;
            }
        }

        // No selectable item found (e.g., at end of list or all collapsed): go to search bar.
        self.focus_query_editor(ctx);
    }

    fn send_open_telemetry(id: &AgentConversationEntryId, ctx: &mut ViewContext<Self>) {
        match id {
            AgentConversationEntryId::Conversation(conversation_id) => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::ConversationOpened {
                        conversation_id: conversation_id.to_string(),
                        opened_from: OpenedFrom::ConversationList,
                    },
                    ctx
                );
            }
            AgentConversationEntryId::AmbientRun(task_id) => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::CloudRunOpened {
                        task_id: task_id.to_string(),
                        opened_from: OpenedFrom::ConversationList,
                    },
                    ctx
                );
            }
        }
    }

    /// Activate the currently selected item by dispatching the appropriate WorkspaceAction
    /// (i.e. opening the selected conversation or starting a new conversation).
    fn activate_selected_item(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(list_item) = self
            .selected_index
            .and_then(|index| self.get_list_item(index))
        else {
            return;
        };

        match list_item {
            ListItem::StartNewConversation => {
                ctx.emit(Event::NewConversationInNewTab);
            }
            ListItem::Conversation { entry, .. } => {
                if let Some(action) = AgentConversationsModel::resolve_open_action(
                    AgentConversationNavigationSubject::Entry(entry.id),
                    None,
                    ctx,
                ) {
                    Self::send_open_telemetry(&entry.id, ctx);
                    ctx.dispatch_typed_action(&action);
                }
            }
            ListItem::SectionHeader(_) | ListItem::ToggleViewAllButton => {}
        }
    }

    fn sync_list_items(&mut self, ctx: &mut ViewContext<Self>) {
        let model = self.view_model.as_ref(ctx);
        let current_ids: std::collections::HashSet<_> = model.current_ids().cloned().collect();

        // Remove stale entries
        self.state_handles
            .item_states
            .retain(|id, _| current_ids.contains(id));

        // Add new entries
        for id in current_ids {
            self.state_handles.item_states.entry(id).or_default();
        }

        // Rebuild list_items with current collapse state
        self.rebuild_list_items(ctx);

        // Adjust selection if it's now invalid.
        if let Some(index) = self.selected_index {
            if index >= self.item_count() {
                self.selected_index = None;
            } else if !self.is_selectable(index) {
                // Search forward for the next valid item, or clear if none found.
                self.selected_index = (index..self.item_count()).find(|&i| self.is_selectable(i));
            }
        }
        ctx.notify();
    }

    fn start_rename(&mut self, id: AgentConversationEntryId, ctx: &mut ViewContext<Self>) {
        let Some(entry) = self.view_model.as_ref(ctx).get_item_by_id(&id, ctx) else {
            return;
        };
        let Some(conversation_id) = entry.identity.local_conversation_id else {
            return;
        };

        // Renaming requires the conversation to be loaded in the history model, which holds
        // for active conversations and any conversation open in an agent view (an open
        // conversation stays in the Past section until a prompt is sent).
        let is_open = ActiveAgentViewsModel::as_ref(ctx)
            .get_terminal_view_id_for_conversation(conversation_id, ctx)
            .is_some();
        if !self.is_in_active_section(id) && !is_open {
            return;
        }

        self.overflow_menu_state = None;
        self.renaming_conversation_id = Some(conversation_id);
        self.selected_index = self.get_index_of_conversation_id(id);
        let title = entry.display.title;
        self.rename_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
            editor.insert_selected_text(&title, ctx);
        });
        ctx.focus(&self.rename_editor);
        ctx.notify();
    }

    fn finish_rename(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(conversation_id) = self.renaming_conversation_id.take() else {
            return;
        };
        let title = self.rename_editor.as_ref(ctx).buffer_text(ctx);
        self.rename_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
        });
        rename_conversation(conversation_id, title, ctx);
        ctx.notify();
    }

    fn cancel_rename(&mut self, ctx: &mut ViewContext<Self>) {
        if self.renaming_conversation_id.take().is_none() {
            return;
        }
        self.rename_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
        });
        ctx.notify();
    }

    fn toggle_section_collapse(
        &mut self,
        section: ConversationSection,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.collapsed_sections.contains(&section) {
            self.collapsed_sections.remove(&section);
        } else {
            self.collapsed_sections.insert(section);
        }

        self.rebuild_list_items(ctx);
        ctx.notify();
    }

    fn get_position_id(&self) -> String {
        format!("conversation_list_{}", self.view_id)
    }
}

/// Renders the zero state for the conversation list view
/// (i.e. when there are no local or ambient conversations).
fn render_zero_state(
    zero_state_button_mouse_state: MouseStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let mut container = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(12.);

    let chat_icon = ConstrainedBox::new(
        Icon::ChatDashed
            .to_warpui_icon(theme.sub_text_color(theme.background()))
            .finish(),
    )
    .with_width(24.)
    .with_height(24.);
    container.add_child(chat_icon.finish());

    let title_and_subtitle = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(4.)
        .with_child(
            Text::new("No conversations yet", appearance.ui_font_family(), 14.)
                .with_color(theme.sub_text_color(theme.background()).into_solid())
                .with_style(Properties::default().weight(Weight::Semibold))
                .finish(),
        )
        .with_child(
            ConstrainedBox::new(
                FormattedTextElement::from_str(
                    "Your active and past conversations with local and ambient agents will appear here.",
                    appearance.ui_font_family(),
                    14.,
                )
                .with_alignment(TextAlignment::Center)
                .with_color(theme.disabled_ui_text_color().into_solid())
                .finish(),
            )
            .with_max_width(216.)
            .finish(),
        );
    container.add_child(title_and_subtitle.finish());

    let new_conversation_button =
        Hoverable::new(zero_state_button_mouse_state, move |mouse_state| {
            let label = Text::new_inline("New conversation", appearance.ui_font_family(), 12.)
                .with_color(theme.main_text_color(theme.background()).into_solid())
                .finish();

            let button_content = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(4.)
                .with_child(
                    ConstrainedBox::new(
                        Icon::Plus
                            .to_warpui_icon(theme.main_text_color(theme.background()))
                            .finish(),
                    )
                    .with_width(14.)
                    .with_height(14.)
                    .finish(),
                )
                .with_child(label)
                .finish();

            let mut container = Container::new(button_content)
                .with_padding(Padding::uniform(0.).with_left(8.).with_right(8.))
                .with_border(Border::all(1.).with_border_fill(theme.surface_3()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
            if mouse_state.is_hovered() {
                container = container.with_background(theme.surface_3());
            }

            ConstrainedBox::new(container.finish())
                .with_height(24.)
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(ConversationListViewAction::NewConversationInNewTab);
        });
    container.add_child(new_conversation_button.finish());

    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_child(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Container::new(container.finish())
                        .with_horizontal_padding(12.)
                        .finish(),
                )
                .finish(),
        )
        .finish()
}

fn render_search_box(query_editor: &ViewHandle<EditorView>, app: &AppContext) -> Box<dyn Element> {
    let theme = Appearance::as_ref(app).theme();

    let search_row = Shrinkable::new(
        1.0,
        Clipped::new(ChildView::new(query_editor).finish()).finish(),
    )
    .finish();

    let search_container = Container::new(search_row)
        .with_padding(Padding::uniform(6.).with_left(12.).with_right(12.))
        .with_border(Border::all(1.).with_border_fill(theme.surface_3()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .finish();

    Container::new(search_container)
        .with_horizontal_padding(12.)
        .with_vertical_padding(4.)
        .finish()
}

fn render_section_header(
    section: ConversationSection,
    is_collapsed: bool,
    mouse_state: MouseStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let chevron_icon = if is_collapsed {
        Icon::ChevronRight
    } else {
        Icon::ChevronDown
    };
    let chevron = ConstrainedBox::new(
        chevron_icon
            .to_warpui_icon(theme.sub_text_color(theme.background()))
            .finish(),
    )
    .with_width(12.)
    .with_height(12.);

    let title_text = Text::new_inline(
        match section {
            ConversationSection::Active => "ACTIVE",
            ConversationSection::Past => "PAST",
        },
        appearance.ui_font_family(),
        11.,
    )
    .with_color(theme.sub_text_color(theme.background()).into());

    let row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(4.)
        .with_child(chevron.finish())
        .with_child(title_text.finish())
        .finish();

    ConstrainedBox::new(
        Hoverable::new(mouse_state, move |mouse_state| {
            let mut container = Container::new(row)
                .with_horizontal_padding(12.)
                .with_vertical_padding(14.);
            if mouse_state.is_hovered() {
                container = container.with_background(theme.surface_overlay_1());
            }
            container.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_hover(|_, ctx, _, _| {
            // Headers are not selectable (i.e. they can't be selected w/ keyboard navigation),
            // so we just clear the selection on hover.
            ctx.dispatch_typed_action(ConversationListViewAction::ClearSelectedIndex);
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ConversationListViewAction::ToggleSection(section));
        })
        .finish(),
    )
    .with_min_height(STATIC_ITEM_MIN_HEIGHT)
    .finish()
}

fn render_list_action_button(button: &ViewHandle<ActionButton>) -> Box<dyn Element> {
    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::Start)
        .with_child(
            Container::new(ChildView::new(button).finish())
                .with_horizontal_padding(12.)
                .with_vertical_padding(8.)
                .finish(),
        )
        .finish()
}

impl Entity for ConversationListView {
    type Event = Event;
}

impl TypedActionView for ConversationListView {
    type Action = ConversationListViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ConversationListViewAction::DeleteConversation {
                conversation_id,
                terminal_view_id,
            } => {
                let window_id = ctx.window_id();
                // A conversation can only be deleted once it's done.
                let conversation_is_done = BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(conversation_id)
                    .is_none_or(|c| c.status().is_done());
                if !conversation_is_done {
                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::error(
                                "Conversations cannot be deleted while in progress.".to_string(),
                            ),
                            window_id,
                            ctx,
                        );
                    });
                    return;
                }

                let id = AgentConversationEntryId::Conversation(*conversation_id);
                let conversation_title = self
                    .view_model
                    .as_ref(ctx)
                    .get_item_by_id(&id, ctx)
                    .map(|entry| entry.display.title)
                    .unwrap_or_else(|| "Conversation".to_string());
                ctx.emit(Event::ShowDeleteConfirmationDialog {
                    conversation_id: *conversation_id,
                    conversation_title,
                    terminal_view_id: *terminal_view_id,
                });
            }
            ConversationListViewAction::ToggleOverflowMenu {
                conversation_id,
                position,
            } => {
                let is_open_for_same_conversation = self
                    .overflow_menu_state
                    .is_some_and(|s| s.conversation_id == *conversation_id);
                if is_open_for_same_conversation {
                    self.overflow_menu_state = None;
                } else {
                    self.overflow_menu_state = Some(OverflowMenuState {
                        conversation_id: *conversation_id,
                        position: *position,
                    });

                    let conversation_id = *conversation_id;
                    let Some(entry) = self
                        .view_model
                        .as_ref(ctx)
                        .get_item_by_id(&conversation_id, ctx)
                    else {
                        return;
                    };

                    let mut delete_item = MenuItemFields::new("Delete")
                        .with_override_text_color(Appearance::as_ref(ctx).theme().ansi_fg_red())
                        .with_on_select_action(ConversationListViewAction::DeleteFromOverflowMenu {
                            conversation_id,
                        })
                        .with_disabled(!entry.capabilities.can_delete);
                    if !entry.capabilities.can_delete {
                        delete_item =
                            delete_item.with_tooltip("This conversation cannot be deleted");
                    }

                    // Only show share item if the conversation is shareable
                    let share_item = if entry.capabilities.can_share {
                        Some(
                            MenuItemFields::new("Share conversation")
                                .with_on_select_action(
                                    ConversationListViewAction::OpenShareDialog { conversation_id },
                                )
                                .into_item(),
                        )
                    } else {
                        None
                    };

                    let fork_items: Option<[MenuItem<ConversationListViewAction>; 2]> =
                        // Forking from a closed ambient agent conversation is not supported at this point.
                        if entry.capabilities.can_fork_locally {
                            Some([
                                MenuItemFields::new("Fork in new pane")
                                    .with_on_select_action(
                                        ConversationListViewAction::ForkConversation {
                                            conversation_id,
                                            destination: ForkedConversationDestination::SplitPane,
                                        },
                                    )
                                    .into_item(),
                                MenuItemFields::new("Fork in new tab")
                                    .with_on_select_action(
                                        ConversationListViewAction::ForkConversation {
                                            conversation_id,
                                            destination: ForkedConversationDestination::NewTab,
                                        },
                                    )
                                    .into_item(),
                            ])
                        } else {
                            None
                        };

                    let mut items = Vec::new();
                    if let Some(share_item) = share_item {
                        items.push(share_item);
                    }
                    if let Some(fork_items) = fork_items {
                        items.extend(fork_items);
                    }

                    if !items.is_empty() {
                        items.push(MenuItem::Separator);
                    }
                    items.push(delete_item.into_item());
                    self.item_overflow_menu.update(ctx, |menu, ctx| {
                        menu.set_items(items, ctx);
                    });
                }
                ctx.notify();
            }
            ConversationListViewAction::OpenShareDialog { conversation_id } => {
                // Clear selection state when opening share dialog
                self.selected_index = None;
                let Some(ai_conversation_id) = self
                    .view_model
                    .as_ref(ctx)
                    .get_item_by_id(conversation_id, ctx)
                    .filter(|entry| entry.capabilities.can_share)
                    .and_then(|entry| entry.identity.local_conversation_id)
                else {
                    return;
                };

                // Set the share dialog target and open it
                self.share_dialog_open_for = Some(*conversation_id);
                self.sharing_dialog.update(ctx, |dialog, ctx| {
                    dialog.set_target(
                        Some(ShareableObject::AIConversation(ai_conversation_id)),
                        ctx,
                    );
                    dialog.report_open(SharingDialogSource::ConversationList, ctx);
                });
                ctx.focus(&self.sharing_dialog);
                ctx.notify();
            }
            ConversationListViewAction::DeleteFromOverflowMenu { conversation_id } => {
                let Some(entry) = self
                    .view_model
                    .as_ref(ctx)
                    .get_item_by_id(conversation_id, ctx)
                else {
                    return;
                };
                let Some(ai_conversation_id) = entry.identity.local_conversation_id else {
                    return;
                };
                if !entry.capabilities.can_delete {
                    return;
                };

                let conversation =
                    BlocklistAIHistoryModel::as_ref(ctx).conversation(&ai_conversation_id);

                if let Some(conversation) = conversation {
                    // Same gate as the deletion path above.
                    if !conversation.status().is_done() && !conversation.is_empty() {
                        let window_id = ctx.window_id();
                        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                            toast_stack.add_ephemeral_toast(
                                DismissibleToast::error(
                                    "Conversations cannot be deleted while in progress."
                                        .to_string(),
                                ),
                                window_id,
                                ctx,
                            );
                        });
                        return;
                    }
                }

                self.selected_index = None;

                let terminal_view_id = ActiveAgentViewsModel::as_ref(ctx)
                    .get_terminal_view_id_for_conversation(ai_conversation_id, ctx);
                let conversation_title = entry.display.title;
                ctx.emit(Event::ShowDeleteConfirmationDialog {
                    conversation_id: ai_conversation_id,
                    conversation_title,
                    terminal_view_id,
                });
            }
            ConversationListViewAction::OpenItem { id } => {
                let Some(action) = AgentConversationsModel::resolve_open_action(
                    AgentConversationNavigationSubject::Entry(*id),
                    None,
                    ctx,
                ) else {
                    return;
                };

                Self::send_open_telemetry(id, ctx);
                ctx.dispatch_typed_action(&action);
            }
            ConversationListViewAction::ArrowUp => {
                self.move_selection_up(ctx);
            }
            ConversationListViewAction::ArrowDown => {
                self.move_selection_down(ctx);
            }
            ConversationListViewAction::Enter => {
                self.activate_selected_item(ctx);
            }
            ConversationListViewAction::SetSelectedIndex(index) => {
                // If the overflow menu is open for a list item, we ignore selection changes
                // (to avoid weirdness where the overflow menu is open for an item, but a different item
                // is selected and has a hover effect).
                if self.overflow_menu_state.is_some() {
                    return;
                }

                self.selected_index = Some(*index);
                ctx.notify();
            }
            ConversationListViewAction::ClearSelectedIndex => {
                self.selected_index = None;
                ctx.notify();
            }
            ConversationListViewAction::NewConversationInNewTab => {
                ctx.emit(Event::NewConversationInNewTab);
            }
            ConversationListViewAction::ToggleSection(section) => {
                self.toggle_section_collapse(*section, ctx);
            }
            ConversationListViewAction::ToggleViewAll => {
                self.view_all = !self.view_all;

                let label = if self.view_all {
                    "Show less"
                } else {
                    VIEW_ALL_LABEL
                };
                self.toggle_view_all_button
                    .update(ctx, |button, ctx| button.set_label(label, ctx));

                self.rebuild_list_items(ctx);

                // If the selection is no longer valid (because it was one of the
                // list items that we're now hiding), select the last selectable item.
                if let Some(index) = self.selected_index
                    && !self.is_selectable(index)
                {
                    self.selected_index = self.find_last_selectable_index();
                }

                ctx.notify();
            }
            ConversationListViewAction::ForkConversation {
                conversation_id,
                destination,
            } => {
                let Some(ai_conversation_id) = self
                    .view_model
                    .as_ref(ctx)
                    .get_item_by_id(conversation_id, ctx)
                    .filter(|entry| entry.capabilities.can_fork_locally)
                    .and_then(|entry| entry.identity.local_conversation_id)
                else {
                    return;
                };

                ctx.dispatch_typed_action(&WorkspaceAction::ForkAIConversation {
                    conversation_id: ai_conversation_id,
                    fork_from_exchange: None,
                    summarize_after_fork: false,
                    summarization_prompt: None,
                    initial_prompt: None,
                    initial_attachments: vec![],
                    destination: *destination,
                });
            }
            ConversationListViewAction::StartRename { id } => {
                self.start_rename(*id, ctx);
            }
            ConversationListViewAction::FinishRename => {
                self.finish_rename(ctx);
            }
            ConversationListViewAction::CancelRename => {
                self.cancel_rename(ctx);
            }
        }
    }
}

impl View for ConversationListView {
    fn ui_name() -> &'static str {
        "ConversationListView"
    }

    fn on_blur(&mut self, _: &BlurContext, ctx: &mut ViewContext<Self>) {
        if !ctx.is_self_or_child_focused() {
            self.selected_index = None;
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let view_model = self.view_model.as_ref(app);

        let has_conversations = view_model.unfiltered_item_count() > 0;
        let content: Box<dyn Element> = if !has_conversations {
            render_zero_state(self.state_handles.zero_state_button.clone(), app)
        } else if self.item_count() == 0 {
            Container::new(
                Text::new_inline(
                    "No matching conversations",
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish(),
            )
            .with_horizontal_padding(12.)
            .with_vertical_padding(8.)
            .finish()
        } else {
            let model_handle = self.view_model.downgrade();
            let item_states = self.state_handles.item_states.clone();
            let start_new_conversation_state =
                self.state_handles.start_new_conversation_item.clone();
            let selected_index = self.selected_index;
            let collapsed_sections = self.collapsed_sections.clone();
            let active_header_mouse_state = self.state_handles.active_header.clone();
            let past_header_mouse_state = self.state_handles.past_header.clone();
            let toggle_view_all_button = self.toggle_view_all_button.clone();
            let list_items = self.list_items.clone();
            let overflow_menu = self.item_overflow_menu.clone();
            let overflow_menu_state = self.overflow_menu_state;
            let focused_conversation = ActiveAgentViewsModel::as_ref(app)
                .get_focused_conversation(self.window_id)
                .map(AgentConversationEntryId::from);
            let sharing_dialog = self.sharing_dialog.clone();
            let rename_editor = self.rename_editor.clone();
            let renaming_conversation_id = self.renaming_conversation_id;
            let open_conversation_ids =
                ActiveAgentViewsModel::as_ref(app).get_all_open_conversation_ids(app);
            let share_dialog_open_for = self.share_dialog_open_for;
            let list_position_id = self.get_position_id();
            let tooltip_opens_right = TabSettings::as_ref(app)
                .header_toolbar_chip_selection
                .left_items()
                .contains(&HeaderToolbarItemKind::ToolsPanel);

            let list = UniformList::new(
                self.state_handles.list_state.clone(),
                self.item_count(),
                move |range: Range<usize>, app: &AppContext| {
                    let model = model_handle
                        .upgrade(app)
                        .expect("Model handle should be valid");
                    let model = model.as_ref(app);

                    range
                        .filter_map(|index| {
                            let list_item = list_items.get(index)?;
                            let is_selected = selected_index == Some(index);

                            match list_item {
                                ListItem::SectionHeader(section) => {
                                    let is_collapsed = collapsed_sections.contains(section);
                                    let mouse_state = match section {
                                        ConversationSection::Active => {
                                            active_header_mouse_state.clone()
                                        }
                                        ConversationSection::Past => {
                                            past_header_mouse_state.clone()
                                        }
                                    };
                                    Some(render_section_header(
                                        *section,
                                        is_collapsed,
                                        mouse_state,
                                        app,
                                    ))
                                }
                                ListItem::Conversation { entry, section } => {
                                    let conversation = model.get_item_by_id(&entry.id, app)?;
                                    let local_conversation_entry_id = conversation
                                        .identity
                                        .local_conversation_id
                                        .map(AgentConversationEntryId::Conversation);
                                    let is_focused_conversation =
                                        focused_conversation.is_some_and(|focused| {
                                            entry.id == focused
                                                || local_conversation_entry_id == Some(focused)
                                        });
                                    let state = item_states.get(&entry.id)?;
                                    let highlight_ref = if entry.highlight_indices.is_empty() {
                                        None
                                    } else {
                                        Some(&entry.highlight_indices)
                                    };
                                    let local_conversation_id =
                                        conversation.identity.local_conversation_id;
                                    let is_renaming = renaming_conversation_id.is_some()
                                        && local_conversation_id == renaming_conversation_id;
                                    // Renaming is allowed for active conversations and for open
                                    // ones (an open conversation stays in the Past section until
                                    // a prompt is sent).
                                    let can_rename = local_conversation_id.is_some_and(|id| {
                                        *section == ConversationSection::Active
                                            || open_conversation_ids
                                                .contains(&ConversationOrTaskId::ConversationId(id))
                                    });

                                    let overflow_menu_display = match overflow_menu_state {
                                        Some(s) if s.conversation_id == entry.id => {
                                            if s.position.is_some() {
                                                OverflowMenuDisplay::OpenAtRightClickPosition
                                            } else {
                                                OverflowMenuDisplay::OpenAtKebab
                                            }
                                        }
                                        _ => OverflowMenuDisplay::Closed,
                                    };
                                    let is_share_dialog_open =
                                        share_dialog_open_for == Some(entry.id);
                                    Some(render_item(
                                        ItemProps {
                                            conversation: &conversation,
                                            highlight_indices: highlight_ref,
                                            is_selected,
                                            is_focused_conversation,
                                            index,
                                            state,
                                            overflow_menu: &overflow_menu,
                                            overflow_menu_display,
                                            conversation_id: entry.id,
                                            is_renaming,
                                            can_rename,
                                            rename_editor: is_renaming.then_some(&rename_editor),
                                            sharing_dialog: &sharing_dialog,
                                            is_share_dialog_open,
                                            list_position_id: &list_position_id,
                                            tooltip_opens_right,
                                        },
                                        app,
                                    ))
                                }
                                ListItem::StartNewConversation => Some(render_static_item(
                                    StaticItemProps {
                                        is_selected,
                                        index,
                                        state: &start_new_conversation_state,
                                    },
                                    app,
                                )),
                                ListItem::ToggleViewAllButton => {
                                    Some(render_list_action_button(&toggle_view_all_button))
                                }
                            }
                        })
                        .collect::<Vec<_>>()
                        .into_iter()
                },
            )
            .finish_scrollable();

            let scrollable = Scrollable::vertical(
                self.state_handles.scroll_state.clone(),
                list,
                ScrollbarWidth::Auto,
                theme.nonactive_ui_detail().into(),
                theme.active_ui_detail().into(),
                Fill::None,
            )
            .with_overlayed_scrollbar()
            .finish();

            Hoverable::new(self.state_handles.list_hover.clone(), move |_| scrollable)
                .on_hover(|is_hovered, ctx, _, _| {
                    if !is_hovered {
                        ctx.dispatch_typed_action(ConversationListViewAction::ClearSelectedIndex);
                    }
                })
                .with_skip_synthetic_hover_out()
                .finish()
        };

        let mut column = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        if has_conversations {
            column = column.with_child(render_search_box(&self.query_editor, app));
        }

        let column_element = column
            .with_child(Shrinkable::new(1.0, content).finish())
            .finish();

        let positioned_content =
            SavePosition::new(column_element, &self.get_position_id()).finish();
        let mut stack = Stack::new().with_child(positioned_content);

        if let Some(position) = self.overflow_menu_state.and_then(|s| s.position) {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.item_overflow_menu).finish(),
                OffsetPositioning::offset_from_parent(
                    position,
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }

        stack.finish()
    }
}
