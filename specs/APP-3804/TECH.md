# TECH.md — Remote Server: Headless App + Message Transport Foundation

Linear: [APP-3721](https://linear.app/warpdotdev/issue/APP-3721)

## 1. Problem

The `remote_server` crate needs to become a standalone binary that communicates with the Warp client over remote connections with length-delimited protobuf messages. In order to support future coding features like the file tree and code review pane, the remote server needs the warpui App to store and handle `Entity`/`SingletonEntity` models like `RepositoryMetadataModel`.

This spec covers the foundation: a shared protocol layer, a minimal request/response client, the headless warpui server runtime, and `Initialize` end-to-end validation.

## 2. Relevant Code

### remote_server crate (current state)
- `remote_server/Cargo.toml` — current deps: `prost`, `tokio`, `prost-build`
- `remote_server/src/lib.rs` — library target re-exporting generated prost types
- `remote_server/proto/remote_server.proto` — `ClientMessage`/`ServerMessage` envelopes with `Initialize`/`InitializeResponse`
- `remote_server/build.rs` — prost codegen for the proto

### Headless warpui App infrastructure
- `crates/warpui/src/platform/app.rs:68-80` — `AppBuilder::new_headless(callbacks, assets, test_driver)` constructor
- `crates/warpui/src/platform/app.rs:107-155` — `AppBuilder::run(init_fn)` wraps init_fn and enters the event loop
- `crates/warpui/src/platform/headless/app.rs` — `App::run()` creates mpsc channel, marks main thread, enters `event_loop::run()`
- `crates/warpui/src/platform/headless/event_loop.rs` — blocking `for event in receiver.iter()` loop processing `RunTask`, `RunCallback`, `Terminate`; includes Ctrl-C handler via `ctrlc::set_handler`

### Entity/Model system
- `ui/src/core/entity.rs:39-54` — `Entity` trait (has `type Event`) and `SingletonEntity` trait (provides `handle()` and `as_ref()`)
- `ui/src/core/app.rs:2060-2077` — `AppContext::add_singleton_model(build_model)` registers a singleton
- `ui/src/core/app.rs:845-847` — `AppContext::background_executor()` returns `&Arc<Background>`

### ModelSpawner
- `ui/src/core/model/context.rs:442-466` — `ModelContext::spawner()` creates a `ModelSpawner<T>` (Send + Clone)
- `ui/src/core/model/context.rs:592-624` — `ModelSpawner<T>` definition; `spawn(work).await` dispatches `work` to main thread and returns the result
- `app/src/ai/agent_sdk/driver.rs:890-1027` — `AgentDriver::run_internal`: long async workflow using `ModelSpawner` to step into the model at specific points
- `app/src/workspace/view/global_search/model.rs:77-178` — `GlobalSearch`: background ripgrep task pushing result batches via `ModelSpawner`

### No-op asset provider
- `ui/src/assets/mod.rs:5-11` — `impl AssetProvider for ()` returns errors for all lookups

### App termination
- `ui/src/core/app.rs:3998-4012` — `AppContext::terminate_app(mode, result)` delegates to platform
- `ui/src/platform/mod.rs:282-292` — `TerminationMode` enum: `Cancellable`, `ForceTerminate`, `ContentTransferred`

## 3. Current State

The current `remote_server` crate has:
- Proto definition for `ClientMessage`/`ServerMessage` with `Initialize`/`InitializeResponse`
- `lib.rs` re-exporting generated prost types via `include!(concat!(env!("OUT_DIR"), "/remote_server.rs"))`
- No `main.rs`, no binary entry point, no I/O code, no warpui dependency

## 4. Proposed Changes

### 4.1. Shared `protocol.rs` in the `remote_server` library

Create `remote_server/src/protocol.rs` and re-export from `lib.rs`.

**Contents:**
- `ProtocolError` enum — covers I/O errors, decode failures, unexpected EOF, and message-too-large
- `read_message<M: prost::Message + Default>(reader) -> Result<M, ProtocolError>` — reads `[4-byte LE length][protobuf bytes]`, decodes into `M`
- `write_message<M: prost::Message>(writer, msg) -> Result<(), ProtocolError>` — encodes `M`, writes `[4-byte LE length][protobuf bytes]`
- Convenience wrappers: `read_client_message`, `write_client_message`, `read_server_message`, `write_server_message` that specialize the generic helpers for `ClientMessage` and `ServerMessage`

**Message size limit:** `read_message` rejects payloads exceeding `MAX_MESSAGE_SIZE` (64 MB) with `ProtocolError::MessageTooLarge` after decoding the `u32` length prefix but before allocating the payload buffer. This prevents OOM from a corrupted or adversarial length prefix. Since both `read_client_message` and `read_server_message` delegate to the generic `read_message`, the size check applies in both directions — protecting the server from oversized client requests and the client from oversized server responses.

The generic `read_message`/`write_message` take `tokio::io::AsyncRead + Unpin` / `tokio::io::AsyncWrite + Unpin` so both the server (stdin/stdout) and client (child process or SSH streams) can use them.

### 4.2. Minimal `RemoteServerClient` in the library

Create `remote_server/src/client.rs` and export from `lib.rs`.

**Structure:**
- `RemoteServerClient` struct owns:
  - `outbound_tx: async_channel::Sender<ClientMessage>` — feeds the background writer task
  - `pending_requests: Arc<DashMap<RequestId, oneshot::Sender<ServerMessage>>>` — maps `request_id` to response sender (shared with the reader task).

**`RequestId` newtype:** Introduce a `RequestId(String)` newtype in the `remote_server` library (e.g. in `protocol.rs`) wrapping the proto `string request_id` field. This provides type safety over raw strings and centralizes ID generation (`RequestId::new()` → `Uuid::new_v4().to_string()`). The proto field stays `string`; conversion happens at the serialization boundary. Use `RequestId` consistently in `RemoteServerClient`, `ServerModel`, and `pending_requests`.
- Constructor takes generic `reader: impl AsyncRead + Unpin + Send + 'static` and `writer: impl AsyncWrite + Unpin + Send + 'static`, plus a handle to the background executor (or accepts a `tokio::runtime::Handle`)
- Spawns two background tasks:
  - **Writer task**: a dedicated background task spawned at construction time. The write half of the connection (`impl AsyncWrite`) is moved into this task — no other code retains a reference. It pulls `ClientMessage`s from `outbound_rx` and writes each one via `protocol::write_client_message`.

    Callers never write to the stream directly. They only hold clones of `outbound_tx`, which enqueue messages into the channel. The channel acts as a FIFO queue: concurrent `send_request` calls are serialized into arrival order, and the writer task drains them one at a time.
  - **Reader task**: reads `ServerMessage`s via `protocol::read_server_message` in a loop, looks up `request_id` in `pending_requests`, sends response through the corresponding `oneshot::Sender`

**Public API:**
- `async fn initialize(&self) -> Result<InitializeResponse, ClientError>` — generates a `request_id`, sends `ClientMessage { initialize }`, awaits the correlated response
- Private `async fn send_request(&self, msg: ClientMessage) -> Result<ServerMessage, ClientError>` — generic request/response correlation
- `ClientError` enum covering disconnection, protocol errors, server-reported errors (`ClientError::ServerError`), and response timeout.

### 4.3. `main.rs` — headless App entry point

Create `remote_server/src/main.rs`. Add `[[bin]]` target in `Cargo.toml` and add `warpui` as a dependency.

```rust
fn main() -> anyhow::Result<()> {
    AppBuilder::new_headless(AppCallbacks::default(), Box::new(()), None)
        .run(|ctx| { /* init_fn */ })?;
    Ok(())
}
```

**Key details:**
- `AppCallbacks::default()` — all fields `None`, no custom callbacks needed
- `Box::new(())` — uses `impl AssetProvider for ()` (no-op, returns errors for all lookups)
- The headless `App::run()` creates the mpsc event channel, marks the current thread as main, and enters the blocking event loop. The `Background` executor inside the App IS the tokio runtime — there is exactly one runtime in the process.
- The headless warpui `App` infrastructure is proven in production (the Oz CLI uses it via `AppBuilder::new_headless` + `add_singleton_model` + `ModelSpawner`). It provides the full entity/model runtime with zero rendering overhead.

**Logging:**

Stdout is the wire transport — any stray output to stdout will corrupt the protocol and cause decode failures on the client. Logging must be configured to write exclusively to stderr before the App starts.

At the top of `main()`, before `AppBuilder::new_headless`:
```rust
env_logger::Builder::from_default_env()
    .target(env_logger::Target::Stderr)
    .init();
```

This ensures all `log::info!`, `log::error!`, etc. macros route to stderr.

**Client-side stderr streaming:** The client reads the server's stderr in a background task to surface server logs locally. It spawns a task that calls `read_line` on the child's stderr in a loop, forwarding each line to the client's own logging. Stderr streaming is the always-on fallback — it requires no protocol changes and continues to flow even when the protocol itself is broken, which is critical for debugging transport-level issues.

**Inside `init_fn`:**
1. Create a typed response channel: `async_channel::unbounded::<ServerMessage>()`
2. Register `ServerModel` as a singleton and obtain a `ModelSpawner` in the same step.
3. Spawn a **background stdin reader task** on `ctx.background_executor()`:
   - Wraps `tokio::io::stdin()` in a `BufReader`
   - Loops: `read_client_message(&mut reader).await` → `spawner.spawn(|model, ctx| model.handle_message(msg, ctx)).await`
   - Handles errors:
     - `Err(ModelDropped)` from `spawner.spawn()` by breaking out of the loop (this means the `ServerModel` was dropped during shutdown — no further messages should be processed)
     - **Recoverable errors** (log a warning and continue to the next message): errors where the stream is still correctly positioned at the next message boundary. Example: `ProtocolError::Decode` — the payload bytes were already consumed, so the next read starts at a valid length prefix.
     - **Fatal errors** (break and begin shutdown): errors where the stream is dead or misaligned. Examples: `ProtocolError::UnexpectedEof` (client disconnected), `ProtocolError::Io` (broken pipe, connection reset), `ProtocolError::MessageTooLarge` (payload not consumed, stream position is invalid).
     - On fatal error: dispatches `spawner.spawn(|_, ctx| ctx.terminate_app(TerminationMode::ForceTerminate, None))`. This is best-effort (`let _ =`) since the model may already be gone.
4. Spawn a **background stdout writer task** on `ctx.background_executor()`:
   - Wraps `tokio::io::stdout()` in a `BufWriter`
   - Receives `ServerMessage`s from the `async_channel::Receiver`
   - Calls `protocol::write_server_message(&mut writer, msg).await` for each
   - Exits naturally when the response channel closes (all senders dropped)

`app/src/lib.rs` stays thin: boot the headless app and register `ServerModel`.
It is called from the `WorkerCommand::RemoteServer` dispatch in `app/src/lib.rs`, which
returns early before the full app initialization path — identical to how `TerminalServer`
and other worker commands are structured.


### 4.4. `ServerModel` — remote-side main-thread orchestrator

Create `remote_server/src/server_model.rs`.

```rust
pub struct ServerModel {
    response_tx: async_channel::Sender<ServerMessage>,
}

impl Entity for ServerModel {
    type Event = ();
}

impl SingletonEntity for ServerModel {}
```

**Responsibilities:**
- Holds the typed response sender
- Exposes `handle_message(&mut self, msg: ClientMessage, ctx: &mut ModelContext<Self>)` — called by the background stdin reader via `ModelSpawner`
- Dispatches on `msg.message` (the `oneof` variant):
  - `Initialize` → constructs `InitializeResponse { server_version }` from `ChannelState::app_version()` (falling back to `env!("CARGO_PKG_VERSION")` in dev builds where `GIT_RELEASE_TAG` is unset), wraps in `ServerMessage { request_id: msg.request_id, message: Some(...) }`, sends via `self.response_tx`
  - `None` (missing variant) → sends `ErrorResponse { code: INVALID_REQUEST, message }` back to the client
  - Future message types will be added as new `oneof` variants in the proto and new match arms here

**Error responses:** The proto defines a shared `ErrorResponse` message (with an `ErrorCode` enum and a human-readable `message` string) as a variant in `ServerMessage.oneof`. This follows the JSON-RPC pattern: one error shape shared across all request types, with a machine-readable code for programmatic handling. The initial codes are `INVALID_REQUEST` and `INTERNAL`; domain-specific codes (e.g. `FILE_NOT_FOUND`) can be added as the protocol grows. On the client side, `ErrorResponse` maps to `ClientError::ServerError { code, message }`.
- Dispatches to future child models via `ctx.update_model(...)`, subscriptions, and emitted events — never ad-hoc cross-thread calls

**Design boundary:** Transport loops and protobuf byte encoding stay outside this model. `ServerModel` receives and sends typed Rust structs, not raw bytes.

### 4.5. Design Decision: `ModelSpawner` vs `spawn_stream_local`

Two warpui primitives could bridge background I/O to main-thread model context:

**`ModelSpawner` (chosen):** The background stdin reader task holds a `ModelSpawner<ServerModel>` and calls `spawner.spawn(|model, ctx| model.handle_message(msg, ctx)).await` for each decoded message. The transport loop is explicit code we own — it controls pacing, handles EOF, and manages shutdown. The model is a passive handler that doesn't know where messages come from.

- Precedent: `AgentDriver::run_internal` (`app/src/ai/agent_sdk/driver.rs:890`) uses `ModelSpawner` for a long async workflow. `GlobalSearch` (`app/src/workspace/view/global_search/model.rs:77`) uses it for a background producer pushing results.
- Advantage: transport-level concerns (reconnect, backpressure, batching, error recovery) stay in the transport loop, not in model callbacks. EOF handling is a simple `break` + terminate dispatch.

**`spawn_stream_local` (considered, not chosen):** The model would call `ctx.spawn_stream_local(request_rx, on_item, on_done)` during construction. Each item is delivered to an `on_item` callback; `on_done` fires on channel close.

- Precedent: `BulkFilesystemWatcher` (`watcher/src/lib.rs:163`) uses this for OS file events.
- Tradeoff: simpler setup for pure event-consumption, but the model owns the ingestion lifecycle. Transport-level logic (reconnect, rate-limiting) would need to live inside model callbacks.

We chose `ModelSpawner` because the remote server's transport layer will likely grow (protocol versioning, multiplexed streams) and keeping that logic in an explicit background loop is easier to extend.

### 4.6. Cargo.toml changes

Add to `remote_server/Cargo.toml`:
- `warpui` dependency (workspace) — for headless App, Entity, ModelContext, ModelSpawner
- `anyhow` (workspace) — error handling in main
- `tokio` features: add `io-std` for stdin/stdout access
- `async-channel` (workspace) — for all async channels (outbound client channel, server response channel). Avoid `tokio::sync::mpsc` for warpui-layer code.
- `log` (workspace) — structured logging
- `env_logger` (workspace) — stderr-only log output
- `dashmap` (workspace) — for lock-free concurrent request tracking in `RemoteServerClient`
- `thiserror` (workspace) — for `ProtocolError` and `ClientError` derive

## 5. End-to-End Flow

### Initialize handshake (client → server → client)

1. **Client** calls `RemoteServerClient::initialize()`:
   - Generates a UUID `request_id`
   - Constructs `ClientMessage { request_id, message: Initialize {} }`
   - Registers a `oneshot::Sender` in `pending_requests` keyed by `request_id`
   - Sends the message through `outbound_tx` to the writer task
2. **Client writer task** receives the `ClientMessage`, calls `protocol::write_client_message(stdout, msg)`:
   - Encodes via `prost::Message::encode`
   - Writes `[4-byte LE length][protobuf bytes]` to the stream
3. **Server stdin reader task** calls `protocol::read_client_message(stdin)`:
   - Reads 4 bytes → interprets as LE u32 length
   - Reads `length` bytes → decodes via `prost::Message::decode` into `ClientMessage`
   - Dispatches to main thread: `spawner.spawn(|model, ctx| model.handle_message(msg, ctx)).await`
4. **ServerModel::handle_message** (main thread):
   - Matches `Initialize` variant
   - Constructs `ServerMessage { request_id, message: InitializeResponse { server_version } }`
   - Sends via `self.response_tx.send(response)`
5. **Server stdout writer task** receives the `ServerMessage`, calls `protocol::write_server_message(stdout, msg)`:
   - Encodes and writes `[4-byte LE length][protobuf bytes]` to stdout
6. **Client reader task** calls `protocol::read_server_message(stdin)`:
   - Decodes `ServerMessage`, looks up `request_id` in `pending_requests`
   - Sends the response through the `oneshot::Sender`
7. **Client** `initialize()` awaits the oneshot, receives `InitializeResponse { server_version }`

### Shutdown (stdin EOF)

1. Server stdin reader task's `read_client_message` returns an error (EOF or broken pipe)
2. Reader loop breaks
3. Reader dispatches `spawner.spawn(|_, ctx| ctx.terminate_app(TerminationMode::ForceTerminate, None))`
4. The headless event loop receives `AppEvent::Terminate(ForceTerminate)` and breaks
5. Dropping all response senders closes the response channel; stdout writer task exits
6. On the client side, the reader task sees EOF on its stream, notifies pending requests of disconnection, and the client tears down

## 6. Risks and Mitigations

- **Client request/response matching**: Responses can arrive out of order once the server handles multiple message types concurrently. Mitigation: track in-flight requests by `request_id` with a `DashMap<RequestId, oneshot::Sender>`
- **warpui compile footprint**: Pulling in `warpui` brings transitive deps (fonts, rendering stubs). These are dead code in the headless binary — same tradeoff as the Oz CLI. No runtime cost, only compile time.
- **Main thread serialization**: All typed request handling runs on the main thread via the event loop. Handlers should be fast (in-memory dispatch and model coordination). Heavy work (filesystem I/O, tree building) must be offloaded to background tasks via `ctx.spawn()` or `ModelSpawner`.

## 7. Testing and Validation

- **Unit tests for `protocol.rs`**: Round-trip encode/decode for `ClientMessage` and `ServerMessage`. Test edge cases: zero-length messages, maximum length, malformed length prefix, truncated payload.
- **Unit tests for `RemoteServerClient`**: Use in-memory `tokio::io::duplex` streams to simulate a server. Verify `initialize()` returns the expected `InitializeResponse`. Verify correct `request_id` correlation. Verify `ClientError::Disconnected` on stream close.
- **Integration test for `Initialize` round trip**: Spawn `warp remote-server` as a child process, create a `RemoteServerClient` over the child's stdin/stdout, call `initialize()`, assert `server_version` is non-empty. The test lives in `app/tests/remote_server_tests.rs` — because the `warp` binary is a `[[bin]]` target in the `app` crate, cargo automatically builds it before running these tests.
- **Shutdown test**: Send an `Initialize`, then close the client's write end. Assert the server process exits cleanly (exit code 0).
- **Build validation**: `cargo build -p warp` produces a binary with the `remote-server` subcommand. `cargo clippy` and `cargo fmt` pass.

## 8. Follow-ups

- Feature-specific message types (file tree listing, filesystem watch events, code review context) as new `oneof` variants in the proto and new `ServerModel` match arms.
- SSH integration: the app wrapper on the local side that spawns the remote server binary over SSH and wraps a `RemoteServerClient` around the SSH channel.
- **Server lifecycle management**: V0 terminates the server immediately on fatal stream errors. Future versions should better handle transient errors, client disconnects, and retries.
- **Protocol-based log streaming**: In addition to stderr, add structured log delivery over the protocol. The server would install a custom `log` layer that, alongside stderr, sends each log event through the response channel as a `ServerMessage`.
