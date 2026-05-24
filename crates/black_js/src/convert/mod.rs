cfg_if::cfg_if! {
    if #[cfg(not(target_family = "wasm"))] {
        mod native;
        pub use native::{IntoWarpJs, FromWarpJs, util};
    }
}
