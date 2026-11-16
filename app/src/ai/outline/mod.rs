use ai::index::Outline;

cfg_if::cfg_if! {
    if #[cfg(target_family = "wasm")] {
        mod wasm;
        pub use wasm::*;
    } else {
        mod native;
        pub use native::*;
    }
}

#[cfg_attr(target_family = "wasm", allow(dead_code))]
#[derive(Debug)]
pub enum OutlineStatus {
    /// The outline is being computed.
    Pending,
    /// The successfully computed outline.
    Complete(Outline),
    /// Outline creation failed.
    Failed,
}
