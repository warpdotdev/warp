#[cfg_attr(feature = "v2", path = "v2/mod.rs")]
#[cfg_attr(not(feature = "v2"), path = "legacy/mod.rs")]
mod imp;

pub use imp::*;

pub mod clap;

#[cfg(feature = "test-util")]
pub mod testing;
