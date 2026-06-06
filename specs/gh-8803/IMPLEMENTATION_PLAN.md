# gh-8803: Implementation Conformance Plan

> Companion to [product.md](./product.md) and [tech.md](./tech.md). This document records the gap between the **current implementation** (branch `brad/8803-lsp-plugins`, commit `dde9375e`) and the **revised authoritative spec** (merged from `brad/8803-spec`, nine review rounds), and lays out a phased plan to close that gap.
>
> Status: **plan only — no remediation code written yet.** Each phase pauses for review.

## Context

The implementation was written against an **earlier draft** of the spec. The spec was then revised across nine review rounds on a separate branch and merged in. The implementation now lags the spec in several places, and an exhaustive per-invariant audit (not just a spec diff) surfaced one outright functional bug plus a set of spec-conformance gaps.

### How these gaps were found

A spec-diff pass (old spec copy vs. revised spec) finds only what *changed between drafts*. That structurally misses (a) requirements present in the spec all along but never implemented, and (b) `tech.md` instructions that were never carried out. To reach 100% coverage, every product invariant (1–32) was audited against the actual code with file:line evidence, and the two highest-impact findings were verified by hand against source. The P0 bug below is exactly the kind a diff cannot catch — it is a `tech.md` Phase-3 instruction that wasn't implemented, with no corresponding product-text change.

## Decisions required before Phase 5

These two are genuine forks that affect scope. They are the spec owner's call.

### Decision A — triple-brace escape (gap 13)

Revised **invariant 6** states *"There is no in-Warp escape for the recognized placeholder set."* However, the shared `crates/handlebars` engine **does** implement a `{{{name}}}` → verbatim-passthrough escape. It is tested (`crates/handlebars/src/lib_tests.rs::preserves_escaped_triple_braces`, asserting `"{{{name}}} {{name}}"` → `"{{{name}}} Warp"`) and is shared with tab-config and MCP rendering. The escape genuinely exists and cannot be removed from the LSP path without either special-casing `placeholder::expand()` or changing shared-engine behavior used by other features.

Options:
- **(a) Revise invariant 6 to re-acknowledge the escape.** Cheapest; matches reality. *Recommended* — "no escape" is the less accurate statement, and the behavior is real, tested, shared, and harmless.
- **(b) Defeat the escape only in the LSP `expand()` path.** Pre/post-process around the engine. Added complexity for a narrow benefit.
- **(c) Fix only the stale doc-comment**, accept the minor spec contradiction.

### Decision B — failure message includes `name` (gap 12)

Product **invariant 18** ("the footer error path shows the server's `name`") is **already satisfied** — the footer status line prefixes the descriptor `name` (`app/src/code/footer.rs:1582`). But `tech.md:213` and the Risks section want the descriptor `name` in the failure *string/toast* too, for attributability when multiple custom servers exist. The raw `LspState::Failed { error }` (`crates/lsp/src/transport.rs:41`) and the toast (`app/src/ai/persisted_workspace.rs:1359`) omit the name.

Options: include the small fix (wrap the spawn error / toast with `name`), or treat footer-only as sufficient.

## Verified gap list

Severity: 🔴 P0 = functional bug (feature broken); 🟠 P1 = spec-mandated correctness/privacy; 🟡 P2 = spec-mandated, smaller blast radius; ⚪ = decision-gated.

