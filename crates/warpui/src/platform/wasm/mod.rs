pub(crate) mod hidden_input;
pub(crate) mod mobile_detection;
pub(crate) mod soft_keyboard;

use gloo::events::{EventListener, EventListenerOptions};
pub use hidden_input::{HiddenInput, HiddenInputEvent, InputCallback};
pub use mobile_detection::{is_mobile_device, is_mobile_user_agent};
pub use soft_keyboard::{SoftKeyboardInput, SoftKeyboardManager, SoftKeyboardState};
// Re-export the functions from the core crate.
pub use warpui_core::platform::wasm::*;
use wasm_bindgen::{JsCast, UnwrapThrowExt};

use super::KEYS_TO_IGNORE;
use crate::keymap::Keystroke;
// Re-export a couple winit types and modules as the concrete implementations
// for the wasm platform.
pub use crate::windowing::winit::app::App;
use crate::windowing::winit::app::CustomEvent;

fn get_visual_viewport_dimensions() -> Option<(f32, f32)> {
    let window = gloo::utils::window();
    let vv = js_sys::Reflect::get(&window, &"visualViewport".into()).ok()?;
    let width = js_sys::Reflect::get(&vv, &"width".into())
        .ok()
        .and_then(|v| v.as_f64())? as f32;
    let height = js_sys::Reflect::get(&vv, &"height".into())
        .ok()
        .and_then(|v| v.as_f64())? as f32;
    (width > 0.0 && height > 0.0).then_some((width, height))
}

/// Listens for visual viewport changes (e.g., soft keyboard appearing) on mobile.
pub(crate) fn setup_visual_viewport_resize_listener(
    event_loop_proxy: winit::event_loop::EventLoopProxy<CustomEvent>,
) {
    if !mobile_detection::is_mobile_device() {
        return;
    }

    let window = gloo::utils::window();
    let visual_viewport = js_sys::Reflect::get(&window, &"visualViewport".into())
        .ok()
        .and_then(|v| v.dyn_into::<web_sys::EventTarget>().ok());

    let Some(visual_viewport) = visual_viewport else {
        log::warn!("Visual viewport API not available");
        return;
    };

    // Fire once immediately so the first render uses the correct visual viewport height.
    if let Some((width, height)) = get_visual_viewport_dimensions() {
        let _ = event_loop_proxy.send_event(CustomEvent::VisualViewportResized { width, height });
    }

    EventListener::new(&visual_viewport, "resize", move |_| {
        if let Some((width, height)) = get_visual_viewport_dimensions() {
            log::debug!("Visual viewport resized to {}x{}", width, height);
            let _ =
                event_loop_proxy.send_event(CustomEvent::VisualViewportResized { width, height });
        }
    })
    .forget();
}

/// Adds an event listener to the main canvas element which calls preventDefault on all important
/// events except for those we explicitly want to pass through to the browser.
pub(crate) fn add_prevent_default_listener(canvas: &web_sys::HtmlCanvasElement) {
    // Event types where we unconditionally call prevent_default.
    let events_types_to_prevent = [
        "touchstart",
        "wheel",
        "contextmenu",
        "pointerdown",
        "pointermove",
    ];

    // Keyboard events where we call prevent_default in some cases.
    let key_events_to_partially_prevent = ["keyup", "keydown"];

    for event_type in events_types_to_prevent.into_iter() {
        let prevent_default_listener = Box::new(EventListener::new_with_options(
            canvas,
            event_type,
            EventListenerOptions::enable_prevent_default(),
            move |event| {
                event.prevent_default();
            },
        ));

        // We want this to live for the lifetime of the page and we're never going to need to
        // interact with it again, so we leak it so it can live forever.
        Box::leak(prevent_default_listener);
    }

    for event_type in key_events_to_partially_prevent.into_iter() {
        let prevent_default_listener = Box::new(EventListener::new_with_options(
            canvas,
            event_type,
            EventListenerOptions::enable_prevent_default(),
            move |event| {
                let event = event.dyn_ref::<web_sys::KeyboardEvent>().unwrap_throw();
                let keystroke = Keystroke {
                    ctrl: event.ctrl_key(),
                    alt: event.alt_key(),
                    shift: event.shift_key(),
                    cmd: event.meta_key(), // The browser's 'meta' corresponds to our 'command'.
                    meta: false,
                    key: event.key(),
                };

                let allow_default_event = KEYS_TO_IGNORE.contains(&keystroke);
                if !allow_default_event {
                    event.prevent_default();
                }
            },
        ));
        Box::leak(prevent_default_listener);
    }
}

pub(crate) fn add_paste_listener(event_loop_proxy: winit::event_loop::EventLoopProxy<CustomEvent>) {
    EventListener::new(&gloo::utils::document(), "paste", move |event| {
        let event = event.dyn_ref::<web_sys::ClipboardEvent>().unwrap_throw();
        let Some(data) = event.clipboard_data() else {
            log::warn!("Received paste event without clipboard data.");
            return;
        };

        let content = crate::clipboard::ClipboardContent {
            plain_text: data.get_data("text").unwrap_or_default(),
            html: data
                .get_data("text/html")
                .ok()
                .and_then(|s| (!s.is_empty()).then_some(s)), // Set this to None if the html data is empty
            ..Default::default()
        };

        let _ = event_loop_proxy.send_event(CustomEvent::Clipboard(
            crate::windowing::winit::app::ClipboardEvent::Paste(content),
        ));
    })
    .forget();
}

pub(crate) fn add_network_connection_listener(
    event_loop_proxy: winit::event_loop::EventLoopProxy<CustomEvent>,
) {
    let event_loop_proxy_clone = event_loop_proxy.clone();

    EventListener::new(&gloo::utils::window(), "offline", move |_event| {
        let _ = event_loop_proxy_clone
            .send_event(crate::windowing::winit::app::CustomEvent::InternetDisconnected);
    })
    .forget();

    EventListener::new(&gloo::utils::window(), "online", move |_event| {
        let _ = event_loop_proxy
            .send_event(crate::windowing::winit::app::CustomEvent::InternetConnected);
    })
    .forget();
}

pub(crate) fn add_system_theme_listener(
    event_loop_proxy: winit::event_loop::EventLoopProxy<CustomEvent>,
) {
    // This could alternatively be written as a listener on "(prefers-color-scheme: light)".
    if let Ok(Some(media_query_list)) =
        gloo::utils::window().match_media("(prefers-color-scheme: dark)")
    {
        EventListener::new(&media_query_list, "change", move |_event| {
            let _ = event_loop_proxy
                .send_event(crate::windowing::winit::app::CustomEvent::SystemThemeChanged);
        })
        .forget();
    }
}
