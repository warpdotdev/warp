use std::collections::HashMap;

#[cfg(target_os = "macos")]
use warpui::platform::mac::Window;
use warpui::{AppContext, Entity, ModelContext, ModelHandle};

use super::pane::{PaneId, TerminalPaneId};
use super::{PaneState, SplitPaneState};

/// Centralized focus state for a pane group.
/// This model tracks which pane is focused, which session is active,
/// and which panes are visible. Individual panes subscribe to this model
/// to automatically update their split pane state.
pub struct PaneGroupFocusState {
    focused_pane_id: PaneId,
    active_session_id: Option<TerminalPaneId>,
    in_split_pane: bool,
    is_focused_pane_maximized: bool,
    /// Per-pane keyboard input source IDs (macOS only).
    /// Maps pane ID to the last known input source ID (e.g., "com.apple.keylayout.ABC").
    pane_input_sources: HashMap<PaneId, String>,
}

#[derive(Debug, Clone)]
pub enum PaneGroupFocusEvent {
    FocusChanged {
        old_focused: PaneId,
        new_focused: PaneId,
    },
    ActiveSessionChanged {
        old_active: Option<TerminalPaneId>,
        new_active: Option<TerminalPaneId>,
    },
    InSplitPaneChanged,
    FocusedPaneMaximizedChanged,
}

impl Entity for PaneGroupFocusState {
    type Event = PaneGroupFocusEvent;
}

impl PaneGroupFocusState {
    pub fn new(
        focused_pane_id: PaneId,
        active_session_id: Option<TerminalPaneId>,
        in_split_pane: bool,
    ) -> Self {
        // Initialize the map and store the current input source for the initial pane.
        let mut pane_input_sources = HashMap::new();
        #[cfg(target_os = "macos")]
        {
            if let Some(source_id) = Window::get_current_input_source_id() {
                pane_input_sources.insert(focused_pane_id, source_id);
            }
        }

        Self {
            focused_pane_id,
            active_session_id,
            in_split_pane,
            is_focused_pane_maximized: false,
            pane_input_sources,
        }
    }

    /// Returns the currently focused pane ID.
    pub fn focused_pane_id(&self) -> PaneId {
        self.focused_pane_id
    }

    /// Returns the active terminal session ID, if any.
    pub fn active_session_id(&self) -> Option<TerminalPaneId> {
        self.active_session_id
    }

    /// Returns true if the given pane is the focused pane.
    pub fn is_pane_focused(&self, pane_id: PaneId) -> bool {
        self.focused_pane_id == pane_id
    }

    /// Returns true if there is more than one visible pane (i.e., panes are split).
    pub fn is_in_split_pane(&self) -> bool {
        self.in_split_pane
    }

    /// Returns true if the focused pane is maximized.
    pub fn is_focused_pane_maximized(&self) -> bool {
        self.is_focused_pane_maximized
    }

    /// Computes the split pane state for a given pane based on current focus state.
    pub fn split_pane_state_for(&self, pane_id: PaneId) -> SplitPaneState {
        // If there's only one visible pane, it's not in a split
        if !self.in_split_pane {
            return SplitPaneState::NotInSplitPane;
        }

        let is_focused = self.focused_pane_id == pane_id;

        if is_focused && self.is_focused_pane_maximized {
            SplitPaneState::InSplitPane(PaneState::Maximized)
        } else if is_focused {
            SplitPaneState::InSplitPane(PaneState::Focused)
        } else {
            SplitPaneState::InSplitPane(PaneState::Unfocused)
        }
    }

    /// Sets the focused pane and emits a FocusChanged event.
    pub(super) fn set_focused_pane(&mut self, pane_id: PaneId, ctx: &mut ModelContext<Self>) {
        let old_focused = self.focused_pane_id;
        if old_focused != pane_id {
            // Save the current input source for the departing pane,
            // then restore the input source for the incoming pane.
            self.save_input_source_for_pane(old_focused);
            self.restore_input_source_for_pane(pane_id);

            self.focused_pane_id = pane_id;
            // When focus changes, clear maximize state
            self.is_focused_pane_maximized = false;
            ctx.emit(PaneGroupFocusEvent::FocusChanged {
                old_focused,
                new_focused: pane_id,
            });
        }
    }