| # | Gap | Sev | Evidence (file:line) |
|---|---|---|---|
| 1 | **`language_id` not threaded to `did_open`** → custom files for any non-built-in filetype get no `didOpen`, so no diagnostics/hover/completions | 🔴 P0 | `app/src/code/lsp_dispatch.rs:67-72` (drops `matched.language_id`); `crates/lsp/src/service.rs:518` (hard-codes `LanguageId::from_path`); `crates/lsp/src/config.rs:43-58` (no `.rb`/`.zig`/… arm) |
| 2 | **Log redaction absent** (inv. 32): no `LogRedactor` trait anywhere; spawn-log emits substituted `command`/`args` verbatim | 🟠 P1 | `crates/lsp/src/config.rs:362-367` |
| 3 | **Error `Display` echoes raw user values** to logs (inv. 32): glob pattern, serde error, command path all logged via `{err}` | 🟠 P1 | `crates/lsp/src/descriptor/validate.rs:44-74`; `app/src/settings/language_servers.rs:42` |
| 4 | **`name` constraints** (1–64 chars, `[A-Za-z0-9._-]`, not `.`/`..`, no leading `.`/`-`) not validated (only non-empty) | 🟠 P1 | `crates/lsp/src/descriptor/parse.rs:85-94` |
| 5 | **Reserved-name check absent** — must reserve the five built-in binary names, case-insensitive, sourced from `LSPServerType::binary_name()` | 🟠 P1 | no reference in `crates/lsp/src/descriptor/validate.rs`; primitives exist at `crates/lsp/src/supported_servers.rs:39,129,212` |
| 6 | **Uniqueness is case-sensitive**; spec requires case-insensitive (`ruby-lsp` == `Ruby-LSP`) | 🟠 P1 | `crates/lsp/src/descriptor/parse.rs:44` (`HashSet<String>` exact key) |
| 7 | **`command` trust boundary not validated** (must be absolute-or-bare-name after `~` expansion); no `UnsafeCommandPath`; `expand_home_prefix` helper absent | 🟠 P1 | `crates/lsp/src/descriptor/parse.rs:96-105`; helper missing in `crates/warp_util/src/path.rs` |
| 8 | **`{{cache_dir}}` joins raw `name`**, not a SHA-256-prefix hash; returns `Option` (spec: `PathBuf`) | 🟡 P2 | `crates/warp_core/src/paths.rs:193-204`; stale tests `crates/warp_core/src/paths_tests.rs:134-149`; caller `app/src/ai/persisted_workspace.rs:1284` |
| 9 | **Unknown fields silently dropped, no warning** (inv. 24 promises "ignored with a warning logged") | 🟡 P2 | `crates/lsp/src/descriptor/parse.rs:63-67` (serde default) |
| 10 | **Undefined `{{env_VAR}}` not logged** (inv. 5: "expands to the empty string and is logged") | 🟡 P2 | `crates/lsp/src/descriptor/placeholder.rs:120-123` |
| 11 | **JSON-schema field doc-comments missing** placeholder enumeration + `~` expansion (inv. 25) | 🟡 P2 | `crates/lsp/src/descriptor.rs:18-25` (no `///` on `name`/`command`/`args`/`filetypes`/`env`) |
| 12 | **Failure message omits `name`** in toast/raw error (footer shows it) | 🟡 P2 / ⚪B | `crates/lsp/src/transport.rs:41`; `app/src/ai/persisted_workspace.rs:1359` |
| 13 | **Triple-brace escape** — stale doc-comment **and** shared engine still passes `{{{…}}}` through verbatim, contradicting revised inv. 6 | ⚪A | `crates/lsp/src/descriptor/placeholder.rs:54-55`; `crates/handlebars/src/parser.rs:212-236,248`, `lib.rs:73` |

### Error-kind coverage (gap 4/5/7 detail)

`LspDescriptorErrorKind` (`crates/lsp/src/descriptor/validate.rs:17-42`) currently has: `DuplicateName`, `EmptyFiletypes`, `MissingName`, `MissingCommand`, `MalformedEntry`, `InvalidGlob`, `UnsupportedGlobFeature`. Against invariant 23's enumerated error classes:

- ✅ duplicate name (but key is case-sensitive — gap 6), empty filetypes, missing name/command, pattern fails glob compile.
- ⚠️ "filetypes entry missing `pattern`" is folded into `MalformedEntry` (behavior correct; taxonomy coarser, and the `MalformedEntry { reason }` raw serde string is a redaction concern — gap 3).
- ❌ **missing**: name-constraint violation (gap 4), reserved-name match (gap 5), command-trust-boundary violation (gap 7).

