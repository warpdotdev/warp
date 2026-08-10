//! `${PLUGIN_ROOT}`/`${PLUGIN_DATA}` expansion and stdio launch planning.
//!
//! Expansion here is not Warp's `{{variable}}` templating and must not be routed through it:
//! Agent Plugins §9.2 defines a single non-recursive textual replacement of exactly two
//! placeholders, applied to exactly three places — every element of `args`, every value in
//! `env`, and `cwd`. Anything else, including `command`, `env` keys, URLs, and headers, stays
//! literal.
//!
//! [`plan_stdio_launch`] produces an inert description of a launch. It creates no directory,
//! spawns no process, and touches no global state, so a plugin's configuration can be validated
//! and displayed without ever executing the package's code.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::diagnostics::{PluginDiagnostic, PluginDiagnosticCode};
use super::paths::{is_plugin_relative, resolve_contained, verify_contained};

/// The environment variable naming the filesystem-resolved plugin root.
pub const PLUGIN_ROOT_VAR: &str = "PLUGIN_ROOT";

/// The environment variable naming the plugin instance's persistent data directory.
pub const PLUGIN_DATA_VAR: &str = "PLUGIN_DATA";

/// The literal text expanded to the plugin root. Shared so that validation elsewhere matches the
/// placeholder itself rather than the bare variable name, which is legitimate text in other
/// positions.
pub(crate) const PLUGIN_ROOT_PLACEHOLDER: &str = "${PLUGIN_ROOT}";

/// The literal text expanded to the plugin's persistent data directory.
pub(crate) const PLUGIN_DATA_PLACEHOLDER: &str = "${PLUGIN_DATA}";

/// The two absolute paths a plugin's placeholders expand to.
#[derive(Debug, Clone, Copy)]
pub struct PluginPlaceholders<'a> {
    pub plugin_root: &'a Path,
    pub plugin_data: &'a Path,
}

/// How a stdio `command` was resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedCommand {
    /// A bare executable name, to be resolved with the platform's executable search rules.
    ///
    /// Agent Plugins leaves whether a configured `PATH` participates in this search up to the
    /// client, and forbids conformant plugins from depending on the answer.
    BareName(String),
    /// A plugin-relative path already resolved and confirmed inside the plugin root.
    PluginRelative(PathBuf),
}

/// Everything needed to launch one plugin stdio server, and nothing that launches it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdioLaunchPlan {
    pub command: ResolvedCommand,
    /// The argument vector, kept separate from `command` so the executable stays one token.
    pub args: Vec<String>,
    /// Variables to overlay on the base environment, in application order.
    ///
    /// The configured `env` comes first and the two authoritative variables come last, so a
    /// package cannot displace them (§9.1).
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
    /// The directory that must exist and be writable before the process starts.
    pub plugin_data: PathBuf,
}

/// Replaces every exact `${PLUGIN_ROOT}` and `${PLUGIN_DATA}` occurrence, once.
///
/// The scan advances past inserted text, so a path that itself contains something that looks
/// like a placeholder is never re-expanded. Unrecognized placeholder-like text is left alone.
pub fn expand_placeholders(input: &str, placeholders: &PluginPlaceholders) -> String {
    let plugin_root = placeholders.plugin_root.to_string_lossy();
    let plugin_data = placeholders.plugin_data.to_string_lossy();

    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        if let Some(remainder) = tail.strip_prefix(PLUGIN_ROOT_PLACEHOLDER) {
            out.push_str(&plugin_root);
            rest = remainder;
        } else if let Some(remainder) = tail.strip_prefix(PLUGIN_DATA_PLACEHOLDER) {
            out.push_str(&plugin_data);
            rest = remainder;
        } else {
            // Not one of ours: emit the `${` literally and keep scanning after it, so a later
            // real placeholder in the same string is still found.
            out.push_str("${");
            rest = &tail[2..];
        }
    }
    out.push_str(rest);
    out
}

/// Builds the launch description for one validated stdio server entry.
///
/// The ordering matters and mirrors Agent Plugins §9: expand placeholders, resolve and contain
/// `command` and `cwd`, overlay the configured `env`, then set the authoritative variables last.
pub fn plan_stdio_launch(
    command: &str,
    args: &[String],
    configured_env: &BTreeMap<String, String>,
    cwd: Option<&str>,
    placeholders: &PluginPlaceholders,
) -> Result<StdioLaunchPlan, PluginDiagnostic> {
    let resolved_command = resolve_command(command, placeholders.plugin_root)?;
    let resolved_cwd = resolve_cwd(cwd, placeholders)?;

    let mut env: Vec<(String, String)> = configured_env
        .iter()
        .map(|(key, value)| (key.clone(), expand_placeholders(value, placeholders)))
        .collect();
    env.push((
        PLUGIN_ROOT_VAR.to_owned(),
        placeholders.plugin_root.to_string_lossy().into_owned(),
    ));
    env.push((
        PLUGIN_DATA_VAR.to_owned(),
        placeholders.plugin_data.to_string_lossy().into_owned(),
    ));

    Ok(StdioLaunchPlan {
        command: resolved_command,
        args: args
            .iter()
            .map(|arg| expand_placeholders(arg, placeholders))
            .collect(),
        env,
        cwd: resolved_cwd,
        plugin_data: placeholders.plugin_data.to_path_buf(),
    })
}

fn resolve_command(command: &str, plugin_root: &Path) -> Result<ResolvedCommand, PluginDiagnostic> {
    if !is_plugin_relative(command) {
        return Ok(ResolvedCommand::BareName(command.to_owned()));
    }
    resolve_contained(plugin_root, command)
        .map(ResolvedCommand::PluginRelative)
        .map_err(|error| {
            PluginDiagnostic::new(
                PluginDiagnosticCode::PathEscapesPluginRoot,
                format!("stdio server 'command' is not usable: {error}"),
            )
        })
}

/// Resolves `cwd`, containing a plugin-relative or `${PLUGIN_ROOT}`-rooted value in the plugin
/// root and a `${PLUGIN_DATA}`-rooted value in the plugin data directory (§7.2.1).
fn resolve_cwd(
    cwd: Option<&str>,
    placeholders: &PluginPlaceholders,
) -> Result<PathBuf, PluginDiagnostic> {
    let Some(cwd) = cwd else {
        return Ok(placeholders.plugin_root.to_path_buf());
    };

    let containment_root = if cwd.starts_with(PLUGIN_DATA_PLACEHOLDER) {
        placeholders.plugin_data
    } else {
        placeholders.plugin_root
    };

    if is_plugin_relative(cwd) {
        return resolve_contained(placeholders.plugin_root, cwd).map_err(cwd_error);
    }
    let expanded = expand_placeholders(cwd, placeholders);
    verify_contained(containment_root, Path::new(&expanded)).map_err(cwd_error)
}

fn cwd_error(error: super::paths::PluginPathError) -> PluginDiagnostic {
    PluginDiagnostic::new(
        PluginDiagnosticCode::PathEscapesPluginRoot,
        format!("stdio server 'cwd' is not usable: {error}"),
    )
}

#[cfg(test)]
#[path = "launch_tests.rs"]
mod tests;
