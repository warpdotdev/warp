# File-based global rules — Tech Spec
Product spec: `specs/APP-3893/PRODUCT.md`

## Context

This change adds a third source of agent-rule context that sits alongside the existing two:

- **Project rules** — `WARP.md` / `AGENTS.md` walked up from the working directory. Indexed by `ProjectContextModel`, sent on every query inside `AIAgentContext::ProjectRules`.
- **Cloud rules** (`AIFact`s) — created in-app, persisted as cloud objects, and applied server-side when `rules_enabled: true` is set on the request. The client only ships an enable flag; the server has the contents.
- **File-based global rules** (this feature) — a Markdown file at a well-known home location (`~/.agents/AGENTS.md`). Indexed and shipped by value the same way project rules are.

Relevant code:

- `crates/ai/src/project_context/model.rs` — `ProjectContextModel` owns project-rule indexing via `path_to_rules: HashMap<PathBuf, ProjectRules>` and watches each project repo via `repo_metadata::DirectoryWatcher`. It exposes the public rule facade: `find_applicable_rules(path)`, `find_applicable_project_rules(path)`, `global_rule_paths()`, and `index_and_store_rules(root)`.
- `crates/ai/src/project_context/global_rules.rs` — owns file-based global-rule source metadata, cached global file contents, home-subdir watcher state, the update channel, and the global-rule `RepositorySubscriber`.
- `app/src/ai/blocklist/context_model.rs:398-451` — calls `find_applicable_rules` and packs the result into `AIAgentContext::ProjectRules { active_rules, additional_rule_paths, root_path }`.
- `app/src/ai/agent/api/convert_to.rs:763` — serializes `AIAgentContext::ProjectRules` into `api::input_context::ProjectRules { active_rule_files, ... }`. The server appends `active_rule_files` to the prompt directly.
- `app/src/ai/mcp/file_mcp_watcher.rs` — pre-existing pattern for watching a known home subdir (e.g. `~/.codex`) plus the home directory itself for subdir creation/deletion. Reused as a template for the global-rule watchers.
- `crates/watcher/src/home_watcher.rs` — `HomeDirectoryWatcher` singleton (non-recursive watch on `$HOME`) used to detect creation/deletion of the rule subdir at runtime.
- `app/src/ai/facts/view/rule.rs` — `RuleView` settings UI with `Global` and `ProjectBased` tabs. Cloud rules render via `CloudRuleRow`; project rules render via the path-only row type.
- `app/src/lib.rs` — singleton bootstrap site for `ProjectContextModel` and the startup call to `index_global_rules`.

## Proposed changes

### Model

`ProjectContextModel` (`crates/ai/src/project_context/model.rs`) remains the rule-context facade, but file-based global rules are isolated behind `GlobalRules` in `crates/ai/src/project_context/global_rules.rs`.

`GlobalRules` owns:

- A `GlobalRuleSource` enum that enumerates known global locations via `strum::EnumIter`. Variants expose `name() / home_subdir() / file_pattern()` accessors. Today there is one variant, `Agents` → `~/.agents/AGENTS.md`. Adding a new global source = one variant + one match arm in each accessor.
- `rules: BTreeMap<PathBuf, ProjectRule>` — discovered file contents, sorted by path so iteration is deterministic.
- `source_watchers: HashMap<PathBuf, GlobalSourceWatcherState>` — keyed by the absolute home subdir path so duplicate registrations naturally dedup.
- `updates_tx: Option<Sender<GlobalRulesUpdate>>` — single channel that all per-source `RepositorySubscriber` instances push into; tagged with the originating `GlobalRuleSource`.

`ProjectContextModel` owns one `global_rules: GlobalRules` field and exposes thin integration methods:

- `index_global_rules(ctx)` — invoked once at startup and delegated to `GlobalRules::index`. Native builds use the file-watching implementation in `global_rules.rs`; non-`local_fs` builds call the no-op `dummy_global_rules.rs` implementation so the public facade and startup call stay unconditional. For each `GlobalRuleSource`, the native delegated logic:
  1. Spawns an async read of the target file via `ctx.spawn`. The callback inserts into `global_rules` and emits `GlobalRulesChanged`.
  2. Subscribes to `HomeDirectoryWatcher` to react to creation/deletion of the subdir at runtime.
  3. If the subdir exists, registers a `repo_metadata::DirectoryWatcher` on it and starts a `GlobalRulesRepositorySubscriber` that funnels per-file events back through the channel.
- Home-subdir watcher events handle deletions before additions so bundled create+delete events do not leave stale watchers or cached rule state behind.
- `find_applicable_rules` is extended to layer global rules on top of project rules in the returned `ProjectRulesResult.active_rules`. Globals iterate via `BTreeMap` order (deterministic). The `pending_context()` consumer in `BlocklistAIContextModel` is unchanged.
- A separate `find_applicable_project_rules(path)` accessor returns *only* indexed project rules — globals are deliberately not layered in. This is the signal callers should use when they want to know "does this repo itself have rules indexed?" rather than "what rules apply to this path?". Two such callers exist today (PRODUCT.md invariant 13): `app/src/terminal/view/init_project/model.rs:189-202` (`should_have_available_steps`) and `app/src/code_review/code_review_view.rs:4505-4534` ("Repo is initialized with a {file_name} file." hint). Both were migrated as part of this change so a stray `~/.agents/AGENTS.md` does not flip every repo into the "already initialized" state. `find_applicable_rules` continues to be the right entry point for the agent-context packing path in `BlocklistAIContextModel::pending_context`.
- `GlobalRules::spawn_global_rule_read` reacts to a failed re-read by removing any previously cached entry for that path and emitting a `GlobalRulesChanged` deletion delta. This covers cases where the FS event arrives but the read fails (file deleted between event and read, perms revoked, replaced with a non-regular file) — silently keeping stale rule text active would surprise users who had thought they removed it. PRODUCT.md invariant 4 is the user-visible promise this enforces.
- The `safe_warn!` calls in `GlobalRules::register_global_source_watcher` keep the underlying error in the `full:` (dogfood) branch only; the `safe:` branch never includes the error or the path because both can embed the user's home directory.
- A new event variant `ProjectContextModelEvent::GlobalRulesChanged(GlobalRulesDelta)` is emitted whenever the set of indexed global rules changes (initial read, FS update, subdir deletion, or a previously-known file becoming unreadable).
- A `pub fn global_rule_paths(&self)` accessor is added for the settings view to read without exposing the full `ProjectRule` content.

