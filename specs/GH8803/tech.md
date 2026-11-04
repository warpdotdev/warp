# Tech Spec: User-configurable language servers

**Issue:** [warpdotdev/warp#8803](https://github.com/warpdotdev/warp/issues/8803)

## Context

Warp's LSP layer currently treats every language server as a closed-set enum variant on `LSPServerType` (`crates/lsp/src/supported_servers.rs`). Each variant has:

- A `LanguageServerCandidate` impl in `crates/lsp/src/servers/<name>.rs` (rust_analyzer, gopls, pyright, intelephense, etc.).
- An entry in `LanguageId` (`crates/lsp/src/config.rs:25-36`) that maps file extensions to language identifiers.
- A `LanguageId::lsp_language_identifier` arm and a `LanguageId::from_path` arm.

Adding a new language requires touching all four sites. PRs #9562 (PHP Intelephense) and #9568 (JSON via vscode-json-languageserver) demonstrated this pattern but were closed by maintainer @kevinyang372 in favor of this user-configurable path. The infrastructure those PRs built (probe-spawn install detection, executable-bit checks, cross-platform PATH handling, bounded-future timeout via `warpui::r#async::FutureExt::with_timeout`) is reusable here.

### Relevant code

| Path | Role |
|---|---|
| `crates/lsp/src/language_server_candidate.rs` | The trait every server impl satisfies. The natural extension point — a new impl, `UserConfiguredLanguageServer`, will go here. |
| `crates/lsp/src/supported_servers.rs` | `LSPServerType` enum + the closed registry of impls. Will grow a new arm carrying user config. |
| `crates/lsp/src/config.rs` | `LanguageId` enum + `from_path` extension mapping + `lsp_language_identifier`. Bypassed for user-configured servers (the user supplies `language_id` directly). |
| `crates/lsp/src/manager.rs` | Spawns/owns running LSP processes. New per-workspace lifecycle logic lives here. |
| `app/src/settings/` | Settings group definitions (see `app/src/settings/input.rs` for the macro pattern). Where `[lsp.servers]` parsing lands. |
| `app/src/code/editor/` | Editor footer rendering — where the "Enable `<name>` for this workspace?" chip surfaces. |
| `app/src/settings/code/` (or current Settings → Code → Indexing/Projects host) | Where the workspace-enablement toggle row is rendered alongside the existing per-workspace project list. |

### Related closed PRs (input to this spec)

- #9562 — PHP Intelephense as built-in. Closed; lessons: probe-spawn `--stdio` with bounded timeout, executable-bit check on Unix, executable's full PATH search via `binary_in_path` helper, cross-platform tests via `std::env::join_paths`.
- #9568 — JSON via vscode-json-languageserver. Closed; lessons: schema-fetching is a security surface; this is documented in user-facing docs but the V0 spec defers `initializationOptions` forwarding.

## Crate boundaries

The user-config type is shared between the `app/` settings layer (which parses TOML into the type) and the `crates/lsp` layer (which constructs `LanguageServerCandidate` implementations from it). `crates/lsp` cannot depend on `app/` (the dependency direction is `app → crates/lsp`, not the reverse).

**Resolution: define the type in `crates/lsp/src/user_config.rs`** and `pub use` it from both layers. `app/src/settings/lsp.rs` imports `warp_lsp::user_config::UserLspServerConfig` and uses it as the field type on the settings group; `crates/lsp` constructs `UserConfiguredLanguageServer` from instances of this same type. This keeps the type in the lower-level crate (where the `LanguageServerCandidate` impl that consumes it lives) and avoids creating a new shared crate just for one struct. Verify the exact module name (`warp_lsp` vs `lsp`) at implementation time against `crates/lsp/Cargo.toml`.

## Proposed changes

### 1. New shared type: `UserLspServerConfig`

**File:** new `crates/lsp/src/user_config.rs`. Exported from `crates/lsp/src/lib.rs` so `app/` can use it.

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserLspServerConfig {
    pub name: String,
    pub command: Vec<String>,
    pub file_types: Vec<String>,
    pub language_id: String,
}

impl UserLspServerConfig {
    /// Stable identity for change-detection on settings reload.
    /// Hashing only `command` + args means metadata-only edits
    /// (e.g. `language_id` change) do NOT trigger re-confirmation.
    pub fn command_fingerprint(&self) -> u64 { ... }
}
```

V0 deliberately omits `root_files` (use Warp's existing root-repo detection), `initialization_options` (no settings shape for arbitrary nested config yet), and `start_timeout_ms` (fixed 5s default in code). These are tracked in Follow-ups.

### 2. New settings group: `LspSettings`

**File:** new `app/src/settings/lsp.rs`. Pattern matches `app/src/settings/input.rs` (`define_settings_group!` macro).

The group holds a single setting, `custom_servers`, of type `Vec<UserLspServerConfig>` (the type is imported from `crates/lsp`, not redefined in `app/`).

**Serde rename mapping:** the user-facing TOML key is `[[lsp.servers]]`, but the Rust field on the settings group is `custom_servers`. The mapping is wired with `#[serde(rename = "servers")]` on the field so the on-disk schema reads naturally:

```toml
[[lsp.servers]]
name = "intelephense"
command = ["intelephense", "--stdio"]
file_types = ["php", "phtml"]
language_id = "php"
```

The Rust field is named `custom_servers` to disambiguate from any built-in registry while keeping the user-facing TOML clean. Document this rename in the field doc-comment so contributors searching for `[[lsp.servers]]` find it.

Validation runs at parse time (in a `validate()` method called from the settings init path):

- `name` non-empty and unique across the vec.
- `command` non-empty.
- `file_types` non-empty; each entry stripped of leading `.`.
- `language_id` non-empty.
- **Cross-entry validation:** the union of all entries' `file_types` must contain no duplicates. If two entries both list `"php"`, the entire `[lsp]` block is rejected with an error pointing at both offending entries.

Validation failures emit a settings-error notification (existing pattern in `app/src/settings/initializer.rs`) with the offending entry index. Other entries continue to load only when the failure is per-entry; cross-entry conflicts disable all custom LSPs until resolved.

### 3. Per-workspace enablement state

**File:** new fields on the existing per-workspace settings struct (verify exact location at implementation time — likely `app/src/workspace/state.rs` or equivalent).

```rust
pub struct WorkspaceLspState {
    /// Map of `UserLspServerConfig.name` → command fingerprint at the time
    /// of last user confirmation. If the live config's fingerprint differs,
    /// the server is treated as "needs re-confirmation".
    enabled_custom_servers: HashMap<String, u64>,
    /// Names dismissed via the chip. Suppresses the chip until cleared
    /// via Settings UI.
    dismissed_custom_servers: HashSet<String>,
}
```

The fingerprint-keyed map is what enforces invariant 11 (re-confirmation on `command` change): an enabled entry whose live fingerprint no longer matches its stored fingerprint is silently demoted to "not enabled, but eligible to chip" — the user sees the chip again with a "command changed" affordance and must explicitly re-enable.

`#[serde(default)]` on both fields handles migration from existing workspaces that have no custom-LSP state.

### 4. New `LanguageServerCandidate` impl

**File:** new `crates/lsp/src/servers/user_configured.rs`.

```rust
pub struct UserConfiguredLanguageServer {
    config: UserLspServerConfig,
    workspace_root: PathBuf,
}

#[async_trait]
impl LanguageServerCandidate for UserConfiguredLanguageServer {
    async fn should_suggest_for_repo(&self, path: &Path, _executor: &CommandBuilder) -> bool {
        // Heuristic: any file in `path` matches one of `file_types`.
        // Root detection itself uses Warp's existing root-repo logic
        // (per @kevinyang372 review), so this method does NOT walk
        // for `root_files` — that field is V0-deferred.
        ...
    }

    async fn is_installed_in_data_dir(&self, _executor: &CommandBuilder) -> bool {
        // We never install user-configured servers. Always false.
        false
    }

    async fn is_installed_on_path(&self, executor: &CommandBuilder) -> bool {
        // Reuse `binary_in_path` from #9562's infrastructure (filesystem
        // check for `command[0]` with executable-bit check on Unix).
        // No probe-spawn — that contract was specific to npm-installed
        // servers that may have stale shims. For user-configured servers
        // an unhealthy spawn surfaces via the start-time error toast.
        binary_in_path(&self.config.command[0], executor.path_env_var())
    }

    async fn install(&self, _: LanguageServerMetadata, _: &CommandBuilder) -> anyhow::Result<()> {
        anyhow::bail!("user-configured LSP `{}` is not installable by Warp", self.config.name)
    }

    async fn fetch_latest_server_metadata(&self) -> anyhow::Result<LanguageServerMetadata> {
        anyhow::bail!("user-configured LSP `{}` has no version metadata", self.config.name)
    }
}
```

### 5. `LSPServerType` extension and built-in suppression

**File:** `crates/lsp/src/supported_servers.rs` and `crates/lsp/src/manager.rs`.

Add a variant:

```rust
pub enum LSPServerType {
    // ...existing variants...
    UserConfigured(UserLspServerConfig),
}
```

The candidate-construction site enforces the **overwrite** semantic per @kevinyang372's review (user-configured servers replace built-ins for matched file types in the workspace where they are enabled):

```rust
fn all_candidates_for(workspace: &Workspace, settings: &LspSettings) -> Vec<Box<dyn LanguageServerCandidate>> {
    let mut suppressed_extensions: HashSet<&str> = HashSet::new();
    let mut out = Vec::new();

    for cfg in &settings.custom_servers {
        let live_fp = cfg.command_fingerprint();
        let confirmed_fp = workspace.lsp_state.enabled_custom_servers.get(&cfg.name).copied();
        if confirmed_fp == Some(live_fp) {
            // Enabled AND command unchanged since last user confirmation.
            for ext in &cfg.file_types {
                suppressed_extensions.insert(ext.as_str());
            }
            out.push(Box::new(UserConfiguredLanguageServer::new(cfg.clone(), workspace.root.clone())));
        }
        // else: not enabled, dismissed, or pending re-confirmation — chip flow handles surface.
    }

    for builtin in built_in_candidates() {
        if builtin.handled_extensions().iter().any(|e| suppressed_extensions.contains(e.as_str())) {
            continue; // user-configured server overrides this built-in for this workspace
        }
        out.push(builtin);
    }

    out
}
```

The built-in candidate trait is extended with a `handled_extensions(&self) -> Vec<String>` method (or equivalent) so the suppression filter knows which built-ins to skip. This is a small additive change to the existing trait surface.

### 6. Language ID handling

The user supplies `language_id` directly in the config; we send it verbatim in `textDocument/didOpen.languageId` for any file matching `file_types`. The closed `LanguageId` enum in `crates/lsp/src/config.rs` is **not** extended — it continues to handle built-in servers only.

The dispatch site (whatever currently calls `LanguageId::from_path(...).lsp_language_identifier()` to populate `didOpen`) gets a small branch: if the active candidate for this open is a `UserConfiguredLanguageServer`, use its `config.language_id`; otherwise use the existing closed-enum path. This avoids polluting `LanguageId` with a `Custom(String)` variant and the cascading match-arm work that would create.

This design directly addresses the oz-for-oss feedback that file extension is insufficient (`sh` → `shellscript`, `phtml` → `php`): the user states the canonical languageId once per server, and Warp forwards it.

### 7. Footer chip

**File:** `app/src/code/editor/` — extending whatever surface renders the LSP-related chip when a built-in server is detected-but-not-installed (find via `grep -rn "Install" app/src/code/editor/`).

The chip rendering checks these states in cascade order (first match wins). Earlier states take precedence — in particular, the command-changed chip (state 2) overrides dismissal because it represents a security signal that the binary the user previously trusted has changed.

1. Server enabled (stored fingerprint exists) AND live `command_fingerprint` ≠ stored fingerprint → "`<name>` command changed — re-enable?" chip. **Surfaces even if user previously dismissed.**
2. Server dismissed in this workspace → no chip (silenced).
3. Server defined, but `command[0]` not on PATH at chip-render time → "Configure `<name>`" chip with `Open settings` action.
4. Server defined, not enabled, not dismissed, binary on PATH → "Enable `<name>`" chip.
5. Server enabled and fingerprint matches → no chip (server already running).

Click handlers:

- `Enable` / `Re-enable` → record live `command_fingerprint` in `WorkspaceLspState.enabled_custom_servers`, persist, trigger `manager::start_for_workspace`. The same code path handles initial enable and re-confirmation.
- `Dismiss` → mutate `dismissed_custom_servers`, persist, no spawn.
- `Open settings` → existing settings-open-with-search action, scoped to `lsp.servers`.

### 8. Settings UI (in scope for V0)

**File:** the existing **Settings → Code → Indexing and Projects** view (locate via `grep -rn "Indexing" app/src/settings/`). Per @kevinyang372's review, no new "Code Intelligence" section is created; we extend the existing pattern.

For each user-configured server that has any per-workspace state in the active workspace — i.e. its `name` appears in `enabled_custom_servers`, `dismissed_custom_servers`, OR has a stored fingerprint that no longer matches live config (pending re-confirmation) — render one row beneath the existing per-workspace project list. Each row shows the server `name`, its current state, and a single context-appropriate action:

| State | Predicate | Action button | Behavior on click |
|---|---|---|---|
| Enabled | `enabled_custom_servers[name]` exists AND equals live `command_fingerprint()` | `Disable` | Removes BOTH `enabled_custom_servers[name]` AND any `dismissed_custom_servers` entry for `name`, fires manager shutdown for this workspace within 1s, restores any built-in server for the relevant `file_types`. Clearing both fields is what makes invariant 12 ("chip reappears on next file open") hold unconditionally. |
| Pending re-confirmation | `enabled_custom_servers[name]` exists but stored fingerprint ≠ live `command_fingerprint()` | `Re-enable` | Records live fingerprint into `enabled_custom_servers[name]` and triggers `manager::start_for_workspace`. Same code path as the chip's re-enable click. |
| Dismissed | `dismissed_custom_servers` contains `name` (and not in either of the above states) | `Re-enable` | Clears `dismissed_custom_servers[name]`. Does not spawn — the user re-enters the chip flow on next matching file open. |

This is the surface that satisfies invariants 11, 12, and 14. Without rows for dismissed and pending-re-confirmation entries, those invariants would be unreachable from the UI, which is why oz-for-oss correctly flagged the prior "follow-up" framing as critical.

### 9. Settings reload handling

**File:** `app/src/settings/initializer.rs` — extend the existing reload path.

On settings change, the diff is computed against the previous `[lsp.servers]` list keyed by `name`:

1. **Added entries:** nothing immediate; chip will appear on next matching file open.
2. **Removed entries:** shut down any running instance for that name in any workspace; clear from `enabled_custom_servers` and `dismissed_custom_servers` everywhere.
3. **Changed entries — metadata only** (`language_id` or `file_types` changes, but `command` and args byte-equal old): if currently running, restart in place with new metadata. Stored fingerprint is unchanged so no re-confirmation prompt fires.
4. **Changed entries — `command` or args differ**: shut down any running instance, **leave the `name → fingerprint` entry in `enabled_custom_servers` unchanged**. The candidate-construction filter above will treat the entry as "pending re-confirmation" because live ≠ stored fingerprint. The chip flow surfaces the re-enable affordance.

This addresses the oz-for-oss security finding directly: a settings-file edit that mutates `command` cannot run a new binary in a workspace that previously trusted a different binary, even if no Warp restart occurs.

### 10. Lifecycle: spawn, initialize, timeout, shutdown

**File:** `crates/lsp/src/manager.rs`.

- Build the `Command` from `config.command[0]` + `config.command[1..]`.
- `stdin/stdout` piped, `stderr` captured to a per-server ring buffer (last 200 bytes for error toasts).
- Use **`tokio::process::Command`** with `kill_on_drop(true)` so any panic / unwinding path between spawn and `initialize` reaps the child. This is portable (Tokio handles the platform differences) — `std::process::Child::drop` is documented to **not** kill on Unix and is therefore unsuitable as a kill guarantee.
- Wrap the `initialize` request in `warpui::r#async::FutureExt::with_timeout(Duration::from_millis(5000))`. Three outcomes (matches the pattern from `feat/9168-php-lsp-intelephense` commit `31285c4`):
  - `Ok(Ok(_))` — server initialized; route LSP traffic.
  - `Ok(Err(err))` — JSON-RPC error; surface notification with err message; `child.kill().await; child.wait().await;`.
  - `Err(timeout)` — explicitly `child.kill().await; child.wait().await;` (don't rely on `Drop` alone), then surface timeout notification.
- On Warp shutdown: existing `shutdown` + `exit` flow already handles all running servers; user-configured servers ride the same path. After the 1s graceful window, fall through to `child.kill().await; child.wait().await;`.

The explicit `kill().await; wait().await;` pair is the answer to the oz-for-oss portability concern: it works on every platform Tokio supports and avoids leaving zombies on Unix.

## Testing and validation

Each invariant from `product.md` maps to a test at this layer:

| Invariant | Test layer | File |
|---|---|---|
| 1, 2 (parse / validate) | unit | `app/src/settings/lsp_tests.rs` (new) — TOML strings → `LspSettings` parse outcomes (success cases + each validation error, including cross-entry duplicate `file_types`). |
| 4, 5, 6, 7 (spawn outcomes) | unit | `crates/lsp/src/servers/user_configured_tests.rs` (new) — mock `CommandBuilder` returning success / error / hang. Assert explicit `kill().await; wait().await;` on the timeout branch. |
| 7 (timeout via `with_timeout` + explicit kill) | unit | same file — wire a future that never resolves, assert the timeout branch fires within `5000ms + 100`, and that `kill` was observed before `wait` returned. |
| 8, 9 (overwrite semantics: built-in suppressed) | unit | `crates/lsp/src/manager_tests.rs` extension — register a built-in candidate handling `php`, register an enabled user-configured entry handling `php`, assert the built-in is NOT in `all_candidates_for(workspace)`. Then disable the user entry and assert the built-in returns. |
| 10 (metadata-only reload restarts in place) | unit | manager test — pre-populate running server, swap `language_id`, assert restart with no chip-flow trigger. |
| 11 (command change → re-confirmation) | unit | manager + workspace-state test — pre-populate running server with stored fingerprint F, swap `command`, assert process is killed AND `enabled_custom_servers[name]` still equals F (not deleted), AND chip flow now reports "pending re-confirmation". |
| 12 (Settings UI disable) | integration | UI integration test — toggle disable in the Settings → Code row, assert process shutdown within 1s, assert built-in resumes. |
| 13 (per-workspace enablement persists across restart) | unit | workspace-state serialization round-trip with the fingerprint map. |
| 3, 8, 14 (chip behavior) | integration | UI integration test under `crates/integration/`. Stub the file-open event, assert chip presence/absence based on enablement+dismissal state. |
| 15 (`languageId` forwarding) | unit | parse a config with `language_id = "shellscript"` and `file_types = ["sh"]`, simulate `didOpen` for `foo.sh`, assert outgoing JSON-RPC contains `"languageId":"shellscript"`. |
| 16 (graceful shutdown on quit) | unit | shutdown handler test — running user-configured server receives `shutdown` then `exit` then has 1s window before explicit `kill().await; wait().await;`. |

### Cross-platform constraints (lessons from #9562/#9568)

- Tests building `PATH` strings must use `std::env::join_paths`, not `:`. Reuse the `make_path_var` helper introduced in #9562's tests.
- On Windows, `command[0]` may need `.exe` / `.cmd` resolution. Reuse `binary_filename` helper (also from #9562's tests).
- `tokio::process::Command::kill_on_drop(true)` is portable; do not substitute `std::process::Child::drop` — it does not kill on Unix.

## End-to-end flow

```
User edits settings.toml
  └─> [LspSettings::reload]                                        (settings/lsp.rs)
        ├─> validate() (rejects malformed; rejects cross-entry duplicate file_types)
        └─> emit SettingsChanged event
              └─> [manager::on_settings_change]                     (lsp/src/manager.rs)
                    ├─> diff old vs new custom_servers (by name)
                    ├─> stop removed entries everywhere
                    ├─> metadata-only changes → restart in place
                    └─> command/args changes → stop running instance,
                          KEEP stored fingerprint (so candidate filter
                          treats as "pending re-confirmation"; chip
                          will surface)

User opens .lua file in workspace W
  └─> [editor::on_file_open]                                       (app/src/code/editor/)
        └─> [chip_renderer]
              ├─> get LspSettings.custom_servers
              ├─> filter where file_types contains "lua"
              ├─> for each: classify state (cascade; first match wins)
              │     ├─> enabled & live fp ≠ stored fp → "command changed — re-enable?"
              │     │   (security override — surfaces even if dismissed)
              │     ├─> dismissed → no chip
              │     ├─> binary not on PATH → "Configure <name>"
              │     ├─> not enabled & binary on PATH → "Enable <name>"
              │     └─> enabled & fp match → no chip (already running)
              └─> render appropriate chip per remaining entry

User clicks Enable / Re-enable
  └─> [chip_handler::on_enable_clicked]
        ├─> live_fp = config.command_fingerprint()
        ├─> W.enabled_custom_servers[name] = live_fp
        ├─> persist W
        └─> [manager::start_candidate]                              (lsp/src/manager.rs)
              ├─> construct UserConfiguredLanguageServer            (servers/user_configured.rs)
              ├─> all_candidates_for(W) now suppresses any built-in
              │   handling the same file_types in W
              ├─> is_installed_on_path → if false, surface "Configure <name>" toast
              ├─> tokio::Command, kill_on_drop(true), stderr→ring buffer
              ├─> initialize.with_timeout(5000ms)
              │     ├─> Ok(Ok(_)) → register, route LSP traffic
              │     ├─> Ok(Err(err)) → child.kill().await; child.wait().await;
              │     │                  surface error toast
              │     └─> Err(_timeout) → child.kill().await; child.wait().await;
              │                          surface timeout toast
              └─> on subsequent file opens, no chip — server already running,
                  built-in for same file_types stays suppressed in W
```

## Risks

- **Built-in suppression in W is invisible until the user notices missing diagnostics.** Mitigation: the Settings → Code row makes the active user-configured server visible per workspace; a contributor doc note explains that enabling a custom server replaces the built-in for matched file types.
- **A configured server that crashes on every spawn could loop forever.** Mitigation: the chip's `Enable` action is one-shot per click. We do not auto-restart on crash; the user re-enables manually. If exit happens after `initialize` succeeded, we surface a "server crashed" toast and the chip returns to the disabled-but-defined state.
- **Per-workspace state migration.** Existing workspaces have no `enabled_custom_servers` field. Mitigation: `#[serde(default)]` on the new fields; absence parses as empty map / set.
- **Fingerprint stability across Warp versions.** If the fingerprint algorithm changes between releases, every enabled server would silently demote to "pending re-confirmation" on first launch of the new version. Mitigation: define `command_fingerprint` as a stable hash of `command` (vec of bytes) only; pin to a stable hasher (e.g. SipHash with a fixed key, not the default `DefaultHasher` which can change).

## Follow-ups (out of this spec)

- `nix flake check`-validated dev shell with all referenced LSP binaries pre-installed (would help testing).
- `root_files` user-supplied glob patterns for root detection (V0 uses Warp's existing root-repo detection per @kevinyang372).
- `initialization_options` forwarding (V0-skipped per @kevinyang372: warp does not yet have settings shape for arbitrary nested config payloads). When added, the JSON-server schema-fetching pattern from #9568's review is the model for documentation.
- User-configurable `start_timeout_ms` (V0 uses fixed 5s).
- `~` and `$VAR` expansion in `command[0]` (consistency with `/open-file`).
- Documentation page `docs/custom-lsp-examples.md` with intelephense / lua-language-server / zls / bash-language-server entries. In scope for the same release as this feature, but tracked separately.
