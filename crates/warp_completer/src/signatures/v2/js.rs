//! Contains `FromWarpJs` trait implementations for converting JavaScript command signatures to
//! `warp_completer::signatures::CommandSignature`s, as well as `IntoWarpJs` implementations for
//! Rust structs that may be passed to JS functions defined on the Command Signature (e.g.
//! `GeneratorCompletionContext`).
use rquickjs::{FromJs, Function, Object, Value};
use warp_js::util::{
    get_one_or_more_optional, get_one_or_more_required, get_optional, get_required,
};
use warp_js::{FromWarpJs, IntoWarpJs, JsFunctionRegistry};

use super::{
    Argument, ArgumentValue, Command, CommandSignature, GeneratorCompletionContext, GeneratorFn,
    GeneratorResults, GeneratorScript, Opt, Priority, Suggestion, TemplateType,
};

impl<'js> FromWarpJs<'js> for CommandSignature {
    fn from_warp_js(
        ctx: rquickjs::Ctx<'js>,
        value: rquickjs::Value<'js>,
        js_function_registry: &mut JsFunctionRegistry,
    ) -> rquickjs::Result<Self> {
        let object = Object::from_value(value)?;
        let command = Command::from_warp_js(ctx, object.get("command")?, js_function_registry)?;
        Ok(Self { command })
    }
}

impl<'js> FromWarpJs<'js> for Command {
    fn from_warp_js(
        ctx: rquickjs::Ctx<'js>,
        value: rquickjs::Value<'js>,
        js_function_registry: &mut JsFunctionRegistry,
    ) -> rquickjs::Result<Self> {
        let object = Object::from_value(value)?;
        let name: String = get_required(&object, "name", js_function_registry, ctx)?;
        let alias: Vec<String> =
            get_one_or_more_optional(&object, "alias", js_function_registry, ctx)?;
        let description: Option<String> =
            get_optional(&object, "description", js_function_registry, ctx)?;
        let arguments: Vec<Argument> =
            get_one_or_more_optional(&object, "arguments", js_function_registry, ctx)?;
        let subcommands: Vec<Command> =
            get_one_or_more_optional(&object, "subcommands", js_function_registry, ctx)?;
        let options: Vec<Opt> =
            get_one_or_more_optional(&object, "options", js_function_registry, ctx)?;
        let priority: Option<i32> = get_optional(&object, "priority", js_function_registry, ctx)?;

        Ok(Command {
            name,
            alias,
            description,
            arguments,
            subcommands,
            options,
            priority: priority.map(Priority::new).unwrap_or_default(),
        })
    }
}

impl<'js> FromWarpJs<'js> for Argument {
    fn from_warp_js(
        ctx: rquickjs::Ctx<'js>,
        value: rquickjs::Value<'js>,
        js_function_registry: &mut JsFunctionRegistry,
    ) -> rquickjs::Result<Self> {
        let object = Object::from_value(value)?;
        let name: String = get_required(&object, "name", js_function_registry, ctx)?;
        let description: Option<String> =
            get_optional(&object, "description", js_function_registry, ctx)?;
        let values: Vec<ArgumentValue> =
            get_one_or_more_optional(&object, "values", js_function_registry, ctx)?;
        let optional: bool =
            get_optional(&object, "optional", js_function_registry, ctx)?.unwrap_or(false);

        Ok(Argument {
            name,
            description,
            values,
            optional,
            arity: None,
        })
    }
}

impl<'js> FromWarpJs<'js> for ArgumentValue {
    fn from_warp_js(
        ctx: rquickjs::Ctx<'js>,
        value: rquickjs::Value<'js>,
        js_function_registry: &mut JsFunctionRegistry,
    ) -> rquickjs::Result<Self> {
        // TODO(zachbai): Implement conversion + Rust representation of ArgumentValue.RootCommand
        // (see typescript schema in command-signature.d.ts).
        if value.is_object() {
            let object = Object::from_value(value)?;
            if object.contains_key("value")? {
                Ok(ArgumentValue::Suggestion(Suggestion::from_warp_js(
                    ctx,
                    object.into_value(),
                    js_function_registry,
                )?))
            } else if let Some(type_name) =
                get_optional::<TemplateType>(&object, "typeName", js_function_registry, ctx)?
            {
                let filter_name: Option<String> =
                    get_optional(&object, "filterName", js_function_registry, ctx)?;
                Ok(ArgumentValue::Template {
                    type_name,
                    filter_name,
                })
            } else if let Some(generate_suggestions_fn) = get_optional::<GeneratorFn>(
                &object,
                "generateSuggestionsFn",
                js_function_registry,
                ctx,
            )? {
                Ok(ArgumentValue::Generator(generate_suggestions_fn))
            } else {
                Err(rquickjs::Error::FromJs {
                    from: "object",
                    to: "ArgumentValue",
                    message: None,
                })
            }
        } else {
            Err(rquickjs::Error::FromJs {
                from: "object",
                to: "ArgumentValue",
                message: None,
            })
        }
    }
}

