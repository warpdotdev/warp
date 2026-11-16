# TECH.md — Remote Server: Error Handling & Abort

Linear: [APP-3987](https://linear.app/warpdotdev/issue/APP-3987)

## 1. Problem

The remote server client/server protocol needs robust error handling for malformed messages and a mechanism for the client to cancel in-progress server requests. Specifically:

1. **Malformed messages** — if the request ID is parseable, return an error for that request; otherwise drop silently
2. **Client-side timeout** — abort requests that don't receive a response within 2 minutes
3. **Abort notification** — a new message type allowing the client to cancel in-progress server requests
4. **Message too big** — already enforced at the sending layer by `write_message`
5. **Unexpected responses** — already handled (drop + log warning)
6. **Stream errors** — no special handling needed

## 2. Relevant Code

- `crates/remote_server/proto/remote_server.proto` — `ClientMessage`/`ServerMessage` envelopes, `ErrorCode` enum, `ErrorResponse`
- `crates/remote_server/src/protocol.rs` — `ProtocolError` enum, `read_message`/`write_message` with length-delimited framing, `MAX_MESSAGE_SIZE` (64 MB), `RequestId` newtype
- `crates/remote_server/src/server_model.rs` — `ServerModel` singleton with stdin reader loop, stdout writer task, `handle_message` dispatch
- `crates/remote_server/src/client.rs` — `RemoteServerClient` with `pending_requests: DashMap<RequestId, oneshot::Sender<ServerMessage>>`, background reader/writer tasks, `ClientError` enum
- `crates/remote_server/src/client_tests.rs` — existing tests using `tokio::io::duplex` for in-memory streams
- `crates/remote_server/src/protocol_tests.rs` — round-trip and edge-case protocol tests

## 3. Current State

The protocol layer handles basic I/O errors with a recoverable/fatal classification:
- `ProtocolError::Decode` is read-recoverable (payload bytes consumed, stream aligned), but the request ID is lost — no error response can be sent back.
- `ProtocolError::MessageTooLarge` is enforced on write via `write_message`. On read, it's fatal because the oversized payload isn't consumed.
- The server's stdin reader loop skips recoverable errors and breaks on fatal ones.

The client's `pending_requests` map uses `oneshot::Sender<ServerMessage>`. There's no timeout — requests wait indefinitely. There's no way for the client to cancel server-side work.

The server has no concept of in-progress request tracking — `handle_message` is synchronous today (only `Initialize` exists). No abort mechanism exists.

## 4. Proposed Changes

### 4.1. Proto: Add `Abort` notification

Add to `remote_server.proto`:

```protobuf
message Abort {
  string request_id_to_abort = 1;
}
```

Add `Abort abort = 3` to `ClientMessage.oneof message`.

Abort is a notification (fire-and-forget) — no server response expected. The `request_id` on the `ClientMessage` envelope is still set (for logging/tracing) but the client does not register a pending request for it.

### 4.2. Protocol layer: Extract request ID from raw bytes on decode failure

Update `protocol.rs`:

- Change `ProtocolError::Decode` to carry the extracted request ID: `Decode(prost::DecodeError, Option<RequestId>)`. The extraction is done inside `read_message` itself using a private `try_extract_request_id` helper, so callers get the `Option<RequestId>` directly without needing to handle raw bytes.
- The private `try_extract_request_id(buf: &[u8]) -> Option<String>` parses only protobuf field 1 (string) using manual wire-format parsing: checks for tag byte `0x0a` (field_number=1, wire_type=2), decodes the varint length, and extracts the UTF-8 string. Stops immediately after field 1, so corruption in later bytes does not affect extraction.

### 4.3. Server: Error response for malformed messages with parseable request ID

Update the stdin reader loop in `ServerModel::new`:

- On `ProtocolError::Decode` with raw buffer, call `try_extract_request_id`. If a request ID is found, send an `ErrorResponse { code: INVALID_REQUEST }` with that request ID through `response_tx`. If no request ID can be extracted, drop the message (log warning, as already done).

### 4.4. Client: Change oneshot type to `Result<ServerMessage, ClientError>`

Change `pending_requests` from `DashMap<RequestId, oneshot::Sender<ServerMessage>>` to `DashMap<RequestId, oneshot::Sender<Result<ServerMessage, ClientError>>>`.

- Reader task sends `Ok(msg)` for normal responses.
- On `ProtocolError::Decode` with raw buffer: try `try_extract_request_id`. If found and a pending request exists, send `Err(ClientError::Protocol(...))` through the oneshot. If not found, drop (already logged).
- `send_request` unwraps the double-Result: `rx.await.map_err(|_| ClientError::ResponseChannelClosed)??`.
- Move `ErrorResponse` handling into `send_request`: after receiving an `Ok(msg)`, check if the message is an `ErrorResponse` and convert to `Err(ClientError::ServerError)`. Callers like `initialize()` then only match on success variants + `_ => UnexpectedResponse`.

### 4.5. Client: 2-minute request timeout

Update `send_request` to wrap the oneshot `rx.await` with `tokio::time::timeout(Duration::from_secs(120), rx)`. On timeout:

- Remove the request from `pending_requests`
- Send an `Abort` message for the timed-out request ID (via `send_notification`)
- Return `ClientError::Timeout`

Add `ClientError::Timeout` to the `ClientError` enum.

### 4.6. Client: `send_notification` helper

Add a private `fn send_notification(&self, msg: ClientMessage)` that sends through `outbound_tx` without registering a pending request. Used by timeout-triggered abort.

### 4.7. Server: In-progress request tracking & abort handling

Add `in_progress: HashMap<RequestId, tokio::sync::oneshot::Sender<()>>` to `ServerModel`. Each entry holds a cancellation signal sender.

- When `handle_message` starts processing a request that may be long-running, it inserts a cancellation receiver. The background work checks the receiver for cancellation.
- For synchronous/fast requests like `Initialize`, no tracking needed.
- On receiving `Abort { request_id_to_abort }`: look up the request ID in `in_progress`, if found send the cancel signal and remove the entry. If not found, no-op.
- When a request completes (response sent), remove it from `in_progress`.

Abort has pure notification semantics — the server does not send any response for abort messages.

Note: today all handlers are synchronous, so abort is mostly a no-op. This is future-proofing for long-running requests (file tree, code review, etc.). The infrastructure should be wired up now.

## 5. End-to-End Flow

### Malformed message with parseable request ID (server-side)

1. Client sends a corrupted protobuf where field 1 (request_id) is intact but other fields are malformed
2. Server stdin reader calls `read_client_message` → `ProtocolError::Decode(err, raw_buf)`
3. Reader calls `try_extract_request_id(&raw_buf)` → `Some("abc-123")`
4. Reader sends `ErrorResponse { code: INVALID_REQUEST, request_id: "abc-123" }` through `response_tx`
5. Server stdout writer sends the error response to the client
6. Client reader task resolves the pending request for "abc-123" with the error

### Client request timeout → abort

1. Client calls `send_request` which registers a oneshot in `pending_requests` and sends the `ClientMessage`
2. Server receives the request and begins (potentially long-running) work
3. After 2 minutes with no response, `tokio::time::timeout` fires in `send_request`
4. Client removes the request from `pending_requests`
5. Client sends `ClientMessage { request_id: new_uuid, message: Abort { request_id_to_abort: original_id } }` via `send_notification`
6. Client returns `ClientError::Timeout` to the caller
7. Server receives the `Abort`, looks up `original_id` in `in_progress`, sends the cancel signal if found

## 6. Risks and Mitigations

- **`ProtocolError::Decode` variant change is a breaking API change** — all existing match arms on `ProtocolError::Decode(_)` must be updated to `ProtocolError::Decode(_, Some(...))` / `ProtocolError::Decode(_, None)`. This is internal-only and caught at compile time.
- **`try_extract_request_id` false negatives** — if corruption hits the request_id field bytes, extraction fails and we fall back to dropping the message. This is the correct behavior since we genuinely can't correlate the error.
- **Oneshot type change ripples** — changing from `oneshot::Sender<ServerMessage>` to `oneshot::Sender<Result<ServerMessage, ClientError>>` affects the reader task and `send_request`. The existing test mock servers need updating since they construct `ServerMessage` directly.

## 7. Testing and Validation

- **Protocol test**: Verify `try_extract_request_id` extracts the ID from corrupted payloads where field 1 is intact, and returns `None` when field 1 is also corrupt.
- **Server malformed message test**: Send a corrupted protobuf with a valid request_id prefix → assert server responds with `ErrorResponse { code: INVALID_REQUEST }`.

## 8. Follow-ups

- Client timeout and abort tests (mock server that never responds, `tokio::time::pause()`)
- Abort cancellation tests (verify server cancels in-progress work)
- Domain-specific error codes (e.g. `FILE_NOT_FOUND`) as the protocol grows
- Structured log delivery over the protocol as an alternative to stderr streaming
