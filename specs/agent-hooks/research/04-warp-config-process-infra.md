# 04 — Warp config, watching, and process infrastructure a hook runner would reuse

Repo: `warp` @ `393e97a30a5ed85cf062b75e88326f83746dbecb` (master). All paths are relative to the repo root. Line numbers are from this commit. "Verified" = read in code. "Inferred" = reasonable conclusion from code I read but did not trace end to end. "Not found" = searched and did not find.

## TL;DR for the architect

- All four execution surfaces (GUI app, headless TUI, Oz cloud agent / `warp agent run` CLI, remote-server daemon) run the same `initialize_app` in `app/src/lib.rs:1495` and register the same config singletons (`ProjectContextModel`, `SkillManager`, `FileMCPWatcher`/`FileBasedMCPManager`, `WarpConfig`). The user's belief is confirmed: surfaces differ in WHICH roots get discovered and from WHERE (terminal cwd navigation vs. cloud environment prep vs. `WARP_SKILL_DIRS`; GUI `~/.warp` vs TUI `~/.warp_cli` config dirs), not in parsing or runtime behavior. See §1.7.
- Project-scoped config discovery is already centralized: `DetectedRepositories` finds the git root (walks up from cwd, stops at `$HOME`), `RepoMetadataModel` runs "standing queries" over the tree for `WARP.md`/`AGENTS.md` and `<provider>/skills/*/SKILL.md`, and consumers (`ProjectContextModel`, `SkillWatcher`) subscribe to `RepoMetadataEvent::StandingQueryResultsUpdated`. A `.cursor/hooks.json` / `.claude/settings.json` discovery would plug into `StandingQueryDefinitions` (`crates/repo_metadata/src/standing_queries.rs:13`) or, more simply, into the `FileMCPWatcher` pattern which already watches per-provider config files at both `$HOME` and repo roots.
- There is no generic "config source" trait. The closest abstractions are the `SKILL_PROVIDER_DEFINITIONS` static list + `SkillProvider` enum (`crates/ai/src/skills/skill_provider.rs:31,106`) and the `MCPProvider` enum with `home_config_path()`/`project_config_path()` (`app/src/ai/mcp/mod.rs:515-564`). Both already know about `.cursor`, `.claude`, `.codex`, `.agents`.
- Subprocess execution: use `command::r#async::Command` (`crates/command/src/async.rs:15`); clippy forbids raw `std::process::Command`/`async_process::Command` (`.clippy.toml:5-9`). For "run in the user's login shell with their PATH", use `LocalCommandExecutor::execute_local_command_in_login_shell` (`app/src/terminal/model/session/command_executor/local_command_executor.rs:226`) plus `LocalShellState` PATH capture (`app/src/terminal/local_shell/mod.rs:88,149`). No shared timeout helper exists; the LSP transport shows the pattern (`crates/lsp/src/transport.rs:149-184`).
- Side LLM calls: none are generic. Every auxiliary model call is a purpose-specific warp-server endpoint (GraphQL `GenerateDialogue`, REST `/ai/generate_block_title`, `/ai/predict_am_queries`, ...). Model choice is server-side per feature. A Cursor-style prompt hook needs a new server endpoint or a new client-callable "evaluate prompt" API (§6).
- Warp does NOT write hook configs into third-party agents in-repo. It installs agent-owned plugins (`claude plugin install warp@claude-code-warp`, etc.) and reads their install state; the hook JSON lives in external repos (`warpdotdev/claude-code-warp`). The only HookConfig shape in-tree is a research note (`specs/GH15768/warp-hooks-sample.md`). See §7.

---

## Section 1 — Config discovery today

### 1.1 Project rules (WARP.md / AGENTS.md)

Verified pipeline:

1. **Project root = git working tree root**, detected by `DetectedRepositories::detect_possible_local_git_repo` (`crates/repo_metadata/src/repositories.rs:91`) which spawns `find_git_repo` (`repositories.rs:338`). `find_git_repo` walks `cwd` upward looking for a `.git` dir, a bare `*.git` dir, or a `.git` gitfile (worktrees/submodules), and **stops when it reaches `$HOME`** (`repositories.rs:343`) — a cwd directly under `$HOME` with no repo yields `None`. Cached roots: `repository_roots: HashSet<LocalOrRemotePath>` (`repositories.rs:41`); lookup by ancestor walk in `get_root_for_path` (`repositories.rs:228`).
   - Trigger (GUI): `TerminalView::apply_block_metadata_update` on precmd / OSC 7 cwd change, `app/src/terminal/view.rs:12291` via wrapper `app/src/util/repo_detection.rs:46 fn detect_possible_git_repo` with `RepoDetectionSource::TerminalNavigation`.
   - Trigger (TUI): `crates/warp_tui/src/terminal_session_view.rs:2242` on `ActiveSessionEvent::UpdatedPwd`, same wrapper, same source.
   - Trigger (Oz cloud / `warp agent run`): `register_cloned_repo` in `app/src/ai/agent_sdk/driver/environment.rs:1332` calls `detect_possible_local_git_repo` with `RepoDetectionSource::CloudEnvironmentPrep` (`environment.rs:1362-1366`) for each cloned repo; skipped for Docker-sandbox-only working dirs (`environment.rs:1349`).
   - Other sources: `RepoDetectionSource` enum `repositories.rs:20` (`ProjectRulesIndexing`, `CodeReviewInitialization`).
