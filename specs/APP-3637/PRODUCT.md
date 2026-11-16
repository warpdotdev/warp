# CLI Agent Rich Input: /skills Product Spec

## Summary
Add support for `/skills` in the CLI agent rich input composer — the input that appears when composing a prompt to send to a running CLI agent (Claude Code, Codex, Gemini CLI, etc.). Only natively supported skills are shown, and the selected skill name is passed through to the CLI agent via PTY write.

## Problem
When users compose prompts for CLI agents through Warp's rich input (Ctrl-G or the Compose button), they cannot browse or invoke skills. The normal Warp agent input supports skills, but the CLI agent input does not. Users must manually type `/skill-name` without any discovery or autocomplete.

The core constraint is that CLI agent input writes plain text to a PTY — so skill invocation must resolve to a plain text `/skill-name args` string that the CLI agent interprets natively.

## Goals
- Let users browse and select skills via `/` in the CLI agent rich input.
- Only show skills that the active CLI agent can natively interpret (passthrough).
- Hide Warp-specific slash commands that don't apply to CLI agents.

## Non-goals
- Surfacing non-natively-supported skills (e.g., bundled Warp skills like `oz-platform`). Only skills the active CLI agent can interpret should appear.
- Implementing client-side argument parsing for skills. The CLI agent handles argument parsing natively.
- Showing native CLI agent slash commands (e.g., Claude Code's `/compact`, `/model`) in the menu. This is a follow-up (see APP-3641).

## Figma
https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7001-18001&p=f&m=dev

## User Experience

### Trigger
User types `/` at the start of the input in the CLI agent rich input, same trigger as in normal Warp input.

### Limitation
`/skills` is only available in local sessions. It is not supported in SSH/remote contexts because skill discovery relies on local filesystem access.

### Menu
The slash command / skill selector menu opens. Static slash commands that are Warp-agent-specific (e.g., `/agent`, `/new`, `/conversations`, `/cloud-agent`) are hidden when the CLI agent rich input is active. The `/skills` command itself remains available so users can browse skills. Individual skills also appear as direct items in the menu.

### Behavior on selection
Only natively supported skills are shown in the menu. When a skill is selected, `/{skill-name} ` is inserted into the buffer. The user can type arguments after it. On submit, the full text (e.g., `/my-skill arg1 arg2`) is written to the PTY — the CLI agent handles argument parsing natively.

### Natively supported skills
Some CLI agents support a skills/agents folder convention. The known mapping is:

- **Codex**: supports `.agents/`, `.claude/`, `.codex/` folders.
- **OpenCode**: supports `.opencode/`, `.agents/`, `.claude/` folders.
- **Claude Code**: supports `.claude/` folder.
- **Gemini CLI**: supports `.agents/`, `.gemini/` folders.
- **Amp**: supports `.agents/` folder.
- **Copilot**: supports `.agents/`, `.copilot/` folders.
- **Droid**: supports `.factory/`, `.agents/` folders.

A skill is shown in the CLI agent input menu if:
1. The active CLI agent is one that supports skills folders, AND
2. The skill's provider (determined from its filesystem path) matches one of the agent's supported providers.

Skills that don't match the active CLI agent's supported providers are hidden from the menu entirely. Non-natively supported skills (including bundled Warp skills like `oz-platform`) are not shown — supporting them is a potential follow-up.

## Success Criteria
- Users can type `/` in the CLI agent rich input, see natively supported skills, select one, and have `/{skill-name} ` inserted for passthrough to the CLI agent.
- Warp-specific slash commands do not appear in the CLI agent menu (except `/skills`).
- Non-native skills are hidden from the menu.
- This feature is only available in local sessions, not SSH/remote.

## Validation
- Open a CLI agent rich input while Codex is running. Type `/`, select a skill that exists in `.agents/`, verify `/{skill-name} ` is inserted. Type arguments and submit. Verify the full text is written to the PTY.
- Open a CLI agent rich input while Claude Code is running. Type `/`, verify only skills with `.claude/` provider appear.
- Verify that `/skills` command works and opens the skill browser.
- Verify that `/agent`, `/new`, etc. do not appear in the `/` menu.
- Verify that `/skills` is not available in SSH/remote sessions.

## Open Questions
- Should we surface native CLI agent slash commands (e.g., Claude Code's `/compact`, `/model`) in the menu alongside skills? This would require keeping command lists in sync with each CLI agent. See APP-3641. **Answer: not for now!**
