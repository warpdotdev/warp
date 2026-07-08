---
name: logging-and-error-reporting
description: How and when to log (log::* levels, safe_* macros) and report errors to Sentry (report_error!) in the Warp codebase. Use when adding or reviewing any logging or error reporting — picking a log level, deciding log vs. report_error!, keeping sensitive data out of logs, or surfacing an error to Sentry.
---

# logging-and-error-reporting

Warp has two related ways to surface what happened at runtime:
- **`log::*`** (`error!`/`warn!`/`info!`/`debug!`/`trace!`) — local diagnostics written to the terminal/log file and, on crash-reporting builds, uploaded to Sentry as **breadcrumbs** (context attached to the next captured event).
- **`report_error!`** — captures a **structured Sentry event** (an actual issue) for errors worth engineering attention.

## How logs reach Sentry (important)

On crash-reporting builds a `SentryLogger` wraps the logger (`warp_logging`). The filter:
- `Error` / `Warn` / `Info` → **breadcrumb** only (not their own Sentry issue).
- `Debug` / `Trace` → **dropped** from Sentry entirely (local-only).
- The `Error`-level line emitted by `report_error!` itself → **ignored** by Sentry (the macro already captured a structured event; the log line would double-report).
- A few noisy targets (wgpu, `panic`, redraw-frame, the crash-reporting module) are dropped.

Consequences:
- **`log::error!` does NOT create a Sentry issue** — it's only a breadcrumb. If a failure should be tracked in Sentry, use `report_error!`. Only `report_error!` and panics create Sentry events.
- Breadcrumbs (Info and above) are uploaded, so they must never contain secrets or PII — see "Sensitive data: safe_* macros" below.

## Choosing: log vs. `report_error!`

- Use **`report_error!`** when a failure should become a Sentry issue an engineer may act on.
- Use **`log::*`** for everything else: lifecycle/state info, expected or handled conditions, and diagnostic dumps or informational "loud logs" (e.g. listing valid options after a lookup miss). If a function's name/doc says it *logs* (or it emits a header line plus one entry per item), it should use `log::*`, not `report_error!`.
- Don't signal the same failure twice. Report it once, at the sink where it stops propagating; `log::*` breadcrumbs on the way there are useful context (see "Report once, at the sink").

## Log levels

The default filter is `Info`, so `debug!`/`trace!` are off unless `RUST_LOG` enables them.
- **`error!`** — a real failure the code handled locally. Remember this is only a breadcrumb; if it warrants a Sentry issue, use `report_error!` instead (or as well, at the sink).
- **`warn!`** — unexpected but recoverable/handled: degraded paths, retries, fallbacks, skipped work.
- **`info!`** — coarse lifecycle/state milestones (startup, connection up/down, feature enabled). On by default and uploaded as breadcrumbs, so keep it low-volume and free of per-item spam.
- **`debug!`** — verbose diagnostics for local development; off by default; never sent to Sentry.
- **`trace!`** — very fine-grained / hot-path detail; off by default; never sent to Sentry.

Guidance:
- Hot loops, per-frame render paths, and per-message handlers must log at `debug!`/`trace!` (never `info!`+), or they flood the log file and breadcrumb buffer.
- Prefer static, greppable message prefixes with structured `key=value` detail (e.g. `"[Remote codebase indexing] … repo_path={} state={:?}"`), matching the surrounding module.
- Use inline format args (`log::warn!("… {err:#}")`) per the workspace clippy config; format an error chain with `{err:#}`.

## Sensitive data: `safe_*` macros

Never log secrets, tokens, or credentials at any level. For messages whose *useful* detail is sensitive-ish — file paths, response payloads, user-generated content — that you want locally but must NOT ship in release-channel logs (which get bundled and become breadcrumbs), use the `safe_*` macros instead of `log::*`.

`safe_error!`, `safe_warn!`, `safe_info!`, `safe_debug!` (plus `safe_anyhow!` to build an `anyhow::Error`, and `safe_eprintln!`) each take a `safe:` and a `full:` arm. Dogfood builds log `full:`; release channels log `safe:`:
```rust
use warp_core::safe_error;

safe_error!(
    safe: ("Remote server unexpected response for Initialize"),
    full: ("Remote server unexpected response for Initialize: response={other:?}")
);
```
- The `safe:` arm must stand on its own and contain no sensitive/verbose detail; put those bits only in `full:`.
- Import from `warp_core` (`use warp_core::{safe_error, safe_warn};`). These live in `warp_core` and depend on channel state, so they're unavailable in the `warp_errors`-only leaf crates (`warpui_core`, `warpui`, `settings`, …) — there, log non-sensitive messages only.

---

# Reporting errors with `report_error!`

