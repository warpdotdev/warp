use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

use crate::workflows::workflow::Argument;
use handlebars::parser::{ParsedArgumentResult, ParsedArgumentsIterator};

/// Represents arguments for workflow to be viewed and edited in ArgumentsEditorView.
///
/// ArgumentsState contains the current state of arguments, and a constructor `::from_string`
/// which will use the remaining ArgumentsState data to connect at some state "k" to next
/// state "k + 1", generated from the next edit. This is necessary to identify how arguments shift
/// and retain existing description and default values; previous arguments are matched either by a
/// query by word index or query by name. (See constructor method for further discusssion.)
#[derive(Debug, Default)]
pub struct ArgumentsState {
    pub arguments: Vec<Argument>,
    /// Hashmap mapping the word index in the input command string to (index) in the
    /// arguments vector. Each index of the arguments vector should be a hashmap value;
    /// only word indexes with arguments (from input string) should be a hashmap key.
    /// Enables `query_argument_by_name`.
    word_index_to_arg_index_map: HashMap<usize, usize>,
    /// Hashmap mapping the argument name in the input command string to (index) in the
    /// arguments vector. Enables `query_argument_by_word_index`.
    arg_name_to_arg_index_map: HashMap<String, usize>,
    number_of_words: usize,
    pub invalid_arguments_char_ranges: Vec<Range<usize>>,
    pub valid_arguments_char_ranges_and_arg_index: Vec<(Range<usize>, usize)>,
}

impl ArgumentsState {
    /// The `::from_string` constructor connects the previous arguments state to the new state and
    /// to retain some descriptions and default values. This is needed because the command string is
    /// regex-ed every edit in `ArgumentsEditorView.update_command`; with default `None` values,
    /// the descriptions and default values are cleared.
    ///
    /// Reasonably, a user expects data from arguments that they did not directly edit to remain intact.
    /// This approach improves handling by indexing each word (as defined by whitespace, `{{`, or `}}`).
    /// If the number of words is the same, an argument in the new formed string will query for an argument
    /// with the same word index in the previous state. If the number of words is not the same, an argument
    /// in the new formed string will query for an argument with the same argument name in the previous state.
    /// In both cases, if an argument is found, the new argument will retain the previous description and
    /// default value. Otherwise, both values default to None.
    ///
    /// Arguments are shown, if valid (see `ParsedArgumentsIterator`), in order of first occurrence with no duplicates.
    /// The word index points to the first occurrence of the argument. For insertion/deletion (+/- number of words),
    /// arguments' descriptions and default values will re-arrange with argument names. For edits (no word delta),
    /// the modified occurrence will retain its description and default value, its former duplicates will not.
    ///
    /// eg. Given workflow `ls {{argument_1}} {{argument_2}} {{argument_3}}` which is then edited to
    /// `ls {{argument_1}} {{argument_1}} {{argument_2}} {{argument_3}}`. The number of words has changed
    /// so we connect arguments to previous values using a by_name search.
    ///
    /// If the edited result is instead `ls {{argument_10}} {{argument_2}} {{argument_3}}`, the number of
    /// words did not change, and so we use a by_word_index search (which will argument_10 to argument_1, etc.).
    pub fn for_command_workflow(prev_state: &ArgumentsState, input_string: String) -> Self {
        Self::new(prev_state, input_string, false)
    }

    pub fn for_saved_prompt(prev_state: &ArgumentsState, input_string: String) -> Self {
        Self::new(prev_state, input_string, true)
    }

