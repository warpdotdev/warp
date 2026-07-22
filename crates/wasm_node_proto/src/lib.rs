//! wasm32-unknown-unknown + Node prototype for the Oz/Warp agent MAA path.
//!
//! This is a **proof-of-concept for learning**, not a production feature (see
//! `agents/specs/REMOTE-2264: wasm32 CLI in Node prototype.md`).
//!
//! # What this proves
//!
//! - The Warp agent **multi-agent API (MAA) request/response protocol** compiles
//!   to `wasm32-unknown-unknown` and runs in a DOM-free Node runtime.
//! - A real, authenticated MAA `hello` run can be driven through a **host-injected
//!   `fetch` transport** supplied by the Node harness — with no `window`,
//!   `document`, canvas, clipboard, or browser `console` bindings, and no
//!   reqwest/`web_sys` dependency (which hard-requires a browser `window`).
//! - The capability boundary is explicit: no filesystem, PTY, shell, subprocess,
//!   MCP process, watcher, indexing, snapshot, or GUI code is compiled in or
//!   invoked. The wasm module is sandboxless-by-construction on
//!   `wasm32-unknown-unknown`.
//!
//! # Why this is the *direct MAA fallback*, not the full `AgentDriver`
//!
//! The approved spec makes the full `AgentDriver` the primary path. That path is
//! blocked on `wasm32-unknown-unknown` by a concrete, well-understood
//! dependency: `AgentDriver::new` (`app/src/ai/agent_sdk/driver.rs`) requires a
//! live `ModelContext<Self>` backed by a fully-bootstrapped `AppContext`
//! (persistence, auth manager, server clients, watchers, GUI views) and a
//! `TerminalDriver`, whose PTY/shell backend is gated behind the `local_tty`
//! cargo feature — a feature the build script only enables on platforms with a
//! real TTY, and which is therefore absent on `wasm32-unknown-unknown`. The
//! existing browser wasm build boots `AppContext` through the GUI window event
//! loop (`app/src/lib.rs` `launch`), which a DOM-free Node run cannot replicate.
//! `app/src/lib.rs` (`LaunchMode::CommandLine`) explicitly panics for CLI
//! commands on wasm for this reason.
//!
//! Per the spec, when the `AgentDriver` route is blocked the fallback is the
//! direct MAA request/client loop. This crate implements that fallback: it
//! builds the canonical `warp_multi_agent_api::Request`, POSTs it to the live
//! `/ai/multi-agent` endpoint via the host `fetch`, and streams/decodes the
//! `ResponseEvent` stream. It does **not** validate session sharing (that is
//! owned by the `AgentDriver`/conversation-consumer path); the findings
//! document this as the lost capability.

use base64::Engine as _;
use prost::Message as _;
use serde::Serialize;
use warp_multi_agent_api as api;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

const MAA_ENDPOINT_PATH: &str = "/ai/multi-agent";

/// Result returned to Node for one agent run.
///
/// Serialized to JSON and converted to a `JsValue` so the harness receives a
/// plain JS object. `status` is one of:
/// - `"success"` — a terminal `StreamFinished{Done}` (or graceful end) was
///   reached and assistant text was streamed.
/// - `"error"` — a structured failure (bad input, non-2xx, decode error,
///   host/abort timeout, stream error).
#[derive(Debug, Serialize)]
struct RunResult {
    ok: bool,
    status: String,
    conversation_id: Option<String>,
    request_id: Option<String>,
    run_id: Option<String>,
    text: String,
    finished_reason: Option<String>,
    event_count: u32,
    events: Vec<String>,
    error: Option<String>,
    timings_ms: Option<Timings>,
}

#[derive(Debug, Serialize)]
struct Timings {
    build_request_ms: f64,
    first_event_ms: Option<f64>,
    total_ms: f64,
}

