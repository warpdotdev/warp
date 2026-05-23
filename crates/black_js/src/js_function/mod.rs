//! This module contains abstractions for registering and calling plugin JS functions.
cfg_if::cfg_if! {
    if #[cfg(not(target_family = "wasm"))] {
        mod native;
        pub use native::*;
    }
}
use std::marker::PhantomData;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Struct to pass serialized function input and output, encapsulating the details of the actual
/// serialization.
///
/// This is required to pass function inputs/outputs across process boundaries, as is done to call
/// JS functions from the rust app process to be executed by the plugin host process.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct SerializedJsValue(Vec<u8>);

impl SerializedJsValue {
    pub fn from_value<T: Serialize>(input: T) -> Result<Self, bincode::Error> {
        Ok(Self(bincode::serialize::<T>(&input)?))
    }

    pub fn to_value<T: DeserializeOwned>(&self) -> Result<T, bincode::Error> {
        bincode::deserialize::<T>(&self.0[..])
    }
}

/// A unique "ref" to a registered JS function parameterized by the function's input and output
/// types.
///
/// `I` is the type of the function's input, which must implement `IntoWarpJs` and be
/// deserializable.
/// `O` is the type of the function's return value, which must implement `FromWarpJs` and be
/// serializable.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct TypedJsFunctionRef<I, O> {
    pub id: JsFunctionId,
    _input_marker: PhantomData<I>,
    _output_marker: PhantomData<O>,
}

#[cfg(feature = "test-util")]
impl<I, O> TypedJsFunctionRef<I, O> {
    pub fn new_for_test() -> Self {
        Self {
            id: JsFunctionId::new(),
            _input_marker: PhantomData,
            _output_marker: PhantomData,
        }
    }
}

/// A unique ID for a plugin function defined in JS.
///
/// This is expected to be unique across every JS function across all registered plugins.
#[derive(Copy, Clone, Serialize, Deserialize, Debug, Hash, Eq, PartialEq)]
pub struct JsFunctionId(Uuid);

impl JsFunctionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for JsFunctionId {
    fn default() -> Self {
        Self::new()
    }
}
