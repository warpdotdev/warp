#[cfg_attr(feature = "v2", path = "v2.rs")]
#[cfg_attr(not(feature = "v2"), path = "legacy.rs")]
mod imp;

pub use imp::*;
