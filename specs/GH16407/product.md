# Receive OSC 7501 program status

Issue: https://github.com/warpdotdev/warp/issues/16407

Status: proposed; awaiting issue readiness and spec approval before implementation.

## Problem

People running several CLI agents need to see which terminal is working, waiting for input, or
finished without opening every tab. Programs can report this directly through the generic
[OSC 7501 Program Status Protocol](https://www.superlogical.com/rex/docs/build/program-status)
(revision 0.4, 2026-10-10).

## Behavior invariants

1. Any program attached to a terminal can report `idle`, `working`, `blocked`, `done`, or `error`.
   Reception does not require recognizing a CLI agent, installing a plugin, or parsing screen text.
   Both BEL and ST terminators work, including when a report spans multiple PTY reads.
2. A valid capability query receives the fixed OSC 7501 reply listing those five states and the
   `permission`, `question`, and `auth` blocked kinds. Reports receive no reply.
3. Each terminal owns an independent set of records. An omitted ID addresses its root record;
   a named ID addresses that record only. Each report replaces the complete addressed record.
   Missing metadata is removed, and an absent `app` inherits from the nearest ancestor with one.
4. A report with `state=clear` removes the addressed record and its descendants. An omitted ID
   clears all records. Clearing `build` never removes a sibling such as `builder`.
5. A new shell prompt or PTY exit removes working and blocked records. Idle records also expire
   at these events. Done and error records survive them. Switching screens and soft reset preserve
   records; a full terminal reset removes all records.
6. A pane displays its highest-priority record in its header, and a tab displays the highest-priority
   record across all its panes, including unfocused panes. Priority is blocked, error, working, done,
   then idle; the most recently updated record breaks ties. Idle has no attention badge. Closing a
   pane immediately removes its contribution to the tab indicator.
7. Working uses the existing busy indicator; blocked, done, and error use the corresponding existing
   status symbols. A tooltip gives a plain-language state, blocked kind when present, pane identity,
   inherited app, title, message, and percentage when supplied. The indicator is also exposed with
   an accessible name. Long text is clipped within the UI's existing tooltip bounds.
8. Merely focusing a terminal does not hide blocked or working status. Keyboard input directed to
   the focused terminal acknowledges its done/error records and removes them. Other panes' results
   remain visible. A later report can restore a result indicator.
9. OSC 7501 status takes precedence over legacy agent status for that terminal while any protocol
   record exists, including idle. When the last record is removed, existing status behavior resumes.
   Other terminals' existing indicators continue to participate in tab aggregation. Existing agent
   conversation, rich-input, plugin, and notification behavior is preserved.
10. Status reports never trigger desktop notifications, sounds, focus changes, or rich-input controls.
    Message and title text is displayed literally; formatting controls are neutralized outside the
    terminal grid. Reports cannot impersonate another pane or change its title.
11. Unknown keys and malformed pairs are ignored; duplicate valid keys use the last value. Missing
    or unknown states and invalid IDs discard the report. Invalid optional app, kind, or progress
    values are absent. Invalid base64, non-UTF-8 text, control characters, or exceeded hard limits
    discard the whole report without changing records or their recency.
12. Storage is bounded to 256 records per terminal with least-recently-updated eviction for a new
    record at capacity. Updates to existing records do not evict others. Record changes are rendered
    through normal UI invalidation; no timer, heartbeat, or report-generated notification is needed.
13. Status is session-local and is not restored from disk or transmitted to shared-session viewers
    in this initial change. A new terminal starts empty.

## Approval decisions

The priority order, acknowledging results on keyboard input, tooltip presentation, and interaction
with existing badges above are proposed product decisions. Maintainers should settle them during
spec review. Acceptance requires the receive path, lifetime behavior, and both tab and pane surfaces;
a parser-only change does not complete the issue.
