//! wasm32-unknown-unknown + Node prototype entrypoint that drives the
//! `AgentDriver` path as far as is constructible on a DOM-free wasm runtime
//! (REMOTE-2264).
//!
//! This is a **proof-of-concept for learning**, not a production feature (see
//! `agents/specs/REMOTE-2264: wasm32 CLI in Node prototype.md`).
//!
//! # Approach
//!
//! Per the approved spec the full `AgentDriver` is the primary path. This
//! module constructs a headless [`warpui::App`] (with an [`AppContext`])
//! **without** running the blocking platform event loop — using
//! [`warpui::platform::headless::new_headless_app`], which builds the headless
//! platform impls and returns the `App` directly. On wasm the foreground/
//! background executors schedule via `wasm_bindgen_futures::spawn_local`, so a
//! `#[wasm_bindgen] pub async fn` can spawn work on the app's foreground
//! executor and `await` it; the JS event loop polls the spawned future as this
//! async fn yields. No DOM/`window`/`document` is required.
//!
//! It then runs a trimmed init (feature flags, `AppExecutionMode`, auth state
//! with the API key, `ServerApiProvider`, `AuthManager`) — the minimum
//! `AgentDriver::new` reads — and reports the furthest stage reached. The
//! remaining concrete blockers for the full `AgentDriver::run` (documented in
//! the findings) are: (a) `AgentDriver::new` requires a `ModelContext<Self>`
//! and a `TerminalDriver` backed by a real `TerminalView`, whose
//! shell/PTY bootstrap is `local_tty`-gated and unavailable on
//! `wasm32-unknown-unknown`; (b) the `http_client` wasm transport uses
//! `web_sys::window()` (browser-only), so the MAA request needs a host-`fetch`
//! injection to cross the Node boundary; (c) `AgentDriver::run_internal` reads
//! many singleton models (skills, MCP, environment, blocklist permissions,
//! execution profiles) that a trimmed init does not register. This module
//! surfaces those as structured errors rather than silently falling back to the
//! direct MAA path.

use std::panic::{AssertUnwindSafe, catch_unwind};

use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::auth::AuthStateProvider;
use crate::features;
use crate::server::server_api::ServerApiProvider;

const STAGE_CONSTRUCTED_APP: &str = "constructed_app";
const STAGE_TRIMMED_INIT: &str = "trimmed_init";

#[derive(Debug, Serialize)]
struct DriverResult {
    ok: bool,
    stage: String,
    error: Option<String>,
    /// The concrete next blocker for the full AgentDriver run, when the
    /// constructible stages succeed.
    next_blocker: Option<String>,
}

const NEXT_BLOCKER: &str = concat!(
    "AgentDriver::new needs a ModelContext<Self> + a TerminalDriver backed by a ",
    "real TerminalView whose shell/PTY bootstrap is local_tty-gated (unavailable ",
    "on wasm32-unknown-unknown); http_client's wasm transport uses ",
    "web_sys::window() (browser-only) so the MAA request needs a host-fetch ",
    "injection; and AgentDriver::run_internal reads many singleton models ",
    "(skills, MCP, environment, blocklist permissions, execution profiles) a ",
    "trimmed init does not register."
);

