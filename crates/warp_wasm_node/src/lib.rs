//! cdylib that re-exports the wasm32 + Node `AgentDriver` prototype entrypoint
//! (REMOTE-2264).
//!
//! The `#[wasm_bindgen]` entrypoint [`run_agent_driver_wasm`] lives in the
//! `warp` app crate (`app/src/wasm_node_driver.rs`), which owns the
//! `pub(crate)` `AgentDriver`/`TerminalDriver` types. This crate is a thin
//! cdylib wrapper so `wasm-bindgen --target nodejs` produces a Node-loadable
//! module exporting `run_agent_driver_wasm`.
//!
//! See `agents/specs/REMOTE-2264: wasm32 CLI in Node prototype.md` and
//! `agents/specs/REMOTE-2264: findings.md`.

#![cfg(target_family = "wasm")]

// Re-export the entrypoint so wasm-bindgen generates the Node-facing export
// from this cdylib.
pub use warp::run_agent_driver_wasm;
