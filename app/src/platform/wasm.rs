use js_sys::ReferenceError;
use thiserror::Error;
pub use warp_web_event_bus::{WarpEvent, emit_event};
use wasm_bindgen::{JsCast, JsValue};

/// This function should be called early in application initialization to ensure that
/// static variables are initialized.
pub(super) fn init() {
    unsafe {
        unsafe extern "C" {
            /// __wasm_call_ctors is a function defined by the `wasm-ld` linker, and is used to
            /// initialize static variables.
            ///
            /// It should be called once at runtime before other code is executed.
            fn __wasm_call_ctors();
        }

        __wasm_call_ctors();
    }
}

mod ffi {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_name = "warpUserHandoff", catch)]
        pub fn user_handoff() -> Result<Option<String>, JsValue>;
    }
}

#[derive(Debug, Clone, Error)]
pub enum AuthHandoffError {
    #[error("The host page doesn't support user handoff")]
    Unsupported,
    #[error("Unexpected handoff error: {0:?}")]
    Unexpected(JsValue),
}

/// Fetch the user's Firebase refresh token from the host React app.
pub fn user_handoff() -> Result<Option<String>, AuthHandoffError> {
    ffi::user_handoff().map_err(|err| {
        if ReferenceError::instanceof(&err) {
            AuthHandoffError::Unsupported
        } else {
            AuthHandoffError::Unexpected(err)
        }
    })
}
