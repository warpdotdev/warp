//! Desktop-only DOM text-input bridge for Warp for Web.
//!
//! Warp for Web renders its entire UI onto a single `<canvas>`, so there is no focused editable
//! DOM element for OS-level dictation and text-input services (e.g. MacWhisper, macOS Dictation)
//! to attach to. This module creates one hidden-but-accessible `<textarea>` and keeps it focused
//! for as long as the focused Warp view reports an active editable text caret (see
//! [`crate::CursorInfo`] and `View::active_cursor_position`). It is a sentinel input sink: the
//! textarea never mirrors the focused view's contents, selection, or caret. The focused Warp view
//! remains the single source of truth for all of those.
//!
//! This is a desktop-only counterpart to the mobile [`super::soft_keyboard`] path, which stays
//! completely unchanged; the two managers are constructed based exclusively on
//! [`super::is_mobile_device`], and never run at the same time.
//!
//! Event ownership, to avoid double insertion:
//! - Hardware keys are forwarded from this module's own `keydown`/`keyup` listeners (winit's
//!   canvas listeners never see them once the textarea is focused). Non-composing keys that
//!   aren't on the browser-shortcut allowlist have their default browser action prevented after
//!   being forwarded, so the textarea never also emits a matching `input` event. Browser
//!   shortcuts (new-tab, reload, history navigation, etc.) are left entirely to the browser: no
//!   Warp event, default action, and propagation untouched.
//! - Dictation and other direct DOM insertion arrive only through `input`; composing and
//!   post-composition-commit `input` events are filtered out by [`reducer::CompositionTracker`].
//! - CJK/IME composition arrives only through the `composition*` events.
//! - Paste is handled by this module's own `paste` listener, which stops propagation so the
//!   document-level paste listener (`platform::wasm::add_paste_listener`) never also sees it.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use gloo::events::EventListener;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CompositionEvent, HtmlTextAreaElement, InputEvent, KeyboardEvent};
use winit::event_loop::EventLoopProxy;

use crate::platform::desktop_text_input_reducer::{
    self as reducer, CompositionAction, CompositionTracker, DesktopKeyboardPayload, DomKeyEventKind,
};
use crate::windowing::winit::app::{ClipboardEvent, CustomEvent};
use crate::{CursorInfo, EntityId};

/// The ID used for the desktop text-input bridge element in the DOM.
const ELEMENT_ID: &str = "warp-desktop-text-input";

/// Input received from the desktop text-input bridge, forwarded to the event loop for dispatch.
#[derive(Debug, Clone)]
pub enum DesktopTextInputEvent {
    /// A hardware key transition captured while the bridge owned focus.
    Key(reducer::KeyConversion),
    /// Dictation or other direct DOM insertion.
    Insert(String),
    /// A deletion inferred from an `input` event.
    Delete(reducer::DeleteDirection),
    /// Composition preedit text changed.
    CompositionUpdate {
        text: String,
        selection: Range<usize>,
    },
    /// Composition committed non-empty text.
    CompositionCommit(String),
    /// Composition was cancelled (blurred, or abandoned by a focus change) before commit.
    CompositionCancelled,
}

impl From<CompositionAction> for DesktopTextInputEvent {
    fn from(action: CompositionAction) -> Self {
        match action {
            CompositionAction::Update { text, selection } => {
                DesktopTextInputEvent::CompositionUpdate { text, selection }
            }
            CompositionAction::Commit(text) => DesktopTextInputEvent::CompositionCommit(text),
            CompositionAction::Cancel => DesktopTextInputEvent::CompositionCancelled,
        }
    }
}

/// Manages the desktop text-input bridge element and its lifecycle.
///
/// Should only be constructed on desktop browsers; mobile continues to use
/// [`super::soft_keyboard::SoftKeyboardManager`] instead (see [`super::is_mobile_device`]).
pub struct DesktopTextInputManager {
    element: HtmlTextAreaElement,
    composition: Rc<RefCell<CompositionTracker>>,
    /// The view currently reporting the active caret. Lets [`Self::sync`] detect focus moving
    /// directly between two editable surfaces, since the bridge itself never loses DOM focus
    /// during such a transition (see [`CursorInfo::view_id`]).
    active_surface: Cell<Option<EntityId>>,
    /// Stores event listeners to keep them alive. When this struct is dropped, the listeners will
    /// be cleaned up.
    _listeners: Vec<EventListener>,
}

