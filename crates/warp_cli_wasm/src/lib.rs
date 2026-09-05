//! WASM/Node prototype: compile `warp_cli` to `wasm32-unknown-unknown` and run
//! an `agent run` entrypoint inside a JavaScript runtime.
//!
//! This is a **feasibility spike** for a sandboxless / WASM-based orchestrator,
//! not production architecture. See `README.md` in this crate and the PR
//! description for the findings writeup.
//!
//! ## Why an alternate entrypoint
//!
//! `warp_cli::Args::from_env()` returns `Args::default()` on
//! `target_family = "wasm"` because WASM has no concept of an environment/argv
//! (see `crates/warp_cli/src/lib.rs`). To drive the CLI parser from a JS host we
//! therefore expose a `#[wasm_bindgen]` function that takes an argv-style string
//! array, builds the real `clap::Command`, and parses it with
//! `try_get_matches_from` — exactly what `from_env` does on native targets, just
//! sourced from the JS caller instead of `std::env::args()`.
//!
//! ## What this proves
//!
//! - The `warp_cli` crate (argument parsing, command model, serde types) compiles
//!   cleanly to `wasm32-unknown-unknown` and executes inside a Node-hosted WASM
//!   module.
//! - An `agent run` command can be constructed and serialized from a JS-provided
//!   argv equivalent to `oz-dev agent run --prompt hello --api-key=...`.
//! - The runtime can make an outbound HTTP request from inside the WASM module
//!   via host-provided `fetch` (the same primitive the browser wasm build's
//!   `http_client` uses), demonstrating the minimum networking needed for an
//!   agent loop.
//!
//! ## What this does NOT do (intentionally)
//!
//! It does not run the full agent execution loop. That path
//! (`warp::ai::agent_sdk::run`) lives in the `warp` (GUI app) crate and requires
//! an `AppContext` with auth, the server API client, AI client, settings, etc.
//! The existing browser wasm build (`script/wasm/bundle`) already compiles that
//! whole app to `wasm32-unknown-unknown`; this spike isolates the CLI layer to
//! measure how much of the orchestrator can run sandboxless in Node. See the
//! findings writeup for the gap analysis.

use serde::{Deserialize, Serialize};
use warp_cli::agent::{AgentCommand, OutputFormat, RunAgentArgs};
use warp_cli::{Args, CliCommand, GlobalOptions};
use wasm_bindgen::prelude::*;

/// Configuration handed in from the JS harness when an argv is awkward to build.
///
/// This mirrors the fields of `oz-dev agent run --prompt hello --api-key=...`
/// that the spike cares about. It is intentionally minimal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunConfig {
    /// The prompt for the agent (equivalent to `--prompt`).
    pub prompt: String,
    /// API key for server authentication (equivalent to `--api-key` / `WARP_API_KEY`).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Optional override of the server root URL (equivalent to `--server-root-url`).
    #[serde(default)]
    pub server_root_url: Option<String>,
    /// Output format: `pretty` | `json` | `ndjson` | `text`.
    #[serde(default = "default_output_format")]
    pub output_format: String,
    /// Execution harness: `oz` | `claude` | `opencode` | `gemini` | `codex`.
    #[serde(default = "default_harness")]
    pub harness: String,
}

fn default_output_format() -> String {
    "pretty".to_string()
}

fn default_harness() -> String {
    "oz".to_string()
}

/// The result returned to JS: the parsed command, serialized as JSON, plus a
/// human-readable summary line. This is the "agent entrypoint was invoked"
/// evidence for the spike.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedAgentRun {
    /// The equivalent argv that was parsed, with any `--api-key` value
    /// redacted (`<redacted>` for the separated form, `--api-key=<redacted>`
    /// for the `=`-joined form) so the raw key is never echoed back.
    pub argv: Vec<String>,
    /// The prompt that was parsed.
    pub prompt: String,
    /// The harness that was selected.
    pub harness: String,
    /// The output format that was selected.
    pub output_format: String,
    /// Whether an API key was provided (the value itself is never returned).
    pub has_api_key: bool,
    /// The fully-parsed `CliCommand::Agent(AgentCommand::Run(..))` serialized to
    /// JSON, proving the command model round-trips through the wasm module.
    pub command_json: String,
    /// A one-line summary suitable for stdout.
    pub summary: String,
}

