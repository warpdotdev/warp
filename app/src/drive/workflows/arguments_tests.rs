use std::{fmt::Debug, iter::zip};

use crate::workflows::workflow::Argument;
use warpui::App;

use super::ArgumentsState;

fn assert_vector_eq<T>(vector: Vec<T>, expected_vector: &Vec<T>)
where
    T: Debug + PartialEq,
{
    assert_eq!(vector.len(), expected_vector.len());
    zip(vector, expected_vector).for_each(|(element, expected)| {
        assert_eq!(element, *expected);
    })
}

fn build_argument(
    name: impl Into<String>,
    description: impl Into<Option<String>>,
    default_value: impl Into<Option<String>>,
) -> Argument {
    Argument {
        name: name.into(),
        description: description.into(),
        default_value: default_value.into(),
        arg_type: Default::default(),
    }
}

#[test]
fn test_arguments_state_from_string() {
    App::test((), |_app| async move {
        let empty_args_state: ArgumentsState = Default::default();

        let mut args_state = ArgumentsState::for_command_workflow(
            &empty_args_state,
            "one two{{three}} {{four}}".to_string(),
        );
        assert_vector_eq(
            args_state.arguments.clone(),
            &vec![
                build_argument("three", None, None),
                build_argument("four", None, None),
            ],
        );

        // Mutate data in current arguments state object
        if let Some(change_index) = args_state.word_index_to_arg_index_map.get(&2) {
            if let Some(change_arg) = args_state.arguments.get_mut(*change_index) {
                change_arg.description = Some("new desc".to_string());
                change_arg.default_value = Some("default value".to_string());
            }
        }

        if let Some(change_index) = args_state.word_index_to_arg_index_map.get(&3) {
            if let Some(change_arg) = args_state.arguments.get_mut(*change_index) {
                change_arg.description = Some("another desc".to_string());
                change_arg.default_value = Some("change default value".to_string());
            }
        }

        // Edits that don't change number of words retain data
        let new_args_state = ArgumentsState::for_command_workflow(
            &args_state,
            "one two {{the}} {{four}}".to_string(),
        );
        assert_vector_eq(
            new_args_state.arguments.clone(),
            &vec![
                build_argument("the", "new desc".to_string(), "default value".to_string()),
                build_argument(
                    "four",
                    "another desc".to_string(),
                    "change default value".to_string(),
                ),
            ],
        );

        // Insertions, retain data from args before and after
        let insertion_args_state = ArgumentsState::for_command_workflow(
            &new_args_state,
            "one two {{the}}{{five}} {{six}}{{four}}".to_string(),
        );
        assert_vector_eq(
            insertion_args_state.arguments.clone(),
            &vec![
                build_argument("the", "new desc".to_string(), "default value".to_string()),
                build_argument("five", None, None),
                build_argument("six", None, None),
                build_argument(
                    "four",
                    "another desc".to_string(),
                    "change default value".to_string(),
                ),
            ],
        );

        // Insertion that modifies argument name does not retain data
        let insertion_into_existing_args_state = ArgumentsState::for_command_workflow(
            &insertion_args_state,
            "one two {{the}}{{five}} {{forever}}ix}} {{four}}".to_string(),
        );
        assert_vector_eq(
            insertion_into_existing_args_state.arguments.clone(),
            &vec![
                build_argument("the", "new desc".to_string(), "default value".to_string()),
                build_argument("five", None, None),
                build_argument("forever", None, None),
                build_argument(
                    "four",
                    "another desc".to_string(),
                    "change default value".to_string(),
                ),
            ],
        );

        // Deletion, args before and after retain data
        let deletion_args_state = ArgumentsState::for_command_workflow(
            &insertion_into_existing_args_state,
            "one two {{the}} {{forever}}ix}} {{four}}".to_string(),
        );
        assert_vector_eq(
            deletion_args_state.arguments.clone(),
            &vec![
                build_argument("the", "new desc".to_string(), "default value".to_string()),
                build_argument("forever", None, None),
                build_argument(
                    "four",
                    "another desc".to_string(),
                    "change default value".to_string(),
                ),
            ],
        );

        // Deletion that modifies argument name does not retain data
        let deletion_into_existing_args_state = ArgumentsState::for_command_workflow(
            &deletion_args_state,
            "one two {{the}} {{forev}} {{four}}".to_string(),
        );
        assert_vector_eq(
            deletion_into_existing_args_state.arguments.clone(),
            &vec![
                build_argument("the", "new desc".to_string(), "default value".to_string()),
                build_argument("forev", None, None),
                build_argument(
                    "four",
                    "another desc".to_string(),
                    "change default value".to_string(),
                ),
            ],
        );

        // Duplicate arguments are not registered separately, only recognize first occurrence index
        let repeated_args_state = ArgumentsState::for_command_workflow(
            &deletion_into_existing_args_state,
            "one two {{the}} {{forev}} {{the}} {{four}}".to_string(),
        );
        assert_vector_eq(
            repeated_args_state.arguments,
            &vec![
                build_argument("the", "new desc".to_string(), "default value".to_string()),
                build_argument("forev", None, None),
                build_argument(
                    "four",
                    "another desc".to_string(),
                    "change default value".to_string(),
                ),
            ],
        );
    });
}