impl DesktopTextInputManager {
    /// Resets the bridge's textarea to its sentinel state: a single space with the cursor
    /// (collapsed selection) after it. See [`reducer::SENTINEL`] for why a sentinel is used.
    fn reset_input_element(element: &HtmlTextAreaElement) {
        element.set_value(reducer::SENTINEL);
        let sentinel_len = reducer::SENTINEL.encode_utf16().count() as u32;
        let _ = element.set_selection_range(sentinel_len, sentinel_len);
    }

    /// Creates the bridge's backing `<textarea>` and wires up its event listeners.
    pub fn new(proxy: EventLoopProxy<CustomEvent>) -> Result<Rc<Self>, JsValue> {
        let document = gloo::utils::document();

        if let Some(existing) = document.get_element_by_id(ELEMENT_ID) {
            existing.remove();
        }

        let element = document
            .create_element("textarea")?
            .dyn_into::<HtmlTextAreaElement>()?;
        element.set_id(ELEMENT_ID);
        element.set_attribute("aria-label", "Warp text input")?;
        element.set_attribute("aria-multiline", "true")?;
        element.set_attribute("autocomplete", "off")?;
        element.set_attribute("autocorrect", "off")?;
        element.set_attribute("autocapitalize", "off")?;
        element.set_attribute("spellcheck", "false")?;

        // Transparent and minimally sized, but deliberately NOT display:none/visibility:hidden/
        // aria-hidden/off-screen: those choices would remove the element from the accessibility
        // tree or place dictation/IME UI outside the viewport. `pointer-events: none` keeps it
        // from intercepting hit-testing or scrolling; `position_at` moves it to the active caret.
        let style = element.style();
        style.set_property("position", "fixed")?;
        style.set_property("opacity", "0")?;
        style.set_property("width", "1px")?;
        style.set_property("height", "1em")?;
        style.set_property("border", "none")?;
        style.set_property("outline", "none")?;
        style.set_property("padding", "0")?;
        style.set_property("margin", "0")?;
        style.set_property("resize", "none")?;
        style.set_property("pointer-events", "none")?;
        style.set_property("left", "0")?;
        style.set_property("top", "0")?;

        gloo::utils::body().append_child(&element)?;
        Self::reset_input_element(&element);

        let composition = Rc::new(RefCell::new(CompositionTracker::default()));
        let listeners = Self::setup_listeners(&element, proxy.clone(), &composition);

        Ok(Rc::new(Self {
            element,
            composition,
            active_surface: Cell::new(None),
            _listeners: listeners,
        }))
    }

