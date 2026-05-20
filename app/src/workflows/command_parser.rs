//! This module contains utilities for computing helper data structures used to render and
//! implement the Workflows UI in the info box and the terminal input.

use std::{
    collections::{HashMap, VecDeque},
    ops::Range,
};

use itertools::Itertools;
use lazy_static::lazy_static;
use regex::Regex;
use string_offset::{ByteOffset, CharCounter, CharOffset};

use crate::server::ids::SyncId;

use super::workflow::{ArgumentType, Workflow};

lazy_static! {
    /// Regex for escaped arguments in workflow command.
    static ref ESCAPED_ARGUMENTS_PATTERN: Regex = Regex::new(r"\{\{\{([^{}]+)\}\}\}").expect("Escaped argument regex should be valid.");
}

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
pub struct WorkflowArgumentIndex(usize);

impl From<usize> for WorkflowArgumentIndex {
    fn from(num: usize) -> Self {
        Self(num)
    }
}

impl std::ops::Deref for WorkflowArgumentIndex {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Helper struct for inputting Workflow arguments within the editor.
#[derive(Debug)]
struct WorkflowArgument<'a> {
    /// The index of the argument in the list of arguments.
    argument_index: WorkflowArgumentIndex,
    argument_name: &'a str,
    /// The argument type of the argument, which includes IDs of objects it references.
    argument_type: &'a ArgumentType,
    /// The text the workflow should replace the argument identifier with.
    replacement_text: &'a str,
    /// The byte indices of the argument in the workflow.
    byte_range: Range<ByteOffset>,
    /// The character indices of the argument in the workflow.
    char_range: Range<CharOffset>,
}

/// Helper struct containing computed metadata about the workflow and its arguments, used to render
/// the workflows UI in both the input editor and workflows "info box".
#[derive(Debug)]
pub struct WorkflowDisplayData {
    /// The command with replaced arguments. Arguments are replaced with their default values (if
    /// they exist) or with their display "placeholders" (e.g. "{{argument_name}}"), or in the case
    /// of executed workflows from history, the arguments used when executing the workflow.
    pub command_with_replaced_arguments: String,

    /// A vector of `ByteOffset` ranges representing the ranges in the original command string
    /// replaced with argument values.
    pub replaced_ranges: Vec<Range<ByteOffset>>,

    /// Index of workflow argument index (in the workflow.arguments() list) mapped to a vector of
    /// indices of the workflow argument instances in the actual replaced command. For instance,
    /// if workflow.arguments = ["foo", "bar"] and the workflow is "echo {{foo}} {{bar}} {{foo}}",
    /// the entry for "foo" would be (0, [0, 2]).
    pub argument_index_to_highlight_index_map: HashMap<WorkflowArgumentIndex, Vec<usize>>,

    /// Index of workflow argument index (in the workflow.arguments() list) mapped to a vector of
    /// [`CharOffset`] ranges in the replaced command indicating the ranges in the
    /// replaced command where text was replaced for the corresponding argument. For example,
    /// if workflow.arguments = ["foo", "bar"] and the workflow is "echo {{foo}} {{bar}} {{foo}}",
    /// the entry for "foo" would be [5-8, 13-16].
    pub argument_index_to_char_range_map: HashMap<WorkflowArgumentIndex, Vec<Range<CharOffset>>>,

    pub argument_index_to_object_id_map: HashMap<WorkflowArgumentIndex, SyncId>,
}

#[derive(Clone)]
enum WorkflowCommandPart {
    CommandPart(String),
    Argument { name: String, value: String },
}

impl WorkflowCommandPart {
    fn to_command_string(&self) -> &String {
        match self {
            Self::CommandPart(value) => value,
            Self::Argument { name: _, value } => value,
        }
    }
}

#[derive(Clone)]
pub struct WorkflowCommandDisplayData {
    command_parts: Vec<WorkflowCommandPart>,
}