/// Run one authenticated agent request through the `AgentDriver` path as far
/// as is constructible on a DOM-free wasm runtime.
///
/// `config` is a JS object: `{ prompt, api_key, server_root_url }`.
///
/// Returns a structured JSON result naming the furthest stage reached
/// (`constructed_app` → `trimmed_init`) and, when the constructible stages
/// succeed, the concrete next blocker for the full `AgentDriver::run`.
#[wasm_bindgen]
pub async fn run_agent_driver_wasm(config: JsValue) -> JsValue {
    console_error_panic_hook::set_once();

    let prompt = js_string(&config, "prompt").unwrap_or_default();
    let api_key = js_string(&config, "api_key").unwrap_or_default();
    let server_root_url = js_string(&config, "server_root_url").unwrap_or_default();

    if prompt.trim().is_empty() {
        return result_to_js(&err_result("missing prompt", STAGE_CONSTRUCTED_APP, None));
    }
    if api_key.trim().is_empty() {
        return result_to_js(&err_result("missing api_key", STAGE_CONSTRUCTED_APP, None));
    }
    if server_root_url.trim().is_empty() {
        return result_to_js(&err_result(
            "missing server_root_url",
            STAGE_CONSTRUCTED_APP,
            None,
        ));
    }

    // Override the server root URL before any auth/server-client construction.
    if let Err(e) =
        warp_core::channel::ChannelState::override_server_root_url(server_root_url.clone())
    {
        return result_to_js(&err_result(
            format!("invalid server_root_url: {e}"),
            STAGE_CONSTRUCTED_APP,
            None,
        ));
    }

    // 1. Construct a headless App (with AppContext) WITHOUT the blocking event
    //    loop. On wasm the foreground executor schedules via spawn_local.
    let mut app = match catch_unwind(AssertUnwindSafe(|| {
        warpui::platform::headless::new_headless_app(Box::new(warp_assets::Assets))
    })) {
        Ok(Ok(app)) => app,
        Ok(Err(e)) => {
            return result_to_js(&err_result(
                format!("new_headless_app: {e:#}"),
                STAGE_CONSTRUCTED_APP,
                None,
            ));
        }
        Err(p) => {
            return result_to_js(&err_result(
                panic_str(&p).unwrap_or_else(|| "new_headless_app panicked".to_string()),
                STAGE_CONSTRUCTED_APP,
                None,
            ));
        }
    };

    // 2. Trimmed init inside an AppContext update closure. Capture panics
    //    (e.g. missing singleton models) as structured errors.
    let api_key_for_init = api_key.clone();
    let result: Result<(), (String, &'static str)> = catch_unwind(AssertUnwindSafe(|| {
        app.update(|ctx| trimmed_init(ctx, &api_key_for_init))
    }))
    .map_err(|p| {
        (
            panic_str(&p).unwrap_or_else(|| "app update panicked".to_string()),
            STAGE_CONSTRUCTED_APP,
        )
    })
    .and_then(|inner| inner);

    match result {
        Ok(()) => result_to_js(&DriverResult {
            ok: true,
            stage: STAGE_TRIMMED_INIT.to_string(),
            error: None,
            next_blocker: Some(NEXT_BLOCKER.to_string()),
        }),
        Err((msg, stage)) => result_to_js(&err_result(msg, stage, None)),
    }
}

fn trimmed_init(ctx: &mut warpui::AppContext, api_key: &str) -> Result<(), (String, &'static str)> {
    features::init_feature_flags();

    // AppExecutionMode: SDK / not sandboxed. AgentDriver reads autonomy/isolation
    // from this.
    ctx.add_singleton_model(|ctx| {
        warp_core::execution_mode::AppExecutionMode::new(
            warp_core::execution_mode::ExecutionMode::Sdk,
            false,
            ctx,
        )
    });

    // Auth: construct AuthState with the API key (initialize() formats the
    // `wk-` prefix and returns early before any secure-storage read), then
    // register AuthStateProvider + ServerApiProvider + AuthManager (the
    // minimum AgentDriver::new reads).
    let auth_state = std::sync::Arc::new(warp_server_auth::auth_state::AuthState::initialize(
        ctx,
        Some(api_key.to_string()),
    ));
    let auth_state_for_provider = auth_state.clone();
    ctx.add_singleton_model(move |_ctx| AuthStateProvider::new(auth_state_for_provider));

    let server_api_provider = ctx.add_singleton_model({
        move |ctx| {
            ServerApiProvider::new(
                auth_state,
                Some(crate::ai::ambient_agents::AgentSource::Cli),
                None,
                ctx,
            )
        }
    });
    let server_api = server_api_provider.as_ref(ctx).get();
    let auth_client = server_api_provider.as_ref(ctx).get_auth_client();
    ctx.add_singleton_model(move |ctx| {
        crate::auth::auth_manager::AuthManager::new(server_api.clone(), auth_client.clone(), ctx)
    });

    Ok(())
}

fn err_result(err: impl Into<String>, stage: &str, next_blocker: Option<String>) -> DriverResult {
    DriverResult {
        ok: false,
        stage: stage.to_string(),
        error: Some(err.into()),
        next_blocker,
    }
}

fn result_to_js(result: &DriverResult) -> JsValue {
    serde_json::to_string(result)
        .map(|s| JsValue::from_str(&s))
        .unwrap_or_else(|e| {
            JsValue::from_str(&format!(
                "{{\"ok\":false,\"stage\":\"\",\"error\":\"serialize: {e}\"}}"
            ))
        })
}

fn js_string(obj: &JsValue, key: &str) -> Option<String> {
    js_sys::Reflect::get(obj, &JsValue::from_str(key))
        .ok()
        .and_then(|v| v.as_string())
}

fn panic_str(p: &Box<dyn std::any::Any + Send>) -> Option<String> {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&'static str>().map(|s| s.to_string()))
}
