# Per-Directory Command History — Tech Spec
Product spec: `specs/GH5336/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/5336
## Context
All up-arrow history surfaces (GUI inline history menu, GUI history search, TUI history menu) source their rows from one shared combiner, so the scoping change lands in exactly one place.

Relevant code:
- `app/src/terminal/history.rs` — the `History` singleton (`:161`). `HistoryEntry` already records the working directory: `pub pwd: Option<String>` (`:235-256`, pwd at `:238`). The pwd is populated for commands executed in Warp from the active block (`for_session_command` `:300-324`, pwd from `active_block.pwd()` at `:311`), for restored blocks (`:326-341`) and completed blocks (`:343-358`). Entries that come only from the shell history file have no metadata (`command_only` `:269-285`); at load time they are enriched from the sqlite `commands` summary when a matching persisted command exists (`load_history_file_commands` `:610-643`, join at `:626-631`). `History::commands` (`:803`) already returns only entries visible to the session and its shell host (`collect_visible_commands_for_session` `:807-865`).
- `app/src/terminal/history/up_arrow.rs` — the shared combiner `History::up_arrow_suggestions_for_terminal_surface` (`:111-174`) with its `UpArrowHistoryConfig { include_commands, include_prompts }` (`:16-38`). It builds the command stream with its existing filters (ignored suggestions `:134-138`, `include_agent_commands_in_history` `:122-128`) and the prompt stream with its gate (`:142-148`), then orders and de-duplicates via `sort_and_dedupe_suggestions` (`:40-76`).
- Consumers of the combiner:
  - GUI legacy/history-search path: `Input::collate_ai_and_command_history` (`app/src/terminal/input.rs:12469-12482`).
  - GUI inline history menu: `InlineHistoryMenuDataSource::run_query` (`app/src/terminal/input/inline_history/data_source.rs:244-348`), calling the combiner at `:105-110` (agent view) and `:270-280` (commands). The data source already holds `active_session: ModelHandle<ActiveSession>` (`:70`).
  - TUI: the owned adapter `tui_up_arrow_history` (`app/src/tui_export/history.rs:24-50`) wraps the combiner for `TuiPromptAndCommandHistoryMenuModel`, which also holds `active_session` (`crates/warp_tui/src/prompt_and_command_history_menu.rs:61`) and refreshes rows on open/keystroke (`:389`).
- Current directory source: `ActiveSession::current_working_directory()` (`app/src/terminal/model/session/active_session.rs:96`), kept in sync from block-metadata events (`:34-50`). This is the same value `TerminalView::pwd()` exposes (`app/src/terminal/view.rs:24650-24655`).
- Settings pattern: `define_settings_group!(InputSettings, ...)` (`app/src/settings/input.rs:36`); a comparable boolean setting with a `toml_path` is `completions_open_while_typing` (`:58-67`); the read pattern is `InputSettings::as_ref(app).<field>.value()` (e.g. `app/src/terminal/input.rs:12572-12576`).
- Action + default-binding pattern: `EditorAction` enum (`app/src/editor/view/mod.rs:996`) with input-specific precedent `InsertLastWordPrevCommand` — declared at `:1090`, given a default editable binding at `:855-861`, surfaced to the input as an `EditorEvent` and handled at `app/src/terminal/input.rs:11630`.

See product spec for user-visible behavior.
## Proposed changes
### 1. New setting in `InputSettings`
In `app/src/settings/input.rs`, add to the `InputSettings` group:
- `directory_scoped_history`: `bool`, default `false`, `SupportedPlatforms::ALL`, `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`, GUI surface, non-private, `toml_path: "terminal.input.directory_scoped_history"`, description along the lines of "Whether up/down arrow command history is filtered to the current directory."
Default off keeps today's behavior for existing users (product invariant 1) and makes this opt-in like the zsh plugin it emulates.
### 2. Extend `UpArrowHistoryConfig` with a directory scope
In `app/src/terminal/history/up_arrow.rs`, add a field to `UpArrowHistoryConfig` (`:16-38`):
- `directory_scope: Option<String>` — the current working directory to scope commands to; `None` means unscoped (global history).
`for_input_config` (`:25-37`) keeps deriving `include_commands`/`include_prompts` as today and sets `directory_scope: None`; callers that want scoping set the field explicitly. Keeping the scope in the config struct (rather than a new parameter) preserves the single-combiner design and avoids touching the call signature that GUI, GUI-inline, and TUI surfaces share.
### 3. Apply the filter in the shared combiner
In `up_arrow_suggestions_for_terminal_surface` (`:111-174`), when `config.directory_scope` is `Some(pwd)` and `config.include_commands` is true, filter the command stream (`:130-140`) to entries where `entry.pwd.as_deref() == Some(pwd.as_str())`. The filter composes with the existing ignored-suggestions and agent-command filters, and runs before `sort_and_dedupe_suggestions` so ordering and de-duplication operate on the scoped list (product invariant 6). The prompt stream (`:161-163`) is untouched (invariant 5). When `directory_scope` is `None`, behavior is exactly today's (invariant 1).
### 4. Callers supply the scope
Each caller reads the setting and the session's current directory at query time, passing `directory_scope = setting.then(|| cwd)`:
- `Input::collate_ai_and_command_history` (`app/src/terminal/input.rs:12469`): read `ActiveSession::current_working_directory()` from the input's active session handle. A `None` cwd yields `directory_scope: None`, which degrades to global history (invariant 7).
- `InlineHistoryMenuDataSource` (`data_source.rs:105-110` and `:270-280`): read the same accessor from `self.active_session` inside `run_query`, so a menu that stays open across a `cd` re-scopes on the next query.
- TUI: `tui_up_arrow_history` (`app/src/tui_export/history.rs:24`) gains the scope the same way — read it from the `active_session` the TUI menu already holds (`crates/warp_tui/src/prompt_and_command_history_menu.rs:61`) and thread it into the config at row-refresh time (`:389`).
Because all command history for a session already comes from that session's shell host (`history.rs:807-865`), pwd string equality is a same-host comparison by construction (invariant 10).
### 5. Toggle action and default binding
Follow the `InsertLastWordPrevCommand` precedent end to end:
- Add `EditorAction::ToggleHistoryDirectoryScope` to the enum (`app/src/editor/view/mod.rs:1090` area) and an `EditorEvent` variant surfaced to the input.
- Register an editable binding `editor_view:toggle_history_directory_scope` ("Toggle directory-scoped history") with `.with_key_binding("ctrl-g")` and context predicate `id!("EditorView") & !id!("IMEOpen")` (pattern at `:855-861`).
- Handle the event in the input (`app/src/terminal/input.rs:11630` area) by flipping `InputSettings::directory_scoped_history` (persisted, so the state survives restart and Settings stays the source of truth — invariant 9) and notifying so open surfaces re-render.
The toggle intentionally updates the persisted setting rather than transient per-view state: it keeps one source of truth, makes the zsh `^G` habit work, and lets users rebind or disable it in Keybindings settings.
### Tradeoffs
- Exact pwd string comparison can miss symlink-equivalent paths (for example `/tmp` vs `/private/tmp` on macOS). Blocks record the pwd the shell reports, which is stable within a shell session, so same-directory navigation matches in practice; canonicalization is deferred (follow-up).
- Histfile-only entries carry no pwd and are invisible in scoped mode (invariant 4). The sqlite summary join (`history.rs:626-631`) mitigates this for any command previously run in Warp, and `Ctrl+G` provides an instant escape hatch.
## Testing and validation
Each numbered invariant in `specs/GH5336/product.md` maps to at least one test or manual step below.
### Unit tests
New cases in `app/src/terminal/history/up_arrow_tests.rs` (the shared combiner's test file), following the existing fixture style:
- `directory_scope_filters_commands_to_matching_pwd` — commands with matching and non-matching pwds in history; scoped config returns only matching entries. Guards invariant 3.
- `directory_scope_excludes_entries_without_pwd` — a histfile-only entry (pwd `None`) is absent from scoped results but present in unscoped results. Guards invariant 4.
- `directory_scope_leaves_prompts_unfiltered` — with prompts included, prompts still appear under scoping. Guards invariant 5.
- `directory_scope_none_preserves_today_behavior` — `directory_scope: None` returns exactly the same list as before the change (assert against the pre-existing expectations in the file). Guards invariant 1.
- `directory_scope_preserves_ordering_dedup_and_filters` — scoped results keep other-session-before-current ordering, newest-occurrence dedup, ignored-suggestion exclusion, and the agent-command setting. Guards invariant 6.
- `directory_scope_unknown_current_directory_is_noop` — a scope built from a `None` cwd collapses to `None` and returns the full list. Guards invariant 7.
Data source coverage in `app/src/terminal/input/inline_history/data_source_tests.rs`:
- a scoped `run_query` returns only same-directory commands and surfaces the existing empty-result path when none match. Guards invariants 3 and 8.
TUI coverage in `crates/warp_tui/src/prompt_and_command_history_menu_tests.rs`:
- scoped rows via `tui_up_arrow_history` (matching and non-matching pwds), proving the TUI surface inherits the shared behavior. Guards invariant 3 on the TUI surface.
### Manual validation
Run with `./script/run` (the implementation PR must attach before/after screenshots per CONTRIBUTING):
- Settings shows the new toggle; default off reproduces today's menus (invariants 1-2).
- In directory `a`, run two commands; `cd b`, run one; Up in `b` lists only the `b` command; `Ctrl+G` reveals all three; `Ctrl+G` again re-scopes (invariants 3, 9).
- In a directory with no recorded commands, Up shows the "No history" empty state; typing shows "No matching history" (invariant 8).
- Agent Mode on: prompts still appear while scoped (invariant 5).
- Fresh session before first prompt: Up shows full history (invariant 7).
- Inside an alt-screen app (e.g. `less`), Up is forwarded and no menu opens (invariant 12).
- Remote/SSH session: scoping uses the remote cwd; local entries never appear (invariant 10).
- Accept a command from the scoped list: fills and executes; a workflow-linked command keeps its metadata (invariant 11).
## Risks and mitigations
- **Symlink/path-form mismatches** make some same-directory commands unmatched. Mitigation: exact match is a documented v1 limitation (product Non-goals); `Ctrl+G` fallback; canonicalization follow-up.
- **Sparse scoped lists for histfile-heavy users** (no pwd metadata on imported entries). Mitigation: the sqlite summary join enriches any command previously run in Warp; the toggle provides global fallback.
- **`Ctrl+G` binding friction** with emacs-style presets. Mitigation: the binding is editable in Keybindings settings; flagged as an open question in the product spec.
- **Config struct change ripples to three callers.** Mitigation: all callers are in-repo and the compiler enforces the update; the change keeps one combiner as the single source of truth, matching the cross-surface rationale used for the up-arrow menu work.
## Follow-ups
- Add a "This directory" filter chip to Command Search (Ctrl+R) by extending the history data source (`app/src/search/command_search/history/history_data_source.rs`) with the same scoping, surfaced through the existing `QueryFilter` chip UX.
- Canonicalize both pwds before comparison (see the `dunce::canonicalize` precedent at `app/src/terminal/view.rs:24683`) to handle symlinks and path-form differences.
- Verify and document real-time behavior across multiplexed windows (requested in the issue): the `History` singleton already shares new entries across sessions of the same host (`session_commands` keyed by `ShellHost`, `history.rs:175`), so another window's commands appear on the next menu open; consider a refresh event while menus stay open.