impl<'js> FromWarpJs<'js> for Suggestion {
    fn from_warp_js(
        ctx: rquickjs::Ctx<'js>,
        value: rquickjs::Value<'js>,
        js_function_registry: &mut JsFunctionRegistry,
    ) -> rquickjs::Result<Self> {
        let object = Object::from_value(value)?;
        let value: String = get_required(&object, "value", js_function_registry, ctx)?;
        let display_value: Option<String> =
            get_optional(&object, "displayValue", js_function_registry, ctx)?;
        let description: Option<String> =
            get_optional(&object, "description", js_function_registry, ctx)?;
        let priority: Option<i32> = get_optional(&object, "priority", js_function_registry, ctx)?;

        Ok(Suggestion {
            value,
            display_value,
            description,
            priority: priority.map(Priority::new).unwrap_or_default(),
        })
    }
}

impl<'js> FromWarpJs<'js> for TemplateType {
    fn from_warp_js(
        ctx: rquickjs::Ctx<'js>,
        value: rquickjs::Value<'js>,
        js_function_registry: &mut JsFunctionRegistry,
    ) -> rquickjs::Result<Self> {
        let type_string = String::from_warp_js(ctx, value, js_function_registry)?;
        match type_string.as_str() {
            "TemplateType.Files" => Ok(TemplateType::Files),
            "TemplateType.Folders" => Ok(TemplateType::Folders),
            "TemplateType.FilesAndFolders" => Ok(TemplateType::FilesAndFolders),
            _ => Err(rquickjs::Error::FromJs {
                from: "string",
                to: "TemplateType",
                message: None,
            }),
        }
    }
}

impl<'js> FromWarpJs<'js> for GeneratorFn {
    fn from_warp_js(
        ctx: rquickjs::Ctx<'js>,
        value: rquickjs::Value<'js>,
        js_function_registry: &mut JsFunctionRegistry,
    ) -> rquickjs::Result<Self> {
        if value.is_object() {
            let object = Object::from_value(value)?;
            let value: Value = object.get("script")?;
            let script = GeneratorScript::from_warp_js(ctx, value, js_function_registry)?;
            let post_process = if object.contains_key("postProcess")? {
                let function: Function = object.get("postProcess")?;
                let function_ref = js_function_registry
                    .register_js_function::<String, GeneratorResults>(function, ctx);
                Some(function_ref)
            } else {
                None
            };
            Ok(GeneratorFn::ShellCommand {
                script,
                post_process,
            })
        } else if value.is_function() {
            let function: Function = Function::from_value(value)?;
            let function_ref = js_function_registry
                .register_js_function::<GeneratorCompletionContext, GeneratorResults>(
                    function, ctx,
                );
            Ok(GeneratorFn::Custom(function_ref))
        } else {
            Err(rquickjs::Error::FromJs {
                from: "generator_fn",
                to: "GeneratorFn",
                message: None,
            })
        }
    }
}

impl<'js> FromWarpJs<'js> for GeneratorScript {
    fn from_warp_js(
        ctx: rquickjs::Ctx<'js>,
        value: rquickjs::Value<'js>,
        js_function_registry: &mut JsFunctionRegistry,
    ) -> rquickjs::Result<Self> {
        if value.is_string() {
            Ok(GeneratorScript::Static(String::from_js(ctx, value)?))
        } else if value.is_function() {
            let script_fn: Function = Function::from_value(value)?;
            let function_ref =
                js_function_registry.register_js_function::<Vec<String>, String>(script_fn, ctx);
            Ok(GeneratorScript::Dynamic(function_ref))
        } else {
            Err(rquickjs::Error::FromJs {
                from: "script",
                to: "GeneratorScript",
                message: None,
            })
        }
    }
}

impl<'js> FromWarpJs<'js> for Opt {
    fn from_warp_js(
        ctx: rquickjs::Ctx<'js>,
        value: rquickjs::Value<'js>,
        js_function_registry: &mut JsFunctionRegistry,
    ) -> rquickjs::Result<Self> {
        let object = Object::from_value(value)?;
        let name: Vec<String> =
            get_one_or_more_required(&object, "name", js_function_registry, ctx)?;
        let description: Option<String> =
            get_optional(&object, "description", js_function_registry, ctx)?;
        let required: bool =
            get_optional(&object, "required", js_function_registry, ctx)?.unwrap_or(false);
        let arguments: Vec<Argument> =
            get_one_or_more_optional(&object, "arguments", js_function_registry, ctx)?;
        let priority: Option<i32> = get_optional(&object, "priority", js_function_registry, ctx)?;

        Ok(Opt {
            name,
            description,
            arguments,
            required,
            priority: priority.map(Priority::new).unwrap_or_default(),
        })
    }
}

impl<'js> FromWarpJs<'js> for GeneratorResults {
    fn from_warp_js(
        ctx: rquickjs::Ctx<'js>,
        value: rquickjs::Value<'js>,
        js_function_registry: &mut JsFunctionRegistry,
    ) -> rquickjs::Result<Self> {
        let object = Object::from_value(value)?;
        let suggestions =
            get_one_or_more_required(&object, "suggestions", js_function_registry, ctx)?;
        let is_ordered =
            get_optional(&object, "is_ordered", js_function_registry, ctx)?.unwrap_or(false);

        Ok(GeneratorResults {
            suggestions,
            is_ordered,
        })
    }
}

impl<'js> IntoWarpJs<'js> for GeneratorCompletionContext {
    fn into_warp_js(self, ctx: rquickjs::Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let object = Object::new(ctx)?;
        object.set("tokens", self.tokens)?;
        object.set("pwd", self.pwd)?;
        Ok(object.into_value())
    }
}
