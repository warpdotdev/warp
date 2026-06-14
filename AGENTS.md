## Cursor Cloud specific instructions

### Overview

Warp is a Rust-based terminal emulator (65+ crate workspace). The OSS binary (`warp-oss`) builds and runs without any external backend services. See `WARP.md` for architecture details and `CONTRIBUTING.md` for the development workflow.

### Building and Running

- **Build**: `cargo build --bin warp-oss --features gui`
- **Run**: `WARP_SKIP_COMMON_SKILLS_INSTALL=1 cargo run --bin warp-oss --features gui` (the skip flag avoids interactive prompts from common-skills installation)
- **Run via script**: `WARP_SKIP_COMMON_SKILLS_INSTALL=1 ./script/run` (also works, auto-detects OSS channel)
- The `gui` feature flag is required for the graphical application.

### Linting

- `cargo fmt -- --check`
- `cargo clippy --workspace --exclude warp_completer --all-targets --tests -- -D warnings`
- `cargo clippy -p warp_completer --all-targets --tests -- -D warnings` (separate run with default features)
- See `./script/presubmit` for the full presubmit suite.

### Testing

- `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2`
- Integration tests (`crates/integration/`) require a display server (X11/Xvfb). In headless Cloud Agent VMs, most integration tests will fail with X11 auth errors — this is expected.
- To run integration tests headlessly, start Xvfb: `Xvfb :99 -screen 0 1280x1024x24 &` and set `DISPLAY=:99`.
- `command-signatures-v2` is excluded from the default test run; it requires Node.js with corepack enabled and yarn 4.

### Environment Gotchas

- **corepack / yarn 4**: The `command-signatures-v2` crate build script requires `corepack enable` and yarn >= 4.0.1. Without this, `cargo clippy`/`cargo build` will fail on that crate.
- **libstdc++.so symlink**: On Ubuntu 24.04, the linker may fail to find `-lstdc++` because `libstdc++.so` is only in `/usr/lib/gcc/x86_64-linux-gnu/13/`. Create a symlink: `sudo ln -sf /usr/lib/gcc/x86_64-linux-gnu/13/libstdc++.so /usr/lib/x86_64-linux-gnu/libstdc++.so`
- **protoc**: Must be >= 3.15 for proto3 `optional` fields. The `script/linux/install_build_deps` installs v25.1 from GitHub releases.
- **`WARP_SKIP_COMMON_SKILLS_INSTALL=1`**: Set this when running `./script/run` to avoid interactive prompts about common-skills installation.