/// Parse a JSON `AgentRunConfig` and build the equivalent `agent run` command
/// via the real `warp_cli` clap parser, returning a JSON `ParsedAgentRun`.
///
/// This is the synchronous "agent entrypoint" for the spike: it proves the CLI
/// argument-parsing layer compiles and runs in WASM and that an `agent run`
/// command can be constructed from a JS-provided config.
#[wasm_bindgen]
pub fn agent_run_from_config(config_json: &str) -> Result<String, String> {
    let config: AgentRunConfig =
        serde_json::from_str(config_json).map_err(|e| format!("invalid config json: {e}"))?;

    let argv = build_argv(&config);
    let parsed = parse_agent_run(&argv, &config)?;
    serde_json::to_string(&parsed).map_err(|e| format!("failed to serialize result: {e}"))
}

/// Parse an argv-style string array (e.g. `["oz", "agent", "run", "--prompt",
/// "hello", "--api-key", "..."]`) via the real `warp_cli` clap parser and return
/// a JSON `ParsedAgentRun`. This is the closest analog to
/// `oz-dev agent run --prompt hello --api-key=$WARP_API_KEY` that a JS host can
/// invoke without an `std::env::args()` equivalent.
#[wasm_bindgen]
pub fn agent_run_from_argv(argv_json: &str) -> Result<String, String> {
    let argv: Vec<String> =
        serde_json::from_str(argv_json).map_err(|e| format!("invalid argv json: {e}"))?;

    // Recover a best-effort config from the argv for the result summary.
    let config = config_from_argv(&argv);
    let parsed = parse_agent_run(&argv, &config)?;
    serde_json::to_string(&parsed).map_err(|e| format!("failed to serialize result: {e}"))
}

/// Make an outbound HTTP GET request from inside the WASM module using the
/// host's `fetch`. Returns the response status text + a truncated body snippet.
///
/// This demonstrates the minimum networking the agent loop needs (an outbound
/// request to the server) can run in the JS-hosted WASM runtime. It uses the
/// same `fetch` primitive the browser wasm build's `http_client` relies on,
/// driven here through `wasm-bindgen-futures` so it works under Node's
/// `--experimental-wasm-modules`-free `wasm-bindgen --target nodejs` output.
#[wasm_bindgen]
pub async fn http_get(url: &str) -> Result<String, String> {
    fetch_text(url.to_string()).await
}

fn build_argv(config: &AgentRunConfig) -> Vec<String> {
    let mut argv = vec![
        "oz".to_string(),
        "agent".to_string(),
        "run".to_string(),
        "--prompt".to_string(),
        config.prompt.clone(),
        "--harness".to_string(),
        config.harness.clone(),
        "--output-format".to_string(),
        config.output_format.clone(),
    ];
    if let Some(api_key) = &config.api_key {
        argv.push("--api-key".to_string());
        argv.push(api_key.clone());
    }
    if let Some(server_root_url) = &config.server_root_url {
        argv.push("--server-root-url".to_string());
        argv.push(server_root_url.clone());
    }
    argv
}

/// Split a long option (`--flag` or `--flag=value`) into its flag name and any
/// inline value. Returns `("--flag", Some(value))` for `--flag=value`,
/// `("--flag", None)` for `--flag`, and `("", None)` for anything that isn't a
/// long option (short flags, values, positional args). clap accepts both the
/// separated (`--flag value`) and `=`-joined (`--flag=value`) long-option
/// forms, so the best-effort metadata parser below mirrors that.
fn split_long_flag(arg: &str) -> (String, Option<String>) {
    let rest = match arg.strip_prefix("--") {
        Some(rest) if !rest.is_empty() && !rest.starts_with('-') => rest,
        _ => return (String::new(), None),
    };
    if let Some((flag, value)) = rest.split_once('=') {
        (format!("--{flag}"), Some(value.to_string()))
    } else {
        (format!("--{rest}"), None)
    }
}

