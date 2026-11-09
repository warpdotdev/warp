# Option to Disable Block Highlighting — Tech Spec
Product spec: `specs/GH9815/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/9815

## Context
Warp draws a colored background fill and accent border on selected terminal blocks in `BlockListElement::paint`. The rendering logic lives in `app/src/terminal/block_list_element.rs (3853-3894)`: when `is_current_block_selected` is true, a rect is drawn with `block_selection_color()` (or `block_selection_as_context_background_color()` for AI-contextable blocks) and a border using `accent()` (or `block_selection_as_context_border_color()`). There is currently no way to suppress this visual treatment.

The existing block-list settings (`app/src/terminal/block_list_settings.rs`) use the `define_settings_group!` macro to define `BlockListSettings` with two boolean settings: `show_jump_to_bottom_of_block_button` and `show_block_dividers`. Each has a corresponding:
- Context flag in `app/src/settings_view/mod.rs (372-491)` for command palette toggle binding state.
- `ToggleSettingActionPair` registration in `app/src/settings_view/appearance_page.rs (180-210)` for command palette entries.
- Widget struct (`JumpToBottomOfBlockWidget`, `ShowBlockDividersWidget`) in `appearance_page.rs (3637-3728)` for the settings UI.
- Context flag setter in `app/src/workspace/view.rs (19223-19234)`.

The new setting follows the identical pattern.

Relevant code:
- `app/src/terminal/block_list_settings.rs` — `define_settings_group!(BlockListSettings, ...)` with the existing two settings.
- `app/src/terminal/block_list_element.rs (3848-3901)` — selected-block highlight rendering and the `draw_border_above_block` suppression logic.
- `app/src/settings_view/appearance_page.rs (424-483)` — `AppearancePageAction` enum.
- `app/src/settings_view/appearance_page.rs (534-577)` — `handle_action` match arms.
- `app/src/settings_view/appearance_page.rs (1324-1331)` — Blocks category widget list construction.
- `app/src/settings_view/appearance_page.rs (2089-2120)` — `toggle_jump_to_bottom_of_block_button` and `toggle_show_block_dividers` implementations.
- `app/src/settings_view/appearance_page.rs (3637-3728)` — `JumpToBottomOfBlockWidget` and `ShowBlockDividersWidget` structs.
- `app/src/settings_view/mod.rs (372-491)` — context flag constants.
- `app/src/workspace/view.rs (19223-19234)` — context flag insertion for block list settings.
- `app/src/terminal/mod.rs` — re-exports from `block_list_settings`.

## Proposed changes

### 1. Add the setting to `BlockListSettings`
In `app/src/terminal/block_list_settings.rs`, add a new entry to the `define_settings_group!` invocation:

```rust
show_selected_block_highlight: ShowSelectedBlockHighlight {
    type: bool,
    default: true,
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "appearance.blocks.show_selected_block_highlight",
    description: "Whether to show the background and border highlight on selected blocks.",
}
```

This generates the `ShowSelectedBlockHighlight` setting type. Add a re-export in `app/src/terminal/mod.rs` alongside the existing `ShowJumpToBottomOfBlockButton` and `ShowBlockDividers` re-exports.

### 2. Add a context flag
In `app/src/settings_view/mod.rs`, add a new constant in the `flags` module:

```rust
pub const SELECTED_BLOCK_HIGHLIGHT_CONTEXT_FLAG: &str = "Selected_Block_Highlight_Enabled";
```

In `app/src/workspace/view.rs`, add a block after the existing `show_block_dividers` context flag insertion (~line 19232-19234) that inserts the new flag when the setting is true:

```rust
if *block_list_settings.show_selected_block_highlight.value() {
    context.set.insert(flags::SELECTED_BLOCK_HIGHLIGHT_CONTEXT_FLAG);
}
```

### 3. Add `AppearancePageAction` variant and handler
In `app/src/settings_view/appearance_page.rs`:

- Add `ToggleSelectedBlockHighlight` to the `AppearancePageAction` enum (after `ToggleShowBlockDividers`).
- Add a match arm in `handle_action` that calls `self.toggle_selected_block_highlight(ctx)`.
- Add a `toggle_selected_block_highlight` method following the exact pattern of `toggle_show_block_dividers`: read the current value from `BlockListSettings`, negate, send telemetry, and set the new value.

### 4. Register the command palette toggle
In `appearance_page::init_actions_from_parent_view`, after the existing `ToggleShowBlockDividers` pair (~line 196-210), add a `ToggleSettingActionPair::new` for the new toggle:

```rust
toggle_binding_pairs.push(
    ToggleSettingActionPair::new(
        "selected block highlight",
        builder(SettingsAction::AppearancePageToggle(
            AppearancePageAction::ToggleSelectedBlockHighlight,
        )),
        context,
        flags::SELECTED_BLOCK_HIGHLIGHT_CONTEXT_FLAG,
    )
    .is_supported_on_current_platform(
        BlockListSettings::as_ref(app)
            .show_selected_block_highlight
            .is_supported_on_current_platform(),
    ),
);
```

This registers the command palette entry and supports an optional keybinding.

### 5. Add the settings widget
Create `ShowSelectedBlockHighlightWidget` following the `ShowBlockDividersWidget` pattern:

```rust
#[derive(Default)]
struct ShowSelectedBlockHighlightWidget {
    switch_state: SwitchStateHandle,
}

