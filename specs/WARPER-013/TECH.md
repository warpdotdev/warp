# WARPER-013: remove the browser WASM target

## Context

WASM here means the Rust application can be compiled to `wasm32-unknown-unknown` and loaded by a web page, so parts of the Warp/Warper UI run inside a browser rather than as the native desktop app. This is not WebAssembly used inside the desktop app for a plugin or sandbox. It is a separate browser build target.

The target is still present:

- `Cargo.toml:446-464` defines `release-wasm`, `release-wasm-debug_assertions`, and `dev-wasm` profiles.
- `app/Cargo.toml:275-292` declares browser-only dependencies such as `wasm-bindgen`, `gloo`, `web-sys`, and `warp_web_event_bus`.
- `app/src/platform/wasm.rs` initializes the WASM runtime and exposes host-page handoff.
- `app/src/wasm_nux_dialog.rs` implements a browser-only dialog that asks users to use the native app or stay on web.
- `crates/warpui/src/windowing/winit/wasm.rs:1-80` implements browser clipboard support through `web_sys::Clipboard`.

The target is also intentionally not equivalent to the local desktop app:

- `app/build.rs:140-141` runs WASM asset-copy behavior only for the browser target.
- `app/build.rs:158-164` enables `local_fs` and `local_tty` only when `target_family != "wasm"`.
- `app/Cargo.toml:607-617` describes `local_fs` as the feature for local filesystem APIs.
- `crates/ai/build.rs` enables `ai/local_fs` only outside WASM.
- `app/src/code_review/code_review_view.rs:1823-1826` contains a WASM branch that says code review is not available on WASM.

The likely original purpose was Warp on Web: a browser-hosted Warp UI that could reuse large parts of the Rust UI stack while substituting browser APIs for clipboard, rendering, networking, asset loading, mobile viewport behavior, and host-page authentication handoff. In Warper's local-first fork, this target appears outside the core desktop goal and keeps compatibility branches alive across many modules.

## Proposed Changes

### 1. Delete build-target entry points

- Delete `release-wasm`, `release-wasm-debug_assertions`, and `dev-wasm` from `Cargo.toml`.
- Delete `[target.'cfg(target_family = "wasm")'.dependencies]` from `app/Cargo.toml`.
- Delete direct workspace dependencies used only by the browser target after `cargo metadata` shows no native package still selects them.
- Delete browser build references from CI, release scripts, docs, and local scripts: `release-wasm`, `dev-wasm`, `wasm32`, `wasm-bindgen`, `wasm-pack`, `ASSET_TARGET_DIR`.

### 2. Apply exact cfg rewrite rules

Use these rules across `app/src` and `crates/**/src`:

| Current pattern | Required operation |
| --- | --- |
| `#[cfg(target_family = "wasm")]` on an item | Delete the item. |
| `#[cfg(target_arch = "wasm32")]` on an item | Delete the item. |
| `#[cfg(not(target_family = "wasm"))]` on an item | Delete only the attribute. Keep the item. |
| `#[cfg(not(target_arch = "wasm32"))]` on an item | Delete only the attribute. Keep the item. |
| `#[cfg_attr(target_family = "wasm", ...)]` | Delete only the `cfg_attr`. Keep the item. |
| `#[cfg_attr(target_arch = "wasm32", ...)]` | Delete only the `cfg_attr`. Keep the item. |
| `#[cfg_attr(not(target_family = "wasm"), path = "native.rs")]` plus `#[cfg_attr(target_family = "wasm", path = "wasm.rs")]` | Replace both attributes with `#[path = "native.rs"]`. Delete `wasm.rs`. |
| `cfg_if!` with a WASM branch and a native branch | Replace the whole `cfg_if!` with the native branch contents. |
| `if #[cfg(target_family = "wasm")]` expression | Delete the WASM arm. Keep the native arm as normal code. |
| `if #[cfg(not(target_family = "wasm"))]` expression | Replace the conditional expression with its native body. |
| `target_family == "wasm"` runtime check | Delete the WASM branch. |
| `target_family != "wasm"` runtime check guarding native work | Remove the WASM comparison and keep the native work. |

Do not add replacement WASM stubs, placeholder modules, no-op implementations, or new "unsupported on wasm" errors. The target state is no browser target, not graceful degradation.

### 3. Delete concrete browser modules

Delete these files and directories:

