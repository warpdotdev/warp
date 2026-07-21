*Spec: downgrade LLM cache write failure to warn (APP-4923)*

== PRODUCT ==
*Summary:* When Warp refreshes its available-LLM list from the server, it best-effort caches the result to the platform-native private preferences store (Windows registry / macOS UserDefaults / Linux file). Today a cache *write* failure is reported with `report_error!`, which escalates to Sentry as an actionable issue. This is non-critical — the freshly fetched models are already in memory and the app keeps working — so it should be a local warning, not a Sentry event. The Sentry issue (`WARP-CLIENT-BETA-STABLE-NBF`) has ~395K occurrences / ~1,057 users in 90 days and is cross-platform (36K Linux file-backed + 26K Windows registry events), so it is pure noise.

*Key design choices:*
- Downgrade only the cache *write* (persistence) failure from `report_error!` to a once-per-run `log::warn!` carrying the platform/backend tag, the serialized byte length, and the full typed error chain (which includes the underlying OS error / Windows HRESULT). This stops the Sentry noise while preserving local/breadcrumb diagnostics for future root-cause work.
- Keep the cache write attempt as-is (best-effort); do NOT change write semantics, add a file-backed fallback, or alter any other user-preference writes. The in-memory model update and `UpdatedAvailableLLMs` event already happen unconditionally around the cache write, so no resilience change is needed.
- Leave the separate "Failed to serialize LLMs for cache" path as `report_error!`: a serialization failure of `ModelsByFeature` is a rare invariant violation (our types should always serialize) and remains Sentry-worthy. It is a different issue from the i/o write noise and is out of scope for this fix.

*Behavior* (numbered, testable invariants from the user's/consumer's view):
1. A successful model refresh always updates the in-memory `models_by_feature` and emits `LLMPreferencesEvent::UpdatedAvailableLLMs`, regardless of whether the cache write succeeds or fails. (Already true today; this fix must not regress it.)
2. A cache *write* failure (registry/file/UserDefaults I/O error) does NOT create a Sentry issue — it is logged once per app run at `warn` level with backend/platform + byte-length + OS-error context, and never logs the serialized model payload or other sensitive data.
3. A cache *read* (`get_cached_models`) and a successful cache *write* behave exactly as before on every platform (Windows, macOS, Linux, WASM, tests).
4. The separate serialization-failure path still reports via `report_error!` (unchanged) — only the persistence path is downgraded.
5. The downgrade is local to the LLM cache path; no other `report_error!` call site or user-preference write changes.