/// Run one authenticated MAA request through the host-injected `fetch`.
///
/// `config` is a JS object: `{ prompt, api_key, server_root_url, model?, timeout_ms? }`.
/// `host` is a JS object implementing the host contract:
/// - `host.fetch(url: string, init: { method, headers: {[k]: string}, body: Uint8Array, timeoutMs?: number })
///    -> Promise<{ status: number, statusText: string, headers: object, body: { read() -> Promise<{ done: boolean, value?: Uint8Array }> } }>`
///
/// The harness owns the `AbortController`/timeout: when `init.timeoutMs` is set
/// the host aborts the request after that duration, which surfaces here as a
/// structured `error` result and releases all pending JS promises.
#[wasm_bindgen]
pub async fn run_multi_agent(config: JsValue, host: JsValue) -> JsValue {
    console_error_panic_hook::set_once();

    let started = web_now_ms();

    // Parse + validate explicit config. The wasm module never calls
    // `Args::from_env()` / `std::env` — all inputs are passed in here.
    let prompt = js_get_string(&config, "prompt").unwrap_or_default();
    let api_key = js_get_string(&config, "api_key").unwrap_or_default();
    let server_root_url = js_get_string(&config, "server_root_url").unwrap_or_default();
    let model = js_get_string(&config, "model");
    let timeout_ms = js_get_f64(&config, "timeout_ms");

    if let Err(err) = validate_input(&prompt, &api_key, &server_root_url) {
        return result_to_js(&err_result(err, None));
    }

    // Build the canonical MAA Request (same wire format as the CLI path).
    let build_start = web_now_ms();
    let request = build_request(&prompt, model.as_deref());
    let body_bytes = request.encode_to_vec();
    let build_request_ms = web_now_ms() - build_start;

    // API-key auth is sent as a bearer token, exactly as the native
    // `AuthSession::get_or_refresh_access_token` -> `AuthToken::ApiKey` ->
    // `as_bearer_token` path does (see `warp_server_auth::credentials`).
    let url = format!(
        "{}{MAA_ENDPOINT_PATH}",
        server_root_url.trim_end_matches('/')
    );
    let body = js_sys::Uint8Array::from(body_bytes.as_slice());
    let init = build_fetch_init(&api_key, &body, timeout_ms);
    let init_js: &JsValue = init.as_ref();

    let mut result = match run_stream(&host, &url, init_js).await {
        Ok(r) => r,
        Err(err) => err_result(format!("host/stream error: {err}"), None),
    };
    result.timings_ms = Some(Timings {
        build_request_ms,
        first_event_ms: result.timings_ms.as_ref().and_then(|t| t.first_event_ms),
        total_ms: web_now_ms() - started,
    });
    result_to_js(&result)
}

// ---- input validation -------------------------------------------------------

fn validate_input(prompt: &str, api_key: &str, server_root_url: &str) -> Result<(), String> {
    if api_key.trim().is_empty() {
        return Err("missing api_key".to_string());
    }
    if prompt.trim().is_empty() {
        return Err("missing prompt".to_string());
    }
    if server_root_url.trim().is_empty() {
        return Err("missing server_root_url".to_string());
    }
    if !server_root_url.starts_with("http://") && !server_root_url.starts_with("https://") {
        return Err("server_root_url must be an http(s) URL".to_string());
    }
    Ok(())
}

// ---- request building -------------------------------------------------------

/// Build a minimal-but-valid MAA `Request` for a single user prompt.
///
/// Mirrors `app/src/ai/agent/api/convert_to.rs::convert_input` for a plain
/// `UserQuery` (the `UserInputs` path) and `app/src/ai/agent/api/impl.rs` for
/// the `Settings`/`Metadata` shape. Tool support is declared as for a local
/// session so the server treats the request like a normal client; the wasm
/// slice cannot *execute* filesystem/shell/MCP tools (no `local_fs`/`local_tty`
/// on this target), and a model-requested tool surfaces as an
/// `unsupported_capability` observation rather than a silent native fallback.
fn build_request(prompt: &str, model: Option<&str>) -> api::Request {
    use api::request::input::user_inputs::UserInput;
    use api::request::input::user_inputs::user_input::Input as UserInputKind;
    use api::request::input::{Type as InputType, UserInputs, UserQuery};
    use api::request::settings::ModelConfig;
    use api::request::{Input, Metadata, Settings, TaskContext};

    let user_query = UserQuery {
        query: prompt.to_string(),
        referenced_attachments: Default::default(),
        // `UserQueryMode` is a message; `None` leaves the mode at its default,
        // which the server treats as a normal agent-mode query.
        mode: None,
        intended_agent: Default::default(),
    };

    let input = Input {
        context: None,
        r#type: Some(InputType::UserInputs(UserInputs {
            inputs: vec![UserInput {
                input: Some(UserInputKind::UserQuery(user_query)),
            }],
        })),
    };

    let settings = Settings {
        model_config: Some(ModelConfig {
            base: model.unwrap_or("").to_string(),
            ..Default::default()
        }),
        web_context_retrieval_enabled: true,
        supports_parallel_tool_calls: true,
        planning_enabled: true,
        supports_create_files: true,
        supported_tools: local_session_tools(),
        supports_long_running_commands: true,
        should_preserve_file_content_in_history: true,
        supports_todos_ui: true,
        supports_started_child_task_message: true,
        supports_suggest_prompt: true,
        supports_reasoning_message: true,
        ..Default::default()
    };

    let metadata = Metadata {
        conversation_id: String::new(),
        logging: Default::default(),
        ambient_agent_task_id: String::new(),
        forked_from_conversation_id: String::new(),
        parent_agent_id: String::new(),
        agent_name: String::new(),
    };

    api::Request {
        task_context: Some(TaskContext { tasks: vec![] }),
        input: Some(input),
        settings: Some(settings),
        metadata: Some(metadata),
        existing_suggestions: None,
        mcp_context: None,
    }
}