`report_error!` is the explicit way to send an error to Sentry as a structured event. At runtime it checks `err.is_actionable()`:
- **actionable** → captured to Sentry AND logged at `Error` level.
- **not actionable** (e.g. registered network errors: reqwest connect/5xx/429, tokio `JoinError` cancelled) → logged at `Warn` level only, never sent to Sentry.

This classification only works if the *typed error* reaches the macro. Sentry buckets events into issues by a **fingerprint** (its grouping key); because these events are stacktrace-less, Sentry derives that fingerprint from the **message text**. Interpolating instance-specific data (ids, paths, counts, a stringified error) into the message changes the fingerprint on every occurrence, so one logical error fragments into countless separate issues (and burns through quota). Keep the message a **static string** so the fingerprint stays stable; per-instance data goes in the error chain (via `.context()`) or a structured `extra:` block — neither of which affects the fingerprint — never interpolated into the message.

Bad (fragments Sentry grouping into one group per distinct value, and — when it stringifies a typed error — hides it from `is_actionable`):
```rust
report_error!("Failed to read persisted data: {err}");
```
Good (canonical in-tree example, `app/src/persistence/sqlite.rs`):
```rust
report_error!(anyhow::Error::new(err).context("Failed to read persisted data"));
```

## Prefer typed error enums over `anyhow` when it makes sense

Don't default to `anyhow`. The conventional Rust guidance is "`anyhow` for applications, `thiserror` for libraries," but the sharper question is: **does anything downstream need to tell the failures apart?** If yes, define a typed error enum (`thiserror`), `impl ErrorExt`, and `register_error!` it. In Warp that "downstream" includes the Sentry reporting layer, not just calling code — so typed errors pay off more often than the plain app-vs-library rule suggests.

Reach for a typed error enum when these hold:
- **A caller branches on the failure** — it `match`es to recover, retry, fall back, render a specific message/state, or map to a status. This is the classic reason and the strongest signal: an `anyhow::Error` is opaque, so callers can essentially only print it.
- **Mixed actionability** — some variants are real bugs worth a Sentry issue and others are expected/environmental (network, auth-expired, user-cancelled, not-found). Per-variant `is_actionable()` reports the bugs and stays silent on the noise; `anyhow` is all-or-nothing.
- **A fixed, known set of failure modes** worth naming — it makes a function's failure surface visible in its signature and gives Sentry stable, meaningful groups (one per variant) instead of one catch-all bucket.
- **The same failure recurs across many call sites** — define the message and classification once on the type, then report once at the sink.

Default to `anyhow` when these hold:
- The error is only **propagated** (`?` / `.context("…")`) up to a sink that logs/reports/displays it — nobody matches on its kind.
- The failure modes are open-ended or not worth enumerating.
- A static `.context("…")` string carries enough for a human reading the Sentry event or log.
- It's leaf/glue code, not an API boundary other code depends on.

It's not either/or: a typed enum can keep an `anyhow` escape hatch for the genuinely-unexpected case (an `Unexpected(#[from] anyhow::Error)` variant), classifying the known failures precisely while still absorbing the rest. Group variants by **failure mode** (what went wrong / what the caller does about it), not by which crate produced the error.

Real example (`UserAuthenticationError`, `crates/warp_server_client/src/auth/mod.rs`):
```rust
#[derive(thiserror::Error, Debug)]
pub enum UserAuthenticationError {
    #[error("Firebase returned a token error when fetching an ID token")]
    DeniedAccessToken(FirebaseError),
    #[error("unexpected error occurred when fetching an ID token: {0:#}")]
    Unexpected(#[from] anyhow::Error),
    // …
}

impl ErrorExt for UserAuthenticationError {
    fn is_actionable(&self) -> bool {
        match self {
            UserAuthenticationError::DeniedAccessToken(_) => false,
            UserAuthenticationError::Unexpected(e) => e.is_actionable(),
            // …
        }
    }
}
register_error!(UserAuthenticationError);
```

## Import the macro

Use an imported, unqualified `report_error!` (not `crate::report_error!` / `warp_core::report_error!` / `warp_errors::report_error!`). Import once per file, per tier:
- `app` and its modules: `use crate::report_error;` (re-exported at `app/src/lib.rs`).
- `warp_core` itself: `use crate::report_error;`.
- Other `warp_core`-dependent crates: `use warp_core::report_error;`.
- The leaf crates that don't depend on `warp_core` (`warpui_core`, `warpui`, `warpui_extras`, `settings`, `command`, `sum_tree`, `asset_cache`, `voice_input`, `jsonrpc`, `watcher`, `input_classifier`, `computer_use`): `use warp_errors::report_error;`.

Add `report_if_error` to the same import when the file uses it. If every call site in a file is behind a `#[cfg(...)]`, gate the import the same way to avoid an unused-import warning. A fully-qualified path is acceptable only to appease macro hygiene inside another `macro_rules!` body (`$crate::report_error!`).

