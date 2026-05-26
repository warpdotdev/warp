use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use rquickjs::{Ctx, Function, Persistent};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::{FromWarpJs, IntoWarpJs, JsFunctionId, SerializedJsValue, TypedJsFunctionRef};

impl<I, O> TypedJsFunctionRef<I, O>
where
    I: for<'a> IntoWarpJs<'a> + DeserializeOwned + Clone + 'static,
    O: for<'a> FromWarpJs<'a> + Serialize + Clone + 'static,
{
    fn new(id: JsFunctionId) -> Self {
        Self {
            id,
            _input_marker: PhantomData,
            _output_marker: PhantomData,
        }
    }
}

/// A thread-local registry for JS plugin functions.
///
/// This exposes methods to register/persist a JS function (e.g. an `rquickjs::Function`) and
/// retrieve a corresponding `CallableJsFunction` given a `JsFunctionId`.
///
/// Internally, functions are wrapped with `TypedJsFunction<I, O>`, which preserves type
/// information for the function's input and output. `TypedJsFunction`s are referred to via the
/// `CallableJsFunction` trait, which makes it possible to store a collection of polymorphic
/// `TypedJsFunction`s in a single collection.
#[derive(Default)]
pub struct JsFunctionRegistry {
    function_map: HashMap<JsFunctionId, Arc<dyn CallableJsFunction>>,
    on_registered_js_function_callback: Option<Box<dyn Fn(JsFunctionId)>>,
}

impl JsFunctionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_registered_js_function(mut self, callback: impl Fn(JsFunctionId) + 'static) -> Self {
        self.on_registered_js_function_callback = Some(Box::new(callback));
        self
    }

    /// Registers the given function and returns its a `TypedJsFunctionRef` that may be used to
    /// call the function.
    ///
    /// This must be called with specified type parameters, where `I` is the type of the function's
    /// input and `O` is the type of the functions output.
    ///
    /// `I` and `O` must have corresponding `IntoWarpJs`/`FromWarpJs` implementations so they
    /// can be converted to/from JavaScript values.
    pub fn register_js_function<'js, I, O>(
        &mut self,
        js_function: Function<'js>,
        ctx: Ctx<'js>,
    ) -> TypedJsFunctionRef<I, O>
    where
        I: for<'a> IntoWarpJs<'a> + DeserializeOwned + Clone + 'static,
        O: for<'a> FromWarpJs<'a> + Serialize + Clone + 'static,
    {
        let persisted_function = Persistent::save(ctx, js_function);
        let function_id: JsFunctionId = JsFunctionId::new();
        let js_function = TypedJsFunction::<I, O>::new(persisted_function);
        self.function_map.insert(function_id, Arc::new(js_function));

        if let Some(on_registered_js_function_callback) = &self.on_registered_js_function_callback {
            on_registered_js_function_callback(function_id);
        }

        TypedJsFunctionRef::<I, O>::new(function_id)
    }

    /// Returns the `CallableJsFunction` corresponding to the given `id`, if any.
    ///
    /// Note that this returns an owned `CallableJsFunction` (backed by an owned
    /// `TypedJsFunction`), which is necessary because any use of the function (e.g. calling it)
    /// consumes the function type itself.
    pub fn get_function(&self, id: &JsFunctionId) -> Option<Arc<dyn CallableJsFunction>> {
        self.function_map.get(id).cloned()
    }
}

#[derive(thiserror::Error, Debug)]
pub enum JsFunctionError {
    #[error("Could not serialize function input to bytes: {0:?}")]
    Serialization(bincode::Error),
    #[error("Could not deserialize function output from bytes: {0:?}")]
    Deserialization(bincode::Error),
    #[error("Error occurred in quickjs: {0:?}")]
    QuickJs(#[from] rquickjs::Error),
}

/// Helper trait that allows us to treat `TypedJsFunction`s, which may have different generic type
/// parameters, uniformly.
///
/// This is akin to the `AnyView` and `AnyModel` traits used by the UI framework to similarly store
/// `View` callbacks that are actually parameterized by the type of the actual `View`
/// implementation.
pub trait CallableJsFunction {
    /// Calls the wrapped JS function with the given `input` deserialized into the appropriate
    /// `IntoWarpJs`-implemmenting Rust type, and returns the serialized bytes representation of
    /// the function's output (which is of a Rust type that implemenets `FromWarpJs`).
    fn call(
        &self,
        input: SerializedJsValue,
        function_registry: &mut JsFunctionRegistry,
        ctx: Ctx<'_>,
    ) -> Result<SerializedJsValue, JsFunctionError>;
}

impl<I, O> CallableJsFunction for TypedJsFunction<I, O>
where
    I: for<'a> IntoWarpJs<'a> + DeserializeOwned + Clone + 'static,
    O: for<'a> FromWarpJs<'a> + Serialize + Clone + 'static,
{
    fn call(
        &self,
        input: SerializedJsValue,
        function_registry: &mut JsFunctionRegistry,
        ctx: Ctx<'_>,
    ) -> Result<SerializedJsValue, JsFunctionError> {
        let input: I = input.to_value().map_err(JsFunctionError::Deserialization)?;
        let input_value = input.into_warp_js(ctx)?;
        let func = self.js_function.clone().restore(ctx)?;
        let output = O::from_warp_js(ctx, func.call((input_value,))?, function_registry)?;
        SerializedJsValue::from_value(output).map_err(JsFunctionError::Serialization)
    }
}

/// A typed wrapper around a "raw" JS function (e.g. an `rquickjs::Function`).
///
/// `I` is the type of the function's input, which must implement `IntoWarpJs`.
/// `O` is the type of the function's return value, which must implement `FromWarpJs`.
#[derive(Clone)]
struct TypedJsFunction<I, O> {
    js_function: Persistent<Function<'static>>,
    _input_marker: PhantomData<I>,
    _output_marker: PhantomData<O>,
}

impl<I, O> TypedJsFunction<I, O>
where
    I: for<'a> IntoWarpJs<'a> + DeserializeOwned + 'static,
    O: for<'a> FromWarpJs<'a> + Serialize + 'static,
{
    fn new(js_function: Persistent<Function<'static>>) -> Self {
        Self {
            js_function,
            _input_marker: PhantomData,
            _output_marker: PhantomData,
        }
    }
}