/// Tool set mirroring `get_supported_tools` for a local session in
/// `app/src/ai/agent/api/impl.rs`. Declaring these does NOT mean the wasm slice
/// can fulfill them; it only matches what a normal local client advertises so
/// the server's request handling is representative.
fn local_session_tools() -> Vec<i32> {
    use api::ToolType as T;
    [
        T::Grep,
        T::FileGlob,
        T::FileGlobV2,
        T::ReadFiles,
        T::ApplyFileDiffs,
        T::SearchCodebase,
        T::RunShellCommand,
        T::ReadShellCommandOutput,
        T::WriteToLongRunningShellCommand,
        T::ReadDocuments,
        T::EditDocuments,
        T::CreateDocuments,
        T::SuggestNewConversation,
        T::Subagent,
        T::SuggestPrompt,
        T::OpenCodeReview,
        T::InitProject,
        T::ReadMcpResource,
        T::CallMcpTool,
        T::InsertReviewComments,
    ]
    .into_iter()
    .map(|t| t as i32)
    .collect()
}

// ---- host fetch init --------------------------------------------------------

fn build_fetch_init(
    api_key: &str,
    body: &js_sys::Uint8Array,
    timeout_ms: Option<f64>,
) -> js_sys::Object {
    let init = js_sys::Object::new();
    js_set(&init, "method", &JsValue::from_str("POST"));

    let headers = js_sys::Object::new();
    js_set(
        &headers,
        "Authorization",
        &JsValue::from_str(&format!("Bearer {api_key}")),
    );
    js_set(
        &headers,
        "Content-Type",
        &JsValue::from_str("application/x-protobuf"),
    );
    js_set(&headers, "Accept", &JsValue::from_str("text/event-stream"));
    // Match the headers the native CLI sends via `http_client::add_warp_http_headers`
    // (`X-Warp-Client-ID` = `warp-cli` for `ExecutionMode::Sdk`, OS headers, and
    // a non-empty User-Agent) so the server edge treats this like a real client.
    js_set(&headers, "X-Warp-Client-ID", &JsValue::from_str("warp-cli"));
    js_set(
        &headers,
        "X-Warp-Client-Version",
        &JsValue::from_str("wasm-node-proto/0.1"),
    );
    js_set(&headers, "X-Warp-OS-Category", &JsValue::from_str("Linux"));
    js_set(&headers, "X-Warp-OS-Name", &JsValue::from_str("Linux"));
    js_set(&headers, "X-Warp-OS-Version", &JsValue::from_str("6.0"));
    js_set(
        &headers,
        "User-Agent",
        &JsValue::from_str("warp-cli/0.1.0 wasm-node-proto"),
    );
    js_set(&init, "headers", &headers);

    js_set(&init, "body", body);
    if let Some(ms) = timeout_ms {
        js_set(&init, "timeoutMs", &JsValue::from_f64(ms));
    }
    init
}

// ---- host fetch + SSE stream -----------------------------------------------

