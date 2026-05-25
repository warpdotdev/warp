//! `{{...}}` placeholder substitution and leading-`~` home expansion for
//! launch-time values inside an `LspServerDescriptor`.
//!
//! Substitution is delegated to `crates/handlebars`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use handlebars::{get_arguments, render_template};
use serde_json::Value;

/// Prefix that marks a placeholder as an environment-variable lookup.
/// `{{env_HOME}}` expands to the value of `$HOME`.
const ENV_PREFIX: &str = "env_";

/// Per-launch context for placeholder expansion. Resolves the four supported
/// `{{...}}` names and the special `{{env_VAR}}` form.
///
/// The context also dedupes "unknown placeholder" warnings so each unique
/// unknown name is logged at most once per launch.
pub struct LspPlaceholderContext {
    /// Absolute path of the workspace this LSP server is being launched for.
    /// Resolves `{{workspace_root}}`.
    pub workspace_root: PathBuf,
    /// Short stable identifier for the workspace, safe as a directory-name
    /// component. Resolves `{{workspace_slug}}`.
    pub workspace_slug: String,
    /// Per-server, per-user cache directory owned by Warp. Resolves
    /// `{{cache_dir}}`.
    pub cache_dir: PathBuf,
    /// Names of unknown placeholders already warned about this launch. Wrapped
    /// in a `Mutex` because expansion runs over many strings (command, args,
    /// env, init options) and we want dedupe across all of them through a
    /// shared `&LspPlaceholderContext`.
    warned: Mutex<HashSet<String>>,
}

impl LspPlaceholderContext {
    pub fn new(workspace_root: PathBuf, workspace_slug: String, cache_dir: PathBuf) -> Self {
        Self {
            workspace_root,
            workspace_slug,
            cache_dir,
            warned: Mutex::new(HashSet::new()),
        }
    }
}

/// Expands `{{...}}` placeholders and a leading `~` / `~/` in the input
/// string and returns the result.
///
/// Substitution is single-pass: a substituted value containing `{{...}}` is
/// not re-expanded. The literal sequence `{{{name}}}` (three braces on each
/// side) passes through verbatim. Unknown placeholders are passed through
/// verbatim and logged once per unique name per launch.
pub fn expand(input: &str, ctx: &LspPlaceholderContext) -> String {
    let context = build_context(input, ctx);
    let rendered = render_template(input, &context);
    shellexpand::tilde(&rendered).into_owned()
}

/// Walks a JSON value and applies `expand` to every string leaf. Non-string
/// leaves pass through unchanged. Used for the `initialization_options`
/// payload.
pub fn expand_json(value: &Value, ctx: &LspPlaceholderContext) -> Value {
    match value {
        Value::String(s) => Value::String(expand(s, ctx)),
        Value::Array(items) => Value::Array(items.iter().map(|v| expand_json(v, ctx)).collect()),
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                // Keys are not expanded; only values.
                out.insert(k.clone(), expand_json(v, ctx));
            }
            Value::Object(out)
        }
        // Numbers, bools, null pass through unchanged.
        _ => value.clone(),
    }
}

/// Builds the substitution context for a single input string. Discovers the
/// placeholders referenced via `get_arguments`, populates known ones with
/// their resolved values, and logs unknown ones once per launch.
///
/// Placeholders absent from the returned map are left as-is by
/// `render_template`, which is how unknown placeholders end up passing
/// through verbatim.
fn build_context(input: &str, ctx: &LspPlaceholderContext) -> HashMap<String, String> {
    let names = get_arguments(input);
    let mut context = HashMap::with_capacity(names.len());

    for name in names {
        match resolve_placeholder(&name, ctx) {
            Some(value) => {
                context.insert(name, value);
            }
            None => {
                warn_unknown(&name, ctx);
                // Intentionally not inserted: `render_template` leaves the
                // placeholder text in place when no value is provided.
            }
        }
    }

    context
}

/// Whitelist of named placeholders. Adding a new name requires a deliberate
/// edit to this match — nothing about Warp's internals (data dir, state dir,
/// config dir, app version, channel/profile, etc.) leaks via the placeholder
/// system unless an arm is added here. The `env_` prefix is the one
/// exception: it intentionally accepts any environment variable name.
fn resolve_placeholder(name: &str, ctx: &LspPlaceholderContext) -> Option<String> {
    match name {
        "workspace_root" => Some(path_to_string(&ctx.workspace_root)),
        "workspace_slug" => Some(ctx.workspace_slug.clone()),
        "cache_dir" => Some(path_to_string(&ctx.cache_dir)),
        other => other
            .strip_prefix(ENV_PREFIX)
            // Undefined variable expands to the empty string.
            .map(|env_name| std::env::var(env_name).unwrap_or_default()),
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn warn_unknown(name: &str, ctx: &LspPlaceholderContext) {
    let is_new = ctx
        .warned
        .lock()
        .expect("warned set lock poisoned")
        .insert(name.to_string());
    if is_new {
        log::warn!("unknown LSP descriptor placeholder {{{{{name}}}}} (passed through verbatim)",);
    }
}

#[cfg(test)]
#[path = "placeholder_tests.rs"]
mod tests;
