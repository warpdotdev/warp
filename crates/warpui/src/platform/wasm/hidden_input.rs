//! Hidden input element for triggering the soft keyboard on mobile browsers.
//!
//! On mobile browsers, the soft keyboard only appears when a native HTML input element
//! is focused. This module creates and manages a hidden `<input>` element that can be
//! programmatically focused to trigger the keyboard.
//!
//! ## Sentinel Character Pattern
//!
//! We use a "sentinel character" pattern to capture mobile keyboard input reliably:
//! - The hidden input always contains a single space " " with the cursor after it
//! - This ensures the keyboard always sees "deletable" text, preventing the
//!   "Android Backspace" bug where empty inputs don't emit backspace events
//! - We listen to `input` events, process them, then reset the input
//!
//! Note: Cursor movement (e.g., iOS trackpad gesture) is not captured here - users tap
//! on the canvas to reposition the cursor. This is a known limitation of the hidden input
//! approach.

use std::cell::RefCell;
use std::rc::Rc;

use gloo::events::EventListener;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{HtmlInputElement, InputEvent, KeyboardEvent};

/// The ID used for the hidden input element in the DOM.
const HIDDEN_INPUT_ID: &str = "warp-soft-keyboard-input";

/// The sentinel character used to ensure backspace events are always emitted.
/// A single space ensures the keyboard always sees "deletable" content.
const SENTINEL: &str = " ";

/// Manages the hidden input element used to trigger the soft keyboard.
///
/// This struct holds a reference to the hidden input and manages its lifecycle.
/// It should be created once when the window is created on mobile WASM.
pub struct HiddenInput {
    element: HtmlInputElement,
    /// Stores event listeners to keep them alive.
    /// When this struct is dropped, the listeners will be cleaned up.
    _listeners: Vec<EventListener>,
}

/// Callback type for input events from the hidden input.
pub type InputCallback = Rc<RefCell<dyn FnMut(HiddenInputEvent)>>;

/// Events that can be emitted by the hidden input.
#[derive(Debug, Clone)]
pub enum HiddenInputEvent {
    /// Text was inserted via the soft keyboard.
    InsertText {
        /// The text that was inserted.
        text: String,
    },
    /// Backspace was pressed (deleteContentBackward).
    Backspace,
    /// Delete was pressed (deleteContentForward).
    Delete,
    /// The hidden input lost focus (keyboard was dismissed externally).
    Blur,
    /// A key was pressed (for keys like Enter that don't trigger input events).
    KeyDown {
        /// The key code (e.g., "Enter").
        key: String,
    },
}

impl HiddenInput {
    /// Resets the hidden input to its sentinel state.
    ///
    /// Sets the value to a single space and positions the cursor after it.
    /// This ensures backspace always has something to delete.
    fn reset_input_element(element: &HtmlInputElement) {
        element.set_value(SENTINEL);
        let _ = element.set_selection_range(1, 1);
    }

    /// Creates a new hidden input element and attaches it to the DOM.
    ///
    /// The input is styled to be invisible but still focusable by the browser.
    /// On mobile devices, focusing this input will trigger the soft keyboard.
    ///
    /// # Arguments
    /// * `callback` - A callback that will be invoked when input events occur.
    ///
    /// # Errors
    /// Returns an error if the DOM element cannot be created or configured.
    pub fn new(callback: InputCallback) -> Result<Self, JsValue> {
        let document = gloo::utils::document();

        // Check if element already exists (e.g., from a previous session)
        if let Some(existing) = document.get_element_by_id(HIDDEN_INPUT_ID) {
            existing.remove();
        }

        // Create the input element
        let element = document
            .create_element("input")?
            .dyn_into::<HtmlInputElement>()?;

        element.set_id(HIDDEN_INPUT_ID);
        element.set_type("text");

        // Apply styles to make it invisible but still focusable.
        // We use a combination of techniques to ensure the input doesn't affect layout
        // or become visible, while still being able to receive focus and trigger the
        // soft keyboard on mobile.
        let style = element.style();
        style.set_property("position", "fixed")?;
        style.set_property("left", "-9999px")?;
        style.set_property("top", "0")?;
        style.set_property("opacity", "0")?;
        style.set_property("width", "1px")?;
        style.set_property("height", "1px")?;
        style.set_property("border", "none")?;
        style.set_property("outline", "none")?;
        style.set_property("padding", "0")?;
        style.set_property("margin", "0")?;
        // iOS Safari auto-zooms the viewport when focusing inputs with font-size < 16px.
        // Setting 16px prevents this unwanted zoom behavior.
        style.set_property("font-size", "16px")?;
        // Prevent the hidden input from intercepting touch/pointer events.
        // Focus/blur will still work when called programmatically.
        style.set_property("pointer-events", "none")?;
        // Ensure the input is behind everything else
        style.set_property("z-index", "-1")?;
        // Disable autocorrect/autocomplete to get raw input
        element.set_attribute("autocomplete", "off")?;
        element.set_attribute("autocorrect", "off")?;
        element.set_attribute("autocapitalize", "off")?;
        element.set_attribute("spellcheck", "false")?;

        // Append to body
        gloo::utils::body().append_child(&element)?;

        // Initialize with sentinel character BEFORE setting up listeners
        Self::reset_input_element(&element);

        // Now set up event listeners
        let listeners = Self::setup_listeners(&element, callback);

        Ok(Self {
            element,
            _listeners: listeners,
        })
    }