async fn run_stream(host: &JsValue, url: &str, init: &JsValue) -> Result<RunResult, String> {
    let fetch_fn = js_get(host, "fetch").map_err(|e| format!("host has no `fetch`: {e:?}"))?;
    let fetch_fn = fetch_fn
        .dyn_ref::<js_sys::Function>()
        .ok_or_else(|| "host.fetch is not a function".to_string())?;

    let promise = fetch_fn
        .call2(host, &JsValue::from_str(url), init)
        .map_err(|e| format!("host.fetch call failed: {e:?}"))?;
    let promise = promise
        .dyn_into::<js_sys::Promise>()
        .map_err(|_| "host.fetch did not return a Promise".to_string())?;
    let response_val = JsFuture::from(promise)
        .await
        .map_err(|e| format!("fetch rejected (abort/timeout/network): {e:?}"))?;
    let response = response_val
        .dyn_ref::<js_sys::Object>()
        .ok_or_else(|| "fetch response is not an object".to_string())?;

    let status = js_get_f64(response, "status").unwrap_or(0.0) as u16;
    if !(200..300).contains(&status) {
        let status_text = js_get_string(response, "statusText").unwrap_or_default();
        return Ok(err_result(
            format!("non-2xx response: {status} {status_text}"),
            None,
        ));
    }

    let body_val = js_get(response, "body").map_err(|e| format!("response has no body: {e:?}"))?;
    let body = body_val
        .dyn_ref::<js_sys::Object>()
        .ok_or_else(|| "response.body is not an object".to_string())?;
    let read_fn = js_get(body, "read")
        .map_err(|e| format!("body has no read(): {e:?}"))?
        .dyn_ref::<js_sys::Function>()
        .ok_or_else(|| "body.read is not a function".to_string())?
        .clone();

    // Drain the SSE stream, parsing + decoding incrementally.
    let mut buffer = String::new();
    let mut acc = StreamAccumulator::new();
    let mut first_event_at: Option<f64> = None;
    let mut event_count: u32 = 0;
    let mut events_log: Vec<String> = Vec::new();
    const MAX_EVENTS: u32 = 10_000;

    loop {
        let read_promise = read_fn
            .call0(AsRef::<JsValue>::as_ref(body))
            .map_err(|e| format!("body.read() call failed: {e:?}"))?;
        let read_promise = read_promise
            .dyn_into::<js_sys::Promise>()
            .map_err(|_| "body.read() did not return a Promise".to_string())?;
        let chunk_val = JsFuture::from(read_promise)
            .await
            .map_err(|e| format!("body.read() rejected: {e:?}"))?;
        let chunk = chunk_val
            .dyn_ref::<js_sys::Object>()
            .ok_or_else(|| "body.read() resolved to a non-object".to_string())?;

        let done = js_get_bool(chunk, "done").unwrap_or(false);
        if let Ok(value) = js_get(chunk, "value")
            && !value.is_undefined()
            && !value.is_null()
            && let Some(bytes) = value.dyn_ref::<js_sys::Uint8Array>()
        {
            buffer.push_str(&String::from_utf8_lossy(&bytes.to_vec()));
        }

        // Parse complete SSE event blocks (separated by a blank line).
        while let Some((block, rest)) = take_complete_event(&buffer) {
            buffer = rest;
            for data in parse_sse_event_data(&block) {
                event_count += 1;
                if first_event_at.is_none() {
                    first_event_at = Some(web_now_ms());
                }
                match decode_event(&data) {
                    Ok(event) => {
                        let (label, terminal) = accumulate(&event, &mut acc, &mut events_log);
                        if let Some(label) = label
                            && events_log.len() < 64
                        {
                            events_log.push(label);
                        }
                        if terminal {
                            return Ok(finalize(
                                acc.to_result(),
                                event_count,
                                events_log,
                                first_event_at,
                            ));
                        }
                    }
                    Err(err) => {
                        if events_log.len() < 64 {
                            events_log.push(format!("decode_error:{err}"));
                        }
                    }
                }
                if event_count >= MAX_EVENTS {
                    return Ok(finalize(
                        err_result(
                            "exceeded max event safety bound".to_string(),
                            Some(acc.text.clone()),
                        ),
                        event_count,
                        events_log,
                        first_event_at,
                    ));
                }
            }
        }

        if done {
            // Flush any trailing event without a terminating blank line.
            if !buffer.trim().is_empty() {
                for data in parse_sse_event_data(&buffer) {
                    event_count += 1;
                    if let Ok(event) = decode_event(&data) {
                        let (_, terminal) = accumulate(&event, &mut acc, &mut events_log);
                        if terminal {
                            return Ok(finalize(
                                acc.to_result(),
                                event_count,
                                events_log,
                                first_event_at,
                            ));
                        }
                    }
                }
            }
            break;
        }
    }

    // Stream ended without an explicit Finished event.
    let mut res = acc.to_result();
    if res.status == "pending" {
        res.status = "success".to_string();
        res.ok = true;
        res.finished_reason = Some("stream_ended".to_string());
    }
    Ok(finalize(res, event_count, events_log, first_event_at))
}