## Choosing the form (variable data out of the grouped message)

**Never stringify an already-typed error.** `report_error!(anyhow::anyhow!("{e}"))` flattens `e` to a `String`, which erases the typed source chain and defeats `is_actionable()` (so registered non-actionable network errors get over-reported). Reserve `anyhow!("…")` for values that are genuinely *not* errors (see rule 4).

Rules, in priority order:

1. **The error IS the payload — report it as an error, never demote it into `extra:`.** `extra:` is only for genuinely incidental data (ids, paths, counts, durations).
2. **Prefer Result-level `.context()` whenever a `Result` is in hand.** Works for any `Result<_, E: std::error::Error>` and for `anyhow::Result`; add `use anyhow::Context` for the trait method. Don't add an `anyhow::Error::new(..)` / `anyhow!(..)` wrapper when `.context()` on the `Result` will do. Avoid the UFCS form `anyhow::Context::context(result, "msg")` — it reads poorly; import the trait, or if you'd rather not import it, restructure to own the error and use the inherent method: `if let Err(e) = some_call() { report_error!(e.context("msg")); }`.
   ```rust
   let data = some_call().context("Failed to load data")?;
   ```
3. **Wrap a bare error value only when there is no `Result` to hang `.context()` on** (closure/callback params, `match` arms that special-case other variants):
   - `e` is a `std::error::Error` but not `anyhow`: `report_error!(anyhow::Error::new(e).context("msg"))`.
   - `e` is already an `anyhow::Error`: `report_error!(e.context("msg"))`.
   - `e` is a *registered* error (`register_error!`): pass it directly — `report_error!(e)` — which keeps it fully typed.
   - Error type differs by feature/config (sometimes `anyhow`, sometimes `StdError`): use the Result-level `.context()` above, or `report_error!(anyhow::Error::from(e).context("msg"))` which compiles for both (do NOT use `Error::from` where `e` is unconditionally `anyhow` — clippy flags it as a useless conversion; use `e.context()` there).
4. **Non-`Error` payload** — a `String`/`&str` message, a const, `format_args!`, or an opaque value that isn't `std::error::Error` — has no typed chain to preserve, so `anyhow::anyhow!("{value}")` (or `"static", extra: { .. }`) is correct here.
5. **The error must be reused or returned, so it can't be consumed** — but first treat this as a **smell**. The **sink** (where the error stops propagating) should normally be the one reporting, and it should *own* the error so it can move it into `report_error!` fully typed and with `.context()`. Needing to report a *borrowed* error usually means you're reporting away from the true sink, or reporting an error you also return (see "Report once, at the sink") — prefer restructuring so the owner reports. When you genuinely can't take ownership, report it **borrowed**, which still keeps it typed:
   - `e` is an `anyhow::Error` or a *registered* error → `report_error!(&e)` (optionally `report_error!(&e, extra: { .. })`). Note this drops any static `.context("…")` message, so `&e` groups by the error's own message.
   - If the later use is itself a **borrow** (building a `format!`/detail string, calling a `&e` method), reorder so that borrow runs first and then **move** `e` into the report last — `report_error!(anyhow::Error::new(e).context("msg"))` (StdError) or `report_error!(e.context("msg"))` (anyhow). This keeps the typed chain AND the static context.
   - `inspect_err(|e| report_error!(..))` only hands you `&e` (a borrow), which forces the stringified `anyhow!("{e}")` form. When you're reporting-and-swallowing (`.ok()` / `.ok()?`) or otherwise discarding the error, switch to `map_err(|e| report_error!(anyhow::Error::new(e).context("msg")))` so `e` is **owned** and stays typed — the closure returns `()`, which composes with a trailing `.ok()`/`?`. (Clippy's `manual_inspect` only fires when a `map_err` closure returns `e` unchanged; returning `()` is fine.)
   - Prefer to **make the error reportable while typed** before falling back to stringify: a Display-only enum or other unregistered concrete error should be upgraded to `#[derive(thiserror::Error)]` (or `register_error!`-ed) so you can `report_error!(anyhow::Error::new(e).context("msg"))` / `report_error!(&e)`. Only when that isn't feasible is `report_error!(anyhow::anyhow!("{e}").context("msg"))` unavoidable.
6. **Non-error bindings** (no error object — `let ... else`, `None =>` arms, count/id mismatches): static message + `extra:`.
7. If a crate lacks an `anyhow` dependency, either use the `extra:` form (needs no `anyhow` at the call site) or add `anyhow.workspace = true` — do not dump a real error into `extra:` just to avoid the dep.

## Report once, at the sink

