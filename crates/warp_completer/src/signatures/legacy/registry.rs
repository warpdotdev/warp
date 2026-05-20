use crate::completer::{CommandExitStatus, CompletionContext, TopLevelCommandCaseSensitivity};
use crate::parsers::SignatureAtTokenIndex;

use itertools::Itertools;
use memo_map::MemoMap;

use std::collections::HashMap;
use warp_command_signatures::{Argument, DynamicCompletionData, IsArgumentOptional, Signature};

pub enum SignatureResult<'a> {
    /// Successfully parsed the signature. We are returning the signature at the token to complete on.
    Success(SignatureAtTokenIndex<'a>),
    /// The command contains an alias. We are returning the expanded command to re-run the parser.
    NeedAliasExpansion(String),
    /// Couldn't find a signature.
    None,
}

type SignatureLookupFn = dyn 'static + Send + Sync + Fn(&str) -> Option<Signature>;

/// A simple structure to cache parsed command signatures.  These are stored as
/// JSON, so this makes it easy for us to lazily load and parse the JSON when
/// a command signature is needed, and only need to do that parsing work once
/// per signature per run of the program.
struct SignatureCache {
    /// A function that, given the name of a command, returns the [`Signature`]
    /// for it.  Should return None if there is no signature available for the
    /// given command.
    lookup_fn: Box<SignatureLookupFn>,
    /// A map from command name to the signature for the command, if any.  The
    /// use of [`MemoMap`] here allows us to safely return references to the
    /// contained signatures (as the map internally is an append-only
    /// structure).  This stores an `Option<Signature>` in order to also store
    /// our knowledge of commands for which we do _not_ have a signature.
    signatures: MemoMap<String, Option<Signature>>,
}

impl SignatureCache {
    fn new(lookup_fn: Box<SignatureLookupFn>) -> Self {
        Self {
            lookup_fn,
            signatures: Default::default(),
        }
    }

    fn get(&self, command: &str) -> Option<&Signature> {
        let command = if cfg!(windows) {
            command.trim_end_matches(".exe")
        } else {
            command
        };
        let command = command.to_lowercase();
        self.signatures
            .get_or_insert(&command, || (self.lookup_fn)(&command))
            .as_ref()
    }

    /// Inserts the given `Signature` into the underlying map, keyed by `Signature::name`.
    ///
    /// If there is already a cached value for the given `Signature::name`, this is a no-op (even
    /// if the cached value is `None`).
    fn insert(&self, signature: Signature) {
        self.signatures
            .insert(signature.name.to_lowercase(), Some(signature));
    }
}

/// This is a wrapper around a HashMap<String, T> to enforce the invariant that all keys must be
/// all lowercase letters.
#[derive(Clone, Debug)]
struct CaseInsensitiveHashMap<T> {
    map: HashMap<String, T>,
}

impl<T> Default for CaseInsensitiveHashMap<T> {
    fn default() -> Self {
        Self {
            map: Default::default(),
        }
    }
}

impl<T> CaseInsensitiveHashMap<T> {
    fn new() -> Self {
        Self::default()
    }

    fn get(&self, key: &str) -> Option<&T> {
        self.map.get(&key.to_lowercase())
    }

    fn insert(&mut self, key: &str, val: T) -> Option<T> {
        self.map.insert(key.to_lowercase(), val)
    }
}

impl<T> FromIterator<(String, T)> for CaseInsensitiveHashMap<T> {
    fn from_iter<I: IntoIterator<Item = (String, T)>>(iter: I) -> Self {
        let mut map = Self::new();
        for (key, val) in iter.into_iter() {
            map.insert(&key, val);
        }
        map
    }
}

pub struct CommandRegistry {
    signatures: SignatureCache,
    dynamic_completion_data: CaseInsensitiveHashMap<DynamicCompletionData>,
}

impl CommandRegistry {
    pub(super) fn new<F>(
        signature_lookup_fn: F,
        dynamic_completion_data: HashMap<String, DynamicCompletionData>,
    ) -> CommandRegistry
    where
        F: 'static + Send + Sync + Fn(&str) -> Option<Signature>,
    {
        CommandRegistry {
            signatures: SignatureCache::new(Box::new(signature_lookup_fn)),
            dynamic_completion_data: dynamic_completion_data.into_iter().collect(),
        }
    }

