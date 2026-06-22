# Handoff-Compose Footer Chip Filtering — Product Spec
Linear: [REMOTE-1595](https://linear.app/warpdotdev/issue/REMOTE-1595)
## Summary
When `&` handoff-compose mode is active, the agent view footer should show only the chips relevant to composing a cloud run — hiding local-context and local-agent-only items that do not apply.
## Problem
The default agent footer shows chips for local context (working directory, git branch, diff stats, SSH, etc.) and local-agent controls (autodetection toggle, fast-forward, context window usage, handoff-to-cloud chip). None of these are meaningful when the user is drafting a prompt to hand off to a cloud agent. Showing them adds clutter and implies they affect the cloud run.
## Behavior
### Shown items
While `&` handoff-compose mode is active, the footer shows only:
1. **Environment selector** — the transient `&` selector, already wired (not a toolbar item).
2. **ModelSelector** — the user may want to choose which model the cloud agent uses.
3. **VoiceInput** — alternative input method for dictating the prompt.
4. **FileAttach** — file/image attachments carry to the cloud run.
### Hidden items
All other `AgentToolbarItemKind` variants are hidden while `&` is active:
- All `ContextChip(...)` variants — local context, not sent to the cloud run.
- `NLDToggle` — input is locked AI in `&` mode; autodetection is irrelevant.
- `ContextWindowUsage` — shows local conversation context, not cloud.
- `FastForwardToggle` — fast-forward state does not carry into cloud runs.
- `HandoffToCloud` — already in handoff mode; redundant.
- `ShareSession` — not relevant to composing a handoff.
### Constraints
- The user's configured footer layout is not modified. Items are only visually hidden while `&` is active; exiting `&` mode restores the normal footer.
- Items the user has explicitly hidden in their toolbar config remain hidden regardless.
- The filtering applies only in the agent view footer, not the CLI agent footer.
## Figma
None provided.
## Validation
- Enter `&` mode in a local fullscreen agent view. Confirm only ModelSelector, VoiceInput, FileAttach, and the environment selector are visible.
- Exit `&` mode. Confirm the full configured footer layout is restored.
- Verify that hidden items are not removed from the user's toolbar config.