- `app/src/platform/wasm.rs`
- `app/src/wasm_nux_dialog.rs`
- `app/src/code/local_code_editor_wasm.rs`
- `app/src/code/wasm.rs`
- `app/src/plugin/host/wasm/`
- `app/src/workspace/view/wasm_view.rs`
- `app/src/workspace/view/global_search/model_wasm.rs`
- `app/src/ai/mcp/templatable_manager/wasm.rs`
- `app/src/ai/outline/wasm.rs`
- `app/src/user_config/wasm.rs`
- `crates/ipc/src/wasm.rs`
- `crates/warp_logging/src/wasm.rs`
- `crates/warpui/src/platform/wasm/`
- `crates/warpui/src/windowing/winit/wasm.rs`
- `crates/warpui/src/windowing/winit/notifications/wasm.rs`
- `crates/warpui_core/src/async/wasm/`
- `crates/warpui_core/src/platform/wasm.rs`

### 4. Rewrite concrete module switches

- In `app/src/platform/mod.rs`, delete `pub mod wasm` and make `init()` an empty native function.
- In `app/src/lib.rs`, delete `mod wasm_nux_dialog` and `mod font_fallback`; delete `cfg(not(target_family = "wasm"))` from native imports and keep the imports.
- In `app/src/code/mod.rs`, replace the `local_code_editor` path switch with the native `local_code_editor.rs` module and replace the `view` path switch with the native `view.rs` module.
- In `app/src/plugin/mod.rs`, replace the native/wasm host path switch with the native host module path.
- In `app/src/ai/mcp/templatable_manager.rs`, remove the WASM module selection and keep the native manager implementation.
- In `crates/ipc/src/lib.rs`, replace the platform path switch with `#[path = "native.rs"] mod platform;` and remove the WebWorkers/WASM wording from the module docs.
- In `crates/warp_logging/src/lib.rs`, replace the native/wasm path switch with the native implementation.
- In `crates/warpui_core/src/async/mod.rs`, replace the `cfg_if!` implementation selection with `mod native; use native as imp;`.
- In `crates/warpui/src/platform/mod.rs`, delete `pub mod wasm`; remove the WASM branch from `current`; make `is_mobile_device()` return `false`.
- In `crates/warpui/src/windowing/winit/mod.rs`, delete `pub mod wasm`.
- In `crates/warpui/src/windowing/winit/notifications/mod.rs`, select the native notification implementation directly.

### 5. Rewrite `app/build.rs`

- Remove `target_family` from `main()` and `add_features()`.
- Change the macOS build guard from `target_os == "macos" && target_family != "wasm"` to `target_os == "macos"`.
- Delete the `target_family == "wasm"` branch that calls `copy_async_assets()`.
- Delete `copy_async_assets()`.
- Remove imports used only by `copy_async_assets()`: `sha2::Digest`, `ASSETS_DIR`, `ASYNC_ASSETS_DIR`, and `REMOTE_ASSETS_DIR`.
- Keep `local_fs` and `local_tty` feature emission unconditional in `add_features()`.

### 6. Remove remaining browser-only branches

- Run `rg 'target_family = "wasm"|target_arch = "wasm32"|wasm32|release-wasm|dev-wasm|wasm-bindgen|wasm-pack|ASSET_TARGET_DIR'`.
- Apply the rewrite rules from section 2 to every remaining source hit.
- Delete browser-only tests selected only for WASM.
- Keep native tests by deleting only their `not(wasm)` attributes.
- Keep `local_fs` and `local_tty` as native capability gates. Do not delete them as part of this spec.

### 7. Update documentation and comments

- Delete comments that describe browser/WASM behavior.
- Delete comments that say a path is "not supported on wasm".
- Document Warper as native desktop and CLI only.
- Document `local_fs` and `local_tty` as native capability gates, not browser compatibility gates.

## Testing and Validation

- Run repository searches proving no active scripts, CI workflows, or docs reference the removed target names: `release-wasm`, `dev-wasm`, `wasm32`, `wasm-bindgen`, `wasm-pack`, and `ASSET_TARGET_DIR`.
- Run repository searches proving no app or crate source still contains `cfg(target_family = "wasm")`, `cfg(target_arch = "wasm32")`, `target_family == "wasm"`, `target_family != "wasm"`, or browser-only WASM modules.
- Run `cargo check` for the default native app target.
- Run the existing native verification used for this fork, including local terminal and code review smoke paths.
- Run `cargo check --all-targets` after the first cleanup pass.
- Verify `cargo metadata` no longer contains browser-only packages introduced solely for the app WASM target.
- Verify `app/build.rs` no longer has target-family WASM behavior.

## Risks and Mitigations

- The main risk is deleting native code protected by `cfg(not(target_family = "wasm"))`. The rule is fixed: remove the attribute and keep the item.
- The second risk is conflating `wasm` with `not local_fs`. Some `local_fs` gates may still be useful for headless or test builds. Do not delete `local_fs` wholesale as part of the first pass.
- The third risk is dependency churn. Browser-only crates may be pulled indirectly by shared crates. Remove direct references first, then let `cargo check` and `cargo metadata` show what remains.