#[test]
fn test_arguments_state_from_string_multicursor() {
    App::test((), |_app| async move {
        let empty_args_state: ArgumentsState = Default::default();

        let mut args_state = ArgumentsState::for_command_workflow(
            &empty_args_state,
            "one two{{three}} {{four}}".to_string(),
        );
        assert_vector_eq(
            args_state.arguments.clone(),
            &vec![
                build_argument("three", None, None),
                build_argument("four", None, None),
            ],
        );

        // Mutate data in current arguments state object
        if let Some(change_index) = args_state.word_index_to_arg_index_map.get(&2) {
            if let Some(change_arg) = args_state.arguments.get_mut(*change_index) {
                change_arg.description = Some("new desc".to_string());
                change_arg.default_value = Some("default value".to_string());
            }
        }

        if let Some(change_index) = args_state.word_index_to_arg_index_map.get(&3) {
            if let Some(change_arg) = args_state.arguments.get_mut(*change_index) {
                change_arg.description = Some("another desc".to_string());
                change_arg.default_value = Some("change default value".to_string());
            }
        }

        // "on|e two{{thre|e}} {{f|our}}"
        // Edit retains data
        let multicursor_edit_args_state = ArgumentsState::for_command_workflow(
            &args_state,
            "onye two{{threye}} {{fyour}}".to_string(),
        );
        assert_vector_eq(
            multicursor_edit_args_state.arguments.clone(),
            &vec![
                build_argument(
                    "threye",
                    "new desc".to_string(),
                    "default value".to_string(),
                ),
                build_argument(
                    "fyour",
                    "another desc".to_string(),
                    "change default value".to_string(),
                ),
            ],
        );

        // "on|ye two{{th|reye}} {{fyour}}|"
        // Insert retains data for matching argument names
        let mut multicursor_insert_args_state = ArgumentsState::for_command_workflow(
            &multicursor_edit_args_state,
            "onyee two{{thereye}} {{fyour}}e".to_string(),
        );
        assert_vector_eq(
            multicursor_insert_args_state.arguments.clone(),
            &vec![
                build_argument("thereye", None, None),
                build_argument(
                    "fyour",
                    "another desc".to_string(),
                    "change default value".to_string(),
                ),
            ],
        );

        if let Some(change_index) = multicursor_insert_args_state
            .word_index_to_arg_index_map
            .get(&2)
        {
            if let Some(change_arg) = multicursor_insert_args_state
                .arguments
                .get_mut(*change_index)
            {
                change_arg.description = Some("test desc".to_string());
                change_arg.default_value = Some("with dvalue".to_string());
            }
        }

        // "on|yee |two{{thereye}} {{fyou|r}}e"
        // Delete retains data for matching argument names
        let multicursor_delete_args_state = ArgumentsState::for_command_workflow(
            &multicursor_insert_args_state,
            "oyeetwo{{thereye}} {{fyor}}e".to_string(),
        );
        assert_vector_eq(
            multicursor_delete_args_state.arguments,
            &vec![
                build_argument(
                    "thereye",
                    "test desc".to_string(),
                    "with dvalue".to_string(),
                ),
                build_argument("fyor", None, None),
            ],
        );
    });
}

#[test]
fn test_arguments_state_from_string_with_leading_whitespace() {
    App::test((), |_app| async move {
        let empty_args_state: ArgumentsState = Default::default();
        let args_state = ArgumentsState::for_command_workflow(
            &empty_args_state,
            "           one two{{three}} {{four}}".to_string(),
        );

        assert_vector_eq(
            args_state.arguments,
            &vec![
                build_argument("three", None, None),
                build_argument("four", None, None),
            ],
        );
    });
}