fn config_from_argv(argv: &[String]) -> AgentRunConfig {
    let mut config = AgentRunConfig {
        prompt: String::new(),
        api_key: None,
        server_root_url: None,
        output_format: default_output_format(),
        harness: default_harness(),
    };
    let mut iter = argv.iter().skip(1);
    while let Some(arg) = iter.next() {
        // `-p value` — short form, separated only.
        if arg == "-p" {
            if let Some(v) = iter.next() {
                config.prompt = v.clone();
            }
            continue;
        }
        // `--flag value` or `--flag=value` — long forms (clap accepts both).
        let (flag, inline_value) = split_long_flag(arg);
        if flag.is_empty() {
            continue;
        }
        // Only consume the following argv element for flags we recognize as
        // value-taking, so an unknown long flag is skipped in place rather than
        // swallowing its neighbor (preserving the original best-effort
        // parser's behavior for args the spike doesn't model).
        let value: Option<String> = if matches!(
            flag.as_str(),
            "--prompt" | "--api-key" | "--server-root-url" | "--output-format" | "--harness"
        ) {
            inline_value.or_else(|| iter.next().cloned())
        } else {
            None
        };
        match flag.as_str() {
            "--prompt" => {
                if let Some(v) = value {
                    config.prompt = v;
                }
            }
            "--api-key" => {
                if let Some(v) = value {
                    config.api_key = Some(v);
                }
            }
            "--server-root-url" => {
                if let Some(v) = value {
                    config.server_root_url = Some(v);
                }
            }
            "--output-format" => {
                if let Some(v) = value {
                    config.output_format = v;
                }
            }
            "--harness" => {
                if let Some(v) = value {
                    config.harness = v;
                }
            }
            _ => {}
        }
    }
    config
}

/// Return a copy of `argv` with any `--api-key` value replaced by
/// `<redacted>`. Handles both the separated (`--api-key value`) and
/// `=`-joined (`--api-key=value`) long-option forms clap accepts, so the raw
/// key is never echoed back through the `ParsedAgentRun` result — the
/// `has_api_key` field carries only the boolean, and the doc claim that the
/// key value is never returned then holds.
fn redact_api_key_in_argv(argv: &[String]) -> Vec<String> {
    let mut redacted: Vec<String> = Vec::with_capacity(argv.len());
    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        if arg == "--api-key" {
            redacted.push(arg.clone());
            if i + 1 < argv.len() {
                redacted.push("<redacted>".to_string());
                i += 2;
            } else {
                i += 1;
            }
        } else if arg.starts_with("--api-key=") {
            redacted.push("--api-key=<redacted>".to_string());
            i += 1;
        } else {
            redacted.push(arg.clone());
            i += 1;
        }
    }
    redacted
}

fn parse_agent_run(argv: &[String], config: &AgentRunConfig) -> Result<ParsedAgentRun, String> {
    // Build the real clap command (the same one `Args::from_env` builds on
    // native targets) and parse the JS-provided argv. This exercises the full
    // argument-parsing + command-model layer of `warp_cli` inside WASM.
    let command = <Args as clap::CommandFactory>::command();
    let matches = command
        .try_get_matches_from(argv.iter())
        .map_err(|e| format!("failed to parse argv: {e}"))?;

    let args: Args = clap::FromArgMatches::from_arg_matches(&matches)
        .map_err(|e| format!("failed to build Args: {e}"))?;

    // Pull out the parsed `agent run` command, if present, and serialize it.
    let command_json = match args.command() {
        Some(warp_cli::Command::CommandLine(boxed)) => match boxed.as_ref() {
            CliCommand::Agent(AgentCommand::Run(run_args)) => serialize_run_args(run_args)?,
            other => {
                return Err(format!(
                    "parsed a command, but it was not `agent run`: {other:?}"
                ));
            }
        },
        Some(other) => {
            return Err(format!(
                "parsed a command, but it was not `agent run`: {other:?}"
            ));
        }
        None => return Err("no subcommand parsed".to_string()),
    };

    let summary = format!(
        "agent run --prompt {:?} --harness {} --output-format {} (api_key={})",
        config.prompt,
        config.harness,
        config.output_format,
        if config.api_key.is_some() {
            "provided"
        } else {
            "absent"
        }
    );

    Ok(ParsedAgentRun {
        argv: redact_api_key_in_argv(argv),
        prompt: config.prompt.clone(),
        harness: config.harness.clone(),
        output_format: config.output_format.clone(),
        has_api_key: config.api_key.is_some(),
        command_json,
        summary,
    })
}

