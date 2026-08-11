---
name: add-feature-flag
description: Add a new feature flag to gate code changes in the Warp codebase.
---

# add-feature-flag

Add a new feature flag to gate code changes in the Warp codebase.

## Overview

Feature flags in Warp are compile-time flags that allow features to be selectively enabled for different channels (e.g.: Dev, Stable). They use a small runtime plumbing layer that checks if a flag is enabled.

## TUI note
The `FeatureFlag` enum (`warp_core/src/features.rs`) and the runtime `FeatureFlag::X.is_enabled()` check are SHARED by both front-ends: the GUI desktop app (`app/`) and the headless TUI (`crates/warp_tui`). The Cargo-feature steps in this skill (`app/Cargo.toml`, `app/src/lib.rs`) are GUI-app-specific. Prefer runtime `is_enabled()` checks so a flag works in both front-ends with no per-binary wiring. Only if you truly need a compile-time Cargo feature in the TUI, wire it into `crates/warp_tui/Cargo.toml` as well. Test the TUI with `./script/run-tui`.

## Steps

### 1. Add to Cargo.toml
Add the feature to `app/Cargo.toml` under the `[features]` section, but **NOT** under the `default` nested stanza:

```toml
[features]
your_feature_name = []
```

### 2. Add to FeatureFlag enum
Add a new variant to the `FeatureFlag` enum in `warp_core/src/features.rs`:

```rust
#[derive(Sequence)]
pub enum FeatureFlag {
    YourFeatureName,
}
```

### 3. Add conditional compilation directive
Add the feature to `app/src/lib.rs` with a corresponding `#[cfg(feature = "...")]` attribute to ensure it's only included when enabled:

```rust
#[cfg(feature = "your_feature_name")]
YourFeatureName,
```

### 4. Gate code with runtime checks
In your code, use the runtime check to conditionally execute feature-gated code:

```rust
if FeatureFlag::YourFeatureName.is_enabled() {
    // feature-gated behavior
}
```

### 5. (Optional) Enable for dogfood builds
To enable the feature by default for Dev/dogfood builds, add it to the `DOGFOOD_FLAGS` array in `features.rs`:

```rust
pub const DOGFOOD_FLAGS: &[FeatureFlag] = &[
    FeatureFlag::YourFeatureName,
];
```

### 6. Running with feature flags
To test locally with the feature enabled:

```bash
cargo run --features your_feature_name

# Multiple features:
cargo run --features your_feature_name,another_feature
```

## Keybindings with Feature Flags

If adding an `EditableBinding` or `FixedBinding` that's part of a gated feature, include an enabled predicate that checks the feature flag. This prevents the keybinding from appearing in keyboard settings when the feature is disabled.

Example:
```rust
EditableBinding::new(
    "action:name",
    "Action description",
    YourAction::Variant
)
.with_enabled(|| FeatureFlag::YourFeatureName.is_enabled())
.with_key_binding("cmdorctrl-key")
```

## Rolling Out to Stable

When ready to enable the feature for all Warp Stable users, add it to the `default` array in `app/Cargo.toml`:

```toml
[features]
default = [
    "your_feature_name",
    # other default features...
]
```

## Best Practices

- **Prefer runtime checks over cfg directives**: Use `FeatureFlag::YourFeatureName.is_enabled()` instead of `#[cfg(...)]` when possible, so flags can be toggled without recompilation and are easier to clean up later
- Use `#[cfg(...)]` only when code cannot compile without the flag (e.g., platform-specific code or missing dependencies)
- Keep flags high-level and product-focused rather than per-call-site
- Remove flags and dead branches after launch has stabilized