impl WorkflowCommandDisplayData {
    /// Use this to change the value of an argument when you want to value to be reflected in the
    /// command.
    ///
    /// This iterates over the command_parts, finds the argument by name and replaces the matched
    /// item the vector.
    /// O(n) where n corresponds to the number of arguments in a workflow
    /// An assumption here is that command will not have many arguments to the point that we will
    /// notice a performance hit.
    pub fn set_argument_value(&mut self, argument_name: String, new_value: String) {
        let new_parts = &self
            .command_parts
            .iter()
            .map(|elem| match elem {
                WorkflowCommandPart::Argument { name, value } => {
                    if argument_name == *name {
                        WorkflowCommandPart::Argument {
                            name: argument_name.clone(),
                            value: new_value.clone(),
                        }
                    } else {
                        WorkflowCommandPart::Argument {
                            name: name.clone(),
                            value: value.clone(),
                        }
                    }
                }
                WorkflowCommandPart::CommandPart(value) => {
                    WorkflowCommandPart::CommandPart(value.clone())
                }
            })
            .collect_vec();

        self.command_parts.clone_from(new_parts);
    }

    pub fn get_argument_values(&self) -> HashMap<String, String> {
        let args = self
            .command_parts
            .iter()
            .filter_map(|part| {
                if let WorkflowCommandPart::Argument { name, value } = part {
                    Some((name.clone(), value.clone()))
                } else {
                    None
                }
            })
            .collect();
        args
    }

    /// Prints out the command as a string
    pub fn to_command_string(&self) -> String {
        self.command_parts
            .iter()
            .map(|part| part.to_command_string().as_str())
            .collect_vec()
            .join("")
    }

    /// Gets the argument byteoffset ranges. This can be used to highlight ranges for arguments in
    /// editors.
    pub fn argument_ranges(&self) -> Vec<Range<ByteOffset>> {
        let mut current_index = 0;
        let mut ranges = vec![];
        for part in &self.command_parts {
            match part {
                WorkflowCommandPart::CommandPart(value) => current_index += value.len(),
                WorkflowCommandPart::Argument { name: _, value } => {
                    ranges.push(
                        ByteOffset::from(current_index)
                            ..ByteOffset::from(current_index + value.len()),
                    );
                    current_index += value.len();
                }
            }
        }
        ranges
    }

    /// Create an empty command display data. Used for new workflows that don't have commands yet.
    pub fn new_empty() -> WorkflowCommandDisplayData {
        WorkflowCommandDisplayData {
            command_parts: Vec::new(),
        }
    }

    /// Create a command display data from an existing workflow.
    pub fn new_from_workflow(workflow: &Workflow) -> WorkflowCommandDisplayData {
        let (workflow_command, workflow_arguments) = parse_and_escape_workflow(workflow);

        let mut command_parts: VecDeque<WorkflowCommandPart> = VecDeque::new();

        let mut start = 0;
        for arg in workflow_arguments {
            // Capture command up until the argument
            command_parts.push_back(WorkflowCommandPart::CommandPart(String::from(
                &workflow_command[start..arg.byte_range.start.as_usize()],
            )));

            // Capture the argument itself
            command_parts.push_back(WorkflowCommandPart::Argument {
                name: String::from(arg.argument_name),
                value: String::from(arg.replacement_text),
            });
            start = arg.byte_range.end.as_usize();
        }

        // Capature the rest of the command with no arguments
        if start != workflow_command.len() {
            command_parts.push_back(WorkflowCommandPart::CommandPart(String::from(
                &workflow_command[start..],
            )));
        }

        WorkflowCommandDisplayData {
            command_parts: Vec::from(command_parts),
        }
    }
}

/// Computes workflow display data for displaying the workflow in the input editor and info box
/// when the workflow is selected for execution.
pub fn compute_workflow_display_data(workflow: &Workflow) -> WorkflowDisplayData {
    compute_workflow_display_data_internal(workflow, None)
}

