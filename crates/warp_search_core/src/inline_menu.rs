//! Headless lifecycle, result, and selection state shared by inline-menu frontends.
/// Whether an input-driven inline menu may react to buffer updates.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InputDrivenInlineMenuLifecycle {
    state: InputDrivenInlineMenuState,
    previous_input_had_trigger: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum InputDrivenInlineMenuState {
    #[default]
    Enabled,
    DisabledUntilEmptyBuffer,
}

impl InputDrivenInlineMenuLifecycle {
    pub fn is_enabled(&self) -> bool {
        matches!(self.state, InputDrivenInlineMenuState::Enabled)
    }

    /// Prevents the menu from reopening after a manual dismissal while input remains.
    pub fn disable_until_empty_buffer(&mut self, input_is_empty: bool) {
        if input_is_empty {
            return;
        }
        self.state = InputDrivenInlineMenuState::DisabledUntilEmptyBuffer;
    }

    /// Updates lifecycle state for the latest buffer contents and returns whether menu processing
    /// is enabled. Clearing the buffer or adding a new trigger re-enables a manually dismissed
    /// menu.
    pub fn input_changed(&mut self, input_is_empty: bool, input_has_trigger: bool) -> bool {
        let did_add_trigger = input_has_trigger && !self.previous_input_had_trigger;
        if input_is_empty || did_add_trigger {
            self.state = InputDrivenInlineMenuState::Enabled;
        }
        self.previous_input_had_trigger = input_has_trigger;
        self.is_enabled()
    }
}

/// Surface-neutral outcome after reconciling an inline menu with its latest result set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineMenuResultsUpdate {
    Loading,
    Empty,
    Ready { selected_index: Option<usize> },
}

/// Logical selection over an ordered list of inline-menu results.
///
/// This type intentionally knows nothing about rendering direction, scrolling,
/// result payloads, or menu lifecycle. Frontends supply the current item count
/// and a predicate that excludes disabled rows and separators.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InlineMenuSelection {
    selected_index: Option<usize>,
}

impl InlineMenuSelection {
    pub fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }

    pub fn clear(&mut self) {
        self.selected_index = None;
    }
    /// Selects the highest-scoring enabled result from a [`crate::mixer::SearchMixer`].
    ///
    /// Mixer results are ordered by increasing score, so the best result is the last enabled
    /// item. Keeping this policy here prevents inline-menu frontends from interpreting the shared
    /// result order differently.
    pub fn reset_to_best(
        &mut self,
        item_count: usize,
        mut is_enabled: impl FnMut(usize) -> bool,
    ) -> Option<usize> {
        self.selected_index = (0..item_count).rev().find(|&index| is_enabled(index));
        self.selected_index
    }
    /// Reconciles selection with the latest mixer state using one policy for every frontend.
    pub fn reconcile_results(
        &mut self,
        is_loading: bool,
        item_count: usize,
        is_enabled: impl FnMut(usize) -> bool,
    ) -> InlineMenuResultsUpdate {
        if is_loading {
            return InlineMenuResultsUpdate::Loading;
        }
        if item_count == 0 {
            self.clear();
            return InlineMenuResultsUpdate::Empty;
        }
        InlineMenuResultsUpdate::Ready {
            selected_index: self.reset_to_best(item_count, is_enabled),
        }
    }

    pub fn select(
        &mut self,
        index: usize,
        item_count: usize,
        mut is_enabled: impl FnMut(usize) -> bool,
    ) -> Option<usize> {
        if index >= item_count || !is_enabled(index) {
            return None;
        }
        self.selected_index = Some(index);
        self.selected_index
    }

    pub fn select_next(
        &mut self,
        item_count: usize,
        is_enabled: impl FnMut(usize) -> bool,
    ) -> Option<usize> {
        let start = match self.selected_index {
            Some(index) if index < item_count.saturating_sub(1) => index + 1,
            Some(_) | None => 0,
        };
        self.select_from(start, item_count, ScanDirection::Forward, is_enabled)
    }

    pub fn select_previous(
        &mut self,
        item_count: usize,
        is_enabled: impl FnMut(usize) -> bool,
    ) -> Option<usize> {
        let start = match self.selected_index {
            Some(index) if index > 0 && index < item_count => index - 1,
            Some(_) | None => item_count.saturating_sub(1),
        };
        self.select_from(start, item_count, ScanDirection::Backward, is_enabled)
    }

    fn select_from(
        &mut self,
        start: usize,
        item_count: usize,
        direction: ScanDirection,
        mut is_enabled: impl FnMut(usize) -> bool,
    ) -> Option<usize> {
        if item_count == 0 {
            self.clear();
            return None;
        }

        self.selected_index = (0..item_count)
            .map(|offset| match direction {
                ScanDirection::Forward => (start + offset) % item_count,
                ScanDirection::Backward => (start + item_count - offset) % item_count,
            })
            .find(|&index| is_enabled(index));
        self.selected_index
    }
}

#[derive(Debug, Clone, Copy)]
enum ScanDirection {
    Forward,
    Backward,
}

#[cfg(test)]
#[path = "inline_menu_tests.rs"]
mod tests;
