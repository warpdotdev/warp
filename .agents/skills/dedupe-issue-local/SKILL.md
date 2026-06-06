---
name: dedupe-issue-local
specializes: dedupe-issue
description: Repo-specific dedupe guidance for warp. Only the categories declared overridable by the core dedupe-issue skill may be specialized here.
---

# Repo-specific dedupe guidance for `warp`

This file is a companion to the core `dedupe-issue` skill. It does not
redefine the duplicate-detection algorithm, the similarity thresholds,
or the output contract. It only specializes the override categories the
core skill marks as overridable.

## Repo-specific normalizations

- Strip low-signal title prefixes such as `Bug:`, `Feature:`, `Request:`, `[Bug]`, `[Feature]`, `Warp:`, and platform tags like `[macOS]`, `[Linux]`, or `[Windows]` before comparing titles.
- Treat app channel/version, OS version, and shell name as supporting evidence, not as duplicate blockers, when the core symptom and reproduction path are otherwise the same.
- Do not collapse distinct Warp surfaces just because they share a word like "agent", "terminal", "MCP", "settings", "search", or "sync". Require overlap in the actual failing behavior or requested capability.
- For terminal issues, compare shell/session context, command output behavior, prompt rendering, input behavior, and remote/tmux involvement before treating two reports as duplicates.
- For agent or MCP issues, compare the trigger path, local vs cloud execution, MCP server/tool, visible error, and expected workflow before treating two reports as duplicates.
- For UI/rendering issues, compare the affected surface and visible symptom. Similar screenshots or recordings are strong duplicate evidence when the title is vague.

## Known-duplicate clusters

No known-duplicate clusters have been captured for this repository yet. The weekly `update-dedupe` loop will propose additions here over time when maintainers repeatedly close issues as duplicates of the same canonical thread.