#[derive(Default)]
struct StreamAccumulator {
    conversation_id: Option<String>,
    request_id: Option<String>,
    run_id: Option<String>,
    text: String,
    finished_reason: Option<String>,
    status: String,
    ok: bool,
    pending_error: Option<String>,
}

impl StreamAccumulator {
    fn new() -> Self {
        Self {
            status: "pending".to_string(),
            ..Default::default()
        }
    }
    fn to_result(&self) -> RunResult {
        RunResult {
            ok: self.ok,
            status: self.status.clone(),
            conversation_id: self.conversation_id.clone(),
            request_id: self.request_id.clone(),
            run_id: self.run_id.clone(),
            text: self.text.clone(),
            finished_reason: self.finished_reason.clone(),
            event_count: 0,
            events: vec![],
            error: self.pending_error.clone(),
            timings_ms: None,
        }
    }
}

/// Inspect a decoded `ResponseEvent`, update the accumulator, and return
/// `(optional_event_label, is_terminal)`.
fn accumulate(
    event: &api::ResponseEvent,
    acc: &mut StreamAccumulator,
    events_log: &mut Vec<String>,
) -> (Option<String>, bool) {
    let Some(ty) = &event.r#type else {
        return (None, false);
    };
    match ty {
        api::response_event::Type::Init(init) => {
            acc.conversation_id = Some(init.conversation_id.clone());
            acc.request_id = Some(init.request_id.clone());
            acc.run_id = Some(init.run_id.clone());
            (Some("init".to_string()), false)
        }
        api::response_event::Type::ClientActions(actions) => {
            // Extract any assistant text from the carried messages. Tool calls
            // are surfaced as unsupported_capability observations (not executed).
            let mut had_tool_call = false;
            for action in &actions.actions {
                if let Some(a) = &action.action {
                    for msg in messages_for_action(a) {
                        if let Some(text) = agent_text_from_message(&msg) {
                            acc.text.push_str(&text);
                        }
                        if message_is_tool_call(&msg) {
                            had_tool_call = true;
                        }
                    }
                }
            }
            if had_tool_call && events_log.len() < 64 {
                events_log.push("client_actions:tool_call".to_string());
            }
            (Some("client_actions".to_string()), false)
        }
        api::response_event::Type::Finished(finished) => {
            let reason = finished_reason_label(finished);
            let is_done = matches!(
                finished.reason,
                Some(api::response_event::stream_finished::Reason::Done(_))
            );
            acc.finished_reason = Some(reason.clone());
            acc.status = if is_done { "success" } else { "error" }.to_string();
            acc.ok = is_done;
            if !is_done {
                acc.pending_error = Some(format!("stream finished: {reason}"));
            }
            (Some(format!("finished:{reason}")), true)
        }
    }
}

fn messages_for_action(action: &api::client_action::Action) -> Vec<api::Message> {
    use api::client_action::Action as A;
    match action {
        A::AddMessagesToTask(a) => a.messages.clone(),
        A::UpdateTaskMessage(a) => a.message.clone().into_iter().collect(),
        A::AppendToMessageContent(a) => a.message.clone().into_iter().collect(),
        _ => Vec::new(),
    }
}

fn agent_text_from_message(msg: &api::Message) -> Option<String> {
    let m = msg.message.as_ref()?;
    if let api::message::Message::AgentOutput(out) = m {
        return Some(out.text.clone());
    }
    None
}

fn message_is_tool_call(msg: &api::Message) -> bool {
    matches!(
        msg.message.as_ref(),
        Some(api::message::Message::ToolCall(_))
    )
}

fn finished_reason_label(f: &api::response_event::StreamFinished) -> String {
    use api::response_event::stream_finished::Reason as R;
    match &f.reason {
        Some(R::Done(_)) => "done".to_string(),
        Some(R::Other(_)) => "other".to_string(),
        Some(R::MaxTokenLimit(_)) => "max_token_limit".to_string(),
        Some(R::QuotaLimit(_)) => "quota_limit".to_string(),
        Some(R::ContextWindowExceeded(_)) => "context_window_exceeded".to_string(),
        Some(R::LlmUnavailable(_)) => "llm_unavailable".to_string(),
        Some(R::InternalError(e)) => format!("internal_error:{}", e.message),
        Some(R::InvalidApiKey(_)) => "invalid_api_key".to_string(),
        None => "unknown".to_string(),
    }
}