/// Computes workflow display data for displaying the workflow in the input editor and info box
/// allowing argument override.
pub fn compute_workflow_display_data_with_overrides(
    workflow: &Workflow,
    override_argument_values: HashMap<String, String>,
) -> WorkflowDisplayData {
    let values = override_argument_values
        .into_iter()
        .map(|(k, v)| (k, ArgumentValue(v)))
        .collect();
    compute_workflow_display_data_internal(workflow, Some(values))
}

/// Computes workflow display data for displaying the workflow in the input editor and info box
/// when a history command associated with a workflow is selected for execution.
pub fn compute_workflow_display_data_for_history_command(
    history_command: &str,
    workflow: &Workflow,
) -> Option<WorkflowDisplayData> {
    let (workflow_command, workflow_arguments) = parse_and_escape_workflow(workflow);

    let argument_values = parse_argument_values_from_command(
        history_command,
        workflow_command.as_str(),
        &workflow_arguments,
    )?;

    let argument_values = argument_values
        .into_iter()
        .zip(workflow_arguments)
        .map(|(value, argument)| (argument.argument_name.to_owned(), value))
        .collect::<HashMap<_, _>>();

    Some(compute_workflow_display_data_internal(
        workflow,
        Some(argument_values),
    ))
}

fn compute_workflow_display_data_internal(
    workflow: &Workflow,
    override_argument_values: Option<HashMap<String, ArgumentValue>>,
) -> WorkflowDisplayData {
    let (command, workflow_arguments) = parse_and_escape_workflow(workflow);
    let mut command_with_replaced_arguments = command.to_owned();

    let mut delta_bytes = 0_isize;
    let mut delta_chars = 0_isize;
    let mut replaced_ranges = vec![];
    let mut argument_index_to_highlight_index_map = HashMap::new();
    let mut argument_index_to_char_range_map = HashMap::new();
    let mut argument_index_to_object_id_map = HashMap::new();

    // Compute the final command (with the argument identifiers replaced with the argument name)
    // and its corresponding text style ranges.
    for (highlight_index, workflow_argument) in workflow_arguments.into_iter().enumerate() {
        let original_char_range = workflow_argument.char_range;
        let original_byte_range = workflow_argument.byte_range;
        let replacement_text = override_argument_values
            .as_ref()
            .and_then(|values| values.get(workflow_argument.argument_name))
            .map(|value| value.0.as_str())
            .unwrap_or(workflow_argument.replacement_text);

        // Compute the range of the argument within the workflow and replace the range in the
        // original workflow command with the argument name.
        let text_byte_range = original_byte_range.start.add_signed(delta_bytes)
            ..original_byte_range.end.add_signed(delta_bytes);
        let text_char_range = original_char_range.start.add_signed(delta_chars)
            ..original_char_range.end.add_signed(delta_chars);

        command_with_replaced_arguments.replace_range(
            text_byte_range.start.as_usize()..text_byte_range.end.as_usize(),
            replacement_text,
        );

        // Compute the delta between the replacement text and the original length of the
        // argument within the command. This is inclusive of the curly braces around the
        // argument name, which is why we add 4 here (since there are two curly braces on each
        // side).
        let original_argument_byte_length = workflow_argument.argument_name.len() + 4;
        delta_bytes += replacement_text.len() as isize - original_argument_byte_length as isize;
        let original_argument_char_length = workflow_argument.argument_name.chars().count() + 4;
        delta_chars +=
            replacement_text.chars().count() as isize - original_argument_char_length as isize;

        argument_index_to_highlight_index_map
            .entry(workflow_argument.argument_index)
            .or_insert_with(Vec::new)
            .push(highlight_index);

        replaced_ranges
            .push(text_byte_range.start..(text_byte_range.start + replacement_text.len()));

        argument_index_to_char_range_map
            .entry(workflow_argument.argument_index)
            .or_insert_with(Vec::new)
            .push(
                text_char_range.start..(text_char_range.start + replacement_text.chars().count()),
            );

        if let ArgumentType::Enum { enum_id } = workflow_argument.argument_type {
            argument_index_to_object_id_map.insert(workflow_argument.argument_index, *enum_id);
        }
    }

    WorkflowDisplayData {
        command_with_replaced_arguments,
        replaced_ranges,
        argument_index_to_highlight_index_map,
        argument_index_to_char_range_map,
        argument_index_to_object_id_map,
    }
}

