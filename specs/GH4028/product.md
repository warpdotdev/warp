# Product Spec: Show tab numbers on tabs

**Issue:** [warpdotdev/warp#4028](https://github.com/warpdotdev/warp/issues/4028)
**Figma:** none provided

## Summary

Add an optional **"Show tab numbers"** setting that prefixes each tab with its 1‑based position number, so the existing `Cmd+1..9` tab‑switch shortcuts are discoverable at a glance. The number is presentation‑only and must never leak into copied titles or tab search.

## Problem

Warp already binds `Cmd+1..8` to "activate the Nth tab" and `Cmd+9` to "activate the last tab" (`WorkspaceAction::ActivateTabByNumber`). But nothing in the UI tells the user which number maps to which tab, so the only way to use the shortcut is to count tabs by hand. Users with many tabs (the issue author keeps ~5 Neovim tabs) cannot tell which `Cmd+N` jumps where, and fall back to the mouse or `Shift+Cmd+{`/`}`.

The issue requests exactly this:

> Can we add a feature to show the tab index, so that we can easily switch tabs?
> `1 Device:~/Documents | 2 Device:~/Documents/Projects | 3 Device:~`

## Goals

- A user-toggleable setting that shows each tab's 1‑based position number.
- Works for both **horizontal tabs** and **vertical tabs** (Warp's two tab layouts).
- The displayed number matches the `Cmd+N` shortcut that activates that tab.
- Off by default (opt‑in), so existing users see no change.
- Presentation‑only: copying a tab title, tab search, and the stored title are unaffected.

## Non-goals

- Re-binding or changing the `Cmd+1..9` shortcuts themselves (already exist).
- Showing numbers for tabs beyond what the shortcuts cover in any special way — every tab simply shows its true position.
- A separate badge/gutter visual style; this spec uses a simple inline number prefix. Visual refinement can be a follow-up.

## User experience

### Setting

- **Settings → Appearance → Tabs → "Show tab numbers"** — a toggle, default **off**.
- Equivalent TOML: `appearance.tabs.show_tab_numbers = true`.
- Searchable in settings by terms like "tab number", "index", "position", "shortcut".

### Behavior when enabled

Horizontal tabs render the number before the title:

```
[ 1  zsh ][ 2  vim ][ 3  logs ● ]
```

Vertical tabs render the number before each tab's icon on the tab's representative row:

```
1  ~/Documents
2  ~/Documents/Projects
3  ~
```

Toggling the setting updates open windows live (no restart).

## Behavior invariants (testable)

1. When `show_tab_numbers` is **off** (default), tabs render exactly as today — no number shown, in both layouts.
2. When **on**, the tab at zero-based index `i` displays the number `i + 1`.
3. The displayed number is consistent with the shortcut: pressing `Cmd+N` activates the tab showing `N` (for `N` in 1..=8; `Cmd+9` activates the last tab, which shows its own position number).
4. In **vertical tabs**, exactly one number is shown per tab (on its representative/first row), never one-per-pane duplicates, across Expanded, Compact, and Summary view modes.
5. Toggling the setting re-renders visible tabs without requiring a restart.
6. The number is presentation-only: the value returned by "Copy tab title", the stored tab title, and tab search text are identical whether the setting is on or off.
7. While a tab is being renamed, the number prefix is not shown (the user is editing the title).

## Edge cases

- **Many tabs (>9):** every tab still shows its true position number; only `Cmd+1..9` have shortcuts, which is unchanged.
- **Filtered/searched vertical tabs:** numbers reflect the true workspace index, so they may be non-contiguous when the list is filtered — this is intentional so the number keeps matching the `Cmd+N` shortcut.
- **Custom-titled tabs:** the number is prefixed before the custom title; the custom title itself is unchanged.
- **Single tab:** shows "1" when enabled.
