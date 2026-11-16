use crate::completer::{CompletionContext, describe_given_token};
use crate::meta::{HasSpan as _, Span, SpannedItem};
use crate::parsers::LiteCommand;
use crate::parsers::simple::all_parsed_commands;
use crate::{ParsedCommandsSnapshot, ParsedTokenData, ParsedTokensSnapshot};

/// Parse the current commands in the editor's buffer and get descriptions
/// for tokens within the commands. Note that this can be somewhat expensive,
/// which is why we execute this function asynchronously.
pub async fn parse_current_commands_and_tokens<T: CompletionContext>(
    buffer_text: String,
    completion_context: &T,
) -> ParsedTokensSnapshot {
    // Parse commands
    let all_commands_iterator =
        all_parsed_commands(buffer_text.as_str(), completion_context.escape_char());
    // Note that we must collect the iterator into a vector to avoid referencing local data i.e. buffer_text within the future's output.
    let all_commands_vec: Vec<LiteCommand> = all_commands_iterator.collect();
    let parsed_commands_snapshot = ParsedCommandsSnapshot {
        buffer_text: buffer_text.clone(),
        parsed_commands: all_commands_vec,
        completion_context,
    };
    // Get descriptions for tokens within commands.
    let parsed_tokens = get_token_descriptions(parsed_commands_snapshot).await;

    ParsedTokensSnapshot {
        buffer_text,
        parsed_tokens,
    }
}

/// Expands aliases in the provided snapshot iteratively until no aliases are found.
pub async fn expand_aliases<T: CompletionContext>(
    mut parsed_tokens_snapshot: ParsedTokensSnapshot,
    completion_context: &T,
) -> ParsedTokensSnapshot {
    // Perform up to three iterations of alias expansion (to hedge against recursive aliases).
    for _ in 0..3 {
        let mut expanded_buffer_text = String::new();
        let mut last_token_end = 0;
        for token in &parsed_tokens_snapshot.parsed_tokens {
            if token.token_index != 0 {
                continue;
            }

            let Some(alias) = completion_context.alias_command(token.token.as_str()) else {
                continue;
            };

            // Push any text between the last token and the current token.
            expanded_buffer_text.push_str(
                &parsed_tokens_snapshot.buffer_text[last_token_end..token.token.span().start()],
            );
            // Push the alias.
            expanded_buffer_text.push_str(alias);
            // Store the position in the buffer text that we've processed up to so far.
            last_token_end = token.token.span().end();
        }

        if expanded_buffer_text.is_empty() {
            // If we didn't find any aliases, we're done.
            break;
        } else {
            // Push any additional trailing content after the last expanded token.
            if last_token_end < parsed_tokens_snapshot.buffer_text.len() {
                expanded_buffer_text
                    .push_str(&parsed_tokens_snapshot.buffer_text[last_token_end..]);
            }
            // Update the parsed tokens snapshot.
            parsed_tokens_snapshot =
                parse_current_commands_and_tokens(expanded_buffer_text, completion_context).await;
        }
    }

    parsed_tokens_snapshot
}

/// Get a vector of parsed tokens data from a given parsed commands snapshot,
/// note that this is meant to run asynchronously.
async fn get_token_descriptions<'a, T: CompletionContext>(
    parsed_commands_snapshot: ParsedCommandsSnapshot<'a, T>,
) -> Vec<ParsedTokenData> {
    let buffer_text = parsed_commands_snapshot.buffer_text.as_str();
    let completion_context = parsed_commands_snapshot.completion_context;
    let mut parsed_token_data = Vec::new();

    for parsed_command in parsed_commands_snapshot.parsed_commands {
        let current_command_span = parsed_command.span();

        for (token_index, token) in parsed_command.parts.into_iter().enumerate() {
            // Split --flag=value tokens into separate flag-name and value entries so that each
            // part gets its own syntax highlighting color and description.
            let eq_split = token
                .item
                .starts_with('-')
                .then(|| token.item.find('='))
                .flatten();

            if let Some(eq_pos) = eq_split {
                let eq_byte_pos = token.span.start() + eq_pos;

                let flag_token = token.item[..eq_pos]
                    .to_string()
                    .spanned(Span::new(token.span.start(), eq_byte_pos));
                let flag_description = describe_given_token(
                    buffer_text,
                    &current_command_span,
                    flag_token.clone(),
                    completion_context,
                )
                .await;
                parsed_token_data.push(ParsedTokenData {
                    token: flag_token,
                    token_index,
                    token_description: flag_description,
                });

                let value_token = token.item[eq_pos + 1..]
                    .to_string()
                    .spanned(Span::new(eq_byte_pos + 1, token.span.end()));
                if !value_token.item.is_empty() {
                    let value_description = describe_given_token(
                        buffer_text,
                        &current_command_span,
                        value_token.clone(),
                        completion_context,
                    )
                    .await;
                    parsed_token_data.push(ParsedTokenData {
                        token: value_token,
                        token_index,
                        token_description: value_description,
                    });
                }
            } else {
                let token_description = describe_given_token(
                    buffer_text,
                    &current_command_span,
                    token.clone(),
                    completion_context,
                )
                .await;

                // Note that this is the token index relative to the current command meaning that
                // in the final flattened vector, we could have multiple tokens with index 0.
                parsed_token_data.push(ParsedTokenData {
                    token,
                    token_index,
                    token_description,
                })
            }
        }
    }

    parsed_token_data
}
