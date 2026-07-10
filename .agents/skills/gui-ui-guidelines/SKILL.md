---
name: gui-ui-guidelines
description: GUI desktop app only. Catalog of guidelines for writing UI code in the Warp client. Read whenever doing any UI work in this repo, up front before writing the change, so the relevant guidelines shape the implementation.
---

# gui-ui-guidelines

**Scope — GUI desktop app only.** This skill applies to Warp's **GUI** desktop front-end (the `app/` crate on the WarpUI pixel/GPU framework). It does **not** apply to the headless **TUI** front-end (`crates/warp_tui`; cell-grid `TuiElement` library under `crates/warpui_core/src/elements/tui`), which has its own components, tests, and change-verification workflow. For TUI work, see the `tui-ui-guidelines`, `tui-testing`, and `tui-verify-change` skills instead.

This skill is a growing catalog of guidelines for working on Warp's UI code. Each guideline captures a lesson that would otherwise be re-learned through review — typically because an agent or contributor reinvented a component, drifted from the design system, or bypassed a shared abstraction.

**How to use this skill:**

- Read through the guidelines below once at the start of any UI task, then keep them in mind while implementing. The list is short enough to scan.
- Each guideline is self-contained. Not every one will apply to every task — use judgment. But if a guideline *does* apply, follow it.
- When in doubt, prefer reusing an existing abstraction over introducing a new one. The Warp UI has accumulated a well-factored set of shared components and themes; new one-offs almost always drift.

New guidelines get added here over time. If you discover a recurring UI mistake that would have been caught by a written rule, add it.

---

## Guideline: Reuse button themes

Button colors come from a shared set of `ActionButtonTheme` impls in `app/src/view_components/action_button.rs` (and the parallel `Theme` impls in `crates/ui_components/src/button/themes.rs`) — `PrimaryTheme`, `SecondaryTheme`, `NakedTheme`, `DangerPrimaryTheme`, etc. These encode the design system and keep button colors consistent across the app.

When styling a button, **use one of the existing themes unchanged**. The shared themes are well-established and vetted; if one looks "wrong" for your use case, the most likely explanation is that you're reaching for the wrong theme, not that the theme is buggy.

Do **not** modify a shared theme on your own initiative. Changing `PrimaryTheme`, `SecondaryTheme`, etc. affects every button in the app, and a tweak that fixes your screen can silently regress others. Only edit a shared theme when the user has explicitly confirmed that the design-system component itself needs to change.

Red flags that you're about to make buttons inconsistent:

- Writing a new `impl ActionButtonTheme for FooPrimaryTheme` that delegates to `PrimaryTheme` and only tweaks one method (usually `text_color`). Almost always the right move is to use `PrimaryTheme` directly and accept the result.
- Hard-coding `ColorU::new(...)` instead of using `appearance.theme()` accessors (`accent`, `font_color(bg)`, `foreground`, etc.).
- Setting `should_opt_out_of_contrast_adjustment` to `true` to force a specific label color.
- Naming a theme after a feature or view (`FooPrimaryTheme`, `BarSubmitTheme`) rather than a design-system role.

If an existing theme genuinely doesn't fit and you think the shared theme should change, surface that to the user before editing it, rather than either editing it unilaterally or papering over it with a one-off.
