# Spec: Unified file-read interface (GH-10049)

## Problem

The codebase uses ad-hoc `std::fs::read`, `read_to_string`, and
similar calls scattered across crates. There is no central place that
enforces:

- A maximum file size (so a stray gigabyte file can't OOM the editor).
- A binary-vs-text decision (so a binary blob can't get fed into a
  string-typed pipeline).
- A consistent error type for "file too big" / "file is binary" /
  "invalid UTF-8".

The existing approach has caused production OOMs and silent garbage
when binary content gets pulled into UI/AI surfaces.

## Goal

Add `crates/warp_files/src/safe_read.rs` (or a sibling crate) exposing
the `safe_read` module. Provide a small set of functions covering the
sync and async paths and migrate existing callers in waves. Add a
clippy lint restricting raw `std::fs::read*` and `tokio::fs::read*`
calls to the new helpers.

## Public API

V1 (sync only — ships first):

- `safe_read::read_to_string(path, opts: impl Into<SafeReadOpts>) -> Result<String, SafeReadError>`
- `safe_read::read_bytes(path, opts: impl Into<SafeReadOpts>) -> Result<Vec<u8>, SafeReadError>`

V1.5 (async helpers — ships before the lint is promoted to deny):

- `safe_read::async_read_to_string(path, opts: impl Into<SafeReadOpts>) -> Result<String, SafeReadError>`
- `safe_read::async_read_bytes(path, opts: impl Into<SafeReadOpts>) -> Result<Vec<u8>, SafeReadError>`

Both async helpers wrap the sync implementations on a Tokio
`spawn_blocking` boundary and apply the same size, UTF-8, and binary
checks. They are the compliant async path the lint will direct callers
to once promoted.

### `SafeReadOpts`

The `opts` argument is a fully-typed configuration struct. All four
public functions accept `impl Into<SafeReadOpts>` so callers may pass
`SafeReadOpts::default()`, a partially-constructed struct via the
`..Default::default()` spread, or build one explicitly.

```rust
pub struct SafeReadOpts {
    /// Maximum bytes to read. Default: 10 MiB.
    /// Reads exceeding this return `SafeReadError::TooLarge` and
    /// short-circuit before allocating the full payload (see B6).
    pub max_bytes: u64,

    /// Binary-content detection strategy for text reads.
    /// Default: `BinaryCheck::NulInFirst8Kib` (V1 heuristic).
    /// Ignored by `read_bytes` / `async_read_bytes`.
    pub binary_check: BinaryCheck,

    /// Tracing label for telemetry / errors. Defaults to `"unspecified"`.
    /// Convention: `"crate::module::function"`.
    pub call_site: &'static str,

    /// If true, follow symlinks. Default: true.
    /// When false, symlink targets return `SafeReadError::Io` with the
    /// underlying `ErrorKind::InvalidInput`.
    pub follow_symlinks: bool,
}

pub enum BinaryCheck {
    /// V1 default: NUL byte in first 8 KiB → `BinaryContent`.
    /// Documented limitation: admits non-text files that lack NULs
    /// (e.g., random UTF-8-encoded bytes, proto wire format without NULs).
    NulInFirst8Kib,
    /// V1.5 strict: full UTF-8 validation plus a control-char ratio check
    /// (>5% non-printable, non-whitespace bytes in first 8 KiB → `BinaryContent`).
    /// Higher CPU cost; recommended for AI/UI surfaces where false negatives
    /// from the default heuristic are unacceptable.
    Strict,
    /// Disable binary detection entirely. The text helpers will still run
    /// the UTF-8 check (B3); only the NUL/heuristic gate is skipped.
    None,
}

impl Default for SafeReadOpts {
    fn default() -> Self {
        Self {
            max_bytes: 10 * 1024 * 1024,
            binary_check: BinaryCheck::NulInFirst8Kib,
            call_site: "unspecified",
            follow_symlinks: true,
        }
    }
}
```

#### Override semantics

- All fields are independent. Callers override individual fields via
  the struct-update syntax: `SafeReadOpts { max_bytes: 1 << 20, ..Default::default() }`.
- There is no global mutable default. Each call resolves `opts` at
  call time; no environment variables or process-wide overrides are
  honored in V1. (Future revisions may add a `SafeReadDefaults`
  registry; explicitly out of scope here.)
- The `&'static str` `call_site` is intentionally not `String` to keep
  the call cheap and to discourage runtime-formatted labels in hot paths.

## Behavior contract

- B1. Text reads (`read_to_string` / `async_read_to_string`) reject
  files larger than `opts.max_bytes` (default: 10 MiB, see
  `SafeReadOpts::default`) with
  `SafeReadError::TooLarge { actual, max, path }`.
- B2. Text reads reject files whose first 8 KiB contain a NUL byte
  with `SafeReadError::BinaryContent { path }` to prevent silent
  corruption of `String`-typed pipelines. Callers that legitimately
  want raw bytes must use `read_bytes` / `async_read_bytes`. This
  is the V1 default (`BinaryCheck::NulInFirst8Kib`); see the
  "Binary-content detection contract" subsection below for the
  documented limitations and the opt-in `Strict` alternative.
- B3. Text reads validate UTF-8 on the full payload. Invalid UTF-8
  returns `SafeReadError::InvalidUtf8 { path, byte_offset }` where
  `byte_offset` points at the first invalid sequence. This error is
  non-retryable (re-reading the same bytes will fail the same way).
- B4. `BinaryContent` and `InvalidUtf8` are both non-retryable.
  Callers must switch to `read_bytes` to handle binary or
  non-UTF-8 content.
- B5. Byte reads (`read_bytes` / `async_read_bytes`) enforce
  `max_bytes` only — no NUL/UTF-8 check.
- B6. All four helpers stream-read for files larger than 1 MiB and
  short-circuit on the size check before allocating the full
  buffer.
- B8. **Non-regular file handling**: before any open, the helpers
  check `Metadata::file_type()`. Only regular files (`is_file()`)
  and symlink targets that resolve to regular files (when
  `opts.follow_symlinks == true`) are read. Every other file type
  — directories, FIFOs/named pipes, character devices, block
  devices, Unix sockets — returns
  `SafeReadError::Io { source: ErrorKind::InvalidInput, path }`
  immediately. This is a hard rule with no opt-out: the helpers
  never read from streams of unknown length, never block waiting
  for FIFO writers, and never read from `/dev/random`, `/dev/zero`,
  `/dev/urandom`, or comparable special devices. The "stream
  without known length" wording in B7 has been removed because no
  such stream is ever read; if a future revision adds support for
  FIFOs it must specify a hard read deadline and a separate opts
  flag.

### Error precedence (B7)

When multiple checks would apply to the same input, the helpers
evaluate them in this fixed order and short-circuit on the first
that matches. Only one `SafeReadError` variant is ever returned for
a single call:

1. **Symlink rejection** (when `opts.follow_symlinks == false`).
   Evaluated against the path metadata before any open. Returns
   `SafeReadError::Io` with `ErrorKind::InvalidInput`.
2. **Open / metadata I/O errors** (permission denied, not found,
   etc.). Returns `SafeReadError::Io { source }`.
3. **Non-regular file rejection** (per B8). If
   `Metadata::file_type()` is anything other than a regular file
   (or a symlink resolving to a regular file when
   `follow_symlinks` is on), returns
   `SafeReadError::Io { source: ErrorKind::InvalidInput, path }`
   without opening the file.
4. **Size check** against `opts.max_bytes`. Evaluated from
   `Metadata::len()` for regular files before any read. Because
   B8 rejects every non-regular file, no streaming size check is
   needed for unknown-length sources. Returns
   `SafeReadError::TooLarge`.
5. **Binary check** (text helpers only) — runs against the first
   8 KiB chunk as it streams in. Returns
   `SafeReadError::BinaryContent`.
6. **UTF-8 validation** (text helpers only) — runs against the
   complete payload after the binary check passes. Returns
   `SafeReadError::InvalidUtf8`.

Rationale: cheap checks first (metadata, then 8 KiB header), so an
overlong binary file returns `TooLarge` rather than reading the 8 KiB
just to call it `BinaryContent`. The order is part of the public
contract and is asserted by `T_error_precedence` (see Test plan).

| Overlap case (text read) | Expected error |
|---|---|
| Path is a symlink + `follow_symlinks=false` + file is huge + binary + invalid UTF-8 | `Io { ErrorKind::InvalidInput }` |
| File is 1 GiB AND contains NUL in first 8 KiB AND has invalid UTF-8 | `TooLarge` |
| File is within `max_bytes` AND has NUL in first 8 KiB AND has invalid UTF-8 | `BinaryContent` |
| File is within `max_bytes` AND no NUL in first 8 KiB AND invalid UTF-8 | `InvalidUtf8` |

### Binary-content detection contract

The V1 default heuristic — NUL byte in first 8 KiB — is a deliberate,
limited contract. It is the right default for the file-mix Warp
encounters in practice (source code, markdown, config), but it is not
a full binary detector.

**What the default catches:**

- Most binaries (ELF, PE, Mach-O, .o object files, common image
  formats with embedded NULs in headers).
- UTF-16 / UTF-32 encoded text (which contains NULs in ASCII-range
  code units).
- Truncated archives whose header contains a NUL.

**Documented false-negatives the default admits** (these will reach
the UTF-8 validator at B3, which catches most but not all of them):

- Random non-text bytes that happen to lack NULs (e.g., an 8 KiB
  prefix of a proto wire-format message that uses no NUL field tags).
- Non-text files in single-byte encodings (Latin-1 binaries) whose
  bytes happen to form valid UTF-8 at the byte level.
- Compressed payloads whose first 8 KiB lacks a NUL by chance.

These false-negatives are accepted as the V1 trade-off in exchange
for low CPU cost on the hot path.

**Callers requiring stronger guarantees** set
`opts.binary_check = BinaryCheck::Strict`, which adds:

1. UTF-8 validation streamed across the first 8 KiB (in addition to
   the full-payload UTF-8 check at B3).
2. A control-character ratio check: if more than 5% of the first
   8 KiB consists of bytes outside `[0x09, 0x0A, 0x0D, 0x20..=0x7E]`
   plus valid multi-byte UTF-8 starts, the read returns
   `BinaryContent`.

`Strict` ships in V1.5 alongside the async helpers. AI/UI surfaces
where false negatives corrupt downstream prompts should adopt
`Strict` once available.

#### Required follow-up: AI/UI surfaces migrate to `Strict` in V1.5

This is a hard follow-up commitment, not a recommendation. The V1
migration moves high-traffic call sites to the `safe_read` helpers
with the default `BinaryCheck::NulInFirst8Kib` so they ship before
`Strict` exists. To ensure prompt and UI reads do not stay on the
weaker heuristic indefinitely, V1.5 has the following acceptance
criteria:

1. `BinaryCheck::Strict` is implemented and merged.
2. The following V1-migrated AI/UI call sites are updated in V1.5 to
   pass `binary_check: BinaryCheck::Strict` (the rest may keep the
   default):
   - `crates/ai/src/skills/parse_skill.rs::parse_markdown_file`
     (skill content goes directly into model prompts).
   - `app/src/code/local_code_editor.rs` file-open path when the
     content is forwarded to an AI agent or rendered into the UI
     viewport (non-AI editor opens may keep the default).
   - Any call site reachable from an "attach file to chat" or
     "include file context" UI affordance.
3. A new V1.5 acceptance test (`T_strict_migration`) asserts each of
   the call sites above resolves `opts.binary_check` to
   `BinaryCheck::Strict`. The test is a `grep`-style audit, similar
   to `T_lint_severity_location`, that fails if any of the listed
   call sites still uses the default.
4. The V1.5 PR description must list each migrated call site
   explicitly, mirroring the V1 migration table.

If V1.5 ships without item 2, the V2 `deny` promotion is blocked
until item 2 is addressed in a follow-up PR. This prevents the
"V1 ships, follow-up forgotten" failure mode where AI/UI surfaces
quietly stay on the weaker heuristic indefinitely.

## Error type

```rust
pub enum SafeReadError {
    TooLarge { actual: u64, max: u64, path: PathBuf },
    BinaryContent { path: PathBuf },
    InvalidUtf8 { path: PathBuf, byte_offset: usize },
    Io { path: PathBuf, source: std::io::Error },
}
```

`BinaryContent` and `InvalidUtf8` are non-retryable: callers must
either use `read_bytes` or surface the failure to the user.

### Path redaction in telemetry and logs

`SafeReadError` carries the full `PathBuf` so the immediate caller
can surface it to the local user (e.g., a toast or editor error
banner). It is **never** safe to forward that path verbatim to
remote telemetry, crash reports, or shared logs. The spec imposes
the following rules on every emit site that consumes
`SafeReadError`:

1. `Display`/`Debug` impls on `SafeReadError` redact the full path
   to the file's basename (`PathBuf::file_name()`) plus its
   classification (e.g., `"<binary>/skill.md"`,
   `"<too-large>/changelog.txt"`). The full path is reachable only
   via the explicit `path()` accessor, which is documented as
   "local-only — do not log".
2. The `call_site: &'static str` field is the canonical telemetry
   key for *where* the read came from. Telemetry events emit
   `{ call_site, error_kind, byte_size_bucket }`; they do not emit
   `path`, `path.file_name()`, or any other user-content-derived
   field.
3. Server-side error logs (warp-internal trace pipelines) follow
   the same rule: paths are redacted to basename, dollar amounts
   and absolute paths are scrubbed by the existing log-redaction
   layer, and raw file contents from the failed read are never
   logged.
4. A unit test (`T_path_redaction`) constructs each
   `SafeReadError` variant with an absolute path containing a
   home-directory prefix, formats it with `Display` and `Debug`,
   and asserts the formatted output contains only the basename and
   never the parent directory components.

## Lint enforcement

`.clippy.toml` carries lint **data** (which methods are disallowed and
why); lint **severity** (`warn` / `deny`) is applied via crate-root
attributes in Rust source. These two surfaces together drive the
`disallowed-methods` clippy diagnostic.

### Lint data: `.clippy.toml`

A `.clippy.toml` already exists at the workspace root with
`disallowed-macros`, `disallowed-types`, and a populated
`disallowed-methods` list (covering
`async_channel::Sender::send_blocking` and
`line_ending::LineEnding::from_current_platform`). The V1 PR
**extends the existing `disallowed-methods` array** by appending the
four new entries below. It does NOT replace, reorder, or remove any
existing entry, and it does NOT modify the `disallowed-macros` or
`disallowed-types` blocks. This file does NOT control whether the
lint fires as a warning or an error — that is set at the crate root
(see "Lint severity" below).

The post-V1 `disallowed-methods` array, with new entries appended at
the end:

```toml
disallowed-methods = [
    # Pre-existing entries (preserved verbatim — do NOT modify):
    { path = "async_channel::Sender::send_blocking", reason = "send_blocking() does not exist for wasm.  Use warpui::r#async::block_on() with send() instead.", allow-invalid = true },
    { path = "line_ending::LineEnding::from_current_platform", reason = "line_ending::LineEnding::from_current_platform does not account for Unix-like subsystems for Windows. In most cases, use warp_core::platform::SessionPlatform::default_line_ending() instead." },

    # New entries added in V1 (#10049):
    { path = "std::fs::read",                 reason = "use safe_read::read_bytes" },
    { path = "std::fs::read_to_string",       reason = "use safe_read::read_to_string" },
    { path = "tokio::fs::read",               reason = "use safe_read::async_read_bytes" },
    { path = "tokio::fs::read_to_string",     reason = "use safe_read::async_read_to_string" },
]
```

Acceptance for the lint-data change is:

1. The two pre-existing `disallowed-methods` entries are byte-identical
   before and after the V1 PR.
2. The four new entries are appended in the order shown.
3. `disallowed-macros` and `disallowed-types` are unchanged.

### Lint severity: crate-root attributes

Lint level is set via `#![warn(...)]` / `#![deny(...)]` attributes at
the root of each affected crate (`lib.rs` for libraries, `main.rs`
for binaries). `.clippy.toml` cannot configure severity for
`disallowed-methods`; the attribute is the only mechanism.

V1 application of `warn`:

```rust
// app/src/lib.rs (and the crate root of every crate listed below)
#![warn(clippy::disallowed_methods)]
```

#### Concrete V1 crate-root coverage list

The V1 PR adds `#![warn(clippy::disallowed_methods)]` to the crate
root (`lib.rs` or `main.rs`) of every workspace crate enumerated
below. This list is the acceptance criterion: the `tests/lint_attribute_audit.rs`
fixture asserts the attribute exists in exactly these files, and CI
fails if a crate is added or removed without updating both the spec
list and the audit test.

`app/` (binary):
- `app/src/main.rs`

`crates/` (libraries — derived from the workspace member list and
filtered to crates with non-test production code):

- `crates/ai/src/lib.rs`
- `crates/app-installation-detection/src/lib.rs`
- `crates/asset_cache/src/lib.rs`
- `crates/asset_macro/src/lib.rs`
- `crates/channel_versions/src/lib.rs`
- `crates/command/src/lib.rs`
- `crates/command-signatures-v2/src/lib.rs`
- `crates/computer_use/src/lib.rs`
- `crates/editor/src/lib.rs`
- `crates/field_mask/src/lib.rs`
- `crates/firebase/src/lib.rs`
- `crates/fuzzy_match/src/lib.rs`
- `crates/graphql/src/lib.rs`
- `crates/handlebars/src/lib.rs`
- `crates/http_client/src/lib.rs`
- `crates/http_server/src/lib.rs`
- `crates/input_classifier/src/lib.rs`
- `crates/ipc/src/lib.rs`
- `crates/isolation_platform/src/lib.rs`
- `crates/jsonrpc/src/lib.rs`
- `crates/languages/src/lib.rs`
- `crates/lsp/src/lib.rs`
- `crates/managed_secrets/src/lib.rs`
- `crates/managed_secrets_wasm/src/lib.rs`
- `crates/markdown_parser/src/lib.rs`
- `crates/natural_language_detection/src/lib.rs`
- `crates/node_runtime/src/lib.rs`
- `crates/onboarding/src/lib.rs`
- `crates/persistence/src/lib.rs`
- `crates/prevent_sleep/src/lib.rs`
- `crates/remote_server/src/lib.rs`
- `crates/repo_metadata/src/lib.rs`
- `crates/serve-wasm/src/lib.rs`
- `crates/settings/src/lib.rs`
- `crates/settings_value/src/lib.rs`
- `crates/settings_value_derive/src/lib.rs`
- `crates/simple_logger/src/lib.rs`
- `crates/string-offset/src/lib.rs`
- `crates/sum_tree/src/lib.rs`
- `crates/syntax_tree/src/lib.rs`
- `crates/ui_components/src/lib.rs`
- `crates/vim/src/lib.rs`
- `crates/virtual_fs/src/lib.rs`
- `crates/voice_input/src/lib.rs`
- `crates/warp_cli/src/main.rs` (binary)
- `crates/warp_completer/src/lib.rs`
- `crates/warp_core/src/lib.rs`
- `crates/warp_features/src/lib.rs`
- `crates/warp_files/src/lib.rs`
- `crates/warp_js/src/lib.rs`
- `crates/warp_logging/src/lib.rs`
- `crates/warp_ripgrep/src/lib.rs`

The list is enumerated explicitly so that the V1 PR's diff can be
reviewed against a closed set rather than "every workspace member".
Test-only support crates (e.g., `crates/integration_testing` if its
every module is `#[cfg(test)]`-gated) and integration test harnesses
do not receive the attribute. `Integration` and `integration` style
crates are inspected before merge: if every public module is
`#[cfg(test)]`-only, they are excluded from the list above.

If new crates are added to the workspace after V1 lands but before V2
flips to `deny`, the V2 PR's first step is to extend the list above
to cover them; otherwise the deny promotion creates a hard CI failure
the moment the new crate first imports `std::fs::read*`.

V2 promotion to `deny` is a single change at each of those same crate
roots:

```rust
// app/src/lib.rs (and every other crate root touched in V1)
#![deny(clippy::disallowed_methods)]
```

`.clippy.toml` itself is not edited at promotion time — only the
attributes change. The V2 PR's diff is purely `warn` → `deny` across
the crate roots, with `.clippy.toml` untouched.

### Path allowlist

The lint is suppressed in the following path patterns. These
suppressions are documented in the implementation crate's lint
configuration (via `#![allow(clippy::disallowed_methods)]` at the
file or module head) so the allowlist is auditable from source:

- `crates/warp_files/src/safe_read.rs` — the implementation itself
  must call the underlying `std::fs` / `tokio::fs` primitives.
- `**/tests/**` — integration test directories.
- `**/*_test.rs` — unit test files following the in-tree convention.
- `**/fixtures/**` — test fixture helpers.
- Any module gated entirely by `#[cfg(test)]`.

Production code outside these patterns has no escape hatch other than
migrating to `safe_read`.

#### Allow-attribute audit

Rust permits `#[allow(clippy::disallowed_methods)]` at the function,
module, or crate level, which would silently defeat this lint. To
keep the "no escape hatch" rule meaningful, the V1 PR adds a
build-time audit test (`tests/disallowed_methods_allow_audit.rs`)
that walks every `.rs` file under `app/` and `crates/` and fails CI
if it finds:

1. `#[allow(clippy::disallowed_methods)]` outside the path
   allowlist above.
2. `#[allow(clippy::disallowed_methods)]` listed in a multi-lint
   `#[allow(...)]` group outside the allowlist (the audit greps
   tokens, not exact attribute strings).
3. `#![allow(clippy::disallowed_methods)]` at any crate root (this
   would defeat the V2 deny entirely).
4. The same checks for `#[expect(...)]` (Rust 1.81+ alternative
   spelling) and tool-prefixed forms (e.g.,
   `#[allow(clippy :: disallowed_methods)]` with whitespace).

Adding a new allow-site to production code requires (a) a code
review approval explicitly citing `disallowed_methods`, (b) an
update to the path allowlist above, and (c) a matching update to
the audit test's allowlist constant. The audit is the only
sanctioned mechanism for granting an exemption; ad-hoc `#[allow]`
attributes fail CI.

The audit fixture lives in `crates/warp_files/tests/` because that
crate already owns the `safe_read` module and its test deps.

### Warn → deny promotion plan

- **V1**: every workspace crate root carries
  `#![warn(clippy::disallowed_methods)]`. The four `safe_read` sync
  helpers ship. The five high-traffic call sites listed below are
  migrated in the same PR. Other call sites remain as warnings so
  contributors see them in CI output without a hard block.
- **V1.5**: ship the four async helpers. Begin migrating async call
  sites. Crate-root attributes remain `warn`.
- **V2**: once the open-warning count from `cargo clippy --all-targets`
  reaches zero on `master` for one release cycle, change the
  crate-root attributes from `#![warn(clippy::disallowed_methods)]` to
  `#![deny(clippy::disallowed_methods)]` in every crate touched in
  V1. `.clippy.toml` is not edited at promotion time. From this point
  forward, any new `std::fs::read*` or `tokio::fs::read*` call outside
  the allowlist fails CI.

The promotion is gated on the warning count rather than a fixed date
so the migration milestone is the trigger.

## High-traffic V1 migration targets

Identified via `git grep "fs::read"` in app/src and crates/:
1. `app/src/code/local_code_editor.rs` (file open path).
2. `crates/ai/src/skills/parse_skill.rs::parse_markdown_file`.
3. `app/src/changelog_model.rs` (changelog read).
4. `app/src/notebooks/file/...` (notebook restore).
5. `app/src/integration_testing/...` (test fixtures, opt-out via
   `#[cfg(test)]`).

## Test plan

- T1. `safe_read::read_to_string(small_text_file, default) -> Ok(content)`.
- T2. `safe_read::read_to_string(huge_file, default) -> TooLarge`.
- T3. `safe_read::read_to_string(binary_file, default) -> BinaryContent`
  (binary fixture contains a NUL byte in the first 8 KiB).
- T_binary_known_limitations. `safe_read::read_to_string(non_text_no_nul, default)`
  on a fixture of non-text bytes that lack NULs in the first 8 KiB
  but are still not legitimately text. Two sub-cases:
    - `non_text_invalid_utf8.bin` — the default heuristic does NOT
      flag this as `BinaryContent`, but B3 catches it as
      `InvalidUtf8`. Asserts the default's documented false-negative
      class is bounded by the UTF-8 validator.
    - `non_text_valid_utf8.bin` — synthesized payload that is
      byte-valid UTF-8 yet semantically non-text (e.g., random
      printable Unicode interspersed with control runs). Default
      returns `Ok(...)` (documented limitation); switching the same
      call to `BinaryCheck::Strict` returns `BinaryContent` via the
      control-character ratio rule. This codifies the contract that
      `Strict` is the supported mitigation.
- T4. `safe_read::read_bytes(binary_file, default) -> Ok(bytes)`.
- T5. `safe_read::read_to_string(invalid_utf8_file, default) -> InvalidUtf8`
  with `byte_offset` pointing at the first invalid sequence.
- T6. Stream-short-circuit: a 100 MiB file rejected without
  allocating 100 MiB (verify peak heap via a `dhat`-instrumented
  test or a smaller `max_bytes` ceiling).
- T7. Clippy lint fires on a fixture call to `std::fs::read`
  outside the implementation and outside the test allowlist.
- T8. Clippy lint does not fire on the same call inside a
  `**/tests/**` file.
- T9. Migration regression: each V1 migrated call site has a
  test that round-trips through the new helper.
- T10. (V1.5) Async parity: `async_read_to_string` and
  `async_read_bytes` produce identical errors and successful
  payloads as their sync counterparts on the same fixtures.
- T_error_precedence. Build a synthetic fixture that violates every
  rule simultaneously: a symlink pointing at a 1 GiB file whose
  first 8 KiB contains a NUL byte at offset 4 KiB and whose tail
  contains an invalid UTF-8 sequence. Run the same path through
  `read_to_string` four times with progressively relaxed `opts`:
    1. `follow_symlinks: false` → expect `Io { ErrorKind::InvalidInput }`.
    2. `follow_symlinks: true, max_bytes: 100` → expect `TooLarge`.
    3. `follow_symlinks: true, max_bytes: 2 GiB` → expect `BinaryContent`.
    4. As (3) but on a sibling fixture without the NUL byte → expect
       `InvalidUtf8`.
  Asserts the precedence ordering documented in B7.
- T_lint_severity_location. Build-level test that asserts the
  V1 PR adds `#![warn(clippy::disallowed_methods)]` to every
  workspace crate root listed in the migration plan, and that
  `.clippy.toml` does not contain a `disallowed-methods-level` or
  similar severity key (since severity lives in the attribute, not
  the data file). Implemented as a `tests/lint_attribute_audit.rs`
  fixture that greps the crate roots for the attribute and parses
  `.clippy.toml` to confirm only the data fields are present.

## Files touched

- `crates/warp_files/src/safe_read.rs` (new — sync helpers in V1,
  async helpers added in V1.5).
- `crates/warp_files/src/error.rs` (new — `SafeReadError` enum).
- `crates/warp_files/src/opts.rs` (new — `SafeReadOpts` struct,
  `BinaryCheck` enum, and the `Default` impl).
- `.clippy.toml` (**modified — existing workspace-root file**).
  Per the "Lint data: `.clippy.toml`" section above, this file
  already exists with `disallowed-macros`, `disallowed-types`,
  and a populated `disallowed-methods` array. The V1 PR appends
  exactly the four new entries shown there to the existing
  `disallowed-methods` array; it does not create a new file,
  remove or reorder existing entries, or touch
  `disallowed-macros` / `disallowed-types`. No severity field is
  added.
- Crate-root attributes: `#![warn(clippy::disallowed_methods)]` added
  to the `lib.rs` / `main.rs` of every production crate in the
  workspace in V1; flipped to `#![deny(...)]` in V2. The exact set
  of crate roots is enumerated in the V1 implementation PR.
- 5 call-site migrations listed above (V1).
- Per-callsite tests plus `tests/lint_attribute_audit.rs`.

## Non-goals

- Configurable per-extension binary detection (`.svg` is XML but may
  have NULs in CDATA). Callers that legitimately need bytes use
  `read_bytes`.
- Migrating every call site in V1. Lint-as-`warn` lets late call
  sites surface gradually without blocking the V1 PR.
- Streaming/iterator APIs that yield content without ever buffering.
  Out of scope for this spec; `safe_read` is for whole-file reads.

Note: async helpers were originally listed as out of scope. They are
now in scope as V1.5 because the lint promotion to `deny` would
otherwise leave async callers with no compliant path.
