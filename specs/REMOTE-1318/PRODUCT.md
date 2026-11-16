# REMOTE-1318: Merge org and user command denylists

## Summary

When an organization enforces a command denylist, users should still be able to add their own entries to make the denylist more restrictive. The org and user denylists merge into one list, where org-provided rows are non-removable and user-provided rows are fully editable. This applies everywhere the denylist appears: the settings page, the execution profile editor, and at command-execution time.

Figma: none provided

## Behavior

### Merged denylist

1. When the org (workspace) sets a command denylist override, the effective denylist is the **union** of the org denylist and the user's profile denylist, with duplicates removed. Org entries appear first in the list, followed by user entries.

2. Duplicate detection compares the string representation of each `AgentModeCommandExecutionPredicate`. If a user entry has the same regex string as an org entry, only the org entry appears in the merged list. The user's copy is still persisted in their profile — it is simply not displayed twice.

3. The merged denylist is used for command execution checks. A command that matches any entry in the merged list is denied (requires user confirmation), regardless of whether the matching entry came from the org or the user.

4. The org denylist applies identically across all execution profiles. There is no per-profile override of org entries.

5. When no org denylist override exists, behavior is unchanged — the user's profile denylist is the only source and all entries are user-editable.

### Settings UI — denylist editor (text input)

6. The denylist text input editor is enabled whenever AI mode is enabled, regardless of whether the org has a denylist override. The user can always type and submit new regex entries.

7. When the user submits a new entry, it is added to the user's profile denylist (not the org list). If the entry duplicates an existing org entry, it is still persisted in the user's profile but does not appear as a separate row (per invariant 2).

### Settings UI — per-row behavior

8. Each row in the denylist has an independent disabled/enabled state, determined by whether the entry came from the org denylist.

9. **Org rows**: the remove (×) button is disabled. Hovering the row shows a tooltip: "This option is enforced by your organization's settings and cannot be customized." The row text renders in the disabled text color.

10. **User rows**: the remove (×) button is enabled. No tooltip on hover. The row text renders in the normal foreground color. Clicking the remove button removes the entry from the user's profile denylist.

11. The denylist section is **not** wrapped in a single tooltip as a whole. Only individual org rows show the tooltip.

12. When the org removes an entry from their override that the user had also added independently, the user's copy becomes the sole source for that entry. It transitions to a user row (removable, no tooltip) on the next settings refresh.

### Settings UI — other lists (allowlist, directory allowlist, MCP lists)

13. Lists other than the command denylist retain their current behavior: when the org has an override, the entire list is disabled with the global tooltip. This spec does not change their behavior.

### Scope

14. The per-row merge and editability applies in both surfaces that render the command denylist:
    - The legacy AI settings page (default profile denylist section).
    - The execution profile editor view (per-profile denylist section).

### Edge cases

15. If the org denylist is set to an empty list (`Some([])`), the org override is considered active (the org has explicitly chosen "no org entries"), but the user's profile entries are still shown as user rows. The editor remains enabled.

16. If the user's profile denylist is empty and the org provides entries, only org rows appear. The editor is still enabled for the user to add entries.

17. When a user removes a user row, the row disappears immediately. The mouse state handles for remaining rows are recreated to stay in sync with the list.
