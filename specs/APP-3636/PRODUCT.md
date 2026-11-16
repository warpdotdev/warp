# CLI Agent Rich Input: @ Context Product Spec

## Summary
Add support for `@` context (files, folders, and code symbols) in the CLI agent rich input composer — the input that appears when composing a prompt to send to a running CLI agent (Claude Code, Codex, Gemini CLI, etc.). Selected items are inserted as repo-relative paths.

## Problem
When users compose prompts for CLI agents through Warp's rich input (Ctrl-G or the Compose button), they cannot reference files or code symbols. Users must manually type file paths without any discovery or autocomplete. The normal Warp agent input supports `@` context, but the CLI agent input does not.

The core constraint is that CLI agent input writes plain text to a PTY — so `@` context must resolve to a plain text path that the CLI agent can interpret.

## Goals
- Let users attach file, folder, and code symbol paths via `@` in the CLI agent rich input.
- Insert repo-relative paths as plain text.
- Hide Warp-specific `@` context categories that CLI agents cannot interpret.

## Non-goals
- Supporting `@` context types other than files/folders and code symbols (e.g., Blocks, Workflows, Diff Sets, Notebooks). These are Warp-specific concepts that CLI agents cannot interpret.

## Figma
https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7001-18001&p=f&m=dev

## User Experience

### Trigger
User types `@` anywhere in the CLI agent rich input. This works regardless of position in the buffer — including when the input starts with a mode-switch prefix like `!` (bash mode in Claude Code) or `&` (background mode). The `@` trigger is not restricted to AI-mode input.

### Limitation
`@` context is only available in local sessions. It is not supported in SSH/remote contexts because file search and code symbol indexing rely on local filesystem access.

### Menu
The AI context menu opens showing the available categories: files/folders and code symbols. These are the only categories relevant for CLI agents — all Warp-specific categories (Blocks, Workflows, Notebooks, Diff Sets, Conversations, etc.) are hidden.

### Search
The user can type after `@` to filter file results, reusing the existing file search logic. For example, typing `@foo` filters to files matching "foo" (e.g., `footer.rs`, `foo_bar.py`). This is the same search behavior already implemented for the file data source in the AI context menu.

### Selection
When the user selects a file or folder, the repo-relative path is inserted into the input buffer as plain text followed by a trailing space (e.g., `src/foo.rs `). The trailing space lets the user continue typing immediately. CLI agents typically reference files relative to the project root, so repo-relative paths are concise and consistent with how these tools handle file references. If the file is outside the project tree, fall back to an absolute path. No special chip or token rendering is needed.

When the user is outside a git repository, the menu shows files from the current working directory (matching existing Warp behavior). Paths are relative to pwd in this case. This is the existing `CurrentFolderFiles` vs `RepoFiles` distinction in the AI context menu.

### Submission
The inserted file path is part of the plain text written to the PTY. No special processing is needed at submit time.

### Edge cases
- Paths with spaces should be inserted as-is (the CLI agent will interpret them in the context of the surrounding prompt text, not as shell arguments).
- If no files are found (e.g. empty directory), the menu should show its standard empty state.

## Success Criteria
- Users can type `@` in the CLI agent rich input, see file/folder and code symbol results, select one, and have the repo-relative path inserted as text.
- Warp-specific `@` context categories do not appear in the menu.
- This feature is only available in local sessions, not SSH/remote.

## Validation
- Open a CLI agent rich input while Claude Code is running. Type `@`, verify files/folders and code symbols categories appear (no Warp-specific categories), select a file, verify the repo-relative path is inserted.
- Verify `@foo` filters files by name.
- Verify `@` works after `!` prefix in the buffer.
- Verify that Warp-specific context categories (Blocks, Workflows, etc.) do not appear.
- Verify that `@` context is not available in SSH/remote sessions.