/// Serialize a `RunAgentArgs` to JSON. `RunAgentArgs` itself does not derive
/// `Serialize`, but its salient fields are plain public types we can project
/// into a small serializable struct — enough to prove the command model
/// round-trips through the wasm module.
fn serialize_run_args(run_args: &RunAgentArgs) -> Result<String, String> {
    #[derive(Serialize)]
    struct RunArgsView<'a> {
        prompt: Option<&'a str>,
        harness: String,
        sandboxed: bool,
        has_api_key: bool,
        model: Option<&'a str>,
        environment: Option<&'a str>,
        conversation: Option<&'a str>,
        mcp_specs: usize,
        profile: Option<&'a str>,
    }

    let prompt_string = run_args.prompt_arg.to_prompt().map(|p| match p {
        warp_cli::agent::Prompt::PlainText(t) => t,
        warp_cli::agent::Prompt::SavedPrompt(id) => id,
    });
    let view = RunArgsView {
        prompt: prompt_string.as_deref(),
        harness: run_args.harness.config_name().to_string(),
        sandboxed: run_args.sandboxed,
        has_api_key: false, // api_key lives on GlobalOptions, not RunAgentArgs
        model: run_args.model.model.as_deref(),
        environment: run_args.environment.as_deref(),
        conversation: run_args.conversation.as_deref(),
        mcp_specs: run_args.all_mcp_specs().len(),
        profile: run_args.profile.as_deref(),
    };

    serde_json::to_string(&view).map_err(|e| format!("failed to serialize run args: {e}"))
}

// `GlobalOptions` and `OutputFormat` document the CLI surface this spike
// touches; reference them so the import is used and the surface is discoverable.
fn _document_surface(_g: GlobalOptions, _o: OutputFormat) {}

// ---------------------------------------------------------------------------
// Host `fetch` binding (the same primitive the browser wasm build's
// `http_client` uses, driven here through `wasm-bindgen-futures`). We bind
// `Response` as an imported type directly rather than pulling in `web-sys`
// (whose workspace feature set does not include `Response`), keeping the spike
// crate's dependency footprint minimal.
// ---------------------------------------------------------------------------

#[wasm_bindgen]
extern "C" {
    /// The subset of the JS `Response` interface the spike needs. Declared as an
    /// imported type so we avoid a `web-sys` dependency.
    pub type Response;

    #[wasm_bindgen(method, getter, js_name = statusText)]
    fn status_text(this: &Response) -> String;

    #[wasm_bindgen(method, catch)]
    fn text(this: &Response) -> Result<js_sys::Promise, JsValue>;

    #[wasm_bindgen(js_name = fetch, js_namespace = global, catch)]
    fn fetch(url: &str) -> Result<js_sys::Promise, JsValue>;
}

fn fetch_text(
    url: String,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>>>> {
    Box::pin(async move {
        let response_value: JsValue = wasm_bindgen_futures::JsFuture::from(
            fetch(&url).map_err(|e| format!("fetch failed: {e:?}"))?,
        )
        .await
        .map_err(|e| format!("fetch await failed: {e:?}"))?;
        let response: Response = response_value
            .dyn_into()
            .map_err(|e| format!("fetch did not return a Response: {e:?}"))?;
        let status = response.status_text();
        let body_promise = response
            .text()
            .map_err(|e| format!("response.text() failed: {e:?}"))?;
        let body_value: JsValue = wasm_bindgen_futures::JsFuture::from(body_promise)
            .await
            .map_err(|e| format!("response.text() await failed: {e:?}"))?;
        let body = body_value.as_string().unwrap_or_default();
        // Truncate the body for the demo result; the point is to prove the
        // request completed and bytes came back.
        let snippet: String = body.chars().take(200).collect();
        Ok(format!("status: {status}\nbody[0..200]: {snippet}"))
    })
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
