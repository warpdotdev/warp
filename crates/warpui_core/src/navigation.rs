use std::fmt::Debug;
use std::time::Duration;

use instant::Instant;

use crate::r#async::Timer;
use crate::{Entity, ModelContext, SingletonEntity, WindowId};

pub const MAX_STACK_SIZE: usize = 100;
pub const DEFAULT_DEBOUNCE_DURATION: Duration = Duration::from_millis(1500);

pub trait NavigationEntry: Clone + Debug + 'static {
    /// Returns `false` if `self` is a duplicate of `existing` and should not
    /// be pushed onto the stack. The default returns `true` (always push).
    fn should_push(&self, _existing: &Self) -> bool {
        true
    }
}

/// A generic forward/backward navigation stack, similar to browser or IDE
/// navigation history.
///
/// The stack is parameterized over the entry type `E`, which the application
/// layer defines to carry whatever state it needs (window, tab, pane,
/// scroll position, etc.).
pub struct NavigationStack<E: NavigationEntry> {
    back: Vec<E>,
    forward: Vec<E>,
    is_navigating: bool,
    pending: Option<E>,
    debounce_duration: Duration,
    last_debounced_push: Option<Instant>,
    expected_focus_loss: Option<WindowId>,
}

impl<E: NavigationEntry> Entity for NavigationStack<E> {
    type Event = ();
}

impl<E: NavigationEntry> SingletonEntity for NavigationStack<E> {}

impl<E: NavigationEntry> Default for NavigationStack<E> {
    fn default() -> Self {
        Self {
            back: Vec::new(),
            forward: Vec::new(),
            is_navigating: false,
            pending: None,
            debounce_duration: DEFAULT_DEBOUNCE_DURATION,
            last_debounced_push: None,
            expected_focus_loss: None,
        }
    }
}

impl<E: NavigationEntry> NavigationStack<E> {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self::default()
    }

    pub fn push(&mut self, entry: E) {
        if self.is_navigating {
            return;
        }

        self.expected_focus_loss = None;

        if let Some(top) = self.back.last()
            && !entry.should_push(top)
        {
            return;
        }

        self.back.push(entry);
        self.forward.clear();

        if self.back.len() > MAX_STACK_SIZE {
            self.back.remove(0);
        }
    }

    pub fn go_back(&mut self, current: E) -> Option<E> {
        let entry = self.back.pop()?;
        self.forward.push(current);
        Some(entry)
    }

    pub fn go_forward(&mut self, current: E) -> Option<E> {
        let entry = self.forward.pop()?;
        self.back.push(current);
        Some(entry)
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub fn peek_back(&self) -> Option<&E> {
        self.back.last()
    }

    pub fn peek_forward(&self) -> Option<&E> {
        self.forward.last()
    }

    pub fn discard_back(&mut self) -> Option<E> {
        self.back.pop()
    }

    pub fn discard_forward(&mut self) -> Option<E> {
        self.forward.pop()
    }
    pub fn clear(&mut self) {
        self.back.clear();
        self.forward.clear();
        self.pending = None;
        self.last_debounced_push = None;
        self.expected_focus_loss = None;
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&E) -> bool) {
        self.back.retain(|entry| keep(entry));
        self.forward.retain(|entry| keep(entry));
        if self.pending.as_ref().is_some_and(|entry| !keep(entry)) {
            self.pending = None;
            self.last_debounced_push = None;
        }
    }

    pub fn set_navigating(&mut self, navigating: bool) {
        self.is_navigating = navigating;
    }

    pub fn is_navigating(&self) -> bool {
        self.is_navigating
    }

    /// Marks that the next focus loss of `window` is caused by a navigation
    /// restore. The application layer should consume this with
    /// [`Self::take_expected_focus_loss`] and skip recording a history entry
    /// for that focus change, since it is system-driven and would otherwise
    /// clear the forward stack. The expectation is cleared by the next
    /// [`Self::push`] or [`Self::clear`] so it cannot suppress a later,
    /// unrelated focus change.
    pub fn expect_focus_loss(&mut self, window: WindowId) {
        self.expected_focus_loss = Some(window);
    }

    /// Consumes a pending focus-loss expectation for `window`. Returns `true`
    /// when the focus loss was expected (and recording should be skipped).
    pub fn take_expected_focus_loss(&mut self, window: WindowId) -> bool {
        if self.expected_focus_loss == Some(window) {
            self.expected_focus_loss = None;
            true
        } else {
            false
        }
    }

    pub fn push_debounced(&mut self, entry: E, ctx: &mut ModelContext<Self>) {
        self.store_pending(entry);
        let duration = self.debounce_duration;
        ctx.spawn(
            async move { Timer::after(duration).await },
            |stack, _, ctx| {
                stack.flush_if_expired();
                ctx.notify();
            },
        );
    }

    fn store_pending(&mut self, entry: E) {
        if self.is_navigating {
            return;
        }
        if self.pending.is_none() {
            self.pending = Some(entry);
            self.last_debounced_push = Some(Instant::now());
        } else {
            self.last_debounced_push = Some(Instant::now());
        }
    }

    pub fn flush(&mut self) {
        if let Some(entry) = self.pending.take() {
            self.last_debounced_push = None;
            self.push(entry);
        }
    }

    pub fn flush_if_expired(&mut self) {
        if let Some(last) = self.last_debounced_push
            && last.elapsed() >= self.debounce_duration
        {
            self.flush();
        }
    }

    pub fn entry_count(&self) -> usize {
        self.back.len() + self.forward.len()
    }

    #[cfg(test)]
    fn back_len(&self) -> usize {
        self.back.len()
    }

    #[cfg(test)]
    fn set_debounce_duration(&mut self, duration: Duration) {
        self.debounce_duration = duration;
    }
}

/// Framework-level actions for navigation history.
///
/// These can be dispatched via `ctx.dispatch_typed_action()` and handled by
/// whichever ancestor view owns the navigation stack (typically the root
/// workspace view).
#[derive(Debug)]
pub enum NavigationAction {
    GoBack,
    GoForward,
}

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod tests;