    fn setup_listeners(
        element: &HtmlTextAreaElement,
        proxy: EventLoopProxy<CustomEvent>,
        composition: &Rc<RefCell<CompositionTracker>>,
    ) -> Vec<EventListener> {
        let mut listeners = Vec::new();

        for (event_type, kind) in [
            ("keydown", DomKeyEventKind::Down),
            ("keyup", DomKeyEventKind::Up),
        ] {
            let proxy = proxy.clone();
            let composition = Rc::clone(composition);
            listeners.push(EventListener::new(element, event_type, move |event| {
                let event = event
                    .dyn_ref::<KeyboardEvent>()
                    .expect("keyboard event listener should receive a KeyboardEvent");

                // Browser/OS shortcuts (new-tab, reload, history navigation, etc.) must keep
                // working untouched: no Warp event, and leave preventDefault/propagation to the
                // browser entirely.
                if super::is_browser_shortcut(event) {
                    return;
                }

                let payload = DesktopKeyboardPayload {
                    kind,
                    key: event.key(),
                    code: event.code(),
                    ctrl: event.ctrl_key(),
                    alt: event.alt_key(),
                    shift: event.shift_key(),
                    meta: event.meta_key(),
                    is_composing: event.is_composing() || composition.borrow().is_composing(),
                };

                // Composition owns the key stream, and keys the bridge doesn't forward as a
                // dedicated event (e.g. a bare dead key) are left to native handling.
                let Some(conversion) = reducer::convert_key(&payload) else {
                    return;
                };

                // Accept ownership: block the textarea's own default handling of this key so it
                // can't also mutate the value and fire a matching `input` event.
                event.prevent_default();
                event.stop_propagation();

                let _ = proxy.send_event(CustomEvent::DesktopTextInput(
                    DesktopTextInputEvent::Key(conversion),
                ));
            }));
        }

        {
            let proxy = proxy.clone();
            let element_clone = element.clone();
            let composition = Rc::clone(composition);
            listeners.push(EventListener::new(element, "input", move |event| {
                let input_event = event.dyn_ref::<InputEvent>();
                let is_composing_event = input_event.map(|e| e.is_composing()).unwrap_or(false);
                let data = input_event.and_then(|e| e.data());

                if composition
                    .borrow_mut()
                    .should_ignore_input_event(is_composing_event, data.as_deref())
                {
                    // Either mid-composition (composition events own the text) or the trailing
                    // `input` event a committing `compositionend` already accounted for.
                    return;
                }

                let input_type = input_event.map(|e| e.input_type()).unwrap_or_default();

                let desktop_event = match reducer::classify_input_type(&input_type) {
                    reducer::InputClassification::Insert => data
                        .filter(|s| !s.is_empty())
                        .or_else(|| {
                            reducer::extract_inserted_text(
                                reducer::SENTINEL,
                                &element_clone.value(),
                            )
                        })
                        .map(DesktopTextInputEvent::Insert),
                    reducer::InputClassification::Delete(direction) => {
                        Some(DesktopTextInputEvent::Delete(direction))
                    }
                    reducer::InputClassification::Unsupported => None,
                };

                Self::reset_input_element(&element_clone);

                if let Some(desktop_event) = desktop_event {
                    let _ = proxy.send_event(CustomEvent::DesktopTextInput(desktop_event));
                }
            }));
        }

        {
            let composition = Rc::clone(composition);
            listeners.push(EventListener::new(
                element,
                "compositionstart",
                move |_event| {
                    composition.borrow_mut().on_composition_start();
                },
            ));
        }

        {
            let proxy = proxy.clone();
            let element_clone = element.clone();
            let composition = Rc::clone(composition);
            listeners.push(EventListener::new(
                element,
                "compositionupdate",
                move |event| {
                    let Some(comp_event) = event.dyn_ref::<CompositionEvent>() else {
                        return;
                    };
                    let text = comp_event.data().unwrap_or_default();
                    let start = element_clone
                        .selection_start()
                        .ok()
                        .flatten()
                        .unwrap_or_default() as usize;
                    let end = element_clone
                        .selection_end()
                        .ok()
                        .flatten()
                        .unwrap_or_default() as usize;
                    let selection = reducer::composition_selection_range(
                        reducer::SENTINEL.encode_utf16().count(),
                        text.encode_utf16().count(),
                        start,
                        end,
                    );
                    let action = composition.borrow().on_composition_update(text, selection);
                    let _ = proxy.send_event(CustomEvent::DesktopTextInput(action.into()));
                },
            ));
        }

        {
            let proxy = proxy.clone();
            let element_clone = element.clone();
            let composition = Rc::clone(composition);
            listeners.push(EventListener::new(
                element,
                "compositionend",
                move |event| {
                    let data = event
                        .dyn_ref::<CompositionEvent>()
                        .and_then(|e| e.data())
                        .unwrap_or_default();
                    Self::reset_input_element(&element_clone);

                    let action = composition.borrow_mut().on_composition_end(data);
                    let _ = proxy.send_event(CustomEvent::DesktopTextInput(action.into()));
                },
            ));
        }

        {
            let element_clone = element.clone();
            listeners.push(EventListener::new(element, "focus", move |_event| {
                Self::reset_input_element(&element_clone);
            }));
        }

        {
            let proxy = proxy.clone();
            let composition = Rc::clone(composition);
            listeners.push(EventListener::new(element, "blur", move |_event| {
                // If the browser didn't already fire `compositionend` before the blur (not
                // guaranteed cross-browser), abandon the in-progress composition rather than
                // committing unfinished text.
                if let Some(action) = composition.borrow_mut().reset() {
                    let _ = proxy.send_event(CustomEvent::DesktopTextInput(action.into()));
                }
            }));
        }

        {
            let proxy = proxy.clone();
            listeners.push(EventListener::new(element, "paste", move |event| {
                let Some(clipboard_event) = event.dyn_ref::<web_sys::ClipboardEvent>() else {
                    return;
                };
                // Prevent the sentinel value from being mutated, and stop the document-level
                // paste listener (`platform::wasm::add_paste_listener`) from also handling it.
                clipboard_event.prevent_default();
                clipboard_event.stop_propagation();

                let Some(data) = clipboard_event.clipboard_data() else {
                    log::warn!(
                        "Desktop text-input bridge received a paste event without clipboard data."
                    );
                    return;
                };

                let content = crate::clipboard::ClipboardContent {
                    plain_text: data.get_data("text").unwrap_or_default(),
                    html: data.get_data("text/html").ok().filter(|s| !s.is_empty()),
                    ..Default::default()
                };

                let _ = proxy.send_event(CustomEvent::Clipboard(ClipboardEvent::Paste(content)));
            }));
        }

        // The document only becomes hidden on a tab switch/minimize; switching to another
        // application while the browser window stays visible does not change `visibilityState`.
        // Cover that case with real window focus/blur below.
        {
            let proxy = proxy.clone();
            listeners.push(EventListener::new(
                &gloo::utils::document(),
                "visibilitychange",
                move |_event| {
                    // Re-evaluate the focus gate; the event loop's handler already applies the
                    // "restore focus only if the browser document is active" rule.
                    let _ = proxy.send_event(CustomEvent::ActiveCursorPositionUpdated);
                },
            ));
        }

        for window_event_type in ["blur", "focus"] {
            let proxy = proxy.clone();
            listeners.push(EventListener::new(
                &gloo::utils::window(),
                window_event_type,
                move |_event| {
                    let _ = proxy.send_event(CustomEvent::ActiveCursorPositionUpdated);
                },
            ));
        }

        listeners
    }