    fn new(prev_state: &ArgumentsState, input_string: String, is_for_saved_prompt: bool) -> Self {
        let mut arg_name_word_index_pairs: Vec<(String, usize)> = Vec::new();
        let mut arg_names = HashSet::new();

        let mut valid_arguments_char_ranges_and_name: Vec<(Range<usize>, String)> = Vec::new();
        let mut invalid_arguments_char_ranges = Vec::new();

        let mut arguments_iterator = ParsedArgumentsIterator::new(input_string.chars());

        for argument_result in arguments_iterator.by_ref() {
            match argument_result.result() {
                ParsedArgumentResult::Valid { current_word_index } => {
                    let start_char_index = argument_result.chars_range().start;
                    let argument_name_length = argument_result.chars_range().end - start_char_index;
                    let argument_name: String = input_string
                        .chars()
                        .skip(start_char_index)
                        .take(argument_name_length)
                        .collect();

                    if !arg_names.contains(&argument_name) {
                        arg_name_word_index_pairs
                            .push((argument_name.clone(), *current_word_index));
                        arg_names.insert(argument_name.clone());
                    }

                    valid_arguments_char_ranges_and_name
                        .push((argument_result.chars_range(), argument_name));
                }
                ParsedArgumentResult::Invalid => {
                    // We don't care about 'invalid' arguments for saved prompts, since the argument
                    // might be intentional/valid. For example, a user's saved prompt might contain
                    // {{.foo}} which isn't intended to be an _argument_.
                    if !is_for_saved_prompt {
                        invalid_arguments_char_ranges.push(argument_result.chars_range());
                    }
                }
            }
        }

        let number_of_words = arguments_iterator.word_count();

        let (arguments, word_index_to_arg_index_map, arg_name_to_arg_index_map) =
            ArgumentsState::build_arguments_and_query_maps(
                prev_state,
                number_of_words != prev_state.number_of_words,
                arg_name_word_index_pairs,
            );

        let valid_arguments_char_ranges_and_arg_index: Vec<(Range<usize>, usize)> =
            valid_arguments_char_ranges_and_name
                .iter()
                .map(|(range, name)| {
                    (
                        range.clone(),
                        *arg_name_to_arg_index_map
                            .get(name)
                            .expect("All valid arguments' names must map to an argument index"),
                    )
                })
                .collect();

        Self {
            arguments,
            word_index_to_arg_index_map,
            arg_name_to_arg_index_map,
            number_of_words,
            invalid_arguments_char_ranges,
            valid_arguments_char_ranges_and_arg_index,
        }
    }

    fn build_arguments_and_query_maps(
        prev_state: &ArgumentsState,
        is_insertion_or_deletion: bool,
        arg_name_word_index_pairs: Vec<(String, usize)>,
    ) -> (Vec<Argument>, HashMap<usize, usize>, HashMap<String, usize>) {
        let mut word_index_to_arg_index_map = HashMap::new();
        let mut arg_name_to_arg_index_map = HashMap::new();

        let arguments: Vec<Argument> = arg_name_word_index_pairs
            .iter()
            .enumerate()
            .map(|(arg_index, (name, word_index))| {
                let prev_argument = if is_insertion_or_deletion {
                    prev_state.query_argument_by_name(name)
                } else {
                    prev_state.query_argument_by_word_index(*word_index)
                };

                let argument: Argument = match prev_argument {
                    Some(prev_argument) => {
                        ArgumentsState::new_argument_with_previous_data(name, prev_argument)
                    }
                    None => Argument::new(name, Default::default()),
                };

                word_index_to_arg_index_map.insert(*word_index, arg_index);
                arg_name_to_arg_index_map.insert(name.to_string(), arg_index);

                argument
            })
            .collect();

        (
            arguments,
            word_index_to_arg_index_map,
            arg_name_to_arg_index_map,
        )
    }

    fn query_argument_by_name(&self, name: &str) -> Option<&Argument> {
        match self.arg_name_to_arg_index_map.get(name) {
            Some(argument_index) => Some(&self.arguments[*argument_index]),
            None => None,
        }
    }

    fn query_argument_by_word_index(&self, word_index: usize) -> Option<&Argument> {
        match self.word_index_to_arg_index_map.get(&word_index) {
            Some(argument_index) => Some(&self.arguments[*argument_index]),
            None => None,
        }
    }

    fn new_argument_with_previous_data(
        new_argument_name: &str,
        prev_argument: &Argument,
    ) -> Argument {
        Argument {
            name: new_argument_name.to_string(),
            description: prev_argument.description.clone(),
            default_value: prev_argument.default_value.clone(),
            arg_type: Default::default(),
        }
    }
}

#[cfg(test)]
#[path = "arguments_tests.rs"]
mod tests;
