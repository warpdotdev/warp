# WARPER-010: deferred rule, skill, and MCP hygiene

## Summary

This is not an implementation spec. The previous version over-promoted local rule, skill, MCP, and selected-context hygiene into build work without a failing WARPER-005 acceptance test. Under the XP bar, these commits stay deferred.

## Deferred Commits

| Commits | Upstream why | Decision |
| --- | --- | --- |
| `5146a5bf`, `b48ece2e`, `ac4225c1` | Upstream fixed rule watcher races and duplicate/unrelated skill scans. | Defer until a WARPER-005 acceptance test proves Warper loses required local rules/skills. |
| `92069590`, `51c380ce` | Upstream capped MCP logs and fixed add-path redaction bypass. | Defer until MCP is part of the current release story and a test proves disk or secret-redaction failure. |
| `65381be1` | Upstream preserved selected-block context on Cmd+Enter new agent conversations. | Defer until an OpenRouter context-fidelity test proves this path loses required context. |

## Reactivation Gate

Create a real implementation spec only after a failing test or reproducible user workflow shows that Warper cannot satisfy WARPER-005 without one of these fixes.
