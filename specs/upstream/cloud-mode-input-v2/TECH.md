# Cloud Mode Input V2 — Tech Spec

Figma: [House of Agents, node 7231-53160](https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7231-53160&m=dev).

## Context

We are reshaping the Cloud Mode composing UI to match the new Figma design. Today (V1), when a user is composing a prompt for a cloud agent, `Input` renders:

1. An optional harness selector row above the input box.
2. The editor/input box.
3. A separate `AgentInputFooter` below the input box containing the environment selector, mic/file/voice buttons, chips, model/profile selector, etc.

V2 changes the layout: a new **top row above the input box** that contains a host selector ("Warp"), the restyled harness selector ("Oz"), and (out of scope for this PR) an MCP-config button; and a **taller input box** whose **control footer is rendered inside the same rounded container**, holding the environment selector, voice button, image button, and profile/model selector. The legacy `AgentInputFooter` is not rendered in V2.

This is gated behind a new feature flag `CloudModeInputV2`. When the flag is off, V1 behavior is unchanged.

### Relevant code

- `app/src/terminal/input/agent.rs` (38-120) — `Input::render_agent_input`. Assembles the optional V1 harness row, `render_input_box`, and `AgentInputFooter` for ambient-agent composing.
- `app/src/terminal/view.rs` (21088-21095, 24871-24938) — **`TerminalView` is `Input`'s parent**. It builds a `Flex::column` that stacks `Shrinkable::new(1., output_area)` (block list, flex=1) on top of `render_input()` at its intrinsic height. That layout pins the input to the bottom of the pane; it is the authoritative reason the V1 input is not centered vertically.
- `app/src/terminal/view/ambient_agent/harness_selector.rs` — `HarnessSelector` view. The *menu* styling (header, items, hover bg, border) matches the Figma spec and is reused. The closed-state *button* uses `AgentInputButtonTheme` today and needs a theme override for V2.
- `app/src/ai/blocklist/agent_view/agent_input_footer/environment_selector.rs` — `EnvironmentSelector`. Already matches V2's chip styling (`Icon::Globe4` + `AgentInputButtonTheme`). Reused as-is.
- `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs` (183-757, 2326-2398) — `AgentInputFooter` owns the mic, file/image, profile/model selector views and the theme structs. V2 reuses those view handles via accessors rather than duplicating construction/event wiring.
- `app/src/terminal/input.rs` (1598-1600, 2140-2159, 3173-3175, 13462-13529) — where `AgentInputFooter` and `HarnessSelector` are constructed on `Input`, and where `render_input_box` lives. V2 adds a new `cloud_mode_input_v2` field on `Input`.
- `crates/warp_features/src/lib.rs` — `FeatureFlag` enum + `DOGFOOD_FLAGS`.
- `crates/warp_core/src/ui/theme/color.rs` (485-560) — `internal_colors::{neutral_2, neutral_3, fg_overlay_1, text_sub, ...}` used for V2 theme tokens.
- `crates/warp_core/src/ui/icons.rs` — `Icon::{Globe4, GitBranch, Microphone, Image, OzCloud, ChevronDown}` already exist. No new icons for this PR.

### Figma → theme token mapping

- `#9b9b9b` (button text @ 90%) → `theme.sub_text_color(surface_1)`
- `#696969` (placeholder) → `internal_colors::text_sub`
- `rgba(255,255,255,0.05)` (input bg + chip bg) → `internal_colors::fg_overlay_1`
- `#1e1e1e` (input border) → `internal_colors::neutral_2`
- `#2b2b2b` (chip border) → `internal_colors::neutral_3`
- `#1ca05a` / `#d22d1e` (diff counts) → `theme.ansi_fg_green()` / `theme.ansi_fg_red()`
- `#127b9c` (caret) → `theme.accent()`

No hex literals in client code.

## Proposed changes

### 1. Feature flag

