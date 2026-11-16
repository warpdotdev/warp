# Custom Host Picker for Orchestration

Linear: [QUALITY-701](https://linear.app/warpdotdev/issue/QUALITY-701)

## Summary

Adds a host picker to the orchestration UI so a user can choose where their cloud child agents run. Today the host is hardcoded to the default Warp cluster; this lets users target a self-hosted worker host, see the most recently used custom host, and pre-select an admin-configured workspace default. The behavior mirrors the Oz webapp's host selector, adapted to the desktop client's compact picker chrome.

## Design
No Figma mock. Design context lives in this Slack thread: https://warpdev.slack.com/archives/C0AAMT5TKC2/p1778542414211539?thread_ts=1778525276.139389&cid=C0AAMT5TKC2 — this implementation follows that direction but keeps things deliberately simpler for the first cut (compact dropdown with an inline custom-mode editor, reusing the existing orchestration picker chrome). Design polish can be a follow-up once the feature is in users' hands.

## Behavior

### Surface

1. The host picker appears next to the model, harness, and environment pickers in the orchestration UI. It is present in both the orchestrate confirmation card and the plan-card orchestration block. Both surfaces show the same options, the same selection, and the same custom-mode editor.

2. The picker is only visible when the execution mode is Cloud (Remote). In Local mode the host concept is not user-facing.

3. The picker visually matches the other orchestration pickers in the same row: same height, border, corner radius, background, padding, and font.

### List mode

4. By default the picker renders as a dropdown showing the currently selected slug. Clicking it opens a menu with the following entries, in this order:
   1. Workspace default slug, when the team has one configured, with a "Default" badge.
   2. `warp` (the default Warp cluster), always present.
   3. The user's most recent custom host slug, when set, rendered as a plain slug (no badge).
   4. A `Custom host…` entry that switches the picker into custom mode.

5. Duplicate rows are suppressed. If the recent custom host equals either `warp` or the workspace default, it does not get its own row.

6. When the user picks `warp`, the workspace default, or a recent slug, the picker closes and the selection is sent to the parent. The selected entry shows in the picker's collapsed state. If the workspace has a configured default, selecting it shows the "Default" badge in the collapsed state too.

7. Clicking outside the open menu, or pressing Escape, closes the menu without changing the selection.

### Custom mode

8. Selecting `Custom host…` swaps the picker top bar for an inline text editor, pre-filled with the current slug (or empty when the current slug is `warp`), and focuses the editor. A small cancel button sits at the right of the editor.

9. Inside the editor the user can type any non-empty slug. Pressing Enter or blurring the editor commits the trimmed value. Pressing Escape or clicking the cancel button reverts to the previous selection without committing.

10. Committing an empty buffer is treated as a revert (no change to the previous selection).

11. Typing `warp` (case-insensitive) and committing collapses back to the standard `warp` selection rather than persisting `warp` as a custom value.

12. When a non-empty, non-`warp` slug is committed, it becomes the current selection and is promoted to the "recent" row in the menu so it stays visible on the next paint. The slug is also persisted (see invariant 17) so it survives across cards and across app restarts.

13. While the editor is in custom mode, the editor's text is vertically centered within the picker box and the box sits at the same y offset as the other pickers in the row.

### Selection model

14. The picker always has a non-empty selection. Empty input from any source (initial state, blur, blank stream) resolves to `warp`.

15. When an external caller sets a slug that doesn't match any known menu option (warp, workspace default, recent), the picker switches into custom mode pre-filled with that slug instead of showing a missing menu entry.

16. The picker exposes the workspace default behavior: when a workspace default is configured and no explicit selection exists yet, the picker pre-selects the workspace default rather than `warp`. A developer-only `WARP_CLOUD_MODE_DEFAULT_HOST` environment variable overrides the workspace default for local testing.

### Persistence and recency

17. When the user commits a custom slug, it is persisted as the "last selected host" so the next plan card or confirmation card shows it as the "recent" entry. `warp` and empty values are never persisted as recent (the warp entry is always present unconditionally).

18. The recent slug is deduplicated against the workspace default. If the user's most recent slug happens to equal the workspace default, the menu shows only the default row; no separate recent row appears.

### Coordination with the rest of the orchestration UI

19. When the user picks or commits a slug, the new value is reflected in the same edit state that powers the other orchestration pickers, and is used by the eventual `RunAgents` dispatch as the `worker_host` field. The plan card additionally persists the new value to the orchestration config snapshot for that plan.

20. The picker's open menu paints above sibling pickers in the row so it doesn't visually collide with the Environment or Base model picker rendered below it. In the confirmation card the menu opens upward (matching the other dropdowns in that card); in the plan card the menu paints in an overlay layer above siblings.

21. When the menu closes (selection, dismissal, or custom-mode commit), parent input focus returns to wherever it was so the user can continue typing without an extra click.
