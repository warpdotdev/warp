# CLI Agent Rich Input — Shell Command Mode (`!` prefix)

## Problem

The CLI agent rich input composer (Ctrl-G) was designed for sending natural language prompts to CLI agents. However, CLI agents like Claude Code support a `!` prefix for running shell commands directly. When users type `!ls` in the composer, they expect the same shell input affordances they get elsewhere in Warp — syntax highlighting, red error underlining for unknown commands, and tab completions. Without this, the composer feels broken for shell commands: no visual feedback, no completions, and the `!` prefix interferes with the parser.

## Goals

- Provide full shell input features (syntax highlighting, error underlining, tab completions, autosuggestions) when the user types `!` in the CLI agent rich input.
- Reuse the existing agent view `!` shell mode infrastructure with minimal new code.
- Correctly send the `!` prefix to the CLI agent on submit so the agent recognises it as a shell command.
- Support Claude Code, Codex, and OpenCode (agents that use `!` for shell mode).

## Non-goals

- Showing the "backspace to exit shell mode" hint text row in the CLI agent footer. This requires new plumbing since the agent message bar pipeline is coupled to `AgentMessageArgs`. Can be explored later.
- Other CLI agents (for now, can expand this later).

## Figma

Figma: none provided. The visual treatment (blue `!` indicator, "Run commands" placeholder) is identical to the existing agent view shell mode.

## User experience

### Entering shell mode

1. User opens CLI agent rich input (Ctrl-G or footer button) while a supported CLI agent (Claude Code, Codex, OpenCode) is running.
2. Input starts in AI mode (locked). Placeholder reads "Tell the agent what to build...".
3. User types `!`.
4. The `!` character is stripped from the buffer.
5. Input switches to Shell mode (locked).
6. A blue `!` indicator appears as a UI prefix in the editor (identical to agent view shell mode).
7. Placeholder changes to "Run commands".
8. The user types a shell command (e.g. `git status`). Full shell features are active:
   - Syntax highlighting (command name in green, arguments colored).
   - Error underlining (unknown commands get red underline, e.g. `gitt`).
   - Tab completions (pressing Tab after `git co` shows completions).
   - Autosuggestions (ghosted text from history).

Note that CMD-I should NOT work for entering/exiting shell mode, and it should be disabled for LRC agent interaction as well (CC or other agents don't use CMD-I).

### Exiting shell mode

Shell mode can be exited in three ways:

1. **Backspace on empty buffer**: When the buffer is empty and the `!` indicator is showing, pressing Backspace removes the indicator and switches back to AI locked mode.
2. **Delete all (Cmd+Backspace)**: Same behavior — clears buffer and exits shell mode.
3. **Submitting a command**: After pressing Enter to submit a `!`-prefixed command, the input automatically reverts to AI locked mode for the next prompt.

After exiting shell mode, the placeholder reverts to "Tell the agent what to build..." and no autodetection runs — the input stays locked in AI mode. The user can type `!` again to re-enter shell mode.

### Submitting

When the user presses Enter in shell mode:

1. The `!` prefix is prepended back to the buffer text.
2. The full text (e.g. `!git status`) is sent to the CLI agent via the existing `SubmitCLIAgentInput` path.
3. The existing submit path already splits the `!` prefix byte with a small delay for agents like Claude Code (`CLI_AGENT_MODE_SWITCH_PREFIXES`).
4. The input reverts to AI locked mode.

### Interaction with other features

- **Autodetection**: Does not run while in CLI agent shell mode. The `!` prefix is the explicit toggle mechanism; autodetection would fight with the locked state.
- **Slash commands**: The slash command menu (`/`) is not affected. It operates independently of shell mode.
- **@ context menu**: Works normally in both AI and shell mode.

### Other Agents

Starting off with Claude Code, Codex and OpenCode for now. Can check behavior/expand to other CLI agents in a follow-up.

## Success criteria

1. Typing `!` in the CLI agent rich input for Claude Code / Codex / OpenCode strips the `!`, shows the blue `!` indicator, switches to Shell locked mode, and changes the placeholder to "Run commands".
2. `!git status` shows `git` with syntax highlighting (green) and `status` colored as an argument.
3. `!gitt` shows `gitt` with a red error underline.
4. Tab after `!git co` opens the completions menu with `commit`, `config`, `checkout`, etc.
5. Autosuggestions appear as ghosted text based on command history.
6. Backspace on empty buffer with `!` showing removes the indicator and returns to AI locked mode with "Tell the agent what to build..." placeholder.
7. Cmd+Backspace (delete all left) exits shell mode the same way.
8. Submitting `!pwd` sends `!pwd` to the CLI agent and reverts to AI locked mode.
9. After exiting shell mode (by any method), typing normal text does NOT get syntax highlighted — the input is in AI mode.
10. Typing `!` for unsupported agents does not trigger shell mode.

## Validation

- **Manual testing**: Open each supported CLI agent (Claude Code, Codex, OpenCode), Ctrl-G, type `!`, verify all behaviors above.
- **Regression**: Verify agent view `!` shell mode still works identically (the same code paths are shared).
- **Edge cases**: Type `!`, backspace, type `!` again — should work. Type `! pwd` (with space) — should work (space is part of the command). Submit empty shell mode (just `!` with empty buffer) — should submit `!` and exit shell mode.

## Open questions

- N/A
