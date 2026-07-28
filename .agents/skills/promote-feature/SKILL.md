---
name: promote-feature
description: Promote a feature-flagged feature to Dogfood, Preview, or Stable in the Warp codebase. Use when a feature behind a FeatureFlag is ready to roll out to a broader audience, including wiring up the compile-time/runtime bridge and deferring flag cleanup safely.
---

# promote-feature

Guides the staged promotion of a gated `FeatureFlag` variant to Dogfood, Preview, or Stable, and schedules the follow-up cleanup.

## Overview

Feature flags have two interacting layers:
- **Runtime** (`warp_core/src/features.rs`): `DOGFOOD_FLAGS`, `PREVIEW_FLAGS`, `RELEASE_FLAGS` — enabled per-channel at startup.
- **Compile-time** (`app/Cargo.toml` + `app/src/lib.rs`): Cargo features in `[features]`. The `default = [...]` array enables a feature for all builds. `enabled_features()` in `app/src/lib.rs` bridges each Cargo feature to its `FeatureFlag` variant via `#[cfg(feature = "...")]`.

**Do not remove the flag immediately after promoting to Stable.** Keep it for at least 1–2 release cycles so a rollback is a one-line PR (remove the entry from `default`). Use the `remove-feature-flag` skill for the cleanup step later.

## TUI note
The per-channel arrays in `warp_core/src/features.rs` (`DOGFOOD_FLAGS`/`PREVIEW_FLAGS`/`RELEASE_FLAGS`) are shared and drive both the GUI desktop app (`app/`) and the headless TUI (`crates/warp_tui`) at runtime. The `app/Cargo.toml` `default` + `app/src/lib.rs` `enabled_features()` compile-time bridge is GUI-app-only. If a promoted feature should also reach the TUI, make sure it is enabled at runtime for the target channel (the shared arrays already do this) and, if the feature relies on a compile-time Cargo feature, that the TUI binary (`crates/warp_tui/Cargo.toml`) enables it too.

## Promote to Dogfood

Add the flag to `DOGFOOD_FLAGS` in `warp_core/src/features.rs`:

```rust
pub const DOGFOOD_FLAGS: &[FeatureFlag] = &[
    // ...
    FeatureFlag::YourFeature,
];
```

No other file changes needed.

## Promote to Preview

1. Add to `PREVIEW_FLAGS` in `warp_core/src/features.rs`.
2. Remove from `DOGFOOD_FLAGS` if present — Preview flags are automatically included in Dogfood builds.

```rust
pub const PREVIEW_FLAGS: &[FeatureFlag] = &[
    // ...
    FeatureFlag::YourFeature,
];
```

## Promote to Stable

This requires changes in **three files**.

### 1. `app/Cargo.toml` — add to `default`

Add the snake_case feature name to the `default = [...]` array:

```toml
default = [
    # ...
    "your_feature_name",
]
```

Prefer this over adding to `RELEASE_FLAGS` (see comment at `warp_core/src/features.rs:787-790`). It compiles the feature into all builds and enables a one-line rollback.

### 2. `app/src/lib.rs` — add to `enabled_features()` bridge

Add a `#[cfg(...)]` entry inside the `flags.extend([...])` block in `enabled_features()`, following the existing pattern:

```rust
#[cfg(feature = "your_feature_name")]
FeatureFlag::YourFeature,
```

Place it near logically related entries.

### 3. `warp_core/src/features.rs` — remove from `PREVIEW_FLAGS` / `DOGFOOD_FLAGS`

Remove the variant from whichever arrays it currently lives in:

```rust
pub const PREVIEW_FLAGS: &[FeatureFlag] = &[
    // Remove FeatureFlag::YourFeature,
];
```

### Validate

```bash
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings
```

### Create a follow-up Linear issue

After the PR lands, create a Linear issue to remind the team to remove the flag. Use the Linear MCP tool:

```
save_issue(
  title: "Remove FeatureFlag::YourFeature after stabilization",
  team: <your team>,
  assignee: "me",
  description: "FeatureFlag::YourFeature was promoted to Stable in <PR link>. Remove the flag and dead code branches after 1–2 release cycles. Follow the `remove-feature-flag` skill.",
  labels: ["tech-debt"],
  priority: 4  // Low
)
```