## Verified PASS — no work needed

Inline-table-only `filetypes` (bare strings rejected via serde); glob vs. literal classification with `**`/`{a,b}` rejection; `language_id` *computation* in the matcher; first-in-source-order; all-or-nothing `from_file_value` returning a single bare key; once-per-workspace dedup via `ServerKey::Custom`; `initialize` sends `workspaceFolders` + substituted `initialization_options`; `expand_json` substitutes string leaves only; single-pass substitution; unknown-placeholder verbatim + warn-once; whitespace-in-braces invalidates; leading-`~`-only expansion; no install/PATH-fallback for customs; no auto-restart on settings edit; custom-first file-open dispatch; footer parity (`Enable {name}`); enable/decline persistence; `kind` column migration + partitioned loader; schema flow-through via `define_settings_group!` + `inventory`. No over-implementation found (the code is uniformly *behind* the spec, except gap 13 which lives in the shared engine).

## Phased plan

Each phase compiles independently, writes unit/integration tests alongside the code, and **pauses for review** before the next phase begins.

### Phase 1 — `language_id` propagation (the P0 bug) — gap 1

The actual broken-feature fix; highest user value, so sequenced first.

- Add a `language_id: String` parameter to `TextDocumentService::did_open` (`crates/lsp/src/service.rs:485`) and use it in place of the `LanguageId::from_path` block (`service.rs:518-526`).
- Thread it through `LspServerModel::did_open_document` (`crates/lsp/src/model.rs:537`) and `spawn_lsp_service` as needed.
- Carry `matched.language_id` out of `resolve_server_for_path` (`app/src/code/lsp_dispatch.rs:66-73`) — add a field to `ResolvedLspServer::Custom` (or return the id alongside). For built-ins, the caller supplies `LanguageId::from_path(path)?.lsp_language_identifier().to_owned()`, matching `tech.md:192`.
- Update the open call site (`app/src/code/global_buffer_model.rs:~1447`).
- **Tests:** integration — open `.rb` against a mock custom server, assert `textDocument/didOpen` fires with the resolved `languageId`; assert a `{ pattern = "*.ts", language_id = "typescriptreact" }` override sends `"typescriptreact"`, not `"typescript"`. Regression — built-ins still send their existing ids.

### Phase 2 — Descriptor validation — gaps 4, 5, 6, 7

All in `crates/lsp/src/descriptor/` plus one helper.