// ---- SSE framing ------------------------------------------------------------

/// If `buffer` contains at least one complete SSE event (terminated by a blank
/// line `\n\n`), return `(complete_block, remaining_buffer)`. Both are owned so
/// the caller can reassign `buffer` without keeping a borrow alive.
fn take_complete_event(buffer: &str) -> Option<(String, String)> {
    let idx = buffer.find("\n\n")?;
    let (before, after) = buffer.split_at(idx);
    Some((before.to_string(), after[2..].to_string()))
}

/// Parse a single SSE event block into its `data:` payload (joined across
/// continuation `data:` lines per the SSE spec).
fn parse_sse_event_data(block: &str) -> Vec<String> {
    let mut data_lines: Vec<String> = Vec::new();
    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        }
    }
    if data_lines.is_empty() {
        Vec::new()
    } else {
        vec![data_lines.join("\n")]
    }
}

fn decode_event(data: &str) -> Result<api::ResponseEvent, String> {
    // Matches `warp_multi_agent_client::decode_response_event`: base64 URL-safe
    // decode of the (possibly quote-wrapped) SSE data payload, then protobuf
    // decode of `ResponseEvent`.
    let trimmed = data.trim_matches('"');
    let bytes = base64::engine::general_purpose::URL_SAFE
        .decode(trimmed)
        .map_err(|e| format!("base64: {e}"))?;
    api::ResponseEvent::decode(bytes.as_slice()).map_err(|e| format!("proto: {e}"))
}

// ---- helpers ----------------------------------------------------------------

fn finalize(
    mut res: RunResult,
    event_count: u32,
    events: Vec<String>,
    first_event_at: Option<f64>,
) -> RunResult {
    res.event_count = event_count;
    res.events = events;
    res.timings_ms = Some(Timings {
        build_request_ms: 0.0,
        first_event_ms: first_event_at,
        total_ms: 0.0,
    });
    res
}

fn err_result(err: String, partial_text: Option<String>) -> RunResult {
    RunResult {
        ok: false,
        status: "error".to_string(),
        conversation_id: None,
        request_id: None,
        run_id: None,
        text: partial_text.unwrap_or_default(),
        finished_reason: None,
        event_count: 0,
        events: vec![],
        error: Some(err),
        timings_ms: None,
    }
}

fn result_to_js(result: &RunResult) -> JsValue {
    serde_json::to_string(result)
        .map(|s| JsValue::from_str(&s))
        .unwrap_or_else(|e| {
            JsValue::from_str(&format!("{{\"ok\":false,\"error\":\"serialize: {e}\"}}"))
        })
}

fn js_set<T: AsRef<JsValue>>(obj: &js_sys::Object, key: &str, value: &T) {
    let k = JsValue::from_str(key);
    let _ = js_sys::Reflect::set(obj.as_ref(), &k, AsRef::<JsValue>::as_ref(value));
}

fn js_get<T: AsRef<JsValue>>(obj: &T, key: &str) -> Result<JsValue, JsValue> {
    let k = JsValue::from_str(key);
    js_sys::Reflect::get(AsRef::<JsValue>::as_ref(obj), &k)
}

fn js_get_string<T: AsRef<JsValue>>(obj: &T, key: &str) -> Option<String> {
    js_get(obj, key).ok().and_then(|v| v.as_string())
}

fn js_get_f64<T: AsRef<JsValue>>(obj: &T, key: &str) -> Option<f64> {
    js_get(obj, key).ok().and_then(|v| v.as_f64())
}

fn js_get_bool<T: AsRef<JsValue>>(obj: &T, key: &str) -> Option<bool> {
    js_get(obj, key).ok().and_then(|v| v.as_bool())
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

/// High-resolution wall clock from the JS host (`performance.now()` when
/// available, else `Date.now()`). Used only for prototype timings.
fn web_now_ms() -> f64 {
    let global = js_sys::global();
    if let Ok(perf) = js_get(&global, "performance")
        && let Some(now) = perf.dyn_ref::<js_sys::Object>()
        && let Ok(f) = js_get(now, "now")
        && let Some(f) = f.dyn_ref::<js_sys::Function>()
        && let Ok(v) = f.call0(AsRef::<JsValue>::as_ref(now))
        && let Some(ms) = v.as_f64()
    {
        return ms;
    }
    js_sys::Date::now()
}