The implementation deliberately does **not** persist global rule paths to SQLite. The locations are well-known constants, so we re-scan on every launch.

### Wire-up

`app/src/lib.rs` calls `ProjectContextModel::handle(ctx).update(ctx, |me, ctx| me.index_global_rules(ctx))` immediately after constructing the singleton. The call is unconditional; platform differences live behind the `GlobalRules` alias (`global_rules.rs` for native file-system builds, `dummy_global_rules.rs` no-op for builds without file-system watcher support).

### Settings UI

`app/src/ai/facts/view/rule.rs`:

- Renames `ProjectScopedRow` → `FileBackedRow` and `RuleRow::ProjectScoped` → `RuleRow::FileBacked` since the row shape (path + open-file button) is now used for two sources, not just project rules.
- Adds `file_backed_global_rules: Vec<FileBackedRow>` to `RuleView`, populated from `ProjectContextModel::global_rule_paths()` in `RuleView::new`.
- Extends the existing `ProjectContextModel` subscription block to also handle `GlobalRulesChanged`, refreshing the new field and calling `ctx.notify()`. The match is exhaustive over event variants now, so future variants will trigger a compile error.
- `get_filtered_rules` for `RuleScope::Global` now chains cloud rows (`RuleRow::Global`) with file-backed-global rows (`RuleRow::FileBacked`). The render path for `FileBacked` is unchanged from the project-based render path; the same `OpenFile(PathBuf)` action is dispatched and the editor opens.
- The Global-tab zero-state string is updated to mention `~/.agents/AGENTS.md` as a second way to add a rule.

### What is reused unchanged

- `AIAgentContext::ProjectRules` proto / serialization. Globals piggyback on the existing variant; the server appends `active_rule_files` regardless of where they came from.
- The `OpenFile(PathBuf)` action / event already plumbed for project rows.
- Persistence: nothing new lands in SQLite.

## Testing and validation

### Unit tests
Located in `crates/ai/src/project_context/model_tests.rs`. They populate `ProjectContextModel` through local test helpers (direct test-visible `global_rules.rules` insertion for globals and `path_to_rules` for project rules), so they exercise the layering logic without spinning up the watcher infrastructure (which requires the warpui runtime).

- Global rule alone, no project rules → `find_applicable_rules` returns it. Covers PRODUCT invariants 8, 10.
- Global rule + project `WARP.md` for the same path → both appear in `active_rules`, ordered global first. Covers invariants 8, 9.
- Global rule + project `WARP.md` and `AGENTS.md` in the same dir → project `WARP.md` shadows project `AGENTS.md`; global is appended. Covers invariant 9.
- No rules anywhere → `None`. Covers invariant 10.
- Global-only → `root_path` falls back to the parent of the global file.
- Multiple global sources both contribute (uses set-based assertions because `BTreeMap` orders by path).
- `find_applicable_project_rules` ignores globals: with only a global indexed, project-only is `None` while layered `find_applicable_rules` is `Some`. Covers invariant 13.
- `find_applicable_project_rules` returns the project rule (only) when a project rule and a global are both indexed. Covers invariant 13.

The project-context tests pass via `cargo nextest run -p ai --features local_fs project_context`.

### Manual end-to-end
Maps directly to the PRODUCT.md behavior section:

1. Without `~/.agents/AGENTS.md`, fire an agent query and confirm no global rule is attached. (Invariant 10.)
2. `mkdir -p ~/.agents && echo "prefer 4-space indentation" > ~/.agents/AGENTS.md`. The next agent query includes the file's contents. (Invariants 1, 2, 8.)
3. Edit the file in any external editor and re-fire. New contents appear. (Invariant 5.)
4. `rm ~/.agents/AGENTS.md` and re-fire. Global is gone. (Invariant 4.)
5. Without restarting, recreate the file. Watcher picks it up. (Invariants 3, 6.)
6. Open Settings → AI → Rules → Global. With the file present, expect a row showing `~/.agents/AGENTS.md` with an "Open file" button. (Invariants 13, 14.)
7. Edit/delete the file while the Global tab is open. Row updates live. (Invariant 15.)
8. Add a cloud rule via the existing "Add" button. Both row types coexist. Search filters across both. (Invariants 16, 17.)
9. With both cloud and file-based empty, the zero-state copy mentions `~/.agents/AGENTS.md`. (Invariant 18.)

### Lint / format
`cargo fmt`, `cargo nextest run -p ai --features local_fs project_context`, and `cargo clippy -p ai --all-features --tests -- -D warnings` pass.

## Follow-ups
- Decide whether the `MemoryEnabled` toggle should also gate file-based global rules (currently it does not — see PRODUCT.md invariant 19). Either gate `find_applicable_rules` on the setting in `BlocklistAIContextModel::pending_context`, or add a separate file-rule toggle.
- Consider exposing the file's content in the Settings row (preview/truncate, like cloud rules) instead of just the path.
- If we want the rule file open-state to feel editable in-app, surface an "Edit" affordance that opens it in Warp's code editor with a buffer rather than just a file open.