    pub fn registered_commands(&self) -> impl Iterator<Item = &str> {
        // Note we need to collect the keys because MemoMap uses a mutex under the hood to control
        // access to the underlying signature data. This means the mutex is locked as long as the
        // iterator returned from `keys()` lives, which means we need to collect keys into a vec
        // and return an owned iterator.
        self.signatures
            .signatures
            .iter()
            .filter_map(|(key, signature)| signature.as_ref().map(|_| key.as_str()))
            .collect::<Vec<_>>()
            .into_iter()
    }

    pub fn signature_from_line(
        &self,
        line: &str,
        command_case_sensitivity: TopLevelCommandCaseSensitivity,
    ) -> Option<SignatureAtTokenIndex<'_>> {
        let names = line.split_whitespace().collect_vec();
        self.signature_from_tokens(
            &names,
            line.ends_with(char::is_whitespace),
            command_case_sensitivity,
        )
    }

    /// Returns a replacement [`Signature`] and its corresponding [`DynamicCompletionData`] iff
    /// the current signature has an argument that should be a top level command and we are in a
    /// position where we would be completing on arguments.
    /// For example: if we had a token list of `sudo git ` (note the whitespace) we should return
    /// the `Signature` for `git`.
    ///
    /// NOTE this function does not handle the case where the `Signature` has multiple arguments
    /// and an argument other than the first should be a top level command. Fig also does not
    /// support this case, see CORE-2154 for more details.
    fn maybe_load_replacement_signature(
        &self,
        signature: &Signature,
        tokens: &[&str],
        current_index: usize,
        token: &str,
        has_post_whitespace: bool,
    ) -> Option<(&Signature, Option<&DynamicCompletionData>)> {
        if !signature.arguments().iter().any(Argument::is_command) {
            return None;
        }

        let is_last_token = tokens.len() - 1 == current_index;

        let replacement_signature = self.signatures.get(token).map(|signature| {
            (
                signature,
                self.dynamic_completion_data.get(signature.name()),
            )
        })?;

        if is_last_token {
            has_post_whitespace.then_some(replacement_signature)
        } else {
            Some(replacement_signature)
        }
    }

    pub async fn signature_with_alias_expansion(
        &self,
        tokens: &[&str],
        has_post_whitespace: bool,
        context: &dyn CompletionContext,
    ) -> SignatureResult<'_> {
        let found_signature = tokens.first().and_then(|command| {
            let command = if cfg!(windows) {
                command.trim_end_matches(".exe")
            } else {
                command
            };
            self.signatures
                .get(command)
                .map(|signature| (signature, self.dynamic_completion_data.get(command)))
        });

        let Some((signature, mut dynamic_completion_data)) = found_signature else {
            return SignatureResult::None;
        };

        let mut signature_start_idx = 0;
        let mut curr_signature = signature;

        // Iterate through tokens after the top level command
        let mut token_idx = 1;
        while token_idx < tokens.len() {
            // If at last token, and there's no post-whitespace, don't actually try to resolve
            // this token as an alias since we're actually completing on that token itself.
            if token_idx == tokens.len() - 1 && !has_post_whitespace {
                break;
            }

            let token = tokens[token_idx];
            // Check if there is any alias at the current signature.
            if let Some(alias) =
                curr_signature.alias(dynamic_completion_data.map(DynamicCompletionData::aliases))
            {
                // Get the shell command to execute for getting the alias.
                let command_to_run = alias.command(&tokens[..token_idx + 1]);

                if let Some(generator_context) = context.generator_context() {
                    if let Ok(output) = generator_context
                        .execute_command_at_pwd(&command_to_run, None)
                        .await
                    {
                        if let Ok(output_string) = output.to_string() {
                            // If the command output was successful, attempt to complete on the alias.
                            match output.status {
                                CommandExitStatus::Success => {
                                    let expanded_command =
                                        alias.on_complete(&output_string, tokens, token_idx);

                                    if let Some(expanded_command) = expanded_command {
                                        return SignatureResult::NeedAliasExpansion(
                                            expanded_command,
                                        );
                                    }
                                }
                                CommandExitStatus::Failure => {
                                    // We purposefully do not log an error here if the command failed because
                                    // many commands (such as `git`) will fail if there isn't a valid alias for
                                    // the token.
                                    log::debug!(
                                        "Execution of `{}` failed with output: {}",
                                        command_to_run,
                                        &output_string
                                    )
                                }
                            }
                        } else {
                            log::debug!(
                                "Execution of `{command_to_run}` returned an unparseable output",
                            );
                        }
                    }
                }
            }

            if let Some((replacement_signature, replacement_completion_data)) = self
                .maybe_load_replacement_signature(
                    signature,
                    tokens,
                    token_idx,
                    token,
                    has_post_whitespace,
                )
            {
                curr_signature = replacement_signature;
                dynamic_completion_data = replacement_completion_data;
                signature_start_idx = token_idx;
            } else {
                match classify_token(
                    curr_signature,
                    token,
                    tokens.len(),
                    token_idx,
                    has_post_whitespace,
                ) {
                    TokenAction::ResolvedSubcommand { signature } => {
                        curr_signature = signature;
                        signature_start_idx = token_idx;
                    }
                    TokenAction::SkippedOption { advance_by } => {
                        token_idx += advance_by;
                    }
                    TokenAction::SkippedUnrecognizedFlag => {}
                    TokenAction::VariadicOption | TokenAction::StopAtCurrentToken => {
                        return SignatureResult::Success(SignatureAtTokenIndex::new(
                            curr_signature,
                            dynamic_completion_data,
                            signature_start_idx,
                        ));
                    }
                }
            }

            token_idx += 1;
        }

        SignatureResult::Success(SignatureAtTokenIndex::new(
            curr_signature,
            dynamic_completion_data,
            signature_start_idx,
        ))
    }

    /// Finds a signature from a list of tokens--returning the index of the token where the
    /// signature starts.
    pub fn signature_from_tokens(
        &self,
        tokens: &[&str],
        has_post_whitespace: bool,
        command_case_sensitivity: TopLevelCommandCaseSensitivity,
    ) -> Option<SignatureAtTokenIndex<'_>> {
        let first_token = *tokens.first()?;

        // Find the top level signature.
        let (signature, mut dynamic_completion_data) = self
            .signatures
            .get(first_token)
            .map(|signature| (signature, self.dynamic_completion_data.get(first_token)))?;

        // Signature lookup is case-insensitive. However, sometimes we need to treat the lookup as
        // case-sensitive. There are 2 variables to check for that. The first is the
        // `command_case_sensitivity` parameter which represents the platform's filesystem
        // case-sensitivity. This, however, may be overridden by
        // `ParserDirectives::always_case_insensitive`. When that is true, we ignore the platform.
        // If we are treating this as a case-sensitive lookup, `signature.name` will contain the
        // canonical stylization of the name, and so we compare what the user typed, `first_token`,
        // to that.
        // For example, on Linux (case-sensitive by default), "GIT" should not match the spec for
        // "git". `first_token` will be "GIT" and `signature.name` will be "git". We return `None`.
        // However, if the user is running PowerShell and calls "set-location", this _should_ match
        // the spec for "Set-Location", so we skip the `signature.name != first_token` check. FYI
        // `signature.name` will be formatted as "Set-Location" as that is the preferred style.
        if command_case_sensitivity == TopLevelCommandCaseSensitivity::CaseSensitive
            && !signature.parser_directives.always_case_insensitive
            && signature.name != first_token
        {
            return None;
        }

        let mut signature_start_idx = 0;
        let mut curr_signature = signature;

        // Iterate through tokens after the top level command
        let mut token_idx = 1;
        while token_idx < tokens.len() {
            let token = tokens[token_idx];

            if let Some((replacement_signature, replacement_completion_data)) = self
                .maybe_load_replacement_signature(
                    signature,
                    tokens,
                    token_idx,
                    token,
                    has_post_whitespace,
                )
            {
                curr_signature = replacement_signature;
                dynamic_completion_data = replacement_completion_data;
                signature_start_idx = token_idx;
            } else {
                match classify_token(
                    curr_signature,
                    token,
                    tokens.len(),
                    token_idx,
                    has_post_whitespace,
                ) {
                    TokenAction::ResolvedSubcommand { signature } => {
                        curr_signature = signature;
                        signature_start_idx = token_idx;
                    }
                    TokenAction::SkippedOption { advance_by } => {
                        token_idx += advance_by;
                    }
                    TokenAction::SkippedUnrecognizedFlag => {}
                    TokenAction::VariadicOption | TokenAction::StopAtCurrentToken => {
                        return Some(SignatureAtTokenIndex::new(
                            curr_signature,
                            dynamic_completion_data,
                            signature_start_idx,
                        ));
                    }
                }
            }

            token_idx += 1;
        }

        Some(SignatureAtTokenIndex::new(
            curr_signature,
            dynamic_completion_data,
            signature_start_idx,
        ))
    }

    pub fn signature(&self, name: &str) -> Option<&Signature> {
        self.signatures.get(name)
    }

    /// Registers the given `Signature`.
    ///
    /// Note the underlying map caches the lookup result for a given signature (regardless of
    /// whether or not it is `Some` or `None`), which means that if there is already a cached
    /// `None` value for the command corresponding to this signature, this is a no-op.
    pub fn register_signature(&self, signature: Signature) {
        self.signatures.insert(signature);
    }
}

