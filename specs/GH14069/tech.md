# Cycle the active tab color with an editable keybinding — Tech Spec

Product spec: [`specs/GH14069/product.md`](product.md)

GitHub issue: https://github.com/warpdotdev/warp/issues/14069

Code references inspected at commit: `c20645b9b0425dd8fb49fe9f9d08280e1e725ce9`

## Context

Issue #14069 asks for one keybinding that rotates the active tab color. A maintainer narrowed the accepted direction to a single cycle action and marked the issue `ready-to-spec`. The existing `/set-tab-color` flow is not an equivalent shortcut for terminal-hosted chat tools because those tools may consume slash-prefixed input.

The current GUI already owns all required color state and rendering:

- [`CONTRIBUTING.md:86-109 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/CONTRIBUTING.md#L86-L109) defines the `ready-to-spec` flow and the required `specs/GH<issue>/product.md` plus `tech.md` layout.
- [`app/src/ui_components/color_dot.rs:18-25 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/ui_components/color_dot.rs#L18-L25) is the canonical ordered list of six tab colors: Red, Green, Yellow, Blue, Magenta, Cyan.
- [`app/src/tab.rs:111-135 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/tab.rs#L111-L135) models a manual color as `Unset`, `Cleared`, or `Color`, and resolves it against a directory-derived default.
- [`app/src/tab.rs:163-216 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/tab.rs#L163-L216) stores both the directory default and manual selection on `TabData`; `TabData::color` exposes the visible result.
- [`app/src/tab.rs:755-825 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/tab.rs#L755-L825) builds the existing picker from the canonical list and dispatches the established tab or tab-group toggle actions.
- [`app/src/workspace/view.rs:5534-5592 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/workspace/view.rs#L5534-L5592) applies a manual tab color, sends existing set/reset telemetry, notifies the view, and contains the current single-color toggle behavior.
- [`app/src/workspace/view.rs:5594-5634 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/workspace/view.rs#L5594-L5634) owns the shared color of a tab group and propagates it through the group's rendering model.
- [`app/src/workspace/action.rs:128-155 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/workspace/action.rs#L128-L155) defines active-tab workspace actions, including the parameterized `SetActiveTabColor`.
- [`app/src/workspace/view.rs:23698-23712 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/workspace/view.rs#L23698-L23712) handles `SetActiveTabColor` and already redirects a grouped active tab to the group's color.
- [`app/src/workspace/mod.rs:515-623 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/workspace/mod.rs#L515-L623) registers editable workspace actions, while [`app/src/workspace/mod.rs:951-971 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/workspace/mod.rs#L951-L971) shows the established keyless-by-default registration pattern.
- [`app/src/settings_view/keybindings.rs:749-785 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/settings_view/keybindings.rs#L749-L785) materializes every editable binding in the Keyboard Shortcuts page.
- [`app/src/search/action/data_source.rs:67-90 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/search/action/data_source.rs#L67-L90) indexes the active view's command bindings, and [`app/src/search/command_palette/data_sources.rs:110-119 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/search/command_palette/data_sources.rs#L110-L119) adds those actions to the Command Palette.
- [`app/src/terminal/input/slash_commands/mod.rs:613-659 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/terminal/input/slash_commands/mod.rs#L613-L659) validates explicit `/set-tab-color` arguments and dispatches `SetActiveTabColor`; this path remains unchanged.
- [`app/src/workspace/action.rs:911-975 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/workspace/action.rs#L911-L975) exhaustively decides which workspace actions save app state, and [`app/src/workspace/view.rs:26049-26052 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/workspace/view.rs#L26049-L26052) dispatches the save after such an action.
- [`app/src/persistence/sqlite.rs:1012-1024 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/persistence/sqlite.rs#L1012-L1024) and [`app/src/persistence/sqlite.rs:2552-2567 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/persistence/sqlite.rs#L2552-L2567) serialize and restore tab-group colors.
- [`app/src/persistence/sqlite.rs:1042-1059 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/persistence/sqlite.rs#L1042-L1059) serializes manual tab colors, and [`app/src/persistence/sqlite.rs:2583-2603 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/persistence/sqlite.rs#L2583-L2603) restores them.
- [`app/src/workspace/view_tests.rs:1407-1488 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/workspace/view_tests.rs#L1407-L1488) is the existing active-tab color regression coverage.
- [`app/src/workspace/view.rs:5201-5208 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/app/src/workspace/view.rs#L5201-L5208) exposes the resolved tab color used by integration assertions.
- [`crates/integration/src/test.rs:1655-1696 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/crates/integration/src/test.rs#L1655-L1696) demonstrates the current custom-keybinding GUI integration-test setup and keystroke dispatch pattern.
- [`crates/integration/src/bin/integration.rs:156-159 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/crates/integration/src/bin/integration.rs#L156-L159) and [`crates/integration/tests/integration/ui_tests.rs:33-38 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/crates/integration/tests/integration/ui_tests.rs#L33-L38) show the two registrations required for a GUI integration test.
- [`script/presubmit:1-70 @ c20645b9b`](https://github.com/warpdotdev/warp/blob/c20645b9b0425dd8fb49fe9f9d08280e1e725ce9/script/presubmit#L1-L70) is the current repository-owned format, inline-test-module, three-profile Clippy, native-format, and test entry point.

No new persistence format, renderer, settings schema, color values, or network API is required.

## Proposed changes

### 1. Define the cycle from the canonical tab-color list

Add a small pure helper in `app/src/tab.rs`, next to `SelectedTabColor`, that accepts the currently resolved `Option<AnsiColorIdentifier>` and returns the next `SelectedTabColor`.

The helper must use `TAB_COLOR_OPTIONS`; it must not duplicate the six variants in a second match or array. Its rules are:

- `None` → the first entry in `TAB_COLOR_OPTIONS`.
- A color found at index `i` → `Color(TAB_COLOR_OPTIONS[i + 1])`.
- The final option → `Cleared`.
- A color not present in `TAB_COLOR_OPTIONS` → the first option.

Returning `Cleared` at wraparound is intentional: it suppresses any directory-derived default so behavior 6 visibly reaches no color. The next invocation passes a resolved `None` back to the helper and starts at Red.

Keeping the transition pure makes the entire sequence deterministic and unit-testable. Keeping it in `tab.rs` lets both ungrouped tabs and groups use the same policy while retaining `color_dot.rs` as the canonical palette owner.

### 2. Add one keyless editable workspace action

Add a payload-free `WorkspaceAction::CycleActiveTabColor` variant in `app/src/workspace/action.rs`.

Register it in `workspace::init` (`app/src/workspace/mod.rs`) as:

```rust
EditableBinding::new(
    "workspace:cycle_active_tab_color",
    "Cycle current tab color",
    WorkspaceAction::CycleActiveTabColor,
)
.with_group(bindings::BindingGroup::Settings.as_str())
.with_context_predicate(id!("Workspace"))
```

Do not call any `with_*_key_binding` method. That leaves the action keyless by default while automatically making it available to:

- Settings → Keyboard Shortcuts, which enumerates editable bindings.
- The Command Palette action data source, which indexes executable bindings for the active workspace.

Mark `CycleActiveTabColor` as `true` in the exhaustive `should_save_app_state_on_action` match, alongside the existing tab color mutations. This reuses the normal post-action `workspace:save_app` dispatch and existing SQLite representation.

### 3. Cycle the visible color of the active target

Handle `CycleActiveTabColor` in `Workspace::handle_action` beside `SetActiveTabColor`.

Resolve the target once:

1. Read the active `TabData` with `self.tabs.get(self.active_tab_index)`. If it is absent, return without notification or error.
2. If the tab has a `group_id`, read the group's resolved color with `group.color.resolve(None)`, calculate the next color, and call `set_tab_group_color`.
3. Otherwise, read `TabData::color()`, calculate the next color, and call `set_tab_color`.

Using the resolved color is required for behavior 5: a directory-derived Yellow advances to Blue rather than restarting at Red. Using `Cleared` at wraparound ensures Cyan advances to visibly uncolored even in a directory with a configured color.

The setter calls retain current behavior:

- ungrouped tabs reuse existing set/reset telemetry and view notification;
- grouped tabs continue to render their shared color on the group header and container without overwriting member tab colors;
- app-state saving happens once through the typed-action postamble after the action handler;
- focus, active-tab selection, terminal input, and the existing `/set-tab-color` and picker paths remain untouched.

Do not dispatch six parameterized actions or add per-color editable bindings. Do not implement the cycle in the slash-command parser or the color-dot renderer.

### 4. Keep existing persistence and rendering formats

No migration is needed. The cycle writes the existing `SelectedTabColor` variants already serialized for tab snapshots and the existing group color state already persisted by workspace snapshots.

The rendered colors continue to resolve through `AnsiColorIdentifier` and the active theme. No literal RGB values are introduced.

## Testing and validation

Map the product invariants to concrete coverage:

### Unit tests

1. Add table-driven tests for the pure cycle helper in `app/src/tab_tests.rs`:
   - `None` and an identifier outside `TAB_COLOR_OPTIONS` advance to Red.
   - Every entry advances to the next entry in `TAB_COLOR_OPTIONS`.
   - Cyan advances to `SelectedTabColor::Cleared`.
   - The test derives expected colors from `TAB_COLOR_OPTIONS` so a future palette-order change cannot silently diverge from the cycle.
   - Covers behavior 3–6 and 13.

2. Extend `app/src/workspace/view_tests.rs` with focused action tests:
   - An ungrouped active tab cycles through all six colors, wraps to visibly no color, then returns to Red.
   - Switching active tabs proves only the active target changes.
   - A tab with `selected_color = Unset` and a directory-derived Yellow advances to Blue; wrapping from Cyan produces no visible color despite the directory default.
   - A grouped active tab changes `TabGroup::color`, leaves each member's `selected_color` unchanged, and leaves an unrelated tab/group unchanged.
   - Dispatching the action with no accessible active tab is a no-op rather than a panic.
   - Covers behavior 3–8 and 12–14.

3. Extend `app/src/workspace/action_tests.rs` to assert that `CycleActiveTabColor.should_save_app_state_on_action()` is true. Existing persistence tests continue to cover `SelectedTabColor` serialization and restoration; this assertion proves the new entry point reaches that save path. Covers behavior 10.

4. Add a binding-registration assertion using the initialized workspace test app:
   - The binding name is `workspace:cycle_active_tab_color`.
   - Its source description is `Cycle current tab color`; the standard binding normalization renders it as `Cycle Current Tab Color`.
   - It has no default trigger.
   - It remains editable and has a workspace context predicate.
   - Covers behavior 1–2 and 11.

5. Cover the Command Palette data-source/action contract in the fully registered GUI integration test below:
   - Searching for the exact label `Cycle current tab color` returns the new workspace action while a workspace is active.
   - The selected result is the exact `AcceptBinding` for `workspace:cycle_active_tab_color` before Enter is dispatched.
   - Invoking that search result advances the active tab from Green to Yellow after the two keybinding invocations below.
   - The active tab, pane, focus, and terminal input remain unchanged.
   - Covers behavior 1, 3, 8–9, and 11.

Run the focused unit tests:

```bash
cargo nextest run -p warp tab::tests
cargo nextest run -p warp workspace::action::tests
cargo nextest run -p warp workspace::view::tests::test_cycle_active_tab_color
```

### GUI integration test

Add `test_cycle_active_tab_color_with_keybinding` to `crates/integration/src/test/workspace.rs`, register it in `crates/integration/src/bin/integration.rs`, and include it in `crates/integration/tests/integration/ui_tests.rs`.

Following the existing custom-keybinding integration pattern:

1. Write a temporary keybindings entry mapping `workspace:cycle_active_tab_color` to a conflict-free test key.
2. Bootstrap a single terminal tab and assert `Workspace::get_tab_color(0) == None`.
3. Send the test key and assert the color is Red.
4. Send it again and assert the color is Green.
5. Open the Command Palette, search for `Cycle current tab color`, invoke the
   result, and assert the color is Yellow.
6. Assert the same tab and pane remain active and the terminal input buffer is
   unchanged across both entry points.

This approach is grounded in the current custom-keybinding integration test and the existing public `Workspace::get_tab_color` assertion surface linked in Context. It covers the actual editable-binding dispatch and behavior 2–4, 8–9, and 11. Run it with:

```bash
WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 cargo run -p integration --bin integration -- test_cycle_active_tab_color_with_keybinding
```

### Manual proof for the implementation PR

On macOS, include:

1. A before screenshot showing that searching Keyboard Shortcuts for “Cycle current tab color” produces no action.
2. An after screenshot showing the new keyless action and an assigned shortcut.
3. An after screenshot showing the same action returned by Command Palette
   search.
4. A short screen recording with a native Codex or Claude Code terminal session focused. Invoke the assigned shortcut repeatedly and show the tab moving through the ordered colors without text appearing in the chat input; also invoke the action once from the Command Palette and show Cyan wrapping to no color.
5. Repeat once with a grouped tab and once with a directory-derived color to demonstrate behaviors 5–7.

No login or network is needed. The recording should keep the tab indicator and focused terminal input visible together so behavior 9 is directly observable.

### Repository checks

Before pushing an implementation update:

```bash
./script/presubmit
```

Use the repository-owned script rather than a generic all-features Clippy command. At the inspected commit, `script/presubmit` runs `./script/format --check`, `./script/check_no_inline_test_modules`, the workspace/default-GUI/`warp_completer` Clippy profiles, native format checks, nextest, and doc tests.

## Parallelization

Do not split the implementation for GH14069 across multiple authoring agents. The action variant, canonical successor helper, action handler, binding registration, and focused tests are small and compile-coupled; parallel edits would overlap `workspace` modules and cost more merge time than they save.

Use one implementation owner for the coherent patch. After it exists, an independent read-only reviewer can inspect product-invariant coverage while the owner runs the focused unit and GUI integration tests. GH14069 can still proceed in parallel with unrelated issue branches because those changes are isolated from this patch.

## Risks and mitigations

### Directory defaults can prevent a visible no-color state

Writing `Unset` after Cyan would reveal the directory default immediately and violate behavior 6.

Mitigation: return `SelectedTabColor::Cleared` at wraparound and test with a directory-derived color.

### Grouped tabs can receive an invisible per-tab override

Writing directly to the active member's `TabData` would not control the group-owned color and could leave hidden state that appears after ungrouping.

Mitigation: resolve `group_id` first and mutate `TabGroup::color` through `set_tab_group_color`, matching the existing `SetActiveTabColor` redirection.

### The cycle can drift from the picker

A duplicated list or exhaustive color match would silently diverge when the available color palette changes.

Mitigation: derive transitions and tests from `TAB_COLOR_OPTIONS`.

### A default shortcut can conflict with terminal applications

Choosing a new global default would introduce an avoidable compatibility decision outside the issue's accepted scope.

Mitigation: register the action as editable and keyless by default.
