//! This module contains Command Signature types for use with the v2 (JS-compatible) completions
//! engine.

// The `js` module contains implementations of `warp_js::{IntoWarpJs, FromWarpJs}` for V2 command
// signatures, which is only supported on native non-wasm platforms.
#[cfg(not(target_family = "wasm"))]
mod js;

mod lookup;
mod registry;

use std::cmp::Ordering;

pub use lookup::*;
pub use registry::*;

use serde::{Deserialize, Serialize};
use warp_js::TypedJsFunctionRef;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "test-util", derive(Default))]
pub struct CommandSignature {
    pub command: Command,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "test-util", derive(Default))]
pub struct Command {
    pub name: String,
    pub alias: Vec<String>,
    pub description: Option<String>,
    pub arguments: Vec<Argument>,
    pub subcommands: Vec<Command>,
    pub options: Vec<Opt>,
    pub priority: Priority,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "test-util", derive(Default))]
pub struct Argument {
    pub name: String,
    pub description: Option<String>,
    pub values: Vec<ArgumentValue>,
    pub optional: bool,
    pub arity: Option<Arity>,
}

impl Argument {
    pub fn is_variadic(&self) -> bool {
        self.arity
            .as_ref()
            .is_some_and(|arity| arity.limit.is_none())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "test-util", derive(Default))]
pub struct Arity {
    pub limit: Option<usize>,
    pub delimiter: Option<ArgumentDelimiter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgumentDelimiter(pub String);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ArgumentValue {
    Suggestion(Suggestion),
    Template {
        type_name: TemplateType,
        filter_name: Option<String>,
    },
    Generator(GeneratorFn),
    /// The argument itself is a root command.
    ///
    /// This is the appropriate `ArgumentValue` for commands that take a full command as an
    /// argument: `time` or `sudo`, for example.
    RootCommand,
}

/// The final set of results returned from a custom `GeneratorFn` or from a `GeneratorFn`'s
/// `post_process` function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratorResults {
    /// The list of completion suggestions.
    pub suggestions: Vec<Suggestion>,

    /// `true` if the order of `suggestions` should be preserved.
    ///
    /// If `false`, `suggestions` may be re-ordered by the internal completions engine in the final
    /// result set.
    pub is_ordered: bool,
}

/// The input struct passed to custom generator functions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratorCompletionContext {
    /// The tokens in the input for which completion suggestions are being generated.
    pub tokens: Vec<String>,

    /// The current working directory of the session.
    pub pwd: String,
}

/// The Rust representation of a JS Function used to generate argument value suggestions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GeneratorFn {
    /// A generator that computes suggestions by executing the given `script` (a sh command) and
    /// "post-processing" its stdout with the given `post_process` fn.
    ShellCommand {
        script: GeneratorScript,

        /// If `None`, a "default" post process function implementation is used, where
        /// `Suggestion`s are created from each line in `script`'s stdout. The returned
        /// `GeneratorResults` object's `is_ordered` is set to `false`.
        post_process: Option<TypedJsFunctionRef<String, GeneratorResults>>,
    },
    /// An entirely user-specified JS function that generates suggestions based on the given
    /// `GeneratorCompletionContext`.
    Custom(TypedJsFunctionRef<GeneratorCompletionContext, GeneratorResults>),
}

/// The command to be executed as part of a `GeneratorFn::ShellCommand` to generate argument
/// suggestion values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GeneratorScript {
    Static(String),
    /// A JS function that dynamically computes the command to be run based on the tokenized input
    /// for which suggestions are being generated.
    Dynamic(TypedJsFunctionRef<Vec<String>, String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "test-util", derive(Default))]
pub struct Suggestion {
    pub value: String,
    pub display_value: Option<String>,
    pub description: Option<String>,
    pub priority: Priority,
}

/// The lowest priority value a completion object can have.
const MIN_PRIORITY: i32 = -100;

/// The priority value of a completion object if not otherwise specifiied.
const DEFAULT_PRIORITY: i32 = 0;

/// The highest priority value a completion object can have.
const MAX_PRIORITY: i32 = 100;

/// Priority is a property of Commands, Subcommands and Options that influences where in the
/// suggestion list those objects appear. It is represented as an integer between -100 and 100
/// (inclusive) with 0 as the default.
#[derive(PartialEq, Eq, Serialize, Deserialize, Debug, Copy, Clone)]
pub struct Priority(i32);

impl Priority {
    /// Creates a Priority value clamped to the range [-100, 100].
    pub fn new(value: i32) -> Self {
        Self(value.clamp(MIN_PRIORITY, MAX_PRIORITY))
    }

    pub fn value(&self) -> i32 {
        self.0
    }

    pub fn min() -> Self {
        Self::new(MIN_PRIORITY)
    }

    pub fn max() -> Self {
        Self::new(MAX_PRIORITY)
    }
}

impl Default for Priority {
    fn default() -> Self {
        Self::new(DEFAULT_PRIORITY)
    }
}

impl Ord for Priority {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl PartialOrd for Priority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TemplateType {
    Files,
    Folders,
    FilesAndFolders,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "test-util", derive(Default))]
pub struct Opt {
    pub name: Vec<String>,
    pub description: Option<String>,
    pub arguments: Vec<Argument>,
    pub required: bool,
    pub priority: Priority,
}

impl Opt {
    /// Returns `true` if this `Opt` has the given name.
    ///
    /// Note that the given `name` should not include any leading hyphens; for example, this
    /// returns true for an `Opt` with names ['-f', '--foo'] given name 'f' or 'foo'.
    pub fn has_name(&self, name: impl AsRef<str>) -> bool {
        self.name.iter().any(|option_name| {
            if let Some(rest) = option_name.strip_prefix("--") {
                rest == name.as_ref()
            } else if let Some(rest) = option_name.strip_prefix('-') {
                rest == name.as_ref()
            } else {
                false
            }
        })
    }
}

#[cfg(test)]
#[path = "signatures_tests.rs"]
mod test;