/// The result of classifying a single token during signature resolution.
enum TokenAction<'a> {
    /// The token matched a subcommand of the current signature.
    ResolvedSubcommand { signature: &'a Signature },
    /// The token matched a recognized option whose last argument is variadic.
    /// The caller should stop walking tokens and return the current signature.
    VariadicOption,
    /// The token matched a recognized option with a fixed number of required
    /// arguments. The caller should advance `token_idx` by `advance_by` to skip
    /// past those arguments (the flag token itself is advanced separately).
    SkippedOption { advance_by: usize },
    /// The token starts with '-' but didn't match any recognized option.
    SkippedUnrecognizedFlag,
    /// The token is not a subcommand, option, or flag-like. The caller should
    /// stop walking tokens and return the current signature.
    StopAtCurrentToken,
}

/// Classifies a token against the current signature's subcommands and options.
///
/// This encapsulates the shared per-token decision logic. Callers
/// handle replacement signatures (e.g. `sudo git`) separately before
/// invoking this function.
fn classify_token<'a>(
    curr_signature: &'a Signature,
    token: &str,
    num_tokens: usize,
    token_idx: usize,
    has_post_whitespace: bool,
) -> TokenAction<'a> {
    if let Some(subcommand) = curr_signature.subcommands().iter().find(|s| {
        should_complete_on_subcmd(s.name(), token, num_tokens, token_idx, has_post_whitespace)
    }) {
        return TokenAction::ResolvedSubcommand {
            signature: subcommand,
        };
    }

    if let Some(option) = find_option_by_name(curr_signature.options(), token) {
        if option.arguments().last().is_some_and(|arg| arg.is_variadic) {
            return TokenAction::VariadicOption;
        }
        let required_arg_count = option
            .arguments()
            .iter()
            .filter(|arg| arg.optional == IsArgumentOptional::Required)
            .count();
        return TokenAction::SkippedOption {
            advance_by: required_arg_count,
        };
    }

    if token.starts_with('-') {
        return TokenAction::SkippedUnrecognizedFlag;
    }

    TokenAction::StopAtCurrentToken
}

