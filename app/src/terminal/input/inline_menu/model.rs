//! Generic model for tracking the selected item in an inline menu.
use std::collections::HashSet;

use warpui::elements::MouseStateHandle;
use warpui::{Entity, ModelContext};

use crate::search::data_source::QueryFilter;
use crate::terminal::input::inline_menu::view::InlineMenuAction;

#[derive(Default)]
pub struct InlineMenuMouseStates {
    pub accept: MouseStateHandle,
    pub accept_secondary: MouseStateHandle,
    pub dismiss: MouseStateHandle,
}

/// Configuration for a single tab in the inline menu.
///
/// Each tab has a label (displayed in the header) and a set of query filters
/// that are applied when the tab is active.
#[derive(Clone)]
pub struct InlineMenuTabConfig<T = ()> {
    pub id: T,
    pub label: String,
    pub filters: HashSet<QueryFilter>,
}

/// This model is generic over the action type `A` (the same type used by the `SearchMixer`).
/// It serves as a denormalized copy of the selected item that can be read/subscribed to
/// without requiring a strict dependency on the inline menu view itself. It also tracks
/// the inline menu's selected tab (if any).
pub struct InlineMenuModel<A: InlineMenuAction, T = ()> {
    selected_item: Option<A>,
    tab_configs: Vec<InlineMenuTabConfig<T>>,
    active_tab_index: usize,
    mouse_states: InlineMenuMouseStates,
    tab_mouse_states: Vec<MouseStateHandle>,
}

impl<A: InlineMenuAction> Default for InlineMenuModel<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: InlineMenuAction> InlineMenuModel<A> {
    pub fn new() -> Self {
        Self {
            selected_item: None,
            tab_configs: Vec::new(),
            active_tab_index: 0,
            mouse_states: InlineMenuMouseStates::default(),
            tab_mouse_states: Vec::new(),
        }
    }
}

impl<A: InlineMenuAction, T: Clone + PartialEq> InlineMenuModel<A, T> {
    pub fn new_with_tabs(tab_configs: Vec<InlineMenuTabConfig<T>>, initial_tab: Option<T>) -> Self {
        let tab_mouse_states = tab_configs
            .iter()
            .map(|_| MouseStateHandle::default())
            .collect();
        let active_tab_index = initial_tab
            .and_then(|id| tab_configs.iter().position(|config| config.id == id))
            .unwrap_or(0);

        Self {
            selected_item: None,
            tab_configs,
            active_tab_index,
            mouse_states: InlineMenuMouseStates::default(),
            tab_mouse_states,
        }
    }

    pub fn active_tab_id(&self) -> Option<T> {
        self.tab_configs
            .get(self.active_tab_index)
            .map(|config| config.id.clone())
    }
}

impl<A: InlineMenuAction, T> InlineMenuModel<A, T> {
    /// Returns a reference to the currently selected item, if any.
    pub fn selected_item(&self) -> Option<&A> {
        self.selected_item.as_ref()
    }

    pub fn tab_configs(&self) -> &[InlineMenuTabConfig<T>] {
        &self.tab_configs
    }

    pub fn active_tab_index(&self) -> usize {
        self.active_tab_index
    }

    pub fn active_tab_filters(&self) -> HashSet<QueryFilter> {
        self.tab_configs
            .get(self.active_tab_index)
            .map(|config| config.filters.clone())
            .unwrap_or_default()
    }

    pub fn mouse_states(&self) -> &InlineMenuMouseStates {
        &self.mouse_states
    }

    pub fn tab_mouse_states(&self) -> &[MouseStateHandle] {
        &self.tab_mouse_states
    }
}

impl<A: InlineMenuAction, T: Send + Sync + 'static> InlineMenuModel<A, T> {
    pub fn set_tab_configs(&mut self, tab_configs: Vec<InlineMenuTabConfig<T>>) {
        if self.tab_mouse_states.len() < tab_configs.len() {
            self.tab_mouse_states
                .resize_with(tab_configs.len(), MouseStateHandle::default);
        }
        self.tab_configs = tab_configs;
        if self.active_tab_index >= self.tab_configs.len() {
            self.active_tab_index = 0;
        }
    }

    pub(super) fn set_active_tab_index(&mut self, index: usize, ctx: &mut ModelContext<Self>) {
        if index != self.active_tab_index && index < self.tab_configs.len() {
            self.active_tab_index = index;
            ctx.emit(InlineMenuModelEvent::UpdatedActiveTab);
        }
    }

    pub(super) fn update_selected_item(&mut self, item: A, ctx: &mut ModelContext<Self>) {
        self.selected_item = Some(item);
        ctx.emit(InlineMenuModelEvent::UpdatedSelectedItem);
    }

    pub(super) fn clear_selected_item(&mut self, ctx: &mut ModelContext<Self>) {
        if self.selected_item.is_some() {
            self.selected_item = None;
            ctx.emit(InlineMenuModelEvent::UpdatedSelectedItem);
        }
    }
}

#[derive(Debug, Clone)]
pub enum InlineMenuModelEvent {
    UpdatedSelectedItem,
    UpdatedActiveTab,
}

impl<A: InlineMenuAction, T: 'static + Send + Sync> Entity for InlineMenuModel<A, T> {
    type Event = InlineMenuModelEvent;
}
