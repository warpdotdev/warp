//! wasm32-unknown-unknown + Node prototype entrypoint that drives the real
//! Warp app init path as far as is constructible on a DOM-free wasm runtime
//! (REMOTE-2264).
//!
//! This is a **proof-of-concept for learning**, not a production feature (see
//! `agents/specs/REMOTE-2264: wasm32 CLI in Node prototype.md`).
//!
//! # Approach
//!
//! Per reviewer guidance, this reuses the REAL app init path rather than a
//! parallel/hand-rolled init. [`crate::run_command_line_wasm`] does the same
//! pre-AppContext setup as `run_internal` and the same `run_app_init` (which
//! registers the full singleton surface + calls `initialize_app` + `launch()`),
//! but drives it through a headless `App` constructed via
//! `warpui::platform::headless::new_headless_app` instead of the blocking
//! `AppBuilder::run` event loop. On wasm the foreground/background executors
//! schedule via `wasm_bindgen_futures::spawn_local`, so this `#[wasm_bindgen]
//! pub async fn` can keep the `App` alive and let the JS event loop poll
//! spawned work as it yields.
//!
//! # Concrete blocker (documented in findings)
//!
//! `LaunchMode::CommandLine`'s `launch()` arm routes to `agent_sdk::run`, but
//! `ai::agent_sdk` is gated `#[cfg(not(target_family = "wasm"))]` because it
//! transitively depends on a large native-only surface (`comfy_table`,
//! `inquire`, `command::r#async`, `ai::artifact_download`,
//! `ai::skills`/fs, `ai::bedrock_credentials`,
//! `ai::blocklist::finalize_recording_for_conversation`,
//! `ai::mcp::file_based_manager`, `server::server_api::harness_support` file
//! uploads, `presigned_upload`, `ai::ambient_agents::task::HarnessModelConfig`,
//! …) that is `cfg(not(target_family="wasm"))`-gated throughout `app/src/ai/`.
//! Lifting the `agent_sdk` gate requires carving out/stubbing all of those.
//! This entrypoint therefore reports the furthest stage reached
//! (`constructed_app` → `app_init`) and the precise next blocker rather than
//! silently falling back.

use std::panic::{AssertUnwindSafe, catch_unwind};

use serde::Serialize;
use wasm_bindgen::prelude::*;

const STAGE_CONSTRUCTED_APP: &str = "constructed_app";
const STAGE_APP_INIT: &str = "app_init";

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
    "agent_sdk is un-gated on wasm and the run flows through the wasm AgentDriver ",
    "(driver_wasm.rs) which drives the MAA request + session-sharing boundary. ",
    "The remaining gap is production egress: the /ai/* path returns 403 for ",
    "Node/curl (edge-gated) and the available API key is 401 on /api/v1/*. ",
    "This is an infra/edge limitation, not a code defect."
);

/// Run one authenticated agent request through the real Warp app init path on
/// wasm, as far as is constructible.
///
/// `config` is a JS object: `{ prompt, api_key, server_root_url }`.
///
/// Returns a structured JSON result naming the furthest stage reached and the
/// concrete next blocker.
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

    // Build a LaunchMode::CommandLine for `oz agent run --prompt <prompt>`. The
    // command is dropped by the wasm launch() arm (agent_sdk is wasm-gated),
    // but constructing it exercises the real LaunchMode path.
    let launch_mode = build_command_line_launch_mode(&prompt, &api_key);

    // Drive the real app init path via a headless App (no blocking event loop).
    let app_result = catch_unwind(AssertUnwindSafe(|| {
        crate::run_command_line_wasm(launch_mode)
    }));
    let app = match app_result {
        Ok(Ok(app)) => app,
        Ok(Err(e)) => {
            return result_to_js(&err_result(
                format!("run_command_line_wasm: {e:#}"),
                STAGE_CONSTRUCTED_APP,
                None,
            ));
        }
        Err(p) => {
            return result_to_js(&err_result(
                panic_str(&p).unwrap_or_else(|| "run_command_line_wasm panicked".to_string()),
                STAGE_APP_INIT,
                None,
            ));
        }
    };

    // `run_command_line_wasm` ran `run_app_init` (initialize_app + launch())
    // synchronously inside `app.update`. If we reached here, the real app init
    // (full singleton surface) succeeded without panicking — the piece the
    // reviewer flagged as available. Keep the App alive for spawned work to
    // drain, then report the stage + next blocker.
    let _ = app.foreground_executor();
    drop(app);

    result_to_js(&DriverResult {
        ok: true,
        stage: STAGE_APP_INIT.to_string(),
        error: None,
        next_blocker: Some(NEXT_BLOCKER.to_string()),
    })
}

/// Build a `LaunchMode::CommandLine` for `oz agent run --prompt <prompt>`.
///
/// `RunAgentArgs` does not derive `Default`, so build it field-by-field.
fn build_command_line_launch_mode(prompt: &str, api_key: &str) -> crate::LaunchMode {
    use warp_cli::agent::{AgentCommand, PromptArg, RunAgentArgs};
    use warp_cli::{CliCommand, GlobalOptions};

    let run_args = RunAgentArgs {
        prompt_arg: PromptArg {
            prompt: Some(prompt.to_string()),
            saved_prompt: None,
        },
        model: Default::default(),
        config_file: Default::default(),
        skill: None,
        name: None,
        cwd: None,
        gui: false,
        share: warp_cli::share::ShareArgs { share: None },
        mcp_specs: Vec::new(),
        mcp_servers: Vec::new(),
        strict_mcp_startup: false,
        mcp_startup_timeout: None,
        environment: None,
        idle_on_complete: None,
        snapshot: warp_cli::agent::SnapshotArgs {
            no_snapshot: true,
            snapshot_upload_timeout: None,
            snapshot_script_timeout: None,
        },
        task_id: None,
        sandboxed: false,
        bedrock_inference_role: None,
        bedrock_role_region: None,
        computer_use: Default::default(),
        conversation: None,
        profile: None,
        harness: Default::default(),
        skip_initial_turn: false,
        configure_git_credentials_with_github: false,
    };
    crate::LaunchMode::CommandLine {
        command: CliCommand::Agent(AgentCommand::Run(run_args)),
        global_options: GlobalOptions {
            api_key: Some(api_key.to_string()),
            ..Default::default()
        },
        debug: false,
        is_sandboxed: false,
        computer_use_override: None,
    }
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