/// Finds an option by exact name match against the token.
fn find_option_by_name<'a>(
    options: &'a [warp_command_signatures::Opt],
    token: &str,
) -> Option<&'a warp_command_signatures::Opt> {
    options
        .iter()
        .find(|option| option.exact_string.iter().any(|s| s == token))
}

/// Returns true iff we should resolve the subcmd as a new [`Signature`].
///
/// If token is the last token, then as long as the subcmd matches the token name and there's whitespace at the end,
/// then we should complete on the signature. If there wasn't whitespace at the end, then we would actually want to
/// complete on the subcmds, and not resolve this subcmd. For example, suppoes the line is 'npm r' vs 'npm r '.
/// In the former, we want to find `npm` subcommand completions. In the latter, we want to find `npm r` completions.
///
/// Otherwise, we are not at the last token so we should recursively resolve as long as the subcmd matches the token.
fn should_complete_on_subcmd(
    subcmd_name: &str,
    token: &str,
    num_tokens: usize,
    curr_token_idx: usize,
    has_post_whitespace: bool,
) -> bool {
    let is_last_token = num_tokens - 1 == curr_token_idx;
    let subcmd_matches_token = subcmd_name == token;

    if is_last_token {
        has_post_whitespace && subcmd_matches_token
    } else {
        subcmd_matches_token
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