- `crates/warp_features/src/lib.rs`: add `FeatureFlag::CloudModeInputV2`; add to `DOGFOOD_FLAGS`.
- `app/Cargo.toml`: add `cloud_mode_input_v2 = ["cloud_mode"]` and include it in the default Warp `[features]` list.
- `app/src/lib.rs`: wire `#[cfg(feature = "cloud_mode_input_v2")] FeatureFlag::CloudModeInputV2` into the compile-time flag list.

Per `add-feature-flag` skill.

### 2. `HostSelector` (new view)

New file `app/src/terminal/view/ambient_agent/host_selector.rs`. Mirrors `HarnessSelector`'s shape (ActionButton + generic `Menu<A>` positioned via `MenuPositioningProvider`), but its menu is currently stubbed with a single "Warp" entry.

```rust path=null start=null
pub enum Host { Warp }

pub enum HostSelectorAction { ToggleMenu, SelectHost(Host) }
pub enum HostSelectorEvent   { MenuVisibilityChanged { open: bool } }

pub struct HostSelector {
    button: ViewHandle<ActionButton>,
    menu: ViewHandle<Menu<HostSelectorAction>>,
    is_menu_open: bool,
    menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
    selected: Host,
}
```

Co-located in the same file: `NakedHeaderButtonTheme` — an `ActionButtonTheme` with no border and no default background (hover → `fg_overlay_1`). Text color is `theme.sub_text_color(surface_1)`. Used by both `HostSelector` *and* (via override) `HarnessSelector` in V2.

### 3. `HarnessSelector`: pluggable closed-state button theme

Add a builder on `HarnessSelector`:

```rust path=null start=null
pub fn with_button_theme(mut self, theme: Arc<dyn ActionButtonTheme>, ctx: &mut ViewContext<Self>) -> Self;
```

V1 keeps the default `AgentInputButtonTheme`; V2 passes `NakedHeaderButtonTheme`. The menu is untouched.

### 4. `CloudModeInputV2` composite view

New file `app/src/terminal/view/ambient_agent/input_v2.rs`. Owns only the views that actually render in V2, plus clones of the existing shared buttons from `AgentInputFooter`. Hidden views (MCP config, branch selector) are **not constructed**.

```rust path=null start=null
pub struct CloudModeInputV2 {
    host_selector: ViewHandle<HostSelector>,                   // new
    harness_selector: ViewHandle<HarnessSelector>,             // existing handle
    environment_selector: ViewHandle<EnvironmentSelector>,     // existing handle from AgentInputFooter
    mic_button: ViewHandle<ActionButton>,                      // shared
    image_button: ViewHandle<ActionButton>,                    // shared (current `file_button`)
    profile_model_selector: ViewHandle<ProfileModelSelector>,  // shared
    ambient_agent_view_model: ModelHandle<AmbientAgentViewModel>,
}

impl CloudModeInputV2 {
    /// Top-row: host_selector + harness_selector on the left. No MCP config.
    pub fn render_top_row(&self, app: &AppContext) -> Box<dyn Element>;

    /// Bottom row, rendered inside the same rounded input container.
    /// Left: env_selector. Right: mic, image, profile/model (adjoined pair).
    pub fn render_control_footer(&self, app: &AppContext) -> Box<dyn Element>;
}
```

`AgentInputFooter` grows small `pub fn mic_button()`, `pub fn file_button()`, `pub fn environment_selector()`, and `pub fn profile_model_selector()` accessors that return clones of the existing `ViewHandle`s. V2 consumes those handles — we do **not** re-add typed action views or duplicate event wiring.

### 5. Construction: lazy & flag-gated

`Input` stores `cloud_mode_input_v2: Option<ViewHandle<CloudModeInputV2>>`, initialized to `None`. The `HostSelector` is constructed *inside* `CloudModeInputV2::new`, not on `Input` directly, so it is never built when V2 is off.

`Input::render_agent_input` lazily constructs the composite the first time V2 is active:

```rust path=null start=null
fn ensure_cloud_mode_input_v2(&mut self, ctx: &mut ViewContext<Self>) -> ViewHandle<CloudModeInputV2> {
    if let Some(v) = self.cloud_mode_input_v2.clone() { return v; }
    let view = ctx.add_typed_action_view(|inner_ctx| {
        CloudModeInputV2::new(
            self.menu_positioning_provider.clone(),
            self.harness_selector.clone(),
            self.agent_input_footer.as_ref(inner_ctx).environment_selector().clone(),
            self.agent_input_footer.as_ref(inner_ctx).mic_button().clone(),
            self.agent_input_footer.as_ref(inner_ctx).file_button().clone(),
            self.agent_input_footer.as_ref(inner_ctx).profile_model_selector().clone(),
            self.ambient_agent_view_model.clone(),
            inner_ctx,
        )
    });
    self.cloud_mode_input_v2 = Some(view.clone());
    view
}
```

Once constructed, the handle is kept (the feature flag is not expected to toggle off mid-session; if it does, we simply stop rendering it — views are cheap and get dropped at `Input` teardown).

### 6. Centering — requires a parent-side change in `TerminalView`

The Figma mock shows the composing UI **centered horizontally and vertically** in the otherwise-empty pane. Centering cannot be achieved purely inside `Input::render_agent_input`, because `Input`'s parent (`TerminalView::render`, `app/src/terminal/view.rs:24879-24938`) already assigns the input only its intrinsic vertical height: the parent column is structured as

```
Flex::column
├─ Shrinkable::new(1., output_area)   // block list — flex factor 1, fills remaining space
└─ render_input()                    // intrinsic height, pinned to bottom
```

That structure is what pins V1's input to the bottom of the pane. Any `Align`/`Expanded` we add inside `render_agent_input` only centers within the slim band the parent grants it.

To vertically center, the parent must yield the remaining vertical space to the input. We modify `TerminalView::render` to branch on V2-composing:

```rust path=null start=null
// app/src/terminal/view.rs, inside TerminalView::render.
let show_cloud_mode_v2 = FeatureFlag::CloudModeInputV2.is_enabled()
    && FeatureFlag::CloudMode.is_enabled()
    && self.ambient_agent_view_model.as_ref(app).is_configuring_ambient_agent();

if show_cloud_mode_v2 {
    // During V2 composing:
    //   - The block list is empty (no blocks until the agent is dispatched;
    //     the progress overlay is already gated by CloudModeSetupV2).
    //   - No waterfall gap / alt-screen path is relevant.
    //   - Input takes the whole pane and centers its own content.
    column.add_child(Expanded::new(1., self.render_input()).finish());
} else {
    column.add_child(Shrinkable::new(1., output_area).finish());
    if model.is_alt_screen_active() && self.should_render_use_agent_footer(&model, app) {
        column.add_child(ChildView::new(&self.use_agent_footer).finish());
    }
    if self.is_input_box_visible(&model, app) {
        column.add_child(self.render_input());
    } else if /* ... pre-first-exchange ... */ {
        column.add_child(ambient_agent::render_loading_footer(appearance));
    }
}
```

With the parent granting all remaining vertical space, `render_agent_input` centers both axes using `Align::new(ConstrainedBox::with_max_width(720.))`:

```rust path=null start=null
// app/src/terminal/input/agent.rs, inside render_agent_input.
const CLOUD_MODE_V2_MAX_WIDTH: f32 = 720.; // From Figma `Frame 42` width.

if show_cloud_mode_v2 {
    let v2 = self.ensure_cloud_mode_input_v2(ctx);
    let stacked = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(10.) // Figma `gap=10`.
        .with_child(v2.as_ref(app).render_top_row(app))
        .with_child(self.render_cloud_mode_v2_input_box(&v2, appearance, app))
        .finish();

    // Align centers along both axes because its parent (Expanded in TerminalView)
    // gives it the full pane.
    column.add_child(
        Align::new(
            ConstrainedBox::new(stacked)
                .with_max_width(CLOUD_MODE_V2_MAX_WIDTH)
                .finish(),
        )
        .finish(),
    );
    // Legacy `AgentInputFooter` is NOT rendered in this branch.
} else {
    // ...existing V1 body (harness_row + render_input_box + AgentInputFooter)...
}
```

