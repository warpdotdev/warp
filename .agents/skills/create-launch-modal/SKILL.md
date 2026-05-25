---
name: create-launch-modal
description: Create a one-time launch modal in the Warp client (feature announcement, onboarding, etc.). Use when adding a new modal that should appear exactly once per user on startup, gated by a feature flag, with colors sourced from Warp theme tokens and terminal theme colors.
---

# create-launch-modal

Create a one-time launch modal — the feature-announcement design used for launches like "Orchestrate any agent, anywhere" or "Warp is now open-source."

## Reference implementation

`app/src/workspace/view/orchestration_launch_modal/` — the canonical, most up-to-date example of this pattern.

## Checklist

- [ ] Feature flag in `warp_features/src/lib.rs`
- [ ] Settings field in `app/src/settings/ai.rs`
- [ ] Trigger logic in `app/src/workspace/one_time_modal_model.rs`
- [ ] View files under `app/src/workspace/view/<name>_launch_modal/`
- [ ] Workspace wiring in `app/src/workspace/view.rs` and `app/src/workspace/mod.rs`
- [ ] Debug actions in `app/src/workspace/action.rs`
- [ ] Hero image at `app/assets/async/png/onboarding/<name>_launch_banner.png`
- [ ] Any custom icons added to `crates/warp_core/src/ui/icons.rs` + SVG in `app/assets/bundled/svg/`

---

## Step 0 – Custom icons (if needed)

If the modal uses icons not yet in the `Icon` enum, add them before writing the view.

In `crates/warp_core/src/ui/icons.rs`:

```rust
// Add to enum
YourIconName,

// Add to From<Icon> for &'static str match
Icon::YourIconName => "bundled/svg/your-icon-name.svg",
```

Drop the SVG file at `app/assets/bundled/svg/your-icon-name.svg`. Use the same 24×24 viewBox format as existing icons.

---

## Step 1 – Feature flag

Add to `crates/warp_features/src/lib.rs`:

```rust
/// Enables the <name> launch modal.
<YourModalName>LaunchModal,
```

Enable for dogfood:

```rust
pub const DOGFOOD_FLAGS: &[FeatureFlag] = &[
    FeatureFlag::<YourModalName>LaunchModal,
    // ...
];
```

---

## Step 2 – Settings field

Add to `app/src/settings/ai.rs` inside `define_settings_group!(AISettings, ...)`.
Pattern: one boolean field per modal, globally synced (not respecting user sync), private.

```rust
// This is not a user-visible setting - it's merely a one-time flag to track if the
// <name> launch modal has been shown to the user.
//
// We model it as a setting so it's only shown once to a given user regardless of the number of
// devices they use.
did_check_to_trigger_<name>_launch_modal: DidShow<Name>LaunchModal {
    type: bool,
    default: false,
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::No),
    private: true,
}
```

---

## Step 3 – OneTimeModalModel

File: `app/src/workspace/one_time_modal_model.rs`

### 3a. Add field to struct

```rust
is_<name>_launch_modal_open: bool,
```

### 3b. Initialize to false in `new()`

```rust
is_<name>_launch_modal_open: false,
```

### 3c. Pre-dismiss for new users (critical)

In the `AuthComplete` → `!is_existing_user` branch, add to the `AISettings::handle` update block alongside the other pre-dismissals. **Without this, new users see the modal on their second startup after onboarding.**

```rust
if let Err(e) = settings
    .did_check_to_trigger_<name>_launch_modal
    .set_value(true, ctx)
{
    log::warn!("Failed to mark <name> launch modal as dismissed: {e}");
}
```

### 3d. Public API methods

```rust
pub fn is_<name>_launch_modal_open(&self) -> bool {
    self.is_<name>_launch_modal_open && self.target_window_id.is_some()
}

pub fn mark_<name>_launch_modal_dismissed(&mut self, ctx: &mut ModelContext<Self>) {
    self.set_<name>_launch_modal_open(false, ctx);
}

#[cfg(debug_assertions)]
pub fn force_open_<name>_launch_modal(&mut self, ctx: &mut ModelContext<Self>) {
    self.set_<name>_launch_modal_open(true, ctx);
}
```

### 3e. Private setter