2. `DetectedRepositoriesEvent::DetectedGitRepo` → `PersistedWorkspace` subscription (`app/src/ai/persisted_workspace.rs:323`) → `index_repo` (`persisted_workspace.rs:680`) → `ProjectContextModel::index_and_store_rules` (`crates/ai/src/project_context/model.rs:355`). Note the comment at `persisted_workspace.rs:310-316`: this subscription is registered regardless of codebase-indexing enablement because it drives rules/skills discovery in the TUI too. It is **disabled** under `cfg!(test | fast_dev | integration_tests)` (`persisted_workspace.rs:318`).
3. `index_and_store_rules` asks `RepoMetadataModel` to `index_lazy_loaded_path` (`crates/repo_metadata/src/local_model.rs:1080`; first-level tree only, but standing queries scan the whole subtree) and then reads standing-query results. Standing queries are defined in `crates/repo_metadata/src/standing_queries.rs`: `StandingQueryDefinitions` (`:13`) with default `project_rule_file_names = ["WARP.md", "AGENTS.md"]` (`:22`, case-insensitive match at `:59`), and `project_skill_provider_paths` set at startup from `SKILL_PROVIDER_DEFINITIONS` (`app/src/lib.rs:2081`). `record_path` (`:110`) is where a new file-name pattern (e.g. `hooks.json`) would be added.
4. `ProjectContextModel` (`crates/ai/src/project_context/model.rs:203`) subscribes to `RepoMetadataEvent::{RepositoryUpdated, StandingQueryResultsUpdated, RepositoryRemoved}` (`model.rs:299-325`) and re-reads rule file contents through an injected `ProjectRuleContentReader` fn pointer (`model.rs:26`; app-side `read_project_rule_contents`, passed at `app/src/lib.rs:2626`) so remote (SSH) hosts can be read too. Refreshes are coalesced per repo (`model.rs:384-455`).
5. **Precedence / merge**: within a directory `WARP.md` shadows `AGENTS.md` (`RuleAtPath::respected_rule`, `model.rs:64`). Rules from every ancestor directory of the query path inside the project root are all active (`find_active_or_applicable_rules`, `model.rs:131`); rules elsewhere in the repo are listed as "available" paths (`model.rs:142`). Global rules are layered first: `find_applicable_rules` (`model.rs:591`), documented order `global > project WARP.md > project AGENTS.md` (`model.rs:582`). Only one global source exists today: `~/.agents/AGENTS.md` (`GlobalRuleSource::Agents`, `crates/ai/src/project_context/global_rules.rs:19-45`).
6. **Consumer**: `BlocklistAIContextModel::pending_context` packs `AIAgentContext::ProjectRules` per query (`app/src/ai/blocklist/context_model.rs:367,403`).
7. **Caching / re-read**: in-memory per repo root (`path_to_rules`, `model.rs:205`); persisted paths in SQLite via `ProjectContextModelEvent::KnownRulesChanged` → `PersistedWorkspace` (`persisted_workspace.rs:287-307`); re-read on change via repo metadata watcher events (yes, hot). Global rules are watched via `DirectoryWatcher` on `~/.agents` + `HomeDirectoryWatcher` for subdir create/delete (`global_rules.rs:93-134, 194-278, 319-361`).

### 1.2 Skills