`render_cloud_mode_v2_input_box` is a new helper on `Input`:

- `Container::new(Flex::column().with_child(editor).with_child(v2.render_control_footer(app)))`
  `.with_background(fg_overlay_1)`
  `.with_border(Border::all(1.).with_border_fill(neutral_2))`
  `.with_corner_radius(8.)`
  where `editor` comes from the existing `render_input_box` path with its padding tweaked so `pt=10`/`pb=8`/`px=16` and a taller max-height (236px, matching `CLI_AGENT_RICH_INPUT_EDITOR_MAX_HEIGHT`).

Key layout contract: `TerminalView` owns *where* the input lives in the pane; `Input` owns *what* the input looks like. The V2 branch is the only place this split is explicit — V1 paths are untouched.

### 7. Field on `Input`

`Input` gains `cloud_mode_input_v2: Option<ViewHandle<CloudModeInputV2>>`, initialized to `None`. When the V2 flag is off, nothing is constructed. `HostSelector` lives entirely inside `CloudModeInputV2` so it is also never constructed when V2 is off.

### 8. Out of scope

- **MCP config button**: hidden entirely for this PR — **not constructed**.
- **Branch selector**: hidden entirely for this PR — **not constructed**.
- V1 code path: left intact.

## Testing and validation

Unit tests are intentionally out of scope for this PR (per explicit decision). Validation is:

1. **`cargo check --features cloud_mode_input_v2`** must pass cleanly, both with the V2 cargo feature enabled and in a default build (V1 path).
2. **Manual verification**.
   - `cargo run --features with_local_server,cloud_mode_input_v2`, then enter cloud mode composing and visually confirm:
     - Host + harness row sits above the input, no background/border in default state, hover shows subtle `fg_overlay_1`.
     - Input box is taller, rounded, with `fg_overlay_1` bg and `neutral_2` border.
     - Env/voice/image/profile+model buttons live *inside* the input box.
     - The whole composing UI is centered horizontally (≤ 720px wide) and vertically in the pane.
     - Toggling the flag off restores the previous layout (input at the bottom, block list above, legacy `AgentInputFooter` rendered).

## Risks and mitigations

- **Shared view handles**: passing `mic_button`/`file_button` etc. into `CloudModeInputV2` means both the old `AgentInputFooter` and the new V2 view can reference them. Because V2 *replaces* the footer at render time (not both), this is safe — each render produces exactly one parent that renders each handle. Guarded by feature flag + `is_configuring_ambient_agent()`.
- **Scope creep**: MCP config and branch selector are intentionally deferred; their absence should not regress any existing flow because they did not exist in V1.
- **Feature flag fallthrough**: both the cargo feature and the runtime flag are required. If only one is set, we fall back to V1.
- **Parent-side layout**: when V2-composing, `TerminalView::render` bypasses the block list, alt-screen, and use-agent-footer branches and yields the whole pane to the input. If a future change starts inserting content into the block list *during composing* (e.g. a new inline banner), it will be invisible in V2. Since unit tests are deferred, rely on manual verification + code review to catch regressions here.
- **Mid-session flag toggle**: `CloudModeInputV2` is a compile-time feature but may be toggled at runtime in dev builds. Because the V2 composite is lazily constructed, flipping the flag *on* mid-session is fine. Flipping it *off* simply stops rendering the composite; its view handle is retained until `Input` is dropped.

## Follow-ups

- Wire the MCP config button (dialog, JSON validation, persistence).
- Replace the hidden branch selector with a real branch picker tied to repo status.
- Populate `HostSelector` with real host options once the server contract exists.