    /// Sets up event listeners on the hidden input element.
    fn setup_listeners(element: &HtmlInputElement, callback: InputCallback) -> Vec<EventListener> {
        let mut listeners = Vec::new();

        // We use 'input' event (fires after modification) because 'beforeinput' preventDefault
        // doesn't work reliably on mobile browsers (iOS Safari, Android Chrome).
        let callback_clone = Rc::clone(&callback);
        let element_clone = element.clone();
        let input_listener = EventListener::new(element, "input", move |event| {
            let input_event = event.dyn_ref::<InputEvent>();

            // Don't process input events during IME composition.
            // Use the browser's built-in isComposing flag.
            if input_event.map(|e| e.is_composing()).unwrap_or(false) {
                return;
            }

            let input_type = input_event.map(|e| e.input_type()).unwrap_or_default();
            let input_data = input_event.and_then(|e| e.data());

            let hidden_event = match input_type.as_str() {
                "insertText" | "insertCompositionText" => input_data
                    .filter(|s| !s.is_empty())
                    .map(|text| HiddenInputEvent::InsertText { text }),
                // Handle both single-char and word-level deletion (long-press backspace)
                "deleteContentBackward" | "deleteWordBackward" => Some(HiddenInputEvent::Backspace),
                "deleteContentForward" => Some(HiddenInputEvent::Delete),
                _ => None,
            };

            // Always reset to sentinel state after processing
            Self::reset_input_element(&element_clone);

            if let Some(hidden_event) = hidden_event {
                callback_clone.borrow_mut()(hidden_event);
            }
        });
        listeners.push(input_listener);

        // Composition end event - ensures we reset the sentinel after IME composition completes.
        // This handles CJK input (Chinese, Japanese, Korean, etc.) where the final composed
        // text is committed.
        let callback_clone = Rc::clone(&callback);
        let element_clone = element.clone();
        let composition_end_listener =
            EventListener::new(element, "compositionend", move |event| {
                log::debug!("IME composition ended");

                // Get the final composed text
                let comp_event = event.dyn_ref::<web_sys::CompositionEvent>();
                let data = comp_event.and_then(|e| e.data()).unwrap_or_default();

                // Reset the input to sentinel state
                Self::reset_input_element(&element_clone);

                // Send the final text if non-empty
                if !data.is_empty() {
                    callback_clone.borrow_mut()(HiddenInputEvent::InsertText { text: data });
                }
            });
        listeners.push(composition_end_listener);

        // Focus event - reset input when focused to ensure clean state
        let element_clone = element.clone();
        let focus_listener = EventListener::new(element, "focus", move |_| {
            Self::reset_input_element(&element_clone);
        });
        listeners.push(focus_listener);

        // Blur event - fires when the hidden input loses focus (keyboard dismissed)
        let callback_clone = Rc::clone(&callback);
        let blur_listener = EventListener::new(element, "blur", move |_| {
            log::debug!("Hidden input blur event - keyboard dismissed externally");
            callback_clone.borrow_mut()(HiddenInputEvent::Blur);
        });
        listeners.push(blur_listener);

        // Keydown event - for keys like Enter that don't trigger input events
        let callback_clone = Rc::clone(&callback);
        let keydown_listener = EventListener::new(element, "keydown", move |event| {
            if let Some(keyboard_event) = event.dyn_ref::<KeyboardEvent>() {
                let key = keyboard_event.key();
                // Only forward Enter key - other keys are handled via input events
                if key == "Enter" {
                    callback_clone.borrow_mut()(HiddenInputEvent::KeyDown { key });
                }
            }
        });
        listeners.push(keydown_listener);

        listeners
    }

    /// Focuses the hidden input element, which triggers the soft keyboard on mobile.
    pub fn focus(&self) -> Result<(), JsValue> {
        self.element.focus()
    }

    /// Blurs (unfocuses) the hidden input element, which dismisses the soft keyboard.
    pub fn blur(&self) -> Result<(), JsValue> {
        self.element.blur()
    }

    /// Returns whether the hidden input currently has focus.
    pub fn has_focus(&self) -> bool {
        gloo::utils::document()
            .active_element()
            .map(|el| el.id() == HIDDEN_INPUT_ID)
            .unwrap_or(false)
    }

    /// Resets the hidden input to its sentinel state.
    ///
    /// Sets the value to a single space and positions the cursor after it.
    /// This ensures backspace always has something to delete.
    pub fn reset_input(&self) {
        Self::reset_input_element(&self.element);
    }
}

impl Drop for HiddenInput {
    fn drop(&mut self) {
        // Remove the element from the DOM when the HiddenInput is dropped
        self.element.remove();
    }
}
