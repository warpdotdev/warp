#![cfg(target_family = "wasm")]

use js_sys::ReferenceError;
use serde::Serialize;
use wasm_bindgen::JsCast;

/// Events emitted from Warp on Web to the host JavaScript app.
///
/// These must stay in sync with the [`WarpEvent` TypeScript type](https://github.com/warpdotdev/warp-server/blob/develop/client/src/warp-client/index.ts).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WarpEvent {
    LoggedOut,
    SessionJoined,
    ErrorLogged { error: String },
    OpenOnNative { url: String },
    ThemeBackgroundChanged { color: String },
}

mod ffi {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        /// Emit an event to the host app. This uses a global event bus implemented in JavaScript.
        ///
        /// If we need to support more complicated embeddings (like having multiple WoW instances
        /// on a page, or un- and re-initializing the app), this may not be flexible enough.
        ///
        /// Some things to consider:
        /// * Using native DOM events. We can get the window's backing `<canvas>` element using
        ///   [`winit::platform::web::WindowExtWebSys`], and dispatch events with custom data using
        ///   [`web_sys::CustomEvent`].
        /// * Passing context directly into the WASM script, instead of using globals. This isn't
        ///   directly supported by wasm-bindgen (see rustwasm/wasm-bindgen#3041 and
        ///   rustwasm/wasm-bindgen#3659), but may be in the future. We could also provide a
        ///   WASM-specific entrypoint (instead of `main`) that takes context before starting the
        ///   app.
        #[wasm_bindgen(js_name = "warpEmitEvent", catch)]
        pub fn emit_event(event: JsValue) -> Result<(), JsValue>;
    }
}

/// Emit an event to the host JavaScript app.
pub fn emit_event(event: WarpEvent) {
    let serialized =
        serde_wasm_bindgen::to_value(&event).expect("Event must convert to JavaScript");
    match ffi::emit_event(serialized) {
        Ok(()) => (),
        Err(err) if ReferenceError::instanceof(&err) => {
            // Assume that we're not running within a JavaScript host, so the FFI is unavailable.
        }
        Err(err) => {
            log::warn!("Unable to report {event:?}: {err:?}");
        }
    }
}
