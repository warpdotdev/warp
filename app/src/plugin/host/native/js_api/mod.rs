use rquickjs::{Ctx, Function, Object};

use super::plugin::PluginHandle;
use crate::report_error;

cfg_if::cfg_if! {
    if #[cfg(feature = "completions_v2")] {
        use rquickjs::{prelude::MutFn, Value};
        use warp_completer::signatures::CommandSignature;
        use warp_js::FromWarpJs;
    }
}

/// Returns a JS object representing the Warp Plugin API exposed to external JavaScript plugins.
///
/// Currently, the API contains a single "completions" namespace for registering command
/// signatures.
pub fn warp(
    #[allow(unused_variables)] plugin: PluginHandle,
    ctx: Ctx<'_>,
) -> rquickjs::Result<Object<'_>> {
    let api = Object::new(ctx)?;
    #[cfg(feature = "completions_v2")]
    api.set("completions", completions(plugin, ctx)?)?;
    Ok(api)
}

/// Returns a JS object to be used as a the `console` global, implementing `console.log()` and
/// `console.err()`.
pub fn console(ctx: Ctx<'_>) -> rquickjs::Result<Object<'_>> {
    let console = Object::new(ctx)?;
    console.set(
        "log",
        Function::new(ctx, |message: String| {
            log::info!("{message}");
        }),
    )?;
    console.set(
        "err",
        Function::new(ctx, |message: String| {
            report_error!("Plugin JS console error", extra: { "message" => %message });
        }),
    )?;
    Ok(console)
}

/// Returns a JS object representing the Completions namespace for the Warp Plugin API.
///
/// API methods:
///
/// `registerCommandSignature(signature: CommandSignature[] | CommandSignature)`: Registers
///     the given command signature(s) to be used for completions.
#[cfg(feature = "completions_v2")]
fn completions<'js>(plugin: PluginHandle, ctx: Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    let completions = Object::new(ctx)?;
    completions.set(
        "registerCommandSignature",
        Function::new(
            ctx,
            MutFn::from(move |val: Value<'js>| {
                if val.is_array() {
                    let mut plugin = plugin.get_mut();
                    match Vec::<CommandSignature>::from_warp_js(
                        ctx,
                        val,
                        plugin.js_function_registry_mut(),
                    ) {
                        Ok(signatures) => plugin.register_command_signatures(signatures),
                        Err(e) => {
                            log::warn!("Attempted to register invalid JS CommandSignatures {e:?}")
                        }
                    }
                } else if val.is_object() {
                    let mut plugin = plugin.get_mut();
                    match CommandSignature::from_warp_js(
                        ctx,
                        val,
                        plugin.js_function_registry_mut(),
                    ) {
                        Ok(signature) => {
                            plugin.register_command_signatures(vec![signature]);
                        }
                        Err(e) => {
                            log::warn!("Attempted to register invalid JS CommandSignature {e:?}")
                        }
                    }
                }
            }),
        ),
    )?;
    Ok(completions)
}
