# AGENTS.md

See [WARP.md](WARP.md) for the full engineering guide (architecture, coding style, feature flags, testing patterns).
See [CONTRIBUTING.md](CONTRIBUTING.md) for the contribution workflow and PR process.

## Cursor Cloud specific instructions

### Environment

- The VM runs Ubuntu 24.04 with a TigerVNC X display at `:1`.
- **XAUTHORITY must be set** for any command that needs X11 (running the app, integration tests):
  ```
  export DISPLAY=:1
  export XAUTHORITY=/home/ubuntu/.Xauthority
  ```
- The `libstdc++.so` linker symlink may be missing; if you see `unable to find library -lstdc++`, run:
  ```
  sudo ln -sf /usr/lib/gcc/x86_64-linux-gnu/13/libstdc++.so /usr/lib/x86_64-linux-gnu/libstdc++.so
  ```

### Building and running

- `cargo build --bin warp-oss` builds without GUI support (faster).
- `cargo run --bin warp-oss --features gui` launches the GUI on the VNC display.
- `./script/run` is a convenience wrapper but may call `resolve_common_skills` interactively. To skip that, set `WARP_SKIP_COMMON_SKILLS_INSTALL=1` or use `cargo run` directly.

### Linting

- `cargo fmt --check` for format checking.
- `cargo clippy --workspace --exclude warp_completer --all-targets --tests -- -D warnings` for the main workspace.
- `cargo clippy -p warp_completer --all-targets --tests -- -D warnings` for the completer crate separately.
- Both `cargo fmt` and `cargo clippy` must pass before creating or updating a PR.

### Testing

- `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2` runs the full test suite.
- **Always set `DISPLAY=:1 XAUTHORITY=/home/ubuntu/.Xauthority`** before running tests; without these, all integration tests (~220 tests) will fail with `NotSupportedError` on event loop creation.
- ~22 tests fail on master in this environment (SSH tests needing gcloud, some UI timeout tests, a few unit test assertions). These are pre-existing and not caused by the cloud environment.

### common-skills repo

- Located at `/agent/repos/common-skills`. It is a documentation/skills repo with no build system.
- Skills are installed into the warp checkout via `../common-skills/scripts/install_common_skills --repo-root /agent/repos/warp --project --if-needed --non-interactive`.