/// Remove extra brackets from escaped arguments in workflow command and compute a list of workflow arguments and their positions in the escaped command.
fn parse_and_escape_workflow(workflow: &Workflow) -> (String, Vec<WorkflowArgument<'_>>) {
    let (escaped_content, shift_indices) = replace_escaped_brackets_in_command(workflow.content());
    let workflow_arguments = parse_workflow_arguments(workflow, shift_indices);

    (escaped_content, workflow_arguments)
}

/// Replaces the triple brackets used for indicating escaped arguments in the command with double brackets.
/// Returns both the modified command and a vector representing the start indices of the escaped arguments, based on the original command.
fn replace_escaped_brackets_in_command(command: &str) -> (String, Vec<usize>) {
    let mut escaped_indices = Vec::new();
    let mut new_command = command.to_owned();
    let mut num_removed_brackets = 0;

    // Iterate through escaped args and remove one bracket from either end
    for cap in ESCAPED_ARGUMENTS_PATTERN.captures_iter(command) {
        let escaped_arg = cap.get(0).expect("First regex group always exists");

        new_command.replace_range(
            escaped_arg.start() - num_removed_brackets..escaped_arg.end() - num_removed_brackets,
            &escaped_arg.as_str()[1..escaped_arg.len() - 1], // remove one bracket from either end
        );

        // Store the location of the escaped argument
        escaped_indices.push(escaped_arg.start());
        num_removed_brackets += 2;
    }

    // Return the new command and the start index of escaped args that had brackets removed
    (new_command, escaped_indices)
}

/// Given a `workflow` and a vector `escaped_indices`, which is presumed to be a vector representing the start
/// indices of escaped arguments in the `workflow` command as generated by `replace_escaped_brackets_in_command`,
/// return a vector of parsed WorkflowArgument objects.
fn parse_workflow_arguments(
    workflow: &Workflow,
    escaped_indices: Vec<usize>,
) -> Vec<WorkflowArgument<'_>> {
    workflow
        .arguments()
        .iter()
        .map(|argument| (argument, format!("{{{{{}}}}}", argument.name())))
        .enumerate()
        .flat_map(|(argument_index, (argument, argument_placeholder))| {
            let mut char_counter = CharCounter::new(workflow.content());
            workflow
                .content()
                .match_indices(argument_placeholder.as_str())
                .filter(|(argument_start_index, _)| {
                    // Skip any escaped arguments (this case only occurs when an escaped argument has the same inner text as a real argument)
                    *argument_start_index == 0
                        || !escaped_indices.contains(&(*argument_start_index - 1))
                })
                .map(|(argument_start_index, argument_placeholder)| {
                    // Based on how many escape argument brackets we have already removed, shift over the stored argument range.
                    let range_offset = escaped_indices
                        .iter()
                        .filter(|x| **x < argument_start_index)
                        .count()
                        * 2;
                    let byte_start = ByteOffset::from(argument_start_index - range_offset);
                    let byte_end = ByteOffset::from(
                        argument_start_index + argument_placeholder.len() - range_offset,
                    );

                    let char_start = char_counter
                        .char_offset(byte_start)
                        .unwrap_or(CharOffset::from(byte_start.as_usize()));
                    let char_end = char_counter
                        .char_offset(byte_end)
                        .unwrap_or(CharOffset::from(byte_end.as_usize()));

                    WorkflowArgument {
                        argument_index: argument_index.into(),
                        argument_name: argument.name(),
                        argument_type: &argument.arg_type,
                        replacement_text: argument
                            .default_value()
                            .as_deref()
                            .unwrap_or_else(|| argument.name()),
                        byte_range: byte_start..byte_end,
                        char_range: char_start..char_end,
                    }
                })
                .collect::<Vec<_>>()
        })
        .sorted_by_key(|workflow_argument| workflow_argument.byte_range.start)
        .collect_vec()
}