- Provider registry: `SkillProvider` enum and `SKILL_PROVIDER_DEFINITIONS` (`crates/ai/src/skills/skill_provider.rs:31, 106-154`). Order = precedence: `.agents/skills` > `.warp/skills` > `.claude/skills` > `.codex/skills` > `.cursor/skills` > `.gemini/skills` > `.copilot/skills` > `.factory/skills` > `.github/skills` > `.opencode/skills` > `.kiro/skills`. `provider_rank` (`:157`), `home_skills_path` (`:166`; Warp's home dir is `warp_core::paths::warp_home_skills_dir()` = `~/.warp[-dev]/skills`, others are `~/<provider>/skills`), `get_provider_for_path` (`:180`), `get_scope_for_path` (`:243`, Home vs Project).
- Project skills: discovered via the same standing queries (`is_project_skill_file`, `standing_queries.rs:51`); `SkillWatcher` subscribes to `RepoMetadataModel` (`app/src/ai/skills/file_watchers/skill_watcher.rs:187-199`) and falls back to a direct `DirectoryWatcher` subscription only if metadata indexing failed (`skill_watcher.rs:58-62`). Home provider dirs (`~/.agents`, `~/.claude`, ...) each get a `DirectoryWatcher` (`skill_watcher.rs:159-180`) plus `HomeDirectoryWatcher` for creation (`:131`). Warp-managed `~/.warp/skills` is watched by `WarpManagedPathsWatcher` (`:138-143`).
- `SkillManager` (`app/src/ai/skills/skill_manager.rs:26`) is the in-memory cache keyed by directory → skill paths. Scoping: `get_skills_for_working_directory_with_origin` (`:113`) includes home skills, then project skills whose directory is an ancestor of cwd AND inside the detected repo root (`:158-175`); in cloud environments (`is_cloud_environment`, set at `agent_setup.rs:425,493`) every known directory is in scope (`:146-156`). Dedup across providers by content, highest-rank provider wins (`:178-182`, `SkillDeduplicator`).
- `skills-lock.json`: not read by the client at runtime (grep in `app/src`, `crates/ai` only hits bootstrap scripts/docs; the `npx skills` installer owns it). The `--skill` CLI spec resolver is `app/src/ai/skills/resolve_skill_spec.rs:126` and global skill repos resolve via `global_skills.rs:17 resolve_skill_repos` (clones GitHub repos into the working dir).
- Cloud env extras: `WARP_SKILL_DIRS` env var → `load_skills_dirs` (`app/src/ai/agent_sdk/driver/agent_setup.rs:515`), treated as home-tier skills.

### 1.3 File-based MCP config (the closest analog to a hooks file)

- Provider enum `MCPProvider { Warp, Claude, Codex, Agents }` (`app/src/ai/mcp/mod.rs:515`). Home paths: `.warp/.mcp.json`, `.claude.json`, `.codex/config.toml`, `.agents/.mcp.json` (`:547`). Project paths: `.warp/.mcp.json`, `.mcp.json`, `.codex/config.toml`, `.agents/.mcp.json` (`:557`). `mcp_provider_from_file_path` (`:571`) maps a path back to a provider (longest suffix wins).
- `FileMCPWatcher` (`app/src/ai/mcp/file_mcp_watcher.rs:146`): on startup scans `global_config_sources()` (`:726`: Warp-managed file + each provider's home file); subscribes to `DetectedRepositories` (`:182`) and registers a `Repository` subscriber per repo root for `TerminalNavigation` (GUI+TUI) and `CloudEnvironmentPrep` (GUI/CLI only) — see `should_watch_repository` (`:693-712`). Per-file parses are generation-tagged and abortable (`update_servers_from_config_file`, `:654`). `${VAR}` substitution from the Warp process env (`substitute_env_vars`, `:766`) — missing var is a hard error. Codex TOML is normalized to JSON (`app/src/ai/mcp/parsing.rs`, `normalize_codex_toml_to_json`). Parse errors surface as `FileMCPConfigDiagnostic` (`:797`) and a `ScanComplete(PendingScan::InitialGlobal|CloudEnvRepo)` event lets the first agent turn wait for startup (`:137`).
- `FileBasedMCPManager::get_servers_for_working_directory` (`app/src/ai/mcp/file_based_manager.rs:166`): candidate roots are exactly `[home_dir(), repo_root]` (`:174`) — **no intermediate-directory walk**; project and home servers are unioned, not shadowed (dedupe across providers is an open TODO, `:279` APP-3429).
- Watch registration for the Warp-managed `.mcp.json` is in `WarpManagedPathsWatcher` (`app/src/warp_managed_paths_watcher.rs:326-358`, non-recursive on the parent dir, filtered to the one file).

### 1.4 `.warp/` directory conventions

- Constant `WARP_CONFIG_DIR = ".warp"` (`crates/warp_core/src/paths.rs:29`), channel-suffixed in the home dir (`.warp-dev`, `.warp-local`, `.warp-oss`, `.warp-integration`; `paths.rs:35-59`). `warp_home_config_dir` (`:66`), `warp_home_skills_dir` (`:70`), `warp_home_mcp_config_file_path` (`:74`).
- Project-level `.warp/`: `.warp/skills` (skill provider), `.warp/.mcp.json` (MCP), `.warp/workflows` (`app/src/workflows/local_workflows.rs:182 load_project_workflows`, which uses `git2::Repository::discover(cwd)` — a third, independent root-finding routine; cached per cwd, not watched).
- Home-level user data: `data_dir()` (`paths.rs:119`; macOS `~/.warp*`, else XDG data dir) holds `themes/`, `workflows/`, `launch_configurations/`, `tab_configs/`, `default_tab_configs/`, `custom_model_routers/` (`app/src/user_config/mod.rs:193-231`). `config_local_dir()` (`paths.rs:155`) holds `settings.toml`, `keybindings.yaml`, `user_preferences.json`. TUI equivalents: `tui_config_local_dir()` (`paths.rs:226`, macOS `~/.warp_cli*`) and `tui_mcp_config_file_path()` (`:243`). State/SQLite: `state_dir`/`secure_state_dir`/`tui_state_dir` (`:260-308`).

### 1.5 Settings file, tab configs, launch configs

- Settings surface: `settings::SettingsMode { Gui, Tui }` (`crates/settings/src/lib.rs:185`), process-global via `set_settings_mode`/`settings_mode()` (`:220,234`); set from `LaunchMode::settings_mode` (`app/src/lib.rs:519`). Public settings live in `settings.toml` at `user_preferences_toml_file_path()` (`app/src/settings/mod.rs:615`) when `FeatureFlag::SettingsFile` is on, backed by `TomlBackedUserPreferences` (`app/src/settings/init.rs:330-352`); private settings in the platform-native store. Settings are declared with `define_settings_group!` (`crates/settings/src/macros.rs`; example `app/src/settings/ai.rs:1211`) and each has a `surface:` (`SettingSurfaces::GUI|TUI|ALL`, `crates/settings/src/lib.rs:245`). Schema entries are collected with `inventory` (`crates/settings/src/schema.rs:52`).
- `WarpConfig` singleton (`app/src/user_config/mod.rs:90`) loads themes, workflows, launch configs, tab configs (`FeatureFlag::TabConfigs`), custom model routers; `native.rs:31 new` loads async; `native.rs:89 handle_warp_managed_paths_event` re-loads whichever directory changed and emits `WarpConfigUpdateEvent::{TabConfigs, TabConfigErrors, Settings, ...}` (`mod.rs:55-80`). Tab configs are user-scoped only (`tab_configs_dir()` = `data_dir()/tab_configs`, `mod.rs:215`); no project-level tab configs were found.

### 1.6 Existing readers of `.cursor/` and `.claude/`

- `.cursor/skills` and `.claude/skills`: skill providers (`skill_provider.rs:117-128`), watched at home and project level.
- `~/.claude.json` and project `.mcp.json`: MCP provider `Claude` (`app/src/ai/mcp/mod.rs:550,561`).
- `~/.claude/plugins/installed_plugins.json` and `~/.claude/settings.json` (`extraKnownMarketplaces`): read by `ClaudeCodePluginManager` (`app/src/terminal/cli_agent_sessions/plugin_manager/claude.rs:270-347`); honors `CLAUDE_CONFIG_DIR` (`:354`). `~/.codex/config.toml` plugin keys: `codex.rs:358`.
- Nothing reads `.cursor/hooks.json`, `.claude/settings.json` `hooks`, or Codex `hooks` today (grep for `hooks.json`, `"hooks"` in `app/src`, `crates/ai`, `crates/mcp` found only the CLI-agent plugin instructions text). **Not found.**

### 1.7 How each surface gets project config

All surfaces go through `initialize_app` (`app/src/lib.rs:1495`), driven by `LaunchMode` (`app/src/lib.rs:385`: `App`, `CommandLine` (= Oz cloud agent / `warp agent run`), `Test`, `RemoteServerProxy`, `RemoteServerDaemon`, `Tui`). Singletons registered in that function: `DirectoryWatcher` + force-included skill paths (`:2037-2049`), `DetectedRepositories` (`:2050`), `HomeDirectoryWatcher` (`:2052`), `RepoMetadataModel` with standing-query definitions (`:2069-2086`), `FileMCPWatcher`/`FileBasedMCPManager` (`:2386-2387`), `SkillManager` (`:2405`), `ProjectContextModel` + `index_global_rules` (`:2623-2633`), `PersistedWorkspace` (`:2639`). Differences:

- **GUI**: roots come from terminal cwd navigation (`view.rs:12291`). SettingsMode Gui, config under `~/.warp*`.
- **TUI**: same pipeline; roots from `terminal_session_view.rs:2242`; SettingsMode Tui → `~/.warp_cli*` settings + `tui_mcp_config_file_path`; `should_watch_repository` ignores `CloudEnvironmentPrep` in Tui mode (`file_mcp_watcher.rs:705`). The TUI also emits OSC 777 CLI-agent events itself (`CLIAgent::WarpTui`, `listener/mod.rs:71`).
- **Oz cloud agent / remote child (run_agents remote)**: `LaunchMode::CommandLine` running `AgentDriver` (`app/src/ai/agent_sdk/driver.rs`). Repos are cloned by environment prep (`environment.rs`), registered via `register_cloned_repo` (`:1332`), then skills are read synchronously from the repo tree (`agent_setup.rs:416-428`) and `SkillManager::set_cloud_environment(true)` widens scope. Extra inputs: `WARP_SKILL_DIRS`, `--skill` specs (`load_global_skills`, `:437`), and an agent config file (`app/src/ai/agent_sdk/config_file.rs:18 AgentConfigSnapshotFile`, keys: name, environment_id, runner_id, model_id, base_prompt, mcp_servers, host, computer_use_enabled — **no hooks key**; precedence CLI > file > default at `:161`). Remote children are launched from a parent via `prepare_remote_child_launch` (`app/src/ai/orchestration/remote_child.rs`) → `AIClient::spawn_agent` (`app/src/server/server_api/ai.rs:1377`); the child is a fresh Oz run so it inherits nothing from the parent's filesystem except what the environment/repo provides (inferred from `RemoteChildLaunchConfig`, `remote_child.rs:42`).
- **Local orchestration children (run_agents local)**: same process, new hidden conversation/pane (`StartAgentExecutor`, `app/src/ai/blocklist/action_model/execute/start_agent.rs:56`), so they share every singleton above; cwd comes from the child's working dir.
- **Remote SSH sessions**: project rules and skills on the remote host arrive as `RepoMetadataModel` remote snapshots (`app/src/lib.rs:2088-2107`) and are read through `ProjectRuleContentReader`/`read_remote_text_file_contents`; `LocalOrRemotePath` carries host identity everywhere. A hook runner would have to decide whether remote hooks execute remotely (via `RemoteCommandExecutor`) — nothing exists for that.

Conclusion: confirmed — parsing/eval plumbing is shared; the surface differences are root discovery triggers, home-config directories (Gui vs Tui `SettingsMode`), and cloud-only extra inputs.

### 1.8 Existing "config source" abstractions a hooks loader could imitate

- `SkillProviderDefinition` static list + `SkillProvider` enum (`skill_provider.rs:71,106`) — path-suffix registry with precedence.
- `MCPProvider::{home_config_path, project_config_path}` (`app/src/ai/mcp/mod.rs:547,557`) + `FileMCPWatcher` + `FileBasedMCPManager` — the complete "watch provider config files at home and repo root, parse, diff, emit" loop. This is the most directly reusable template for `.cursor/hooks.json` + `~/.cursor/hooks.json`.
- `StandingQueryDefinitions` (`standing_queries.rs:13`) — for discovering files anywhere in the tree (needed if hooks may live in nested dirs; Cursor's spec is root-only, so the MCP template probably suffices).
- `RepositorySubscriber` trait (`crates/repo_metadata/src/repository.rs:29`) — per-repo filesystem subscription with `on_scan`/`on_files_updated`.
- No trait like `trait ConfigSource { fn home_path(); fn project_path(); fn parse(); }` exists. **Not found.**

---

## Section 2 — File watching

- Crate: `crates/watcher` wrapping `notify-debouncer-full` (`crates/watcher/Cargo.toml:15`). `BulkFilesystemWatcher` (`crates/watcher/src/lib.rs:149`) runs a background thread owning the debouncer (`:160`); `register_path(path, WatchFilter, RecursiveMode)` (`:218`) and `unregister_path` (`:197`) return futures; raw events are coalesced into `BulkFilesystemWatcherEvent { added, modified, deleted, moved }` (`:98`) by `deduplicate_and_merge_raw_notifier_events` (`:283`), which also handles Windows `ModifyKind::Any` (`:296`) and macOS trash/rename quirks (`:320-359`). Empty paths are rejected to avoid a macOS crash (`:140`). Native only (`ai` crate pulls `watcher` under `cfg(not(wasm32))`, `crates/ai/Cargo.toml`).
- Scoping / debounce: every owner creates its own `BulkFilesystemWatcher` with its own debounce: `DirectoryWatcher` 500 ms (`crates/repo_metadata/src/watcher.rs:27,62`), `HomeDirectoryWatcher` 500 ms non-recursive on `$HOME` (`crates/watcher/src/home_watcher.rs:10,29-35`), `WarpManagedPathsWatcher` 500 ms (`app/src/warp_managed_paths_watcher.rs:19`).
- Repo watches: `DirectoryWatcher` (singleton, `watcher.rs:36`) maps `StandardizedPath` → `Repository` model; `add_directory[_with_git_dir]` (`:268,278`); watching starts on the first `Repository::start_watching(mode, subscriber)` (`repository.rs:387`) with `RepositoryWatchMode::{FilesystemOnly, GitRepository}` (`:54`); the recursive watch uses a gitignore-aware descend filter plus `force_included_paths` so gitignored `.agents/skills` still gets watched (`watcher.rs:119,366-376`). Linux lazy roots are watched non-recursively to avoid inotify exhaustion (`local_model.rs:1169-1178`).
- Examples of watched config files: `settings.toml` (`user_config/native.rs:162`), `tab_configs/*.toml` (`:130`), `themes/`, `launch_configurations/`, `custom_model_routers/` (`:97-160`), `~/.warp/.mcp.json` (`warp_managed_paths_watcher.rs:339-358`), `~/.agents/AGENTS.md` (`global_rules.rs`), `~/.claude.json`/`~/.codex/config.toml` via `FileMCPWatcher` home-subdir watchers (`file_mcp_watcher.rs:44,157`).
- Helpers: `repository_update_touches_path/prefix`, `filter_repository_update_by_prefix` (`warp_managed_paths_watcher.rs:101-116`).

---

## Section 3 — Subprocess execution outside the PTY

### 3.1 Primitives

- `command` crate (`crates/command/src/lib.rs`): `command::blocking::Command` (std wrapper) and `command::r#async::Command` (`async-process` wrapper, `crates/command/src/async.rs:15`, native only `lib.rs:9`). `new_with_session` (setsid, unix, `:54`), `new_with_process_group` (`:81`; unix only, Windows TODO at `:90`). On Windows it sets `CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB` (`:100-109`). Defaults: `spawn()`/`status()` null all stdio, `output()` pipes stdout/stderr (`:363-437`). WSL program translation (`crate::wsl::translate_program_for_spawn`). Clippy bans the raw types (`.clippy.toml:5-9`).
- `std::process::Command` direct uses are confined to `command` internals, build scripts, and `app/src/ai/agent_sdk/driver/git_credentials.rs` (allowed exception). `tokio::process::Command` is used only inside `crates/mcp/src/runtime.rs:228` (rmcp requires tokio) and `crates/computer_use`.
- Timeouts/cancellation: no shared helper (grep for `fn timeout|with_timeout` in `warp_core`, `warpui_core`, `warp_util` → not found). Pattern used by LSP: `futures::select!` against `warpui_core::r#async::Timer::after`, then SIGTERM + 2 s grace + `kill()` (`crates/lsp/src/transport.rs:149-184`). `LocalCommandExecutor` relies on `kill_on_drop(true)` plus SIGKILL of the whole process group on cancel/drop (`local_command_executor.rs:17-48, 106-133, 318-341`); no wall-clock timeout.

### 3.2 Running in the user's shell environment

- `LocalShellState` singleton (`app/src/terminal/local_shell/mod.rs:20`, `cfg(feature = "local_tty")`): at startup runs the preferred shell as a login shell with `echo $PATH` (`:110-129`) and caches `path_env_var`; `get_interactive_path_env_var` (`:149`) lazily captures the interactive-shell PATH with sentinels (`:244-258`). This is the only "capture the user's login-shell environment" mechanism; it captures **PATH only**, not the full env. **Not found**: a full login-shell environment snapshot.
- `LocalCommandExecutor` (`local_command_executor.rs:186`): `execute_local_command` runs `<shell> -f|--norc|--no-config|-NoProfile -c <cmd>` (`:202-224`); `execute_local_command_in_login_shell` runs `<shell> -l -c <cmd>` (`:226-253`); both accept `current_directory_path` and an env map (`:306-313`), always `HOME` + optional `PATH` when called through `local_shell::execute_command` (`local_shell/mod.rs:204-242`). Output is collected via `child.output()` into `warp_completer::completer::CommandOutput` (stdout/stderr bytes + status). Windows: PowerShell may be swapped for `cmd.exe /Q /C` (`:266-281`). Process group registration enables `cancel_active_commands` (`:373`).
- `CommandExecutor` trait (`app/src/terminal/model/session/command_executor.rs:68`) abstracts local / remote-SSH (`RemoteCommandExecutor`) / WSL / MSYS2 / in-band executors behind one `execute_command(command, shell, cwd, env, options)`; factory `new_command_executor_for_session` (`:95`). This is the natural seam if hooks must run where the session runs (local vs SSH host).

### 3.3 Concrete examples

- **MCP stdio servers**: `mcp::runtime::spawn_server` (`crates/mcp/src/runtime.rs:188`). Uses rmcp `TokioChildProcess` over `tokio::process::Command` (`:227-250`); cwd from config `cwd_parameter` else inherits Warp's process cwd (`:230`, see error text `:253-260`); env = Warp process env + per-server `static_env_vars` (empty values skipped, `:233-242`); Windows wraps in `cmd.exe /c` so PATH resolution follows Windows rules (`:205-214`) and sets `CREATE_NO_WINDOW` (`:246`); stderr forwarded line-by-line to a `SimpleLogger` file (`:272-294`). No login-shell PATH injection is visible here — **inferred** that MCP servers inherit whatever PATH the Warp process has (which may be minimal when launched from Finder; this is exactly what `LocalShellState` exists to fix for other paths).
- **git**: `warp_util::git::run_git_command[_with_env]` (`crates/warp_util/src/git.rs:8,16`): `command::r#async::Command::new("git")`, `current_dir(repo_path)`, env `GIT_OPTIONAL_LOCKS=0` + optional `PATH`, `kill_on_drop(true)`, piped stdout/stderr, exit-code 1 tolerated for diffs (`:50`). WSL UNC cwd is rerouted through `wsl.exe --distribution ... --cd ... --exec` with `WSLENV` (`:63-96,113-152`). Sync variant `app/src/util/git.rs:19`.
- **Codebase indexing / ripgrep**: `crates/warp_ripgrep` (not read in detail); `app/src/search/files/model.rs` uses `command::r#async`.
- **LSP (JSON over stdio)**: `ProcessTransport` (`crates/lsp/src/transport.rs:18`): pipes all three stdio, background task drains stderr (`:61-90`), `Content-Length` framing (`:103-147`). Closest existing "send JSON on stdin, read JSON on stdout" code.
- **CLI agent plugin install**: `run_cli_command_logged` (`app/src/terminal/cli_agent_sessions/plugin_manager/mod.rs:96`) → `LocalCommandExecutor::execute_local_command_in_login_shell` with PATH from `plugin_manager_for_with_shell(..., path_env_var)` (`:250`) — a good example of "run a user-space CLI with the user's PATH and collect a log".
- **Warp CLI / oz**: the `warp_cli` crate (`crates/warp_cli`) only defines clap args; `app/src/ai/agent_sdk/mod.rs:182 run` dispatches. The warp binary itself is the CLI (`LaunchMode::CommandLine`). I did not find an `oz-dev` binary in this repo (**not found**; `rg oz-dev` not run beyond `app/src` module listing).
- **Skill repo clone**: `app/src/ai/skills/resolve_skill_spec.rs:180` (`AsyncCommand::new("git")`, `current_dir(working_dir)`).

### 3.4 Platform / target gating

- `command::r#async` and all spawning code are `cfg(not(target_family = "wasm"))`; `warp_util::git` provides wasm stubs (`git.rs:187-199`); `ai` and `repo_metadata` gate filesystem code on the `local_fs` cargo feature (`app/Cargo.toml:798-805` fans it out). The wasm CI job runs `cargo clippy --target wasm32-unknown-unknown` on the default workspace members (`.github/workflows/ci.yml:789`), which includes `app` and therefore every crate it depends on; new code must compile for wasm or be gated.
- Windows specifics: no-window flags, `cmd.exe` wrapping (MCP, `LocalCommandExecutor::CmdExe`), process-group kill is unix-only (`local_command_executor.rs:47`), WSL path translation (`command::wsl`, `warp_util::git`).

### 3.5 Sandboxing / risk classification for Warp-spawned processes

- **Not found**: any sandboxing, allowlist, or risk classification applied to MCP servers, git, LSP, or plugin-install processes. They run with Warp's own privileges. The agent's command-permission system (`BlocklistAIPermissions`, `app/src/ai/blocklist/permissions.rs:134`; allow/deny lists `:376-460`) applies only to agent-requested shell commands and MCP tool calls, not to processes Warp starts for itself.
- Environment-level isolation detection only: `warp_isolation_platform::IsolationPlatformType` (`crates/isolation_platform/src/lib.rs:33`; Docker, DockerSandbox, Kubernetes, namespace) and the local Docker sandbox shell starter (`app/src/terminal/local_tty/docker_sandbox.rs`). A hook runner would need its own policy (e.g. trust prompt for project-scoped hooks).

---

## Section 4 — Crate layering and placement

### 4.1 Where things live

- Agent loop / turn driver: `app/src/ai/blocklist/controller.rs`, `app/src/ai/agent/conversation_driver.rs` (`ConversationDriver`, `:20`, `executes_tool_calls_locally` `:33`), `app/src/ai/agent/conversation.rs`. Tool queueing and execution: `BlocklistAIActionModel` (`app/src/ai/blocklist/action_model.rs:225`; `execute_action` `:814`, `queue_actions` `:968`, `apply_finished_action_result` `:1073`) with per-tool executors under `app/src/ai/blocklist/action_model/execute/` dispatched exhaustively on `AIAgentActionType` in `execute.rs:510-593` (`preprocess_action`) and `action_phase` (`:452`). Cloud-side equivalent driver: `app/src/ai/agent_sdk/driver.rs` (`AgentDriver`) + `driver/terminal.rs`.
- Tool/action definitions (portable data): `crates/ai/src/agent/action/mod.rs:38 AIAgentActionType`, `crates/ai/src/agent/action_result/mod.rs:20 AIAgentActionResultType`.
- Permissions: `app/src/ai/blocklist/permissions.rs:134 BlocklistAIPermissions` (profiles via `AIExecutionProfilesModel`).
- MCP client: `crates/mcp` (rmcp runtime, OAuth, tool_call); managers in `app/src/ai/mcp/`.
- Project rules + skills data/parsing: `crates/ai` (`project_context`, `skills`); watchers/managers in `app/src/ai/skills`, `app/src/ai/mcp`.
- Repo detection + fs standing queries: `crates/repo_metadata`. FS watcher: `crates/watcher`. Subprocess: `crates/command`. Paths/flags/telemetry: `crates/warp_core`, `crates/warp_features`. Settings: `crates/settings` + `app/src/settings`.

### 4.2 Dependency direction rules

From the `extract-type-from-crate` skill: never introduce upward deps into `warp` (the `app` crate); portable data/serde/protocol conversions go to lower crates (`ai` or below); anything touching session/terminal/UI/app state stays in `app`; watch orphan-rule coherence when moving types. Observed graph (from `Cargo.toml`s): `app` → `ai` → {`repo_metadata`, `warp_core`, `warp_util`, `warpui_core`, `warpui_extras`, `persistence`, `watcher` (native)}; `app` → `mcp` → {`warp_core`, `warpui`}; `repo_metadata` → `watcher`; `command` → only `warp_errors` + `async-process`. `warp_tui` depends on `warp` (the app crate) with `features = ["tui", ...]` (`crates/warp_tui/Cargo.toml:73`), so anything placed in `app/src/ai/...` is reachable from the TUI, and the Oz cloud agent is the same binary (`LaunchMode::CommandLine`).

### 4.3 wasm

- Must build for `wasm32-unknown-unknown`: `app` and its whole dependency closure (`ci.yml:789` runs clippy on default members: app, channel_versions, command, editor, graphql, markdown_parser, sum_tree, warpui, warp_completer, warp_terminal, warp_util). Crates do this by cfg-gating: `ai` keeps `local_fs`-gated modules and `cfg(not(wasm32))` deps (`crates/ai/Cargo.toml:70-82`); `mcp` gates child-process transport (`crates/mcp/Cargo.toml:36-38`); `command::r#async` is native-only; `app/src/util/repo_detection.rs:81-89` shows the wasm stub pattern; `app/src/warp_managed_paths_watcher.rs:221-236,400-410` shows a stub model pattern.
- Native-only: `watcher`, `isolation_platform` internals, `lsp` transport, `local_tty` feature code.

### 4.4 Placement suggestion (inferred, for the architect to weigh)

- Hook config types + parsers (Cursor/Claude/Codex formats, event names, matchers) → `crates/ai` (e.g. `crates/ai/src/hooks/`), alongside `skills/` and `project_context/`; pure serde, wasm-safe. If the hook schema must be shared with warp-server protos, put the wire types in `ai` and keep conversions there (orphan-rule guidance from the skill).
- Discovery/watching (`HookConfigWatcher` modelled on `FileMCPWatcher`) and the hook runner (subprocess via `command::r#async` or `LocalCommandExecutor`, needs `LocalShellState`, sessions, permissions) → `app/src/ai/hooks/`; `cfg(feature = "local_fs")` / `not(wasm)` gated with stubs. That makes it reachable from GUI, TUI (via `warp` dep), and Oz (same binary) with no new crate. A separate crate is only warranted if something below `app` (e.g. `agent_sdk` is already in `app`) needs it — nothing does today.
- Interception points are in `app` regardless: `BlocklistAIActionModel::execute_action` / `preprocess_action` (`execute.rs:494`) for pre/post tool use, `BlocklistAIController` for prompt submit / turn end, and `AgentDriver` for the cloud lifecycle.

### 4.5 Registry patterns to imitate

- Static ordered table: `SKILL_PROVIDER_DEFINITIONS` (`skill_provider.rs:106`).
- Enum + per-variant path methods, iterated with `strum::EnumIter`: `MCPProvider` (`app/src/ai/mcp/mod.rs:514-564`), `GlobalRuleSource` (`global_rules.rs:18-45`).
- Trait object per external agent: `CliAgentPluginManager` + factory `plugin_manager_for_with_shell` (`plugin_manager/mod.rs:145,250`); `CLIAgentSessionHandler` + `create_handler` (`listener/mod.rs:14,59`).
- Link-time registries via `inventory`: settings schema (`crates/settings/src/schema.rs:52`) and telemetry events (`crates/warp_core/src/telemetry.rs:132`).
- Versioned parser table: `VERSIONED_PARSERS` (`cli_agent_sessions/event/mod.rs:82`).

---

## Section 5 — Settings, feature flags, telemetry, command palette (entry points only)

- Feature flag: add a variant to `FeatureFlag` in `crates/warp_features/src/lib.rs:8` (re-exported by `crates/warp_core/src/features.rs:1`); optionally list in `DOGFOOD_FLAGS` (`:1028`), `PREVIEW_FLAGS` (`:1089`), `RELEASE_FLAGS` (`:1094`), `RUNTIME_FEATURE_FLAGS` (`:1110`, toggleable from the debug menu); check with `FeatureFlag::X.is_enabled()` (`:1113`). Cargo-feature wiring for GUI builds: `app/Cargo.toml` `[features]` + `app/src/lib.rs` cfg list (see `.agents/skills/add-feature-flag/SKILL.md`). Prefer runtime checks so the TUI gets it for free.
- Toggleable setting: declare in a `define_settings_group!` (e.g. `app/src/settings/ai.rs:1211`, with `surface:` and `toml_path:`); add a widget on the relevant settings page (`app/src/settings_view/*_page.rs`, rules in `.agents/skills/gui-settings-ui/SKILL.md`: one widget per setting, title in the `PageType` slot, gate static flags at page-build time); add a command-palette enable/disable pair with `ToggleSettingActionPair::new(description, action, context, flag)` (`app/src/settings_view/mod.rs:807,843`; examples `warp_agent_page.rs:136-246`), define the context flag constant in `settings_view::flags` (`mod.rs:624-675`) and set it in `Workspace::keymap_context` (`app/src/workspace/view.rs`).
- Telemetry: feature-local enum implementing `warp_core::telemetry::TelemetryEvent` + `TelemetryEventDesc`, registered with `warp_core::register_telemetry_event!`, sent via `send_telemetry_from_ctx!` / `send_telemetry_from_app_ctx!` (`.agents/skills/add-telemetry/SKILL.md`; reference impl `app/src/antivirus/telemetry.rs`; skills example `app/src/ai/skills/telemetry.rs`).

---

## Section 6 — Side LLM calls

Verified: every auxiliary model call goes through warp-server; the client never calls a model provider directly (no `api.anthropic.com` / `api.openai.com` URLs outside secret/harness tests). Model selection is server-side per feature (`AIClient::get_feature_model_choices`, `app/src/server/server_api/ai.rs:1314`; `ModelsByFeature { agent_mode, coding, cli_agent, computer_use }`). Existing calls:

- GraphQL mutations on `ServerApi` (`app/src/server/server_api/ai.rs`): `generate_commands_from_natural_language` (`:2016`, `GenerateCommands`), `generate_dialogue_answer` (`:2054`, `GenerateDialogue` — closest thing to "prompt in, text out"), `generate_metadata_for_command` (`:2109`). All with `AI_ASSISTANT_REQUEST_TIMEOUT_SECONDS`.
- REST under `{server_root}/ai/...`: `/ai/generate_block_title` (`app/src/server/server_api/block.rs:141-162`, request `app/src/ai/generate_block_title/api.rs`), `/ai/generate_input_suggestions` (`app/src/server/server_api.rs:1186`), `/ai/relevant_files` (`:1216`), `/ai/generate_am_query_suggestions` (`:1246`), `/ai/predict_am_queries` (`:1285`), `/ai/transcribe` (`:1314`), `/ai/generate_code_review_content` (`server_api/ai.rs:3525`).
- Conversation title/rename is a server mutation on an existing conversation (`app/src/ai/conversation_rename.rs`, `AIClient::rename_conversation` `:1398`), not a standalone model call. Context summarization is driven inside the main agent stream (`FeatureFlag::SummarizationViaMessageReplacement`), not a separate client call (inferred).
- Risk classification of commands is produced by the main agent model as part of the tool call (`risk_category` on run_shell_command) — no separate classifier call found.

Implication: Cursor prompt-based hooks (LLM judges a natural-language condition) need either (a) a new warp-server endpoint (`/ai/evaluate_hook_prompt` or similar, modelled on `generate_shared_block_title`), or (b) reuse of `GenerateDialogue`, which is a legacy chat path with no model pinning. There is no client-side "cheap model" selector. BYO/custom endpoints (`app/src/ai/custom_endpoints.rs`, `custom_model_routers.rs`) are configuration forwarded to the server, not a local inference path (inferred).

---

## Section 7 — Adjacent existing system: CLI-agent notification plugins (half page)

What it is: Warp detects third-party CLI agents running in a PTY (`CLIAgent` enum, `app/src/terminal/cli_agent.rs`) and listens for OSC 777 notifications whose title is the sentinel `warp://cli-agent` (`crates/warp_core/src/cli_agent_protocol.rs:5`, wire struct `CLIAgentNotification` `:19`). Events are parsed by versioned parsers (`app/src/terminal/cli_agent_sessions/event/mod.rs:82,87`) into `CLIAgentEvent` (`:68`) with `CLIAgentEventType` (`:14`: SessionStart, PromptSubmit, ToolComplete, Stop, StopFailure, PermissionRequest, PermissionReplied, QuestionAsked, NeedsInput, IdlePrompt). Per-agent handlers live in `listener/mod.rs` (`CLIAgentSessionHandler` `:14`, `is_agent_supported` `:39`, `create_handler` `:59`); Codex/Grok also accept a plain-text OSC 9 fallback.

Who emits the events: agent-owned plugins/hooks that Warp installs but does not author in-repo. `CliAgentPluginManager` (`plugin_manager/mod.rs:145`) implementations: Claude (`claude.rs`: `claude plugin marketplace add warpdotdev/claude-code-warp` + `claude plugin install warp@claude-code-warp`, `:98-108`; state read from `~/.claude/plugins/installed_plugins.json` and `~/.claude/settings.json`, `:270-347`), Codex (`codex.rs`, reads `~/.codex/config.toml`, `:358`), Gemini (`gemini.rs`, extensions dir), OpenCode (`opencode.rs`); others return `None` (`mod.rs:288-306`). Install commands run through `LocalCommandExecutor` in a login shell with the captured PATH (`run_cli_command_logged`, `mod.rs:96`). Reviewers explicitly rejected an in-client hook writer for Grok (`specs/GH15768/TECH.md:23,80-90`); the Claude-style HookConfig shape (`{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "..."}]}]}}`) exists only as a research note in `specs/GH15768/warp-hooks-sample.md`. There is **no** Rust `HookConfig` type to reuse.

What not to confuse: this system is the *producer* side for other agents (Warp consumes their hook output over the terminal). The new feature is the *consumer* side for Warp's own agent (Warp runs the user's hooks). Reusable pieces: the event-name vocabulary and payload fields (`CLIAgentEventPayload`, `event/mod.rs:40`) as a cross-check against Cursor/Claude event names; `warp_core::cli_agent_protocol` env vars (`WARP_CLI_AGENT_PROTOCOL_VERSION`, `WARP_CLIENT_VERSION`) as precedent for advertising a protocol version to child processes; the `CliAgentPluginManager` factory shape.

Surfacing hook output to the user: `AgentNotificationsModel` (`app/src/ai/agent_management/agent_management_model.rs:26`) is the notifications mailbox (categories/origins in `agent_management/notifications.rs`; fed by conversation status and CLI session events, gated by `FeatureFlag::HOANotifications`); ephemeral toasts via `ToastStack::add_ephemeral_toast` (`app/src/workspace/toast_stack.rs:10`); orchestration inter-agent inbox via `OrchestrationEventService` (`app/src/ai/blocklist/orchestration_events.rs:99`) and `send_message`/`list_messages` tools. A hook's `user_message`/`agent_message` (Cursor semantics) would map to a toast/notification and to injected conversation content respectively; nothing exists for the latter.

---

## Section 8 — Summary

### Reusable building blocks

- `DetectedRepositories::get_root_for_path` / `detect_possible_git_repo` (`crates/repo_metadata/src/repositories.rs:228,61`; wrapper `app/src/util/repo_detection.rs:46`) — canonical project root (git root), already triggered on every cwd change in GUI, TUI, and cloud prep.
- `RepoMetadataEvent::StandingQueryResultsUpdated` + `StandingQueryDefinitions` (`crates/repo_metadata/src/standing_queries.rs:13,110`) — add a file-name pattern and get create/modify/delete deltas for free, local and remote.
- `FileMCPWatcher` / `FileBasedMCPManager` (`app/src/ai/mcp/file_mcp_watcher.rs:146`, `file_based_manager.rs:166`) — template for "home + repo-root provider config files, parse with diagnostics, generation-tagged re-parse, startup ScanComplete gate".
- `MCPProvider`/`SkillProvider` enums (`app/src/ai/mcp/mod.rs:515`, `crates/ai/src/skills/skill_provider.rs:31`) — provider-directory registries that already include Cursor/Claude/Codex.
- `ProjectContextModel::find_applicable_rules` (`crates/ai/src/project_context/model.rs:591`) — layered global>project precedence model and the `ProjectRuleContentReader` injection for remote reads.
- `BulkFilesystemWatcher` / `DirectoryWatcher` / `HomeDirectoryWatcher` / `WarpManagedPathsWatcher` (`crates/watcher/src/lib.rs:149`, `crates/repo_metadata/src/watcher.rs:36`, `crates/watcher/src/home_watcher.rs:18`, `app/src/warp_managed_paths_watcher.rs:231`) — debounced, platform-normalized file watching.
- `command::r#async::Command` (`crates/command/src/async.rs:15`) — the only sanctioned spawn API; process-group/session variants; Windows flags handled.
- `LocalCommandExecutor::execute_local_command[_in_login_shell]` + `LocalShellState::get_interactive_path_env_var` (`local_command_executor.rs:202,226`; `local_shell/mod.rs:149`) — run with the user's shell + PATH, cwd, env map, piped output, group kill on cancel.
- `CommandExecutor` trait (`command_executor.rs:68`) — abstraction over local/SSH/WSL execution contexts.
- `lsp::ProcessTransport` (`crates/lsp/src/transport.rs:18`) — stdin/stdout JSON framing + stderr drain + timeout/SIGTERM/kill shutdown pattern.
- `BlocklistAIPermissions` (`app/src/ai/blocklist/permissions.rs:134`) — per-profile allow/deny plumbing a hook policy (block/allow/ask) could mirror.
- `AgentNotificationsModel` / `ToastStack` (`agent_management_model.rs:26`, `toast_stack.rs:10`) — user-facing notification surfaces.
- `ToggleSettingActionPair`, `define_settings_group!`, `FeatureFlag`, `register_telemetry_event!` — standard feature scaffolding (§5).
- `warp_core::cli_agent_protocol` (`crates/warp_core/src/cli_agent_protocol.rs`) — event vocabulary and env-var versioning precedent.

### Gaps (do not exist yet)

- No hook config schema/parser (Cursor `hooks.json`, Claude `settings.json` `hooks`, Codex) anywhere in Rust; no "HookConfig" type.
- No generic config-source trait; discovery is per-feature (rules via standing queries, MCP via provider file list, workflows via `git2::discover`, tab configs user-only). A hooks loader must pick one to imitate.
- No subprocess timeout/cancellation helper; no "spawn with JSON on stdin and parse JSON stdout" utility (LSP transport is the nearest, but framed, long-lived).
- No full login-shell environment capture — only PATH (and HOME) are injected. Hooks expecting the user's exported env (e.g. API keys) would not see it unless run via `execute_local_command_in_login_shell` (which sources rc files on each call, slower).
- No sandboxing, trust prompt, or risk classification for processes Warp spawns itself; project-scoped hooks from an untrusted repo would run with full user privileges by default.
- No client-callable generic/cheap LLM evaluation endpoint and no client-side model picker for auxiliary calls; prompt-based hooks need server work.
- No mechanism to inject hook output into the running conversation as agent-visible context, or to block/modify a queued tool call from outside the action executors (the hooks would be new branches in `execute.rs` / the controller).
- No remote-host hook execution story: `CommandExecutor` can run commands over SSH, but config discovery for remote repos only yields paths/contents through `RepoMetadataModel` snapshots, and `FileMCPWatcher` is local-only.
- `AgentConfigSnapshotFile` (cloud agent config file) has no `hooks` key; Oz runs would rely purely on repo-level files plus whatever env/skill-dir style override is added.
- `RepoDetectionSource::CloudEnvironmentPrep` is ignored by `FileMCPWatcher` in TUI mode (`file_mcp_watcher.rs:705`); any new watcher must decide the same matrix explicitly.
