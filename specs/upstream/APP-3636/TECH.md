# CLI Agent Rich Input: @ Context Technical Spec

## Summary
This spec covers implementing `@` context (files, folders, code symbols) in the CLI agent rich input composer. The approach reuses the existing AI context menu with mode-based category filtering, restricting it to files/folders and code symbols.

## Relevant Code
- `app/src/search/ai_context_menu/view.rs` — `AIContextMenu`, `get_categories_for_mode()`, `refresh_categories_state()`
- `app/src/terminal/input.rs` — `Input` view; handles `EditorEvent::AcceptAIContextMenuItem`, `InsertFilePath` action
- `app/src/terminal/cli_agent_sessions/mod.rs` — `CLIAgentSessionsModel`, `CLIAgentInputState`
- `app/src/search/ai_context_menu/files/data_source.rs` — `file_data_source_for_current_repo()`, `file_data_source_for_pwd()`
- `app/src/search/files/model.rs` — `FileSearchModel` (relies on `ActiveSession::path_if_local()` — local only)

## Current State
The AI context menu (`AIContextMenu`) already supports mode-based category filtering via `get_categories_for_mode()`. It takes flags like `is_ai_or_autodetect_mode`, `is_shared_session_viewer`, and `is_in_ambient_agent` to determine which categories to show. When only one category is available, `refresh_categories_state()` automatically skips the category picker and jumps to the search results view.

The file data sources (`file_data_source_for_current_repo`, `file_data_source_for_pwd`) use `ActiveSession::path_if_local()` and local `FileSearchModel`/`RepositoryMetadataModel` — these only work for local sessions, not SSH/remote.

The `InsertFilePath` handler in `EditorEvent::AcceptAIContextMenuItem` already handles file path insertion via `replace_at_symbol_with_text()`. In AI mode it inserts repo-relative paths; in terminal mode it computes the shorter of relative-to-cwd or absolute.

The `@` trigger in the editor fires when the user types `@` and the input is in AI mode. The CLI agent rich input calls `set_input_mode_agent`, so the `@` trigger should already work.

## Proposed Changes

### 1. Filter AI context menu categories for CLI agent input

**What changes**: `get_categories_for_mode()` gains an additional `is_cli_agent_input: bool` parameter. When true, it uses a **positive allowlist** to determine which categories to show: `RepoFiles`, `CurrentFolderFiles`, and `Code`. All other categories are excluded by default. This is safer than a blocklist because new categories added to the enum in the future won't accidentally leak into the CLI agent menu.

**Source of truth**: Rather than storing a duplicated flag on `AIContextMenuState`, callers should read `CLIAgentSessionsModel::is_input_open(terminal_view_id)` and pass the result into `get_categories_for_mode()`. The `AIContextMenu` needs to subscribe to `CLIAgentSessionsModel` events and call `refresh_categories_state()` when the input session changes — same pattern used elsewhere (e.g., `UseAgentToolbar` subscribes to `CLIAgentSessionsModel` and re-renders on change). This avoids stale duplicated state.

### 2. Path insertion: use repo-relative paths

**Insertion**: The `InsertFilePath` action in `EditorEvent::AcceptAIContextMenuItem` already handles file path insertion via `replace_at_symbol_with_text()`. In AI mode, this appends a trailing space after the inserted path (`format!("{text} ")`). For CLI agent input, use the same AI-mode behavior (repo-relative paths with trailing space). The trailing space is desirable — it lets the user continue typing the prompt naturally after the path.

### 3. Ensure @ trigger works in CLI agent mode

**Trigger gating**: The `@` trigger must work anywhere in the buffer, including after mode-switch prefixes like `!`. The CLI agent input already calls `set_input_mode_agent`, so the existing `@` detection logic should fire. Verify this works — if the `@` detection is gated differently, add an explicit check for `CLIAgentSessionsModel::is_input_open()`.

## End-to-End Flow
1. User opens CLI agent rich input (Ctrl-G or Compose button).
2. User types `@` anywhere in the buffer.
3. Editor detects `@` and opens the AI context menu.
4. `AIContextMenu` reads `CLIAgentSessionsModel::is_input_open()` = true, returns only file/folder and code symbols categories.
5. User types to filter, selects a file.
6. `InsertFilePath` action fires → `replace_at_symbol_with_text()` inserts the repo-relative path.
7. User presses Enter → buffer text (including the file path) is written to the PTY.

## Risks and Mitigations

### @ trigger not firing in CLI agent mode
The editor's `@` detection may be gated on AI mode. The CLI agent input does call `set_input_mode_agent`, so this should work, but needs verification. If gated differently, add an explicit check for CLI agent input being open.

### Mode-switch prefix interaction with @
When input starts with `!` (bash mode), the `@` trigger must still work at positions after the prefix. The existing `@` detection is position-based (tracks the byte offset of the `@` character) and should work regardless of preceding content. No special handling needed.

## Testing and Validation
- Verify `@` opens the context menu with files/folders and code symbols (no Warp-specific categories) in CLI agent rich input.
- Verify `@foo` filters files by name.
- Verify selecting a file inserts the repo-relative path as plain text.
- Verify `@` works after `!` prefix in the buffer.
- Verify all inserted content submits correctly to the PTY.
- Verify no regressions in normal Warp agent input (`@` context still works as before).

## Follow-ups
- Support `@` context in SSH/remote sessions (requires remote file discovery).
