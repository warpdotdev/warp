//! This module contains utilities for extracting properties from JS objects in Rust.
//!
//! Where possible, these utilities should be used rather than the rquickjs APIs directly to ensure
//! JS objects are converted into Rust with consistent semantics.
use rquickjs::{Ctx, Object, Result, Value};

use super::{FromWarpJs, JsFunctionRegistry};

/// Returns the value for an optional property that maybe set to either a single `T` or an array of
/// `T`'s in Javascript.
///
/// This is different from calling get_optional::Vec<String>() because it handles a values of type
/// `T` in addition to `T[]`.
///
/// For example, calling get_one_or_more_optional::<String>(object, "foo"):
///
///  { "foo": "bar" } -> Returns Ok(["bar"])
///  { "foo": ["bar", "baz"] } -> Returns Ok(["bar", "baz"])
///  {} ->  Returns Ok([])
pub fn get_one_or_more_optional<'js, T>(
    object: &Object<'js>,
    field_name: &str,
    js_function_registry: &mut JsFunctionRegistry,
    ctx: Ctx<'js>,
) -> Result<Vec<T>>
where
    T: FromWarpJs<'js>,
{
    if object.contains_key(field_name)? {
        get_one_or_more_required(object, field_name, js_function_registry, ctx)
    } else {
        Ok(vec![])
    }
}

/// Returns the value for a required property that maybe set to either a single `T` _or_ an array of
/// `T`'s in Javascript.
///
/// This is different from calling get_required::Vec<String>() because it handles a values of type
/// `T` in addition to `T[]`.
///
/// For example, calling get_one_or_more_required::<String>(object, "foo"):
///
///  { "foo": "bar" } -> Returns Ok(["bar"])
///  { "foo": ["bar", "baz"] } -> Returns Ok(["bar", "baz"])
///  {} ->  Returns Err([])
pub fn get_one_or_more_required<'js, T>(
    object: &Object<'js>,
    field_name: &str,
    js_function_registry: &mut JsFunctionRegistry,
    ctx: Ctx<'js>,
) -> Result<Vec<T>>
where
    T: FromWarpJs<'js>,
{
    let value: Value = object.get(field_name)?;
    if value.is_array() {
        Vec::<T>::from_warp_js(ctx, value, js_function_registry)
    } else {
        Ok(vec![T::from_warp_js(ctx, value, js_function_registry)?])
    }
}

/// Returns the value for a required property of type `T`.
///
/// For example, calling get_required::<String>(object, "foo"):
///
///  { "foo": "bar" } -> Returns Ok(["bar"])
///  { "foo": 13 } -> Returns Err(..)
///  {} ->  Returns Err(..)
pub fn get_required<'js, T>(
    object: &Object<'js>,
    field_name: &str,
    js_function_registry: &mut JsFunctionRegistry,
    ctx: Ctx<'js>,
) -> Result<T>
where
    T: FromWarpJs<'js>,
{
    let value = object.get(field_name)?;
    T::from_warp_js(ctx, value, js_function_registry)
}

/// Returns the value for an optional property of type `T`.
///
/// For example, calling get_optional::<String>(object, "foo"):
///
///  { "foo": "bar" } -> Returns Ok(Some("bar"))
///  { "foo": 13 } -> Returns Err(..)
///  {} ->  Returns Ok(None)
pub fn get_optional<'js, T>(
    object: &Object<'js>,
    field_name: &str,
    js_function_registry: &mut JsFunctionRegistry,
    ctx: Ctx<'js>,
) -> Result<Option<T>>
where
    T: FromWarpJs<'js>,
{
    if object.contains_key(field_name)? {
        Ok(Some(get_required::<T>(
            object,
            field_name,
            js_function_registry,
            ctx,
        )?))
    } else {
        Ok(None)
    }
}
