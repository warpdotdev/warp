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

Use ripgrep to find all occurrences. The shared `FeatureFlag` may be used from the headless TUI, so search `crates/warp_tui/` (and other non-`app/` crates), not just `app/` and `warp_core/`:
```bash
rg "YourFeatureName" app/ warp_core/ crates/warp_tui/
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
# Run affected tests first
cargo nextest run -p <affected-package>

# Then run the applicable Clippy check
cargo clippy -p <affected-package> --all-targets --tests -- -D warnings

# Format once after the code is settled
./script/format
```

Add affected packages or test filters when the flag crosses package boundaries.

Do not run the full workspace suite, launch the GUI or TUI, rerun earlier checks after formatting, or add `./script/presubmit` unless the user, task, or approved spec explicitly requires it.

CI owns broader platform and workspace coverage.

## Best Practices

- Remove feature flags promptly after they're no longer needed to reduce technical debt
- When removing a flag, remove ALL related code (checks, dead branches, keybinding predicates)
- Use grep/ripgrep to ensure you've found all occurrences
- Test the affected behavior after removal to ensure no regressions
- Consider doing flag removal in a separate PR for easier review

## Example Search Commands

```bash
# Find all occurrences of the flag name (include the TUI and other non-app crates)
rg "YourFeatureName" app/ warp_core/ crates/warp_tui/

# Find feature flag checks
rg "FeatureFlag::YourFeatureName" app/ crates/warp_tui/

# Find cfg attributes
rg 'cfg\(feature = "your_feature_name"\)' app/
```
