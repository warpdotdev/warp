# Option to Disable Block Highlighting — Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/9815
Figma: none provided

## Summary
Add an Appearance > Blocks setting that lets users disable the visual highlight (background color and accent border) drawn on selected terminal blocks. The setting is exposed as a toggle in the settings UI and as a command palette entry with an optional keybinding.

## Goals / Non-goals
In scope:
- A persisted boolean setting controlling whether the selected-block highlight is drawn.
- A toggle in Appearance > Blocks and a corresponding command palette entry with an optional keybinding.
- When disabled, the selected-block background fill and accent border are suppressed.

Out of scope:
- Disabling block selection itself. Blocks continue to be logically selected; keyboard navigation, context menu actions, copy, AI context inclusion, and all other selection-dependent behaviors work exactly as today. Only the visual highlight is suppressed.
- Disabling the failed-block background tint, restored-block overlay, AI context stripe, snackbar hover effect, or shared-session participant selection borders. Those are independent visual treatments and are not affected by this setting.
- Disabling block dividers. Block dividers are controlled by the existing "Show block dividers" setting and are unrelated to the selected-block highlight.

## Behavior

1. A new boolean setting "Show selected block highlight" appears in the Appearance > Blocks category of the settings page. It defaults to `true` (highlight shown), preserving today's behavior for all existing users.

2. The toggle renders in the same style as the adjacent block toggles ("Compact mode", "Show Jump to Bottom of Block button", "Show block dividers") — a labeled row with a switch widget. It appears after the existing block setting widgets in the Blocks category.

3. When the setting is `true` (default), selected blocks render exactly as they do today: a background fill (either `block_selection_color()` or `block_selection_as_context_background_color()` when the block can be AI context) and an accent border (either `accent()` or `block_selection_as_context_border_color()`).

4. When the setting is `false`, the selected-block background fill and accent border are not drawn for the user's own selection. The block remains logically selected — all actions that operate on the selected block (copy block output, share block, use as AI context, keyboard block navigation, right-click context menu) continue to work.

5. Shared-session participant selection borders and background tints are not affected by this setting. Other participants' colored selection borders continue to render regardless of the local user's highlight preference. This ensures collaborative visibility is never broken by one participant's local setting.

6. The failed-block background tint (`failed_block_color`), the restored-block overlay, the AI context stripe, the inline agent view active-block background, and the snackbar hover highlight are all independent of this setting and render the same whether the highlight is enabled or disabled.

7. The setting is available in the command palette as a toggle ("selected block highlight"). Toggling it from the command palette has the same effect as toggling the switch in the settings UI. The toggle supports an optional keybinding that users can assign in the keyboard shortcuts page.

8. The setting persists across sessions via the standard settings persistence mechanism, using the TOML path `appearance.blocks.show_selected_block_highlight`. It syncs to the cloud the same way the existing block settings do (globally, respecting the user's sync preference).

9. Search within settings finds the new toggle when the user types terms such as "block highlight", "selected block", or "highlight". The existing block-related search terms for other toggles are unchanged.

10. The draw-border-above-block logic that suppresses the gray divider border when a selection top-border is present must also account for the highlight setting. When the highlight is disabled, the gray divider border is never suppressed on account of a selection border (since no selection border is drawn). This prevents a missing divider line when the highlight is off and block dividers are on.

11. The setting applies consistently across themes and appearances. The only user-visible difference is the presence or absence of the selected-block background and border.

12. Toggling the setting takes effect immediately — no restart or re-render cycle is required. The next frame after the setting changes reflects the new state.