Report a failure where it stops propagating, not at every layer it flows through. If a function returns or propagates the error (`?`, `return Err(..)`), don't also `report_error!` it there — whoever ultimately handles or swallows it reports it. Reporting in both the callee and the sink double-counts the same failure in Sentry.
- A callee that wants a local breadcrumb while still returning the error should use `log::warn!`/`log::error!`, and leave the `report_error!` to the sink.
- `inspect_err(|e| report_error!(..)).ok()` (report-and-swallow) is a legitimate terminal decision — the error is consumed there, not returned.
- For a registered error surfaced through many internal failure points, implement `is_actionable` on the type and `report_error!` it once at the top-level sink (e.g. a driver's `run`), instead of reporting at each internal `Err`.

## `extra:` syntax

Attach incidental data as a structured Sentry "details" context block. `%` forces `Display`, `?` forces `Debug`, a bare expr defaults to `Display`:
```rust
report_error!(
    "Could not find data for pane",
    extra: { "pane_id" => ?pane_id, "count" => %count }
);
```
Combine a real error with incidental data:
```rust
report_error!(
    anyhow::Error::new(e).context("Failed to write attachment"),
    extra: { "path" => %path.display() }
);
```

## Throttling with ReportErrorLogMode::OncePerRun

Sites that can fire repeatedly (hot loops, per-frame paths, enum-fallback conversions from GraphQL/protobuf) should report only once per app run so they don't flood Sentry. Default is `EveryTime`.
```rust
use warp_core::errors::ReportErrorLogMode; // leaf crates: use warp_errors::ReportErrorLogMode;

report_error!(err, ReportErrorLogMode::OncePerRun);
// with a static message + incidental data:
report_error!(
    "Invalid LlmProvider; update client GraphQL types",
    extra: { "provider" => %value },
    ReportErrorLogMode::OncePerRun
);
```

## No secrets or PII

This applies to `report_error!` messages/`extra:` AND to `log::*` at Info and above (both are uploaded to Sentry — the report as an event, the log as a breadcrumb). Never place secrets, tokens, credentials, or user-generated content (file contents, prompts, command text, personal data) in any of them — Sentry retains everything sent. Limit reported/logged data to non-sensitive diagnostics: ids, paths, counts, durations, and error types. When the useful detail is sensitive but helpful locally, use the `safe_*` macros (see "Sensitive data" above) so it only appears in dogfood logs.

## Best practices

- Static, descriptive grouping message; variable data via `.context()` or `extra:`.
- When the grouping message is static, put the inputs that explain *why this instance fired* in `extra:` — the offending values, not just identifiers (e.g. an invalid-geometry report carries the sizes/offsets that produced it; a bounds violation carries the actual `min`/`max`). A static message with no diagnostic `extra:` is hard to act on.
- Preserve the typed error chain (`.context()` / `anyhow::Error::new`) so `is_actionable()` can suppress registered non-actionable (network) errors — stringifying with `anyhow!("{e}")` defeats this.
- Prefer a typed, registered error enum (`thiserror` + `ErrorExt` + `register_error!`) over `anyhow` when a caller or the Sentry layer needs to tell failures apart (branching, or mixed actionability); reserve `anyhow` for errors that are only propagated and reported (see "Prefer typed error enums over `anyhow` when it makes sense").
- Match log level to volume and audience: hot paths at `debug!`/`trace!`, milestones at `info!`, and reserve `report_error!` for Sentry-worthy failures.

## Anti-patterns

```rust
// Interpolates variable data into the grouped message.
report_error!("Failed for user {user_id}: {e}");

// Stringifies an owned, typed error — erases the source chain and defeats
// is_actionable(). Use anyhow::Error::new(e).context("msg") instead.
report_error!(anyhow::anyhow!("{e:#}").context("msg"));

// Demotes a real, typed error into extra: (loses is_actionable classification).
report_error!("Request failed", extra: { "error" => %e });

// UFCS .context() reads poorly — import anyhow::Context, or `if let Err(e)` and
// report e.context("msg").
report_error!(anyhow::Context::context(some_call(), "msg").unwrap_err());

// Redundant wrapper when a Result is in hand — use .context() on the Result.
report_error!(anyhow::Error::new(some_call().unwrap_err()).context("msg"));

// log::error! for a Sentry-worthy failure — this is only a breadcrumb, not an
// issue. Use report_error! if it should be tracked in Sentry.
log::error!("Failed to sync: {e:#}");

// Sensitive detail logged unconditionally — ships to release-channel logs and
// breadcrumbs. Use safe_error!(safe: (..), full: (..)) instead.
log::warn!("Bad response body={body:?}");

// info! (or higher) in a hot/per-frame path — floods logs and breadcrumbs.
// Use debug!/trace! for high-frequency diagnostics.
log::info!("rendered frame {n}");
```
