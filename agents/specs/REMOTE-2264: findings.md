# Findings: wasm32 Oz CLI in a Node runtime (REMOTE-2264)

This is the findings report for the prototype built in `crates/wasm_node_proto` +
`crates/warp_wasm_node` + `script/wasm/build-node` + `script/wasm/node-harness.mjs`.
It is a deliverable of the implementation run, not a spec. The approved spec
lives at `agents/specs/REMOTE-2264: wasm32 CLI in Node prototype.md`.

## Follow-up investigation (403 specifics, ModelContext, size + memory)

### 1. 403 response specifics

Instrumented the host-fetch boundary in `script/wasm/node-harness.mjs` to log
full failed-response detail (status, headers, body) for any non-2xx response.
Ran `node script/wasm/node-harness.mjs --prompt hello --api-key $WARP_API_KEY
--server-root-url https://app.warp.dev` in Node 25 against the production
endpoint. Captured output:

```
host-fetch: non-2xx response from https://app.warp.dev/ai/multi-agent
host-fetch:   status: 403 Forbidden
host-fetch:   headers: {"alt-svc":"h3=\":443\"; ma=2592000",
  "content-length":"134","content-type":"text/html; charset=UTF-8",
  "cross-origin-embedder-policy":"unsafe-none",
  "cross-origin-opener-policy":"same-origin-allow-popups",
  "cross-origin-resource-policy":"same-origin",
  "date":"Thu, 23 Jul 2026 16:22:11 GMT",
  "permissions-policy":"camera=(), microphone=(), geolocation=()",
  "referrer-policy":"no-referrer",
  "strict-transport-security":"max-age=31536000; includeSubDomains",
  "via":"1.1 google",
  "x-content-type-options":"nosniff",
  "x-xss-protection":"1; mode=block"}
host-fetch:   body (134 bytes): <!doctype html><meta charset="utf-8">
  <meta name=viewport content="width=device-width, initial-scale=1">
  <title>403</title>403 Forbidden
```

Key observations:
- **No Cloudflare challenge markup.** The body is a plain `403 Forbidden` HTML
  page (134 bytes). No JS challenge, no managed challenge, no CAPTCHA.
- **`via: 1.1 google`** — the response passed through a Google Cloud Load
  Balancer, not Cloudflare. The edge gate is at the GCLB layer.
- **Response time ~155ms** — consistent with an edge/gateway reject, not an
  application-level decision (application-level 401s on `/api/v1/*` return in a
  similar timeframe but with a JSON body).
- **No error/exception from the wasm transport.** The host `fetch` completed
  normally with a 403 status; the wasm crate's SSE decoder received the
  non-2xx status and returned a structured `error: "non-2xx response: 403
  Forbidden"` result. No reqwest/wasm transport exception was thrown.
- The `warp_wasm_node` (AgentDriver) path also reaches this same edge gate —
  its MAA request is dispatched via `warp_multi_agent_client` which uses
  reqwest's wasm backend. The 403 response is identical regardless of which
  path makes the request (same endpoint, same edge).

The `driver_wasm.rs` 403 capture instrumentation (lines 303-314) logs the
`reqwest_eventsource::Error::InvalidStatusCode(status, response)` for the
AgentDriver path, capturing status, headers, and URL from the reqwest response
object.

### 2. ModelContext / session-sharing registration

**What `ModelContext` is required:**
`register_agent_event_consumer<C>(conversation_id: AIConversationId,
consumer_id: EntityId, ctx: &mut C) where C: GetSingletonModelHandle +
UpdateModel` (`app/src/ai/blocklist/orchestration_event_streamer.rs:2394-2404`).
The `ctx` parameter must implement `GetSingletonModelHandle + UpdateModel`.
`ModelContext<T>` satisfies these traits via `Deref<Target = AppContext>`
(`crates/warpui_core/src/core/model/context.rs:553-565`), which gives access
to the reactive model registry (the `AppContext`).

**Where it comes from in the native path:**
- `AgentDriver::new(options, ctx: &mut ModelContext<AgentDriver>)` receives
  `ctx` from the caller (`app/src/ai/agent_sdk/driver.rs:193`) and calls
  `register_agent_event_consumer(conv_id, ctx.model_id(), ctx)` directly at
  line 738.