    /// Saves the current system input source ID for the given pane.
    /// On macOS, this is a no-op when called from a background thread
    /// (Carbon TIS APIs require the main thread).
    #[cfg(target_os = "macos")]
    fn save_input_source_for_pane(&mut self, pane_id: PaneId) {
        if let Some(source_id) = Window::get_current_input_source_id() {
            self.pane_input_sources.insert(pane_id, source_id);
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn save_input_source_for_pane(&mut self, _pane_id: PaneId) {
        // No-op on non-macOS platforms
    }

    /// Restores the previously saved input source for the given pane.
    /// If the pane has no saved input source (e.g., it's newly created),
    /// inherits the current system input source.
    /// On macOS, this is a no-op when called from a background thread
    /// (Carbon TIS APIs require the main thread).
    #[cfg(target_os = "macos")]
    fn restore_input_source_for_pane(&mut self, pane_id: PaneId) {
        if let Some(source_id) = self.pane_input_sources.get(&pane_id) {
            Window::select_input_source(source_id);
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn restore_input_source_for_pane(&mut self, _pane_id: PaneId) {
        // No-op on non-macOS platforms
    }

    /// Sets the active terminal session and emits an ActiveSessionChanged event.
    pub(super) fn set_active_session(
        &mut self,
        session_id: Option<TerminalPaneId>,
        ctx: &mut ModelContext<Self>,
    ) {
        let old_active = self.active_session_id;
        if old_active != session_id {
            self.active_session_id = session_id;
            ctx.emit(PaneGroupFocusEvent::ActiveSessionChanged {
                old_active,
                new_active: session_id,
            });
        }
    }

    /// Sets whether or not the pane group has multiple split panes.
    pub(super) fn set_in_split_pane(&mut self, in_split_pane: bool, ctx: &mut ModelContext<Self>) {
        if self.in_split_pane != in_split_pane {
            self.in_split_pane = in_split_pane;
            ctx.emit(PaneGroupFocusEvent::InSplitPaneChanged);
        }
    }

    /// Sets whether the focused pane is maximized.
    pub(super) fn set_focused_pane_maximized(
        &mut self,
        maximized: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.is_focused_pane_maximized != maximized {
            self.is_focused_pane_maximized = maximized;
            ctx.emit(PaneGroupFocusEvent::FocusedPaneMaximizedChanged);
        }
    }

    /// Toggles whether the focused pane is maximized.
    pub(super) fn toggle_focused_pane_maximized(&mut self, ctx: &mut ModelContext<Self>) {
        self.is_focused_pane_maximized = !self.is_focused_pane_maximized;
        ctx.emit(PaneGroupFocusEvent::FocusedPaneMaximizedChanged);
    }

    /// Test-only method to set the in_split_pane state.
    #[cfg(test)]
    pub fn set_in_split_pane_for_test(
        &mut self,
        in_split_pane: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.set_in_split_pane(in_split_pane, ctx);
    }
}

#[derive(Clone)]
pub struct PaneFocusHandle {
    focus_state: ModelHandle<PaneGroupFocusState>,
    pane_id: PaneId,
}

impl PaneFocusHandle {
    pub fn new(pane_id: PaneId, focus_state: ModelHandle<PaneGroupFocusState>) -> Self {
        Self {
            focus_state,
            pane_id,
        }
    }

    /// The current split-pane state of this pane.
    pub fn split_pane_state(&self, app: &AppContext) -> SplitPaneState {
        self.focus_state
            .as_ref(app)
            .split_pane_state_for(self.pane_id)
    }

    /// True if this pane is currently maximized.
    pub fn is_maximized(&self, app: &AppContext) -> bool {
        self.split_pane_state(app).is_maximized()
    }

    /// True if this pane is part of a split.
    pub fn is_in_split_pane(&self, app: &AppContext) -> bool {
        self.split_pane_state(app).is_in_split_pane()
    }

    /// True if this pane is focused.
    pub fn is_focused(&self, app: &AppContext) -> bool {
        self.split_pane_state(app).is_focused()
    }

    /// True if this pane is the active terminal session.
    pub fn is_active_session(&self, app: &AppContext) -> bool {
        self.pane_id
            .as_terminal_pane_id()
            .is_some_and(|terminal_id| {
                self.focus_state.as_ref(app).active_session_id() == Some(terminal_id)
            })
    }

    /// Returns a reference to the underlying focus state model handle.
    /// This can be used to subscribe to focus state changes.
    pub fn focus_state_handle(&self) -> &ModelHandle<PaneGroupFocusState> {
        &self.focus_state
    }

    /// Returns the pane ID associated with this focus handle.
    pub fn pane_id(&self) -> PaneId {
        self.pane_id
    }

    /// Whether or not a focus-change event affects the pane associated with this handle.
    ///
    /// The implementation prioritizes correctness over efficiency:
    /// * Changes in focus affect this pane if it gains or loses focus.
    /// * Changes in the active session affect this pane if it was or became active.
    /// * Changes to maximization and whether or not there are split panes *always* affect this pane.
    pub fn is_affected(&self, event: &PaneGroupFocusEvent) -> bool {
        match event {
            PaneGroupFocusEvent::FocusChanged {
                old_focused,
                new_focused,
            } => old_focused == &self.pane_id || new_focused == &self.pane_id,
            PaneGroupFocusEvent::ActiveSessionChanged {
                old_active,
                new_active,
            } => match self.pane_id.as_terminal_pane_id() {
                Some(id) => Some(id) == *old_active || Some(id) == *new_active,
                None => false,
            },
            PaneGroupFocusEvent::InSplitPaneChanged => true,
            PaneGroupFocusEvent::FocusedPaneMaximizedChanged => true,
        }
    }
}
