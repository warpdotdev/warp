use serde::{Deserialize, Serialize};

/// Wrapper around `u32` for use with the `Uint` GraphQL scalar.
///
/// Cynic's `impl_scalar!` macro can't target a primitive type directly (the
/// orphan rule forbids implementing a foreign trait on a foreign type), so we
/// expose this local newtype instead.
#[derive(Copy, Clone, Debug, Default, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(transparent)]
pub struct Uint32(pub u32);

impl From<u32> for Uint32 {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<Uint32> for u32 {
    fn from(value: Uint32) -> Self {
        value.0
    }
}