impl SettingsWidget for ShowSelectedBlockHighlightWidget {
    type View = AppearanceSettingsPageView;

    fn search_terms(&self) -> &str {
        "selected block highlight"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let block_list_settings = BlockListSettings::as_ref(app);
        let enabled = block_list_settings.show_selected_block_highlight.value();
        render_body_item::<AppearancePageAction>(
            "Show selected block highlight".into(),
            None,
            LocalOnlyIconState::for_setting(
                ShowSelectedBlockHighlight::storage_key(),
                ShowSelectedBlockHighlight::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
            appearance
                .ui_builder()
                .switch(self.switch_state.clone())
                .check(*enabled)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(
                        AppearancePageAction::ToggleSelectedBlockHighlight,
                    );
                })
                .finish(),
            None,
        )
    }
}
```

Add this widget to the Blocks category widget list in the `build_page` method (~line 1324-1331), after the existing block widgets:

```rust
block_settings_widgets.push(Box::new(ShowSelectedBlockHighlightWidget::default()));
```

### 6. Gate the highlight rendering
In `app/src/terminal/block_list_element.rs`, the `paint` method needs to read the new setting and conditionally skip the selected-block highlight. The `BlockListElement` already holds a locked `TerminalModel` during paint, and `BlockListSettings` is a singleton entity accessible via `BlockListSettings::as_ref(app)`.

At the point where `is_current_block_selected` is checked (~line 3853), wrap the entire highlight-drawing block in a check:

```rust
let show_highlight = *BlockListSettings::as_ref(app)
    .show_selected_block_highlight
    .value();

if is_current_block_selected && show_highlight {
    // existing background + border rendering (lines 3854-3894)
}
```

The `draw_border_above_block = false` assignment at line 3899 must also be gated:

```rust
if is_top_of_continuous_selection && show_highlight {
    draw_border_above_block = false;
}
```

This ensures the gray divider border is not suppressed when the highlight is disabled (Behavior #10).

The shared-session participant selection rendering (~lines 3906-3957) must remain ungated — it draws other participants' colored borders and is independent of the local user's highlight preference (Behavior #5).

Read `show_highlight` once at the top of the paint loop iteration rather than inside the `if` block, since it is also needed for the `draw_border_above_block` guard.

### 7. Update `app/src/terminal/mod.rs` re-exports
Add `ShowSelectedBlockHighlight` to the existing re-export line that brings in `ShowJumpToBottomOfBlockButton` and `ShowBlockDividers`, so the settings view can reference it without a fully-qualified path.

## Testing and validation

### Unit tests
- `app/src/settings_view/mod_test.rs` (or the existing settings round-trip tests) — verify `ShowSelectedBlockHighlight` defaults to `true`, can be toggled to `false`, and round-trips through the TOML path `appearance.blocks.show_selected_block_highlight`.

### Build verification
- `cargo fmt` and `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings` must pass.
- `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2` must pass, confirming no compile errors from the new setting field and no regressions in existing block-related tests.

### Manual validation
Behavior-to-verification mapping (from `product.md`):

- Behavior #1, #2: Open Appearance > Blocks in settings. Confirm the new "Show selected block highlight" toggle appears after the existing block toggles, defaulted to on.
- Behavior #3: With the setting on, click a block and verify the blue/accent highlight renders as today.
- Behavior #4: Toggle the setting off. Click a block and verify no background fill or accent border appears. Verify block actions still work: right-click context menu, copy block output, keyboard navigation between blocks, AI context inclusion (shift-click).
- Behavior #5: In a shared session, have another participant select a block. Verify their colored border still renders when the local user has the highlight disabled.
- Behavior #6: With the highlight disabled, run a failing command and verify the failed-block tint still renders. Restore a session and verify the restored-block overlay still renders.
- Behavior #7: Open the command palette and search for "selected block highlight". Confirm the toggle entry appears and toggling it matches the settings switch state.
- Behavior #8: Toggle the setting off, quit Warp, relaunch. Confirm the setting persists as off.
- Behavior #9: In settings search, type "block highlight" and verify the new toggle is found.
- Behavior #10: Enable "Show block dividers" and disable "Show selected block highlight". Click a block and verify the divider border above the selected block is still visible (not suppressed).
- Behavior #11: Switch themes and verify the highlight correctly appears/disappears based on the setting in each theme.
- Behavior #12: Toggle the setting while a block is selected and verify the highlight appears/disappears immediately.
