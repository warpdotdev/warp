//! Traits for converting Rust values into `rquickjs`-compatible values on native (non-wasm)
//! platforms.
pub mod util;

use rquickjs::{Ctx, FromJs, IntoJs, Value};

use crate::JsFunctionRegistry;

/// Trait to be implemented for converting JS values into Rust values.
///
/// This is similar to `rquickjs`'s native `FromJs` trait, except it enables registering JS
/// functions in the given `JsFunctionRegistry` so these functions can be called arbitrarily.
pub trait FromWarpJs<'js>: Sized {
    fn from_warp_js(
        ctx: Ctx<'js>,
        value: Value<'js>,
        js_function_registry: &mut JsFunctionRegistry,
    ) -> rquickjs::Result<Self>;
}

/// Trait to be implemented for converting Rust values into JS values.
///
/// This is a wrapper trait over `rquickjs`'s `IntoJs` trait, which is required due to a
/// shortcoming of the Rust compiler that's surfaced by attempting to provide a blanket
/// implementation of `super::js_function::CallableJsFunction` over all `T` that implement
/// `IntoJs`. `rquickjs` contains recursive blanket implementations of `IntoJs` for generic types
/// like `Vec`, which causes issues when the Rust compiler attempts to generate `CallableJsFunction`
/// for monomorphized `TypedJsFunction`s (which have generic type params).
pub trait IntoWarpJs<'js>: Sized {
    fn into_warp_js(self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>>;
}

impl<'js, T> FromWarpJs<'js> for Vec<T>
where
    T: FromWarpJs<'js>,
{
    fn from_warp_js(
        ctx: Ctx<'js>,
        value: Value<'js>,
        js_function_registry: &mut JsFunctionRegistry,
    ) -> rquickjs::Result<Vec<T>> {
        if value.is_array() {
            let values = value.get::<Vec<Value>>()?;
            Ok(values
                .into_iter()
                .flat_map(|value| T::from_warp_js(ctx, value, js_function_registry).ok())
                .collect())
        } else {
            Err(rquickjs::Error::FromJs {
                from: "object",
                to: "Vec<T>",
                message: None,
            })
        }
    }
}

impl<'js> FromWarpJs<'js> for String {
    fn from_warp_js(
        ctx: Ctx<'js>,
        value: Value<'js>,
        _js_function_registry: &mut JsFunctionRegistry,
    ) -> rquickjs::Result<String> {
        String::from_js(ctx, value)
    }
}

impl<'js> FromWarpJs<'js> for bool {
    fn from_warp_js(
        ctx: Ctx<'js>,
        value: Value<'js>,
        _js_function_registry: &mut JsFunctionRegistry,
    ) -> rquickjs::Result<bool> {
        bool::from_js(ctx, value)
    }
}

impl<'js> FromWarpJs<'js> for i32 {
    fn from_warp_js(
        ctx: Ctx<'js>,
        value: Value<'js>,
        _js_function_registry: &mut JsFunctionRegistry,
    ) -> rquickjs::Result<i32> {
        i32::from_js(ctx, value)
    }
}

impl<'js> IntoWarpJs<'js> for String {
    fn into_warp_js(self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        self.into_js(ctx)
    }
}

impl<'js> IntoWarpJs<'js> for Vec<String> {
    fn into_warp_js(self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        self.into_js(ctx)
    }
}
