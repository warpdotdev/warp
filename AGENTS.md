# AGENTS.md

## Cursor Cloud specific instructions

This section is for Cursor Cloud agents running in headless Linux VMs (~15GB RAM, 4 CPUs).

### Environment prerequisites

The update script (run automatically on VM startup) handles dependency refresh. The following system packages and tools are expected to already be installed:

- **Rust 1.92.0** with `rustfmt` and `clippy` (pinned via `rust-toolchain.toml`)
- **protoc v25.1** (required by `prost-build` in `app/build.rs` and `crates/remote_server/build.rs`)
- **System build deps**: `build-essential`, `cmake`, `pkg-config`, `libssl-dev`, `libfreetype-dev`, `libexpat1-dev`, `libgit2-dev`, `libfontconfig1-dev`, `libasound2-dev`, `libclang-dev`, `clang-format`
- **Runtime deps**: `libx11-6`, `libxcb1`, `libxi6`, `libxcursor1`, `libxkbcommon-x11-0`, `libwayland-client0`, `libwayland-egl1`, `mesa-vulkan-drivers`, `libegl1`
- **Test deps**: `zsh`, `fish`, `vim`
- **Cargo tools**: `cargo-binstall`, `cargo-nextest`, `wgslfmt`
- **Node.js** with `corepack enable` (needed by `command-signatures-v2` crate)
- **libstdc++.so symlink**: `/usr/lib/x86_64-linux-gnu/libstdc++.so` must exist (symlink to `/usr/lib/gcc/x86_64-linux-gnu/13/libstdc++.so`); without it, `voice_input` and some test binaries fail to link.

### Key commands

Standard build/test/lint commands are documented in `WARP.md` and `CONTRIBUTING.md`. Summary:

- **Build**: `cargo build --bin warp-oss --features gui`
- **Run** (needs display): `./script/run` or `cargo run --bin warp-oss --features gui`
- **Lint**: `cargo fmt -- --check` and `cargo clippy --workspace --exclude warp_completer --all-targets --tests -- -D warnings` then `cargo clippy -p warp_completer --all-targets --tests -- -D warnings`
- **Tests**: `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2`
- **Presubmit**: `./script/presubmit` (runs fmt, clippy, clang-format, wgslfmt, and nextest)

### Cloud VM gotchas

1. **OOM during full workspace test linking**: The `app` crate's test binary (~929MB) requires significant memory to link. On 15GB VMs without swap, linking may be killed by the OOM killer. Workaround: test individual crates or use `--build-jobs 1`. The most important crate tests can be run with: `cargo nextest run -p warp_editor -p warp_graphql -p settings -p persistence -p warpui_core -p warp_terminal -p warp_util -p sum_tree -p markdown_parser -p command -p channel_versions -p warp_features -p fuzzy_match`
2. **No display server**: The GUI application (`warp-oss`) cannot run interactively in headless VMs. Use `generate_settings_schema` binary for non-GUI validation. The app binary will start but exit with EGL/XDG errors.
3. **warp_completer test failures**: Some `warp_completer` tests fail with "Tried to check FeatureFlag before feature flags were initialized" — these are pre-existing and not caused by environment issues.
4. **corepack**: Must run `corepack enable` before building, otherwise the `command-signatures-v2` crate's build script fails looking for yarn 4.
5. **Git LFS**: Run `git lfs pull` after cloning to fetch binary assets.