== TECH ==
*Context:*
- `app/src/ai/llms.rs:1703-1752 @ 7d2304d` — `LLMPreferences::on_server_update`: replaces `models_by_feature` in memory (line 1706), serializes, calls `ctx.private_user_preferences().write_value(MODELS_BY_FEATURE_CACHE_KEY, serialized_update)` (lines 1712-1715), and on `Err` does `report_error!(e)` (line 1717) — the noisy line. The event is emitted unconditionally at line 1751.
- `app/src/ai/llms.rs:1685-1701 @ 7d2304d` — `refresh_available_models` / `update_feature_model_choices` feed `Ok(models)` into `on_server_update` (the test entry point).
- `app/src/ai/llms.rs:172 @ 7d2304d` — `MODELS_BY_FEATURE_CACHE_KEY = "AvailableLLMs"`.
- `app/src/ai/llms.rs:1976-2002 @ 7d2304d` — `get_cached_models` reads via `read_value(...).ok().flatten()` (swallows read errors → `None`).
- `crates/warpui_extras/src/user_preferences/mod.rs:18-132 @ 7d2304d` — `UserPreferences` trait + `Error` enum (`IoError(#[from] std::io::Error)` whose Display is `"i/o error"`; the OS detail lives in the error *source chain*, not Display).
- `crates/warpui_extras/src/user_preferences/registry_backed.rs:43-47 @ 7d2304d` — Windows `write_value` (`set_string`; error → `Error::IoError(io::Error::from(HRESULT))`).
- `crates/warpui_extras/src/user_preferences/file_backed.rs:68-95 @ 7d2304d` — Linux `write_value` → `flush` → `std::fs::write`.
- `crates/warpui_extras/src/user_preferences/in_memory.rs @ 7d2304d` — never-failing backend used in tests today.
- `crates/warp_core/src/user_preferences.rs:19-31 @ 7d2304d` — `GetUserPreferences::private_user_preferences()` returns `&dyn UserPreferences` via the `PrivatePreferences` singleton.
- `app/src/settings/init.rs:282-315, 455-463 @ 7d2304d` — `init_platform_native_preferences` / `init_private_user_preferences`; the test seam `init_and_register_user_preferences` registers `PrivatePreferences::new(InMemoryPreferences)`.
- `app/src/test_util/settings.rs:5-129 @ 7d2304d` — `initialize_settings_for_tests` (calls the seam above).
- `app/src/settings/init_tests.rs:48-59 @ 7d2304d` — `init_test_app` shows the manual `PrivatePreferences::new(backend)` registration pattern.
- `app/src/ai/llms_tests.rs:506-588, 834-853 @ 7d2304d` — existing `App::test` scaffold + `server_llm`/`available`/`agent_llm` fixtures used to drive `update_feature_model_choices`.
- `crates/warp_errors/src/lib.rs:49-182 @ 7d2304d` — `report_error!`: actionable → `err.report_error()` (Sentry capture) **and** `log::log!(target: "errors::report_error", Level::Error, "{:#}", err)`; `ReportErrorLogMode::OncePerRun` via a per-callsite `AtomicBool`. `anyhow::Error`'s `{:#}` renders the full source chain (so the OS error/HRESULT is visible).
- `crates/warp_errors/src/errors_tests.rs:14-54 @ 7d2304d` — capturing `TestLogger` pattern (`log::set_logger` + `OnceLock<Mutex<Vec<LogEntry>>>` + `logged_report_count` filtering `target == "errors::report_error"` && `level == Error`).
- `app/.agents/skills/logging-and-error-reporting/SKILL.md` — `log::warn!` is breadcrumb-only (never a Sentry issue); the right level for "non-ideal but largely expected / skipped work / fallback."

*Design alternatives:*
- *How to stop the Sentry noise:*
  - (Chosen) Downgrade the cache-write failure to `log::warn!` with a once-per-run guard. Pros: local, minimal, matches the logging skill's definition of `warn` (skipped best-effort work); preserves diagnostics in breadcrumbs/logs. Cons: loses the Sentry event (intended — it was noise).
  - Register `user_preferences::Error` with `is_actionable() -> false` and keep `report_error!`. Rejected: that would suppress Sentry for *all* user-preference write failures app-wide (every setting write), not just the LLM cache — far too broad, and it would hide genuinely-actionable preference-write bugs elsewhere.
  - Keep `report_error!` with `ReportErrorLogMode::OncePerRun`. Rejected: `OncePerRun` only throttles the *log/capture* frequency; for an actionable error it still captures to Sentry (once per run per client) — still ~1K users × many runs of noise, and the issue stays "unresolved" in Sentry. The goal is no Sentry event, not a throttled one.
- *How to preserve the OS error detail:* `user_preferences::Error::IoError`'s Display is just `"i/o error"` (no `{0}`), so a plain `{e}` / `{e:#}` on the raw `thiserror` value loses the OS message. (Chosen) wrap with `anyhow::Error::from(e).context(...)` and render `{e:#}` so the source chain (`i/o error: <HRESULT / io::ErrorKind message>`) is visible. Alternative: walk `Error::source()` manually — rejected as ugly/unnecessary given `anyhow` is already in scope in `llms.rs`.
- *Rate limiting:* (Chosen) a local `static CACHE_WRITE_FAILURE_WARNED: AtomicBool` swapped once per process, mirroring `report_error!`'s `OncePerRun` semantics, so a persistently-failing cache logs one warning per app run. Alternative: warn every refresh — rejected because `on_server_update` can fire repeatedly (login, reconnect, periodic refresh) and a sticky failure would spam breadcrumbs.
- *Scope of the serialize-failure path:* (Chosen) leave `report_error!` for `"Failed to serialize LLMs for cache"`. Rationale: a `ModelsByFeature` that can't serialize is a rare invariant violation / malformed-server-payload signal — `report_error!`-worthy per the logging skill — and is a *different* Sentry issue than the i/o write noise. Alternative: downgrade it too — rejected as it would silence a genuinely-actionable bug signal.

