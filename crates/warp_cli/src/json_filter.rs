//! Reusable JSON output formatting component.

use std::fmt;
use std::ops::Deref;
use std::sync::Arc;

use clap::Args;
use jaq_all::data::{self, DataKind};
use jaq_all::load::FileReportsDisp;

/// A jq filter, compiled and ready to execute against a [`jaq_json::Val`].
///
/// This wraps the compiled [`jaq_all::data::Filter`] with `Clone` and `Debug`
/// implementations.
#[derive(Clone)]
pub struct JqFilter(Arc<data::Filter>);

impl fmt::Debug for JqFilter {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_tuple("JqFilter").field(&"<compiled>").finish()
    }
}

impl Deref for JqFilter {
    type Target = data::Filter;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// CLI argument bundle with flags relevant for commands that produce
/// JSON output.
///
/// Embed with `#[command(flatten)]` on any command which can produce
/// JSON output, and use the `print_raw_json` utility in the `app` crate
/// to format that output.
#[derive(Clone, Debug, Default, Args)]
pub struct JsonOutput {
    /// A filter to select values from the response using jq syntax.
    ///
    /// Example: `--jq '.runs[].creator'
    ///
    /// When set, the result of the filter expression is printed instead of
    /// the full JSON output. Top-level scalar outputs are automatically
    /// unquoted.
    #[arg(long = "jq", value_parser = parse_jq_filter, value_name = "FILTER")]
    pub filter: Option<JqFilter>,
}

impl JsonOutput {
    /// Returns true if this argument bundle requires JSON output regardless
    /// of the user-selected `--output-format`.
    ///
    /// For example, `--jq` runs against JSON, so setting it implies the
    /// command must fetch and process JSON even when the user asked for
    /// pretty/text output.
    pub fn force_json_output(&self) -> bool {
        self.filter.is_some()
    }
}

/// Parse and compile a jq filter source string.
///
/// Used as a clap `value_parser` so invalid filters (syntax errors, unknown
/// names) fail during argument parsing.
pub fn parse_jq_filter(src: &str) -> Result<JqFilter, String> {
    let compiled = jaq_all::compile_with::<DataKind>(src, jaq_all::defs(), data::base_funs(), &[])
        .map_err(|reports| {
            let detail = reports
                .iter()
                .map(|report| FileReportsDisp::new(report).to_string())
                .collect::<String>();
            format!("invalid jq filter `{src}`:\n{detail}")
        })?;
    Ok(JqFilter(Arc::new(compiled)))
}

#[cfg(test)]
#[path = "json_filter_tests.rs"]
mod tests;
