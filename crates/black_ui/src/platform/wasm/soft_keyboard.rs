//! Soft keyboard support for mobile WASM.
//!
//! On mobile browsers, the soft keyboard only appears when a native HTML input element
//! is focused. This module provides utilities to manage a hidden input element to
//! trigger the soft keyboard when needed.
//!
//! ## Architecture
//!
//! - `SoftKeyboardManager`: Coordinates the hidden input element and keyboard state
//! - Mobile detection utilities are in the `mobile_detection` submodule

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsValue;

use super::hidden_input::{HiddenInput, HiddenInputEvent};

// ============================================================================
// Soft Keyboard State
// ============================================================================

/// Represents the visibility state of the soft keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SoftKeyboardState {
    /// The soft keyboard is hidden.
    #[default]
    Hidden,
    /// The soft keyboard is visible (or should be shown).
    Visible,
}

impl SoftKeyboardState {
    /// Returns true if the keyboard should be visible.
    pub fn is_visible(&self) -> bool {
        matches!(self, Self::Visible)
    }
}

/// Maps a HiddenInputEvent to a SoftKeyboardInput.
fn map_hidden_input_event(event: HiddenInputEvent) -> Option<SoftKeyboardInput> {
    match event {
        HiddenInputEvent::InsertText { text } => Some(SoftKeyboardInput::TextInserted(text)),
        HiddenInputEvent::Backspace | HiddenInputEvent::Delete => {
            Some(SoftKeyboardInput::Backspace)
        }
        HiddenInputEvent::Blur => Some(SoftKeyboardInput::KeyboardDismissed),
        HiddenInputEvent::KeyDown { key } => Some(SoftKeyboardInput::KeyDown(key)),
    }
}

// ============================================================================
// Soft Keyboard Manager
// ============================================================================

/// Callback type for soft keyboard input events.
/// The callback receives the processed input event.
pub type SoftKeyboardInputCallback = Box<dyn FnMut(SoftKeyboardInput)>;

/// Processed input from the soft keyboard.
#[derive(Debug, Clone)]
pub enum SoftKeyboardInput {
    /// Text was inserted.
    TextInserted(String),
    /// Backspace was pressed.
    Backspace,
    /// The keyboard was dismissed externally (e.g., iOS "Done" button).
    KeyboardDismissed,
    /// A special key was pressed (e.g., Enter).
    KeyDown(String),
}

/// Manages the soft keyboard for mobile WASM.
///
/// This struct coordinates:
/// - The hidden input element that triggers the keyboard
/// - The current keyboard state (visible/hidden)
/// - Processing input events and forwarding them to the app
///
/// # Usage
///
/// ```ignore
/// // Create the manager (only on mobile)
/// if mobile_detection::is_mobile_device() {
///     let manager = SoftKeyboardManager::new(|input| {
///         // Handle input from soft keyboard
///     })?;
///     
///     // Show keyboard when text input is focused
///     manager.show_keyboard();
///     
///     // Hide keyboard when text input is blurred
///     manager.hide_keyboard();
/// }
/// ```
pub struct SoftKeyboardManager {
    hidden_input: HiddenInput,
    state: RefCell<SoftKeyboardState>,
}

impl SoftKeyboardManager {
    /// Creates a new soft keyboard manager.
    ///
    /// This creates the hidden input element and sets up event forwarding.
    /// Should only be called on mobile devices (check `is_mobile_device()` first).
    ///
    /// # Arguments
    /// * `on_input` - Callback invoked when the user types on the soft keyboard.
    ///
    /// # Errors
    /// Returns an error if the hidden input element cannot be created.
    pub fn new(on_input: SoftKeyboardInputCallback) -> Result<Rc<Self>, JsValue> {
        let on_input = RefCell::new(on_input);

        // Create a callback that processes hidden input events and forwards them
        let callback: super::hidden_input::InputCallback =
            Rc::new(RefCell::new(move |event: HiddenInputEvent| {
                if let Some(input) = map_hidden_input_event(event) {
                    on_input.borrow_mut()(input);
                }
            }));

        let hidden_input = HiddenInput::new(callback)?;

        Ok(Rc::new(Self {
            hidden_input,
            state: RefCell::new(SoftKeyboardState::Hidden),
        }))
    }

    /// Shows the soft keyboard by focusing the hidden input.
    ///
    /// This should be called when a text input in the app gains focus.
    pub fn show_keyboard(&self) {
        // Always call focus() - the browser handles redundant calls gracefully.
        // We don't rely on our internal state because the user can dismiss the keyboard
        // via browser controls (e.g., "Done" button), which doesn't update our state.

        if let Err(e) = self.hidden_input.focus() {
            log::warn!("Failed to focus hidden input for soft keyboard: {:?}", e);
        }
        *self.state.borrow_mut() = SoftKeyboardState::Visible;
    }

    /// Hides the soft keyboard by blurring the hidden input.
    ///
    /// For canvas-based apps, this must be called explicitly when the user taps
    /// outside a text input area, since the browser can't detect "outside" taps
    /// when everything renders to a single canvas element.
    pub fn hide_keyboard(&self) {
        if let Err(e) = self.hidden_input.blur() {
            log::warn!("Failed to blur hidden input for soft keyboard: {:?}", e);
        }
        *self.state.borrow_mut() = SoftKeyboardState::Hidden;
    }

    /// Returns the current keyboard state.
    pub fn state(&self) -> SoftKeyboardState {
        *self.state.borrow()
    }

    /// Returns whether the soft keyboard is currently visible.
    pub fn is_visible(&self) -> bool {
        self.state.borrow().is_visible()
    }

    /// Returns whether the hidden input element currently has focus.
    ///
    /// This is used to detect when browser focus events are due to the soft keyboard
    /// rather than the user actually switching away from the window.
    pub fn has_focus(&self) -> bool {
        self.hidden_input.has_focus()
    }

    /// Resets the hidden input to its sentinel state.
    ///
    /// Sets the value to a single space and positions the cursor after it.
    /// This is automatically called on focus and after every input event,
    /// but can be called manually if needed.
    pub fn reset_input(&self) {
        self.hidden_input.reset_input();
    }
}

#[cfg(test)]
#[path = "soft_keyboard_tests.rs"]
mod tests;
