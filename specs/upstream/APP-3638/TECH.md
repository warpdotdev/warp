# CLI Agent Rich Input: /prompts Technical Spec

## Summary
This spec covers enabling `/prompts` (saved prompts) in the CLI agent rich input composer. The existing prompt insertion flow works as-is — no code changes are needed beyond ensuring the prompts menu is accessible when CLI agent input is active.

## Relevant Code
- `app/src/terminal/input/prompts/view.rs` — `InlinePromptsMenuView`
- `app/src/terminal/input.rs` — `handle_inline_prompts_menu_event()`, `show_workflows_info_box_on_workflow_selection()`

## Current State
The prompts menu (`InlinePromptsMenuView`) and the workflow info box insertion flow already exist in the normal Warp input. When a prompt is selected, `handle_inline_prompts_menu_event()` calls `show_workflows_info_box_on_workflow_selection()`, which inserts the workflow template into the editor with argument highlighting and the shift-tab UX.

The CLI agent rich input reuses the same `Input` view and editor. On submit, `input_enter()` detects `CLIAgentSessionsModel::is_input_open()` and emits `Event::SubmitCLIAgentInput` with the raw buffer text, which is written to the PTY.

## Proposed Changes

### 1. Allowlist the `/prompts` static command in CLI agent input

The `/prompts` static command is currently filtered out when CLI agent rich input is active (along with all other static slash commands). It needs to be **allowlisted** in `SlashCommandDataSource::recompute_active_commands()` so users can open the prompts browser from the slash menu. This is the same allowlisting mechanism used for `/skills` in the skills spec.

### 2. Prompt insertion: no changes needed

The existing `handle_inline_prompts_menu_event()` flow works as-is in CLI agent input mode — the editor is the same, and on submit the buffer text (with filled-in arguments) goes through `SubmitCLIAgentInput` → PTY write as plain text. The shift-tab argument editing is useful for CLI agent prompts too.

The prompts menu is already rendered through the `suggestions_mode_model` system, which works in CLI agent mode.

## End-to-End Flow
1. User opens the prompts menu in CLI agent rich input (via `/prompts` or the slash menu).
2. User selects a saved prompt via click or Enter. This is a **menu selection** — `input_enter()` checks `is_prompts_menu()` first and routes to `accept_selected_item()`, not the PTY submission path. The prompt template is inserted into the editor buffer with argument highlighting.
3. The prompts menu closes. The user is now in the normal editor with the prompt content in the buffer.
4. User edits arguments via shift-tab if needed.
5. User presses Enter again → this time no menu is open, so `input_enter()` takes the CLI agent submission path and writes the buffer text to the PTY.

## Testing and Validation
- Verify selecting a saved prompt inserts the prompt text with argument highlighting.
- Verify shift-tab argument editing works in CLI agent rich input.
- Verify the prompt content submits correctly to the PTY.
- Verify no regressions in normal Warp agent input (prompts still work as before).