    /// Re-evaluates whether the bridge should be focused, based on whether the focused Warp view
    /// reports an active editable text caret and whether the browser document is currently
    /// active. Also repositions the bridge at the active caret when one is present.
    pub fn sync(&self, cursor: Option<&CursorInfo>) {
        let new_surface = cursor.map(|c| c.view_id);
        if self.active_surface.replace(new_surface) != new_surface {
            self.handle_surface_change();
        }

        if let Some(cursor) = cursor {
            self.position_at(cursor);
        }

        let document_active = gloo::utils::document().has_focus().unwrap_or(false);
        let should_focus = document_active && cursor.is_some();

        if should_focus
            && !self.has_focus()
            && let Err(err) = self.element.focus()
        {
            log::warn!("Failed to focus desktop text-input bridge: {err:?}");
        } else if !should_focus && self.has_focus() {
            let _ = self.element.blur();
        }
    }

    /// Resets the bridge's own composition state and sentinel when the caret-owning surface
    /// changes, so a pending sentinel diff or composition bookkeeping from the old surface can't
    /// leak into a newly focused one that also reports an active caret.
    ///
    /// Deliberately does *not* dispatch a `ClearMarkedText`/cancellation event here: by the time
    /// this runs, Warp's own focus has already moved to the new surface (this bridge's `sync()`
    /// only observes that change after the fact), so a dispatched event would be delivered to the
    /// new surface, not the one that was actually composing. Clearing the old surface's own
    /// marked text is instead each editable view's own responsibility on `on_blur`: `EditorView`
    /// and `TerminalView` both discard (never commit) their own incomplete marked text there,
    /// since that's the only place that observes the old surface's own focus loss.
    fn handle_surface_change(&self) {
        self.composition.borrow_mut().reset();
        Self::reset_input_element(&self.element);
    }

    /// Returns whether the bridge's textarea currently has focus.
    pub fn has_focus(&self) -> bool {
        gloo::utils::document()
            .active_element()
            .map(|el| el.id() == ELEMENT_ID)
            .unwrap_or(false)
    }

    /// Positions the bridge at `cursor`'s caret, converting from canvas-relative logical
    /// coordinates to viewport CSS coordinates via the canvas's bounding rect.
    fn position_at(&self, cursor: &CursorInfo) {
        let Some(canvas) = gloo::utils::document()
            .query_selector("canvas")
            .ok()
            .flatten()
        else {
            return;
        };
        let canvas_rect = canvas.get_bounding_client_rect();
        let x = canvas_rect.left() + cursor.position.origin_x() as f64;
        let y = canvas_rect.top() + cursor.position.origin_y() as f64;

        let style = self.element.style();
        let _ = style.set_property("left", &format!("{x}px"));
        let _ = style.set_property("top", &format!("{y}px"));
        let _ = style.set_property("font-size", &format!("{}px", cursor.font_size));
        let _ = style.set_property("line-height", &format!("{}px", cursor.font_size));
    }
}

impl Drop for DesktopTextInputManager {
    fn drop(&mut self) {
        self.element.remove();
    }
}
