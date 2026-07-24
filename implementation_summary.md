# Implementation Summary

Implemented cross-window tab group dragging for issue #14152.

## Changes
- Extended the cross-window tab drag state machine to track multi-tab/group payload metadata, including source group ID, source tab count, and whether the source window should close after transfer.
- Added group-aware preview window creation so dragging a tab group out of a tab bar creates a preview/new window containing the entire group and all member pane groups.
- Added target-window handoff support for tab groups, transferring every member pane group into the target window and inserting the group as one contiguous block without splitting existing groups.
- Preserved tab group metadata during transfer, including name, color, collapsed state, and pinned state.
- Wired horizontal and vertical tab group draggables so group headers can leave the tab strip when the existing drag-tabs-to-windows feature flag is enabled, while preserving axis-locked same-window behavior when it is disabled.
- Updated drop handling so releasing a cross-window group drag finalizes the drag and performs source cleanup.
- Updated existing cross-window tab drag unit-test helper call sites for the expanded multi-tab drag API.

## Validation
- Passed: `cargo fmt --manifest-path /workspace/warp/Cargo.toml -p warp`
- Passed: `cargo check --manifest-path /workspace/warp/Cargo.toml -p warp --lib`
- Passed: `git -C /workspace/warp --no-pager diff --check`
- Attempted: `cargo test --manifest-path /workspace/warp/Cargo.toml -p warp --lib cross_window_tab_drag`
  - Result: killed by SIGKILL during test compilation before tests ran.