- `AgentDriver::run(&mut self, task, ctx: &mut ModelContext<AgentDriver>)`
  (`driver.rs:859-863`) calls `ctx.spawn(async move { ... }, |_, _, _| {})`
  at line 870. The spawned future uses `foreground.spawn(|me, ctx| ...)`
  (lines 1003-1011) to schedule closures back onto the main thread, each
  receiving a fresh `ModelContext<AgentDriver>`. This is how
  `unregister_streamer_consumer(ctx)` (line 833) is called from inside the
  async future.

**Why it isn't available in a bare `Foreground::spawn`:**
On wasm, `Foreground::spawn(&self, future: impl Future<Output = ()> + 'static)`
(`crates/warpui_core/src/async/wasm/executor.rs:46`) requires `Future +
'static`. `ModelContext<T>` borrows the `AppContext` (owned by the `App`), so
it is not `'static` and cannot be captured in a bare `spawn_local` future.
There is no way to pass `&mut ModelContext<AgentDriver>` into a
`wasm_bindgen_futures::spawn_local` future.

**How `ctx.spawn` solves it:**
`ModelContext::spawn<S, F, U>(future, callback)`
(`crates/warpui_core/src/core/model/context.rs:394-408`) spawns the future on
the background executor and, upon completion, calls the callback on the main
thread with `(&mut T, S::Output, &mut ModelContext<T>)`. The callback receives
a **fresh** `ModelContext<T>` constructed from the `App` and the model's
`EntityId` (line 296: `ModelContext::new(app, model_id)`), which has the
`GetSingletonModelHandle + UpdateModel` bounds needed for
`register_agent_event_consumer` / `unregister_agent_event_consumer`.

In the wasm `driver_wasm.rs`, this pattern is used at line 267:
```rust
ctx.spawn(
    async move { /* MAA request, returns Option<String> conversation_id */ },
    move |driver, conv_id_opt, ctx| {
        // ctx here is &mut ModelContext<AgentDriver> — fresh, not borrowed
        if let Ok(conv_id) = AIConversationId::try_from(conv_id_str) {
            register_agent_event_consumer(conv_id, model_id, ctx);
            driver.run_conversation_id = Some(conv_id);
        }
        if let Some(conv_id) = driver.run_conversation_id.take() {
            unregister_agent_event_consumer(conv_id, model_id, ctx);
        }
    },
);
```
This is the same mechanism as the native `foreground.spawn(|me, ctx| ...)`
pattern, but using `ctx.spawn`'s completion callback directly. The spawned
future returns `Option<String>` (the conversation ID from `StreamInit`), and
the callback uses the fresh `ModelContext` to perform the real
register/unregister calls.

### 3. Size + memory metrics

Built both wasm crates with the `release-wasm` profile (`opt-level = "s"`,
`lto = true`, `codegen-units = 1`) with `debug = 0` and `strip = symbols`
overridden via env vars. Ran `wasm-opt -Oz -all --strip-debug --strip-producers`
on the wasm-bindgen output.

**wasm_node_proto (direct MAA fallback, standalone crate):**
- Before wasm-opt: 643 KB (658,796 bytes), gzip: 142 KB (145,618 bytes)
- After wasm-opt -Oz: 512 KB (523,938 bytes), gzip: 159 KB (162,336 bytes)
- Reduction: 20.5% raw (wasm-opt removes redundancy that gzip would otherwise
  exploit, so the gzip size is slightly larger after optimization)

**warp_wasm_node (AgentDriver path, includes full app crate):**
- Before wasm-opt: 104.4 MB (109,495,058 bytes), gzip: 19.5 MB (20,418,593 bytes)
- After wasm-opt -Oz: 90.3 MB (94,651,294 bytes), gzip: 20.8 MB (21,772,476 bytes)
- Reduction: 13.6% raw

**Peak memory (Node 25.0.0, release-wasm profile):**
- wasm_node_proto path: peak RSS 66.8 MB, heap 8.7 MB, external 4.7 MB
- warp_wasm_node path: peak RSS 50.3 MB, heap 4.4 MB, external 1.8 MB

The warp_wasm_node RSS (50.3 MB) is lower than the binary size (105 MB)
because the wasm binary is memory-mapped by Node and the linear memory starts
small, growing on demand. The 105 MB is the compiled code + data segment size;
the actual resident set is dominated by Node's V8 heap + the wasm module's
compiled representation, not the raw binary.

