---
name: remove-feature-flag
description: Remove a feature flag after it has been rolled out and stabilized in the Warp codebase.
---

# remove-feature-flag

Remove a feature flag after it has been rolled out and stabilized in the Warp codebase.

## Overview

After a feature flag has been enabled for all users and has stabilized in production, the flag should be removed to reduce technical debt and simplify the codebase. This involves removing the flag definition and all conditional checks.

## When to Remove

Remove a feature flag when:
- The feature has been enabled in `default` features in `app/Cargo.toml`
- The feature has been stable in production for a reasonable period
- There are no plans to disable the feature or provide configuration options
- The team agrees the feature is permanent

## Steps

### 1. Remove from app/Cargo.toml
Remove the feature from both the `[features]` section and the `default` array:

```toml
[features]
default = [
    # Remove "your_feature_name" from here
]

# Remove this line:
# your_feature_name = []
```

### 2. Remove from FeatureFlag enum
Remove the variant from the `FeatureFlag` enum in `warp_core/src/features.rs`:

```rust
#[derive(Sequence)]
pub enum FeatureFlag {
    // Remove YourFeatureName,
}
```

### 3. Remove from app/src/lib.rs
Remove the conditional compilation directive:

```rust
// Remove these lines:
// #[cfg(feature = "your_feature_name")]
// YourFeatureName,
```

### 4. Remove from DOGFOOD_FLAGS/PREVIEW_FLAGS/RELEASE_FLAGS
If the flag was listed in any of these arrays in `features.rs`, remove it:

```rust
pub const DOGFOOD_FLAGS: &[FeatureFlag] = &[
    // Remove FeatureFlag::YourFeatureName,
];
```

### 5. Remove all runtime checks and dead code
Find and remove all `FeatureFlag::YourFeatureName.is_enabled()` checks throughout the codebase:

**Before:**
```rust
if FeatureFlag::YourFeatureName.is_enabled() {
    // new behavior
} else {
    // old behavior (dead code)
}
```

**After:**
```rust
// new behavior (unconditionally enabled)
```

Use ripgrep to find all occurrences:
```bash
rg "YourFeatureName" app/ warp_core/
```

### 6. Remove keybinding predicates
If the feature flag was used in keybinding enabled predicates, remove the predicate:

**Before:**
```rust
EditableBinding::new(
    "action:name",
    "Action description",
    YourAction::Variant
)
.with_enabled(|| FeatureFlag::YourFeatureName.is_enabled())
.with_key_binding("cmdorctrl-key")
```

**After:**
```rust
EditableBinding::new(
    "action:name",
    "Action description",
    YourAction::Variant
)
.with_key_binding("cmdorctrl-key")
```

### 7. Clean up dead code branches
Remove any code paths that were only executed when the feature was disabled (the `else` branches in feature checks). These are now dead code.

### 8. Run tests and validation
After removing the flag:

```bash
# Format and lint
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings

# Run tests
cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2

# Build the app
cargo run
```

## Best Practices

- Remove feature flags promptly after they're no longer needed to reduce technical debt
- When removing a flag, remove ALL related code (checks, dead branches, keybinding predicates)
- Use grep/ripgrep to ensure you've found all occurrences
- Test thoroughly after removal to ensure no regressions
- Consider doing flag removal in a separate PR for easier review

## Example Search Commands

```bash
# Find all occurrences of the flag name
rg "YourFeatureName" app/ warp_core/

# Find feature flag checks
rg "FeatureFlag::YourFeatureName" app/

# Find cfg attributes
rg 'cfg\(feature = "your_feature_name"\)' app/
```
