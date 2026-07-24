Closes #14152

## Summary
- add group-aware cross-window tab drag state, preview positioning, and drop finalization
- transfer complete tab groups between windows while preserving group metadata and member pane-group state
- enable horizontal and vertical group draggables to detach when drag-tabs-to-windows is enabled

## Validation
- `cargo fmt --manifest-path /workspace/warp/Cargo.toml -p warp`
- `cargo check --manifest-path /workspace/warp/Cargo.toml -p warp --lib`
- `git -C /workspace/warp --no-pager diff --check`
- Attempted `cargo test --manifest-path /workspace/warp/Cargo.toml -p warp --lib cross_window_tab_drag`; the process was killed by SIGKILL during test compilation before tests ran.