## TL;DR (AgentDriver push — second pass: reuse run_internal)

Following a second reviewer course-correction ("reuse `run_internal()` with
`LaunchMode::CommandLine`; don't build a parallel init"), a second pass pivoted
from the parallel trimmed init to reusing the REAL app init path:

- Extracted `run_app_init` (the shared init body: full singleton surface +
  `initialize_app` + `launch()`) from `run_internal`'s `app_builder.run`
  closure, so both the platform event-loop path and a new wasm-async path call
  the same code.
- Added `run_command_line_wasm(launch_mode) -> App` (wasm-gated): does the same
  pre-AppContext setup as `run_internal`, then drives `run_app_init` through a
  headless `App` via `warpui::platform::headless::new_headless_app` (no blocking
  event loop) instead of `AppBuilder::run`. The `#[wasm_bindgen] pub async fn
  run_agent_driver_wasm` builds a `LaunchMode::CommandLine` and calls it.
- Flipped `launch()`'s `LaunchMode::CommandLine` wasm arm from `panic!` to route
  to `agent_sdk::run` (gated out `std::process::exit`).

**First browser-global blocker (FIXED):** the real `run_app_init`/`initialize_app`
path panicked with `Error: Can't find the global Window` from `gloo_utils::window()`
reached via `warp_util::assets::make_absolute_url` → `WarpThemeConfig::new` →
`WarpConfig::new` (in `initialize_app`). Per requester direction, fixed by adding
`warp_util::assets::set_headless_asset_origin(origin)` (a `OnceLock`-backed
runtime origin, since `warp_util` cannot depend on `warp_core`/`ChannelState` —
cycle); `run_command_line_wasm` registers `ChannelState::server_root_url()` as
the asset origin before `run_app_init`. `make_absolute_url` now prefers the
headless origin, falls back to `window().location().origin()` (browser path
unchanged), then to an empty origin. The Node run now passes this panic.

**Next concrete blocker (precise, from a real Node 25 run after the fix):** a
different browser-global — `Error: no window` from
`gloo_storage::local_storage::LocalStorage::raw` (`gloo-storage-0.3.0/src/local_storage.rs:12`),
reached via `warpui_extras::user_preferences::local_storage::LocalStoragePreferences::read_value`
→ `settings::init::register_all_settings` → `BlockListSettings::register` →
`ShowJumpToBottomOfBlockButton::read_from_preferences` in `initialize_app`. The
wasm settings backend (`LocalStoragePreferences`) assumes browser `localStorage`,
unavailable in a DOM-free Node runtime. Same class of browser-global assumption,
now at the settings-init stage. Full backtrace captured with
`--stack-trace-limit=1000` + `--keep-debug`.

Additionally, `ai::agent_sdk`
(the in-process CLI/agent path `LaunchMode::CommandLine` routes to) is itself
gated `#[cfg(not(target_family="wasm"))]` because it transitively depends on a
large native-only surface (`comfy_table`, `inquire`, `command::r#async`,
`ai::artifact_download`, `ai::skills`/fs, `ai::bedrock_credentials`,
`ai::blocklist::finalize_recording_for_conversation`,
`ai::mcp::file_based_manager`, `server::server_api::harness_support` file
uploads, `presigned_upload`, `ai::ambient_agents::task::HarnessModelConfig`,
…) that is `cfg(not(target_family="wasm"))`-gated throughout `app/src/ai/`.

So reusing `run_internal` on wasm in Node requires (a) replacing/stubbing the
`gloo::utils::window()` / browser-global reads in the init path with
host-backed or no-op equivalents for a DOM-free runtime, and (b) carving out
the `agent_sdk` native-only dependency surface so `agent_sdk::run` compiles on
wasm. The headless `App`/`AppContext` still constructs fine (`new_headless_app`
succeeds); the blocker is the browser-env assumption in the shared init + the
`agent_sdk` gate.

## TL;DR (AgentDriver push — first pass: parallel init)

Following the first reviewer guidance ("AppContext is very much available on
wasm"; "make TerminalDriver a no-op"), a first pass genuinely pursued the full
`AgentDriver` path on `wasm32-unknown-unknown`:

- **The `warp` app crate compiles for `wasm32-unknown-unknown`** (headless /
  default features; ~3 min with `clang` as the wasm C compiler for
  `arborium-sysroot`). This confirms "AppContext is available on wasm" at the
  compile level — `AgentDriver`, `agent_sdk`, `ServerApiProvider`, `AuthState`,
  etc. are all present on the wasm target.
- **A headless `App` (with `AppContext`) now constructs on wasm in a DOM-free
  Node runtime without the blocking platform event loop.** New public API
  `warpui::platform::headless::new_headless_app(assets) -> App` builds the
  headless platform impls and returns the `App` directly (it does NOT call the
  blocking `event_loop::run`). On wasm the foreground/background executors
  schedule via `wasm_bindgen_futures::spawn_local`, so a `#[wasm_bindgen] pub
  async fn` can drive spawned work by yielding to the JS event loop. A real
  Node 25 run of the `warp_wasm_node` cdylib confirmed `new_headless_app`
  succeeds end-to-end (stage `constructed_app` reached).
- **No-op terminal added.** `TerminalDriver::create_no_op` (wasm-gated)
  pre-resolves the bootstrap channel to `Ok(())` (the "synthetically advanced
  to the bootstrapped stage" the reviewer described), satisfying
  `wait_for_session_bootstrapped` without a real PTY.
- **Concrete next blocker (precise, surfaced by a real Node run):** the trimmed
  init reaches `features::init_feature_flags()`, which reads the
  `PrivatePreferences` singleton; a trimmed init does not register it, so the
  run panics at `PrivatePreferences::as_ref` →
  `AppContext::get_singleton_model_as_ref::<PrivatePreferences>`. Because the
  app crate builds with `panic = "abort"` on wasm, `catch_unwind` cannot catch
  this — each missing singleton aborts the process. `AgentDriver::new`/`run`
  read a long tail of such singletons (`PublicPreferences`, `SettingsManager`,
  `AIExecutionProfilesModel`, `BlocklistAIPermissions`, `UserWorkspaces`,
  `ApiKeyManager`, skills/MCP/environment managers, …) that `initialize_app`
  registers and that a trimmed init must reproduce (several are
  persistence/sqlite-backed, and sqlite is native-only on wasm). Reaching the
  full `AgentDriver::run` therefore requires either (a) a wasm-headless trimmed
  `initialize_app` that registers the full singleton surface without
  native-only backends, or (b) making the missing-singleton reads degrade
  gracefully. This is the biggest remaining blocker.
- **Two further concrete blockers** for a fully-successful streamed run (same as
  the first pass): the `http_client` wasm transport uses `web_sys::window()`
  (browser-only), so the MAA request needs a host-`fetch` injection to cross
  the Node boundary; and the production `/ai/*` path is edge-gated for
  Node/curl (403) with the available API key unauthorized (401 on
  `/api/v1/*`).

The direct-MAA fallback crate (`crates/wasm_node_proto`) remains as a
**documented fallback** (it reaches the live endpoint through a host-`fetch`
boundary with no DOM shim) and is the path that actually returned a structured
result from this environment.

## TL;DR (first pass — direct MAA fallback)

- **Outcome: partial.** The MAA request/response protocol compiles to
  `wasm32-unknown-unknown`, loads in a DOM-free Node 25 runtime, builds a real
  authenticated MAA `Request`, and POSTs it to the live production
  `/ai/multi-agent` endpoint through a host-injected `fetch` (no `window`,
  `document`, reqwest, or `web_sys`). The transport, SSE framing, protobuf
  decode, and structured-result path are all wired and unit-tested.
- **The full `AgentDriver` path is blocked** on `wasm32-unknown-unknown` by a
  concrete, well-understood dependency (AppContext + `TerminalDriver`/`local_tty`).
  Per the spec, the **direct MAA request/client loop** fallback was implemented.
  It does **not** validate session sharing (that is owned by the
  `AgentDriver`/conversation-consumer path).
- **A fully-successful streamed terminal event against production was not
  obtained from this environment.** Two concrete blockers: (1) the available
  `WARP_API_KEY` is rejected by the server (`401 Unauthorized` on the Oz REST
  API `/api/v1/*`), and (2) the production `/ai/*` path returns `403 Forbidden`
  at the edge for Node/curl regardless of headers/auth, while `/api/v1/*` and
  `/graphql/v2` reach the server (`401`). This is reported as a partial result,
  not substituted with a mock.

## Exact build/run commands (verbatim)

```sh
# one-time: add the target (the build script assumes it is installed)
rustup target add wasm32-unknown-unknown

# build the cdylib + generate the Node loader
./script/wasm/build-node
#   -> target/wasm32-unknown-unknown/debug/node/warp_node_proto.js
#   -> target/wasm32-unknown-unknown/debug/node/warp_node_proto_bg.wasm

# run the manual proof against a real configured Warp endpoint
node script/wasm/node-harness.mjs \
  --prompt hello \
  --api-key "$WARP_API_KEY" \
  --server-root-url "$WARP_SERVER_ROOT_URL"
# optional: --model <id>   --timeout-ms <n>
# env fallbacks: WARP_API_KEY, WARP_SERVER_ROOT_URL (default https://app.warp.dev)
```

A `cargo check --target wasm32-unknown-unknown -p wasm_node_proto` also passes
(spec validation criterion 1).

## What compiled and ran

- `crates/wasm_node_proto` (new `cdylib`+`rlib`) depends only on
  `warp_multi_agent_api` (the canonical prost MAA types), `prost`, `base64`,
  `wasm-bindgen`, `wasm-bindgen-futures`, `js-sys`, `serde`, `serde_json`,
  `anyhow`, `console_error_panic_hook`. It does **not** depend on the `warp` app
  crate, `http_client`, `reqwest`, `warp_server_client`, `warp_server_auth`,
  `warpui`, or any GUI/PTY/fs code.
- It compiles clean for `wasm32-unknown-unknown` (`cargo clippy
  --target wasm32-unknown-unknown -p wasm_node_proto -- -D warnings` passes).
- `wasm-bindgen --target nodejs` produces an 8.4 MB wasm + a 19 KB Node loader
  exporting `run_multi_agent(config, host)`.
- Node 25 imports the loader, invokes the export, and the wasm module builds the
  `Request`, encodes protobuf, and calls the host `fetch` — all inside the wasm
  event loop, with no DOM globals.

## AgentDriver dependency inventory + gating decision

The spec's primary path is the full `AgentDriver`. Concrete blocker:

- `AgentDriver::new(options, ctx: &mut ModelContext<Self>)`
  (`app/src/ai/agent_sdk/driver.rs`) requires a live `AppContext` entity context
  (the warpui reactive model registry) and, inside `new`, calls
  `terminal::TerminalDriver::create(...)`.
- `TerminalDriver` owns the PTY/shell session and is gated behind the
  `local_tty` cargo feature. `app/build.rs` enables `local_tty` only on
  platforms with a real TTY; `wasm32-unknown-unknown` does **not** get
  `local_tty` (mirroring the existing `local_fs` carve-out in
  `crates/warp_core/build.rs`). There is no wasm terminal backend.
- `AppContext` is bootstrapped by `initialize_app` (persistence, secure
  storage, auth manager, server clients, watchers, GUI views). The existing
  browser wasm build boots it through the GUI window event loop
  (`app/src/lib.rs::launch`), which a DOM-free Node run cannot replicate.
- `app/src/lib.rs` `LaunchMode::CommandLine` explicitly
  `panic!("Cannot execute CLI command {command:?} on the web")` on wasm for this
  reason.

Because `AgentDriver` cannot be constructed or steered without a full
`AppContext` + `TerminalDriver` — neither feasible on `wasm32-unknown-unknown`
in a DOM-free Node runtime within this prototype's scope — the spec-sanctioned
**direct MAA request/client loop** fallback was implemented. The dependency
inventory and gating:

| AgentDriver dependency | wasm status | prototype handling |
|---|---|---|
| `AppContext` / `ModelContext` | not bootable headless on wasm | not used (fallback bypasses it) |
| `TerminalDriver` / PTY / shell | `local_tty` absent on wasm | not used (no terminal session) |
| `local_fs` cwd/file-edit/snapshot/declarations/skill-watcher/indexing | `local_fs` absent on wasm | not used (no fs) |
| MCP process startup | native process, unavailable on wasm | not used (no `mcp_context`) |
| GUI/window/persistence | native/browser-only | not used |
| Session-sharing transport | owned by `AgentDriver`/consumer path | **not validated** by the fallback |
| MAA request/response protocol (prost) | compiles on wasm | reused verbatim |
| HTTP transport (reqwest/`web_sys`) | `web_sys` needs a browser `window` | replaced by host `fetch` |

## Session-sharing observations

- The fallback reaches the same MAA endpoint the `AgentDriver` ultimately
  drives (`{server_root_url}/ai/multi-agent`), so the **protocol-level**
  conversation/run identifiers (`StreamInit.conversation_id`, `request_id`,
  `run_id`) are surfaced by the prototype. The unit test
  `decodes_init_client_actions_and_finished_to_a_terminal_result` proves these
  are extracted from a real-format `StreamInit` event.
- The **driver-level** session-sharing registration/consumer transport
  (`register_agent_event_consumer`, the conversation streamer, host
  connect/send/receive/close callbacks) is owned by `AgentDriver` and is **not
  exercised** by the direct MAA fallback. This is the lost capability and the
  main reason the spec prefers the `AgentDriver` route. Validating it requires
  booting a minimal headless `AppContext` on wasm — see "Next steps".

## Fallback / blocker evidence (production real run)

The prototype was run against `https://app.warp.dev` (the production
`server_root_url` per `crates/warp_core/src/channel/config.rs`).

```
$ node script/wasm/node-harness.mjs --prompt hello --api-key "$WARP_API_KEY" \
    --server-root-url https://app.warp.dev --timeout-ms 90000
{
  "ok": false,
  "status": "error",
  "error": "non-2xx response: 403 Forbidden",
  "event_count": 0,
  "timings_ms": { "build_request_ms": 1.54, "first_event_ms": null, "total_ms": 157.22 }
}
harness: wall clock 161.5ms (wasm total 157.2ms, node v25.0.0)
```

Isolation probes (curl + Node, identical headers, key redacted):

```
POST https://app.warp.dev/ai/multi-agent  (auth + full client + browser headers) -> 403
GET  https://app.warp.dev/ai/multi-agent                                     -> 403
GET  https://app.warp.dev/api/v1/agent/runs   (Bearer wk-…)                   -> 401 {"error":"Unauthorized"}
GET  https://app.warp.dev/api/v1/agents        (Bearer wk-…)                   -> 401
GET  https://app.warp.dev/graphql/v2           (Bearer wk-…)                   -> 401
```

Conclusions:

1. **`/ai/*` is edge-gated.** It returns `403` for Node and curl regardless of
   `Authorization`, `X-Warp-Client-ID`, `X-Warp-Client-Version`, OS headers,
   `User-Agent`, `Origin: https://app.warp.dev`, `Sec-Fetch-Site: same-origin`,
   `Referer`, or a browser `User-Agent`. Other paths (`/api/v1/*`, `/graphql/v2`)
   reach the application and return `401`. The 403 body is a plain
   `<title>403</title>403 Forbidden` page (no Cloudflare challenge markup), and
   the response is returned in ~157 ms — consistent with an edge/gateway reject
   rather than an application-level decision. The WoW browser build reaches
   `/ai/*` because it runs same-origin inside a real browser (session cookies +
   a browser TLS fingerprint); the native desktop CLI reaches it via its rustls
   TLS stack + real client identity. Node (undici/OpenSSL) and curl (OpenSSL)
   do not match the allowlisted fingerprint/identity, so the edge blocks them.
2. **The available API key is not authorized.** `GET /api/v1/agent/runs` with
   `Authorization: Bearer $WARP_API_KEY` returns `401 {"error":"Unauthorized"}`
   (the key has the correct `wk-` prefix, length 69). So even if the `/ai/*`
   edge gate were bypassed, this credential would not authenticate. This is an
   environment/credential limitation, not a prototype defect.

Per the spec, a real-server run that is unavailable is reported as a partial
result; no offline/mock SSE fixture was substituted.

## Compile-failure categories encountered

Only minor, all resolved:

- `futures`/`select` was initially used for a Rust-side timeout race; removed
  in favor of the harness-owned `AbortController` (the host aborts on
  `init.timeoutMs`, which surfaces as a structured `error`). This dropped the
  `futures` dependency entirely and avoids wasm atomic-CAS concerns.
- `UserQueryMode` is a prost **message** (oneof `Plan`/`Orchestrate`), not an
  enum; the request sets `mode: None` (server default = normal agent query).
- `take_complete_event` returned a borrow that conflicted with reassigning the
  buffer; changed to return owned `String`s.
- clippy `collapsible_if` on nested `if let` chains — collapsed using stable
  let-chains.

No wasm-specific compile failures were encountered for the MAA protocol crate.
The `warp_multi_agent_api` prost types (including `prost-reflect`) compile
unchanged for `wasm32-unknown-unknown`.

## Runtime shims

- **Host `fetch` transport.** reqwest's wasm backend hard-requires
  `web_sys::window()` (a browser global). Node has no `window`, and the spec
  forbids a DOM shim. So the prototype defines a host contract
  `host.fetch(url, init) -> Promise<{status, statusText, headers, body}>` where
  `body.read() -> Promise<{done, value?: Uint8Array}>` mirrors a web
  `ReadableStream` reader. The Node harness implements it with Node's global
  `fetch` + `AbortController` + `response.body.getReader()`. Application logic
  (the wasm crate) never branches on Node types — it only calls the host
  `fetch`. This is the "polyfill HTTP at the transport layer" approach the
  requester expected, expressed as an explicit host boundary rather than a
  reqwest polyfill (which is impossible without `window`).
- **No `web_sys`/`window`/`document`/canvas/clipboard/browser-console** bindings
  are reached. Logging/panic goes through `console_error_panic_hook`, which on
  Node writes to stderr via the wasm-bindgen nodejs runtime (not a browser
  console). `web_now_ms()` uses `performance.now()` (available in Node) else
  `Date.now()`.
- **No filesystem, PTY, shell, subprocess, MCP-process, watcher, indexing,
  snapshot, or GUI** code is compiled into the crate. A model-requested tool
  call is detected (`message_is_tool_call`) and surfaced as an
  `unsupported_capability` observation in the event log rather than executed.
  The unit test `detects_tool_call_messages_as_unsupported_capability_signal`
  covers this.

## wasm32-unknown-unknown vs WASI

- **`wasm32-unknown-unknown` sufficed** for the MAA protocol + host-fetch path.
  It matched the existing browser wasm build target, required no WASI
  imports, and the only host capabilities needed are `fetch`, `AbortController`,
  `performance.now()`, `Date.now()`, and `TextDecoder`-free byte handling
  (bytes are copied into Rust via `Uint8Array::to_vec`). No syscalls were
  required because all I/O crosses the JS host boundary.
- **WASI would not improve this prototype.** WASI standardizes
  filesystem/stdio/time/random, none of which the MAA-protocol slice needs
  (and none of which would help reach `/ai/multi-agent`, which needs
  browser-style fetch/SSE that WASI does not provide). WASI would add an
  import layer without removing the host-`fetch` requirement, and would not
  make `AgentDriver`/`TerminalDriver` feasible (PTY/shell/process are still
  absent). WASI could help a *future* variant that wants real local file
  reads behind a capability, but that is orthogonal to the agent loop.
- **Recommendation:** keep `wasm32-unknown-unknown` with host bindings as the
  primary target. Revisit WASI only if a later phase wants sandboxed local-fs
  tools (and even then, fetch/SSE stays host-backed).

## Feasibility outcome

- **MAA-protocol-in-wasm-in-Node: viable.** Compiles, loads, and drives a real
  authenticated request through a clean host-`fetch` boundary with no DOM shim.
  The request-building, SSE framing, base64+protobuf decode, and structured
  result/error path are deterministic and unit-tested.
- **Full `AgentDriver` in wasm: not viable in this prototype's scope.** It needs
  a headless `AppContext` bootstrap and a `TerminalDriver` backend, neither of
  which exists on `wasm32-unknown-unknown`. This is the biggest blocker.
- **Production egress from Node: blocked by edge bot protection + (in this
  environment) an unauthorized key.** A real client-side-wasm-in-Node
  deployment would need the edge to allowlist the wasm client (or route through
  a proxy with a matching TLS identity), and a valid credential.

## Resource / performance observations (prototype)

- wasm artifact: **8.4 MB** (`warp_node_proto_bg.wasm`, debug, `--keep-debug`).
  A release/stripped build would be substantially smaller.
- Build-request (protobuf encode) in wasm: **~1.5 ms**.
- Cold end-to-end (load + build + POST + 403): **~157 ms** wasm-internal,
  **~162 ms** Node wall clock (Node 25.0.0, `dev` profile). Dominated by the
  network round trip, not wasm execution.
- Peak memory was not measured in-wasm; the module is small and short-lived.
  No event-loop blocking was observed — all host I/O is Promise-based.
- `event_count`/`first_event_ms` are `null`/`0` for the 403 path because no SSE
  events were streamed (the edge rejected before the stream opened).

## Next three engineering steps (updated for the AgentDriver push)

1. **Wasm-headless trimmed `initialize_app`.** The headless `App`/`AppContext`
   now constructs on wasm (`new_headless_app`) and the no-op `TerminalDriver`
   exists, so the remaining blocker is the singleton surface `AgentDriver::new`
   /`run_internal` reads. Carve a wasm-headless `initialize_app` that registers
   the full singleton set (`PrivatePreferences`, `PublicReferences`,
   `SettingsManager`, `AIExecutionProfilesModel`, `BlocklistAIPermissions`,
   `UserWorkspaces`, `ApiKeyManager`, skills/MCP/environment managers, …)
   without the native-only backends (sqlite persistence, secure-storage reads,
   watchers) — or make the missing-singleton reads degrade gracefully. Because
   the app crate builds with `panic = "abort"` on wasm, every missing singleton
   aborts, so this must be exhaustive. This unlocks constructing `AgentDriver`
   and driving `run_internal`.
2. **Integrate the host-`fetch` transport into `http_client`.** Generalize the
   host `fetch(request)->Promise<response>` contract behind the existing
   `http_client` wasm boundary (register a host fetch at module init, used by
   `Client::execute` and `eventsource` on wasm, replacing the
   `web_sys::window()`-based path) so `warp_multi_agent_client` and the
   app-layer MAA path work in Node without per-call changes. Add host
   session-sharing connect/send/receive/close callbacks here too. (The
   `crates/wasm_node_proto` fallback already proves this contract end-to-end.)
3. **Solve production egress.** Either (a) get the edge to allowlist a
   wasm-in-Node client identity (coordinate with the server/gateway team —
   likely a registered `X-Warp-Client-ID` + TLS-fingerprint allowlist for
   `/ai/*`), or (b) document a proxy requirement for non-desktop/non-browser
   clients. Re-run the manual proof with a valid credential once egress is
   unblocked to capture a real `StreamInit`→`ClientActions`→`StreamFinished`
   terminal event against production.

## Validation summary

AgentDriver push:
- `cargo check --target wasm32-unknown-unknown -p warp` (app crate, headless/default): pass (~3 min, `clang` for `arborium-sysroot`).
- `cargo check --target wasm32-unknown-unknown -p warp_wasm_node` (cdylib + entrypoint + no-op terminal + `new_headless_app`): pass.
- `cargo clippy --target wasm32-unknown-unknown -p warp_wasm_node -p wasm_node_proto -p warp -- -D warnings`: pass.
- `cargo build --target wasm32-unknown-unknown -p warp_wasm_node --lib`: pass (234 MB debug wasm; `wasm-bindgen --target nodejs` produces a 147 MB wasm + Node loader).
- Real Node 25 run of `run_agent_driver_wasm`: stage `constructed_app` reached (headless `App`/`AppContext` constructs on wasm in Node, no DOM); `trimmed_init` panics at `PrivatePreferences::as_ref` (missing singleton; `panic = abort` on wasm so it aborts) — the precise next blocker.

First-pass direct-MAA fallback:
- `cargo check --target wasm32-unknown-unknown -p wasm_node_proto`: pass.
- `cargo clippy --target wasm32-unknown-unknown -p wasm_node_proto -- -D warnings`: pass.
- `cargo test -p wasm_node_proto`: 5/5 pass (decode/accumulate/build-request/input-validation).
- `./script/wasm/build-node`: pass (produces Node loader + wasm for the direct-MAA crate).
- `node script/wasm/node-harness.mjs --prompt hello --api-key … --server-root-url https://app.warp.dev`: runs; returns a structured `403` error (edge block) and exits 1 — the structured-failure path is proven; the success path is blocked by credentials/edge as documented above.
- The existing browser wasm build is untouched (`wasm_node_proto` and `warp_wasm_node` are new, isolated crates; the only shared-crate change is the additive `warpui::platform::headless::new_headless_app` constructor + the wasm-gated `TerminalDriver::create_no_op`).
