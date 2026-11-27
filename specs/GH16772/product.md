# Compact horizontal tabs

Proposal for [GH16772](https://github.com/warpdotdev/warp/issues/16772). This design is
pending maintainer triage and approval.

## Problem

Users who prefer a denser horizontal tab bar currently have to reduce the scale of
the entire interface. Offer a smaller tab style independently of other UI text.

## Behavior invariants

1. **Opt in:** Settings → Appearance → Tabs contains a searchable **Compact
   horizontal tabs** switch. It defaults to off; existing users keep the current
   horizontal tab styling.
2. **Compact appearance:** When enabled, horizontal tab pills are 20 logical pixels
   high, at most 170 logical pixels wide, and have rounded corners. Existing UI
   zoom scales these dimensions. Colors continue to come from the active theme,
   including custom tab colors and selection/hover states.
3. **Immediate and reversible:** Changing the switch updates all horizontal tabs
   in the workspace without restarting, recreating terminal sessions, or changing
   the active tab. Turning it off restores the existing style.
4. **Persistence and discovery:** Both choices survive restarting Warp. The choice
   uses the normal public settings and settings-sync infrastructure. Settings
   search can isolate the row; clearing search restores the other rows. The
   Command Palette exposes the appropriate enable or disable action.
5. **Existing interactions:** Selection, title editing, indicators, close buttons,
   pinned tabs, tab groups and drag/drop continue to work. Long titles truncate
   within the available width. Changing density must not discard tab contents.
6. **Vertical layout:** The choice remains available and stored when using vertical
   tabs, but does not change their layout or density. Returning to horizontal tabs
   uses the stored choice.

## Non-goals

Changing the window title-bar height, resizing shared panel headers, adding a
general CSS mechanism, or replacing the vertical-tabs density controls.

## Open questions

The dimensions and rounded treatment are proposed defaults for the opt-in style,
subject to maintainer design feedback, particularly for accessibility and very
large UI zoom levels.
