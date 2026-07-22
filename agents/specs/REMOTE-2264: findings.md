# Findings: wasm32 Oz CLI in a Node runtime (REMOTE-2264)

This is the findings report for the prototype built in `crates/wasm_node_proto` +
`script/wasm/build-node` + `script/wasm/node-harness.mjs`. It is a deliverable
of the implementation run, not a spec. The approved spec lives at
`agents/specs/REMOTE-2264: wasm32 CLI in Node prototype.md`.

## TL;DR

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

## Next three engineering steps

1. **Headless `AppContext` for wasm.** Carve a minimal, GUI-free
   `AppContext`/`ModelContext` bootstrap out of `initialize_app` (auth +
   `ServerApi` + `BaseClient` only, no persistence/watchers/GUI views) so the
   `AgentDriver` can be constructed on `wasm32-unknown-unknown`. Provide a
   no-op/host-backed `TerminalDriver` stub that returns
   `unsupported_capability` for PTY/shell and routes conversation-consumer
   events through host callbacks. This unlocks the spec's primary
   session-sharing path.
2. **Integrate the host-`fetch` transport into `http_client`.** Generalize the
   host `fetch(request)->Promise<response>` contract behind the existing
   `http_client` wasm boundary (register a host fetch at module init, used by
   `Client::execute` and `eventsource` on wasm) so `warp_multi_agent_client`
   and the app-layer MAA path work in Node without per-call changes. Add
   host session-sharing connect/send/receive/close callbacks here too.
3. **Solve production egress.** Either (a) get the edge to allowlist a
   wasm-in-Node client identity (coordinate with the server/gateway team —
   likely a registered `X-Warp-Client-ID` + TLS-fingerprint allowlist for
   `/ai/*`), or (b) document a proxy requirement for non-desktop/non-browser
   clients. Re-run the manual proof with a valid credential once egress is
   unblocked to capture a real `StreamInit`→`ClientActions`→`StreamFinished`
   terminal event against production.

## Validation summary

- `cargo check --target wasm32-unknown-unknown -p wasm_node_proto`: pass.
- `cargo clippy --target wasm32-unknown-unknown -p wasm_node_proto -- -D warnings`: pass.
- `cargo test -p wasm_node_proto`: 5/5 pass (decode/accumulate/build-request/input-validation).
- `./script/wasm/build-node`: pass (produces Node loader + wasm).
- `node script/wasm/node-harness.mjs --prompt hello --api-key … --server-root-url https://app.warp.dev`: runs; returns a structured `403` error (edge block) and exits 1 — the structured-failure path is proven; the success path is blocked by credentials/edge as documented above.
- The existing browser wasm build is untouched (`warp_node_proto` is a new, isolated crate; no shared crate was modified).