#[derive(Debug)]
pub struct ArgumentValue(String);

/// Attempts to parse argument values from `command`, as specified in the given `workflow_command`.
/// `workflow_arguments` are presumed to be the result of calling `parse_and_escape_workflow` on the
/// given `workflow`.
///
/// If successful, returns a `Vec` containing the inferred argument values in the order they
/// appeared in `command`.
///
/// If `command` doesn't appear to match the given `workflow`, returns `None`.
fn parse_argument_values_from_command(
    history_command: &str,
    workflow_command: &str,
    workflow_arguments: &Vec<WorkflowArgument>,
) -> Option<Vec<ArgumentValue>> {
    // Short-circuit if the workflow has no arguments, in which case we can just compare the commands directly.
    // It's possible to unify the codepath below to handle the no-argument corner case, but I don't think it's
    // actually worth the complexity.
    if workflow_arguments.is_empty() {
        return if history_command == workflow_command {
            Some(vec![])
        } else {
            None
        };
    }

    // Compute a list of the "static" segments of the workflow command (e.g. the segments of the
    // workflow command that are not argument placeholders). For example, the segments for workflow
    // "echo {{foo}}; cat {{bar}}" would be ["echo ", "; cat "].
    //
    // These are matched against the command, with the unmatched parts of the
    // command inferred to be argument values.
    let mut static_workflow_segments = VecDeque::new();

    let mut start = 0;
    for arg in workflow_arguments {
        static_workflow_segments
            .push_back(&workflow_command[start..arg.byte_range.start.as_usize()]);
        start = arg.byte_range.end.as_usize();
    }
    if start != workflow_command.len() {
        static_workflow_segments.push_back(&workflow_command[start..]);
    }

    // Iterate through history command, matching each static workflow segment in order against the
    // command.
    let mut argument_values = vec![];
    let mut end_of_last_matched_segment = 0;

    // This is the number of chars in `command`, which is distinct from command.len(), which returns
    // the number of bytes in `command` and doesn't properly account for multi-byte chars.
    // let command_char_count = command.chars().count();
    let mut char_iter = history_command.char_indices().map(|(i, _)| i).peekable();
    while let Some(&i) = char_iter.peek() {
        // Attempt to match the next static workflow command segment against suffix of
        // `command` starting from `i`.
        let did_match_segment = match static_workflow_segments.front() {
            Some(segment) => history_command[i..].starts_with(segment),
            None => {
                argument_values.push(history_command[i..].to_owned());
                break;
            }
        };

        if did_match_segment {
            // If the segment matched, then we infer the unmatched prefix up until `i` to be a
            // workflow argument value.
            let matched_segment = static_workflow_segments.pop_front();
            if i != end_of_last_matched_segment {
                argument_values.push(history_command[end_of_last_matched_segment..i].to_owned());
            }
            for _ in 0..matched_segment.expect("should exist").chars().count() {
                char_iter.next();
            }
            if let Some(i) = char_iter.peek().copied() {
                end_of_last_matched_segment = i;
            }
        } else {
            char_iter.next();
        }
    }

    if argument_values.len() != workflow_arguments.len() || end_of_last_matched_segment == 0 {
        None
    } else {
        Some(argument_values.into_iter().map(ArgumentValue).collect())
    }
}

/// Returns `true` if the given `command` is an instance of the given `workflow`.
pub fn command_matches_workflow(command: &str, workflow: &Workflow) -> bool {
    let (workflow_command, workflow_arguments) = parse_and_escape_workflow(workflow);
    parse_argument_values_from_command(command, workflow_command.as_str(), &workflow_arguments)
        .is_some()
}

#[cfg(test)]
#[path = "command_parser_tests.rs"]
mod tests;