- Add `pub fn expand_home_prefix(...)` to `crates/warp_util/src/path.rs` (expand leading `~`/`~/` only; embedded `~` untouched; `~someuser` unsupported). Used by command validation.
- New `LspDescriptorErrorKind` variants: `InvalidName { reason }`, `ReservedName`, `UnsafeCommandPath { command, reason }`.
- **Name validation**: length 1–64; charset `[A-Za-z0-9._-]`; not `.`/`..`; no leading `.`/`-`.
- **Reserved-name check**: build the set once from `LSPServerType::all().map(|t| t.binary_name())`, compare case-insensitively (ASCII fold). Report the original-cased name.
- **Case-insensitive uniqueness**: key the dedup set on `name.to_ascii_lowercase()` (`parse.rs:44`), preserving first-in-source-order winner and original-cased error.
- **Command trust boundary**: after `expand_home_prefix`, accept if absolute (Unix leading `/`; Windows drive-letter `C:\`/`C:/` or UNC `\\`/`//`) **or** contains no `/` or `\`; reject otherwise (`./server`, `bin/server`, `..\server`, Windows `\path`).
- **Tests:** valid + rejection cases for each rule, including `ruby-lsp`/`Ruby-LSP` duplicate, `RUST-ANALYZER` reserved, `a b` invalid charset, `./server` unsafe, `~/bin/server` accepted after expansion.

### Phase 3 — Log redaction — gaps 2, 3

The privacy-critical phase; crosses the `lsp ↔ app` boundary.

- Define `pub trait LogRedactor: Send + Sync { fn redact_for_log<'a>(&self, value: &'a str) -> Cow<'a, str>; }` in `crates/lsp`.
- Carry `Arc<dyn LogRedactor>` on `LspPlaceholderContext` (or `CustomLspServerConfig`), populated by app-side wiring backed by `app/src/settings/privacy.rs::CustomSecretRegex`.
- At the spawn-log site (`crates/lsp/src/config.rs:362-367`), redact `resolved_command` and each `resolved_args` element before `log::info!`. Never log `initialization_options` verbatim (emit a structural summary, e.g. key count). `env` values: redact if ever logged (today they are not). Keep `name`/`workspace_root`/`workspace_slug`/`cache_dir` verbatim per invariant 32.
- Split `LspDescriptorError`: a redaction-safe `log_summary()` (entry `name` or `"anonymous"` + variant name + structural detail, no raw user values) used by `log::warn!` at `app/src/settings/language_servers.rs:42` and any parse/validate log line; reserve the value-bearing `Display` for UI/tests. This also removes the raw-serde-`reason` and raw-`pattern` leaks from `MalformedEntry`/`InvalidGlob`.
- **Tests:** redactor injected with a secret-shaped value → log output redacted; `log_summary` emits no raw user value; verbatim-OK fields still appear.

### Phase 4 — cache_dir hashing — gap 8

- `crates/warp_core/src/paths.rs:193` — change `lsp_server_cache_dir` to join `warp_util::path::workspace_hash(...)` of the name (16-hex SHA-256 prefix) and return `PathBuf` (drop `Option`; hashing makes every name a safe segment).
- Update the caller `app/src/ai/persisted_workspace.rs:1284` — remove the `let Some(...) else { skip }` guard and the now-dead "unsafe name" warn.
- Rewrite `crates/warp_core/src/paths_tests.rs:134-149` to assert the hashed segment; delete `lsp_server_cache_dir_rejects_unsafe_names`.

### Phase 5 — Smaller conformance fixes — gaps 9, 10, 11, 12, 13

- **Unknown-field warning (gap 9)**: walk the entry's top-level keys; `log::warn!` one line per key not in `{name, command, args, filetypes, env, initialization_options}` (single-sourced from `RawDescriptor` fields per `tech.md:84`). Keep serde's ignore behavior (no `deny_unknown_fields`).
- **Undefined-env-var log (gap 10)**: in the `env_` arm of `resolve_placeholder` (`placeholder.rs:120-123`), emit a deduped `log::warn!` when `std::env::var` returns `Err` (defined-but-empty must not warn).
- **Schema field docs (gap 11)**: add `///` doc comments to `command`/`args`/`env` (and expand `initialization_options`) in `crates/lsp/src/descriptor.rs` enumerating `{{workspace_root}}`, `{{workspace_slug}}`, `{{cache_dir}}`, `{{env_VAR}}` and the leading-`~`/`~/` expansion (these become the JSON-schema `description` per invariant 25).
- **Failure-message name (gap 12)** — *pending Decision B*: attach the descriptor `name` to the spawn-failure string and the toast (`app/src/ai/persisted_workspace.rs:1359`).
- **Triple-brace (gap 13)** — *pending Decision A*: at minimum, correct the stale doc-comment at `crates/lsp/src/descriptor/placeholder.rs:54-55`; broader handling per Decision A.

### Phase 6 — Test plan & docs refresh

- Update `specs/gh-8803/MANUAL_TEST_PLAN.md` with steps for the new invariants: redaction at launch logging, name constraints, reserved names, case-insensitive uniqueness, command trust boundary, cache-dir hashing, and the `language_id` fix (open a `.rb` file and confirm diagnostics actually appear).

## Sequencing

Phase 1 first (it fixes the broken feature). Phases 2–4 are independent and can land in any order; 3 is the largest. Phase 5 depends on Decisions A and B. Phase 6 last.

The spec-directory cleanup (deleted old `specs/GH8803/` copies, moved `MANUAL_TEST_PLAN.md`) is staged in the working tree but not yet committed.