```rust
fn set_<name>_launch_modal_open(&mut self, is_open: bool, ctx: &mut ModelContext<Self>) -> bool {
    if self.is_<name>_launch_modal_open != is_open {
        self.is_<name>_launch_modal_open = is_open;
        ctx.emit(OneTimeModalEvent::VisibilityChanged { is_open });
        return true;
    }
    false
}
```

### 3f. Add to `is_any_modal_open`

```rust
|| self.is_<name>_launch_modal_open
```

### 3g. Trigger function

```rust
fn check_and_trigger_<name>_launch_modal(&mut self, ctx: &mut ModelContext<Self>) -> bool {
    if !FeatureFlag::<Name>LaunchModal.is_enabled() {
        return false;
    }

    let ai_settings = AISettings::as_ref(ctx);
    if *ai_settings.did_check_to_trigger_<name>_launch_modal {
        return false;
    }

    AISettings::handle(ctx).update(ctx, |settings, ctx| {
        if let Err(e) = settings
            .did_check_to_trigger_<name>_launch_modal
            .set_value(true, ctx)
        {
            log::warn!("Failed to mark <name> launch modal as dismissed: {e}");
        }
    });

    let should_show = !matches!(ChannelState::channel(), Channel::Integration);
    self.set_<name>_launch_modal_open(should_show, ctx);
    should_show
}
```

### 3h. Call from `check_and_trigger_all_modals`

Insert before `check_and_trigger_hoa_onboarding`:

```rust
if self.check_and_trigger_<name>_launch_modal(ctx) {
    return;
}
```

---

## Step 4 – View

Create `app/src/workspace/view/<name>_launch_modal/mod.rs`:

```rust
mod view;
pub use view::{init, <Name>LaunchModal, <Name>LaunchModalEvent};
```

Create `app/src/workspace/view/<name>_launch_modal/view.rs`. Copy from `orchestration_launch_modal/view.rs` and adapt. Key details:

### Color sources (important)

- Prefer Warp theme tokens for modal backgrounds, text, overlays, and borders:
  - background surfaces: `appearance.theme().surface_3()` (or another `surface_*` token when needed)
  - primary/subtext: `appearance.theme().main_text_color(...)` and `appearance.theme().sub_text_color(...)`
  - overlays/hover fills: `appearance.theme().surface_overlay_1()` / `surface_overlay_2()`
  - subtle borders: `appearance.theme().outline()`
- Use terminal theme colors for terminal-color accents (for example, magenta launch badge accents):
  - `appearance.theme().terminal_colors().normal.magenta`
  - `appearance.theme().ansi_overlay_1(magenta)` for low-alpha backgrounds
- Avoid hardcoded hex colors.

### Hero image

- Store at `app/assets/async/png/onboarding/<name>_launch_banner.png`
- **Aspect ratio matters**: if the image is wider than `MODAL_WIDTH/HERO_HEIGHT` (420/92 ≈ 4.57), wrap the hero `ConstrainedBox` in `Clipped::new(...)` to prevent horizontal bleed when `cover()` scales it
- Images pre-sized to exactly 420×92 need no `Clipped`; images only taller (aspect ratio < 4.57) are fine without it

```rust
const MODAL_WIDTH: f32 = 420.;
const HERO_HEIGHT: f32 = 92.;
const HERO_IMAGE_PATH: &str = "async/png/onboarding/<name>_launch_banner.png";

fn render_hero(&self) -> Box<dyn Element> {
    let hero = Clipped::new(          // only needed if image ratio > 4.57
        ConstrainedBox::new(
            Image::new(AssetSource::Bundled { path: HERO_IMAGE_PATH }, CacheOption::Original)
                .with_corner_radius(CornerRadius::with_top(Radius::Pixels(8.)))
                .cover()
                .top_aligned()
                .finish(),
        )
        .with_width(MODAL_WIDTH)
        .with_height(HERO_HEIGHT)
        .finish(),
    )
    .finish();
    // ... close button overlay via Stack + add_positioned_child
}
```

### "New" badge

Use the standard badge — 24 px tall, 8 px horizontal padding, 14 px font, pill corners, with magenta sourced from terminal theme colors:

```rust
fn render_badge(appearance: &Appearance) -> Box<dyn Element> {
    let magenta = appearance.theme().terminal_colors().normal.magenta;
    let text = Text::new_inline("New".to_string(), appearance.ui_font_family(), 14.)
        .with_color(magenta.into())
        .finish();
    ConstrainedBox::new(
        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Min)
                .with_child(text)
                .finish(),
        )
        .with_horizontal_padding(8.)
        .with_background(Fill::Solid(appearance.theme().ansi_overlay_1(magenta)))
        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
        .finish(),
    )
    .with_height(24.)
    .finish()
}
```

### URLs

Always use `https://`, not `http://`:

```rust
const LEARN_MORE_URL: &str = "https://warp.dev/your-blog-link";
```

---

## Step 5 – Workspace wiring

### `app/src/workspace/view.rs`

```rust
// Module declaration (top)
pub(crate) mod <name>_launch_modal;

// Import
use crate::workspace::view::<name>_launch_modal::{<Name>LaunchModal, <Name>LaunchModalEvent};

// Struct field
<name>_launch_modal: ViewHandle<<Name>LaunchModal>,

// In Workspace::new()
let <name>_launch_view = ctx.add_typed_action_view(<Name>LaunchModal::new);
ctx.subscribe_to_view(&<name>_launch_view, |me, _, event, ctx| {
    me.handle_<name>_launch_modal_event(event, ctx);
});

// In struct initialization
<name>_launch_modal: <name>_launch_view,

// In OneTimeModalModel subscription handler
} else if model_ref.is_<name>_launch_modal_open() {
    me.focus_<name>_launch_modal(ctx);

// In View::render (inside the should_show_modal block)
if should_show_modal && one_time_modal_model.is_<name>_launch_modal_open() {
    stack.add_child(ChildView::new(&self.<name>_launch_modal).finish());
}
```

Add event handler and focus helper:

```rust
fn handle_<name>_launch_modal_event(&mut self, event: &<Name>LaunchModalEvent, ctx: &mut ViewContext<Self>) {
    match event {
        <Name>LaunchModalEvent::Close => {
            OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
                model.mark_<name>_launch_modal_dismissed(ctx);
            });
            self.focus_active_tab(ctx);
            ctx.notify();
        }
    }
}

fn focus_<name>_launch_modal(&mut self, ctx: &mut ViewContext<Self>) {
    ctx.focus(&self.<name>_launch_modal);
}
```

### `app/src/workspace/mod.rs`

```rust
// In pub fn init()
view::<name>_launch_modal::init(app);

// In debug bindings block
EditableBinding::new(
    "workspace:open_<name>_launch_modal",
    "[Debug] Open <Name> Launch Modal",
    WorkspaceAction::Open<Name>LaunchModal,
)
.with_context_predicate(id!("Workspace")),
EditableBinding::new(
    "workspace:reset_<name>_launch_modal_state",
    "[Debug] Reset <Name> Launch Modal State",
    WorkspaceAction::Reset<Name>LaunchModalState,
)
.with_context_predicate(id!("Workspace")),
```

---

## Step 6 – Debug actions

In `app/src/workspace/action.rs`:

```rust
/// Open the <Name> Launch Modal (for debugging)
#[cfg(debug_assertions)]
Open<Name>LaunchModal,
/// Reset the <name> launch modal dismissed state (for debugging)
#[cfg(debug_assertions)]
Reset<Name>LaunchModalState,
```

Add both variants to the `is_visible_in_command_palette` `false` arm.

In `app/src/workspace/view.rs` `TypedActionView::handle_action`:

```rust
#[cfg(debug_assertions)]
Open<Name>LaunchModal => {
    OneTimeModalModel::handle(ctx).update(ctx, |model, ctx| {
        model.force_open_<name>_launch_modal(ctx);
    });
    ctx.notify();
}
#[cfg(debug_assertions)]
Reset<Name>LaunchModalState => {
    AISettings::handle(ctx).update(ctx, |settings, ctx| {
        if let Err(e) = settings
            .did_check_to_trigger_<name>_launch_modal
            .set_value(false, ctx)
        {
            log::warn!("Failed to reset <name> launch modal state: {e}");
        }
    });
}
```

---

## Behavior summary

| User type | Sees modal? |
|---|---|
| New signup | No — pre-dismissed in `AuthComplete` new-user branch |
| Not signed in | No — trigger never fires without `AuthComplete` |
| Existing user, flag enabled | Yes — on first startup after cloud prefs load |
| Integration channel | No — suppressed by `Channel::Integration` check |
| Already seen it | No — setting persists globally across devices |