*Proposed changes:*
- In `app/src/ai/llms.rs`, replace the cache-write failure handling in `on_server_update` (lines ~1712-1718) so that on `Err` it emits a once-per-run `log::warn!` instead of `report_error!(e)`:
  - Keep a `.context(...)` (or wrap via `anyhow::Error::from(e).context(...)`) so `{e:#}` renders the OS error/HRESULT from the source chain.
  - Include a static platform/backend tag (e.g. a small `cfg!`-based `private_preferences_backend_label()` returning `"Windows registry"` / `"macOS UserDefaults"` / `"Linux file"` / `"local storage"` / `"test"`) and the serialized byte length (`serialized_update.len()`).
  - Exclude the serialized payload and any model content from the message.
  - Gate with a local `static CACHE_WRITE_FAILURE_WARNED: AtomicBool` (`swap(true, Relaxed)`) so it warns at most once per process.
  - Use inline format args (`log::warn!("... {e:#}")`) per the workspace clippy config.
- Add the `AtomicBool` static (and the `cfg!`-based label helper, if introduced) near `on_server_update` / `MODELS_BY_FEATURE_CACHE_KEY` in `llms.rs`.
- Do NOT touch: the serialization-failure `report_error!` (lines ~1720-1722), `get_cached_models`, any backend (`registry_backed` / `file_backed` / `user_defaults` / `in_memory`), or any other `report_error!` site.
- Imports: `log::warn!` and `anyhow::Context` are already in scope in `llms.rs`; `std::sync::atomic::{AtomicBool, Ordering}` may need importing (place at top of file per repo convention).

*Test approach (regression + characterization):*
- Add a test-only `UserPreferences` backend in `app/src/ai/llms_tests.rs` (e.g. `FailingWritePreferences`) whose `write_value` returns `Err(user_preferences::Error::IoError(io::Error::new(io::ErrorKind::Other, "test cache write failure")))` and whose `read_value` returns `Ok(None)`.
- Register it as the `PrivatePreferences` singleton via the cleanest available seam — either a new `initialize_settings_for_tests_with_private_preferences(app, backend)` helper in `app/src/test_util/settings.rs` (mirroring `initialize_settings_for_tests_with_mode` but passing the custom private backend into an `init_and_register_user_preferences_with_private` variant), or the manual `PrivatePreferences::new(backend)` registration pattern from `app/src/settings/init_tests.rs:init_test_app` — combined with the existing singleton set used by `active_models_fall_back_to_usable_choice...` (`ServerApiProvider`, `AuthStateProvider`, `AuthManager`, `NetworkStatus`, `UserWorkspaces`, `CloudModel`, `TeamTesterStatus`, `SyncQueue`, `UpdateManager`, `TemplatableMCPServerManager`, `AIExecutionProfilesModel`, `LLMPreferences`).
- Install a capturing test logger (the `crates/warp_errors/src/errors_tests.rs` `TestLogger` pattern: `log::set_logger` ignoring the already-set error, `set_max_level(Trace)`, a shared `OnceLock<Mutex<Vec<LogEntry>>>` cleared before the assertion). Filter assertions by `target == "errors::report_error"` AND `level == Level::Error` AND message contains `"Failed to cache LLMs"` so concurrent tests' unrelated `report_error!` calls don't pollute.
- Drive the failing-write path: `llm_preferences.update(&mut app, |p, ctx| p.update_feature_model_choices(Ok(models), ctx))`.
- Assert: (a) `models_by_feature` equals the new models (resilience — passes before & after, locks in non-critical behavior); (b) `LLMPreferencesEvent::UpdatedAvailableLLMs` is emitted (resilience); (c) **zero** `Level::Error` log entries at `target == "errors::report_error"` whose message contains `"Failed to cache LLMs"` (the Sentry-downgrade — **fails before** the fix because `report_error!` fires an Error-level `errors::report_error` log, **passes after** because only `log::warn!` fires at a different target/level).
- The once-per-run guard is verified by code review (the `AtomicBool` swap) rather than a separate test, since the critical assertion (c) already proves the `report_error!` is gone.

