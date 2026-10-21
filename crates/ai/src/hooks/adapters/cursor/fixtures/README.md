# Cursor adapter fixtures

These files are loaded with `include_str!` by the adapter's tests. The field shapes come from
`specs/agent-hooks/research/01-cursor-hooks-catalog.md`, cited below by section. Cursor's reference
documents fields rather than full example payloads, so the `stdin.json` files are what Warp encodes
for a fixed test envelope with those fields, and the `stdout.json` files are the documented output
shape for the event with concrete text.

- `full_config.json`: every event name and every per-definition option (§1.1, §3 headings).
- `invalid_matcher.json`: a definition whose matcher does not compile beside one that does (§1.4).
- `unknown_event.json`: an event name Cursor does not document beside a known one (§1.1).
- `prompt_hook.json`: a `type: "prompt"` definition beside a command hook (§4).
- `claude_shape_output.json`: Claude's nested `hookSpecificOutput` for `preToolUse`, and the nested
  and flat `decision: "block"` shapes for `stop`, as Cursor translates them (§6.3).
- One directory per event Warp fires in Phase 1, each with `hooks.json`, `stdin.json`, and
  `stdout.json`: `sessionStart` (§3.1), `preToolUse` (§3.3), `postToolUse` (§3.4),
  `postToolUseFailure` (§3.5), `beforeShellExecution` (§3.8), `afterShellExecution` (§3.9),
  `beforeMCPExecution` (§3.10), `afterMCPExecution` (§3.11), `beforeSubmitPrompt` (§3.14), `stop`
  (§3.16), `afterAgentResponse` (§3.17), `afterAgentThought` (§3.18). The common fields in each
  `stdin.json` are §2.1.