*Open questions resolved:*
- Is the dominant cause registry value size, EDR/locking, or file permissions? — Not determinable without a Windows/native repro (the Linux sandbox cannot run the Windows registry path, and Cargo could not bootstrap in the prior triage environment). The Sentry data is cross-platform (36K Linux + 26K Windows events), which *weakens* a Windows-registry-size-only theory. This fix deliberately does **not** attempt a root-cause repair (no file-backed fallback, no write-semantics change); instead it adds the byte-length + OS-error diagnostics that a future root-cause investigation needs. Root-cause is explicitly **out of scope** and left as a follow-up.
- Should the serialize-failure path also be downgraded? — No (see Design alternatives): it stays `report_error!`.
- Should `user_preferences::Error` be registered as non-actionable? — No (see Design alternatives): too broad.
- Is this user-facing (needs computer-use screenshot proof)? — No. The change affects only Sentry/log output; no UI renders differently. Per `factory-verification`, a headless/logging change needs the code-level regression test + `./script/presubmit`, not computer-use.

*Risks / blast radius:*
- **Low.** The change is one branch in one function (`on_server_update`); the in-memory update and event emission are untouched. Worst case: a real, actionable cache-write regression goes unreported to Sentry. Mitigation: it still logs at `warn` (visible in breadcrumbs/local logs) with the OS error, and the separate serialize-failure path still escalates; if a future root-cause is found, the reporting can be revisited.
- The once-per-run guard could hide a *change* in failure mode after the first occurrence. Mitigation: acceptable for a non-critical cache; the first warning carries the diagnostic.
- Test-logger global-state flakiness: mitigated by filtering on the specific message substring (`"Failed to cache LLMs"`) rather than target/level alone.

*Validation & verification criteria* (must ALL pass before merge):
1. **Sentry noise eliminated (code-level):** `app/src/ai/llms.rs` `on_server_update` cache-write failure branch uses `log::warn!` (not `report_error!`); `grep -n "report_error" app/src/ai/llms.rs` shows no `report_error!` on the `write_value` / `"Failed to cache LLMs"` path (the separate `"Failed to serialize LLMs for cache"` `report_error!` remains). Checked by reading the diff + the grep.
2. **Diagnostics preserved:** the `warn` message renders the full error chain including the underlying OS error/HRESULT (via `{e:#}` on an anyhow-wrapped error), includes a static platform/backend tag and `serialized_update.len()`, and contains no model payload. Checked by reading the diff.
3. **Once-per-run:** a `static ... : AtomicBool` swap gates the `log::warn!` so it fires at most once per process. Checked by reading the diff.
4. **Regression test (fail-before / pass-after):** a new test in `app/src/ai/llms_tests.rs` (e.g. `on_server_update_cache_write_failure_does_not_report_to_sentry_and_keeps_in_memory_update`) injects a `UserPreferences` backend whose `write_value` returns `Err`, drives `update_feature_model_choices(Ok(models))` under a capturing `TestLogger`, and asserts **zero** `Level::Error` logs at `target == "errors::report_error"` containing `"Failed to cache LLMs"`. This fails before the fix (`report_error!` fires) and passes after. Checked by running the `warp` crate unit tests via `./script/presubmit` (the test lives in the inline `#[cfg(test)]` module `app/src/ai/llms_tests.rs`); confirm it fails on `master` and passes on the branch.
5. **Characterization (resilience) assertion:** the same test (or a companion) asserts `models_by_feature` equals the new models and `LLMPreferencesEvent::UpdatedAvailableLLMs` is emitted despite the write failure. Checked by the test.
6. **No collateral damage:** successful cache writes and cache reads still work — covered by the existing `llms_tests.rs` tests that call `update_feature_model_choices` with the default `InMemoryPreferences` (e.g. `active_models_use_default_when_usable`, `reconcile_preserves_*`) continuing to pass. Checked by `./script/presubmit`.
7. **Lint/format/build:** `./script/presubmit` passes (fmt, clippy with `-D warnings`, the affected tests, build). Inline format args used in the `log::warn!`. Checked by `./script/presubmit` from the repo root.
8. **Scope guard:** no changes to `registry_backed.rs`, `file_backed.rs`, `user_defaults.rs`, `in_memory.rs`, `get_cached_models`, or the serialize-failure `report_error!`. Checked by reading the diff (`git --no-pager diff --stat` + the changed hunks).
