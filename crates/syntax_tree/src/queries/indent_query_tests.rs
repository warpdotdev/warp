use std::sync::Arc;

use arborium::tree_sitter::Tree;
use languages::{Language, language_by_filename};
use warp_editor::content::buffer::{Buffer, BufferSnapshot};
use warp_editor::content::selection_model::BufferSelectionModel;
use warp_editor::content::text::IndentBehavior;
use warp_util::standardized_path::StandardizedPath;
use warpui_core::App;

use super::*;
use crate::SyntaxTreeState;

// Simple stub function to allow compilation - can be improved later
fn mock_buffer_and_tree(text_content: &str, language: Arc<Language>) -> (Buffer, Tree) {
    // Create a tree by parsing the text
    let snapshot = BufferSnapshot::from_plain_text(text_content);
    let tree = warpui_core::r#async::block_on(async {
        SyntaxTreeState::parse_text(snapshot, None, &language).await
    })
    .expect("test buffer is small and should parse");

    // Create a minimal buffer
    let buffer = Buffer::new(Box::new(|_, _| IndentBehavior::Ignore));
    (buffer, tree)
}

fn test_path(filename: &str) -> StandardizedPath {
    StandardizedPath::try_new(&format!("/{filename}")).expect("test path should be absolute")
}

#[test]
fn test_indent_query() {
    App::test((), |mut app| async move {
        let language = language_by_filename(&test_path("test.rs"))
            .expect("Should contain language rule for rust");
        let text_content = r#"impl Test {
        fn first_func() {

        }

        fn second_func() {
            if true {

            }
}
    }"#;

        let buffer_handle = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer_handle.clone()));

        buffer_handle.update(&mut app, |buffer, ctx| {
            *buffer = Buffer::from_plain_text(
                text_content,
                None,
                Box::new(|_, _| IndentBehavior::Ignore),
                selection,
                ctx,
            );
        });

        let buffer_snapshot = buffer_handle.read(&app, |buffer, _| buffer.buffer_snapshot());
        let tree = warpui_core::r#async::block_on(async {
            SyntaxTreeState::parse_text(buffer_snapshot, None, &language).await
        })
        .expect("test buffer is small and should parse");

        let query = language.as_ref().indents_query.as_ref().unwrap();

        buffer_handle.read(&app, |buffer, _| {
            // Check that the top level code is not improperly marked as indented because of the indentation
            // in the string literal.
            assert_eq!(
                indentation_delta(buffer, &tree, Point { row: 0, column: 0 }, query)
                    .unwrap()
                    .delta,
                0
            );

            assert_eq!(
                indentation_delta(buffer, &tree, Point { row: 1, column: 0 }, query)
                    .unwrap()
                    .delta,
                1
            );

            // Indentation level in first_func should be 2.
            assert_eq!(
                indentation_delta(buffer, &tree, Point { row: 2, column: 0 }, query)
                    .unwrap()
                    .delta,
                2
            );

            // Indentation level between first_func and second_func definition should be 1.
            assert_eq!(
                indentation_delta(buffer, &tree, Point { row: 4, column: 0 }, query)
                    .unwrap()
                    .delta,
                1
            );

            // Indentation level inside the if statement in second_func should be 3.
            assert_eq!(
                indentation_delta(buffer, &tree, Point { row: 7, column: 0 }, query)
                    .unwrap()
                    .delta,
                3
            );

            // Indentation level at the start of the closing bracket should be 1.
            assert_eq!(
                indentation_delta(buffer, &tree, Point { row: 9, column: 0 }, query)
                    .unwrap()
                    .delta,
                1
            );
        });
    });
}

#[test]
fn test_indent_query_on_go() {
    App::test((), |mut app| async move {
        let language = language_by_filename(&test_path("test.go"))
            .expect("Should contain language rule for go");
        let text_content = r#"package logic
        import (
            "context"
            "fmt"
        )

        type TestType struct {
            Attribute1 int
            Attribute2 int
        }

        func CreateTestType(ctx context.Context, db types.SqlQuerier) (*TestType, error) {
            if !testTypeExist() {
                return nil, testTypeNotExistError
    }

            return ctx.GetTest(), nil
        }"#;

        let buffer_handle = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer_handle.clone()));

        buffer_handle.update(&mut app, |buffer, ctx| {
            *buffer = Buffer::from_plain_text(
                text_content,
                None,
                Box::new(|_, _| IndentBehavior::Ignore),
                selection,
                ctx,
            );
        });

        let buffer_snapshot = buffer_handle.read(&app, |buffer, _| buffer.buffer_snapshot());
        let tree = warpui_core::r#async::block_on(async {
            SyntaxTreeState::parse_text(buffer_snapshot, None, &language).await
        })
        .expect("test buffer is small and should parse");

        let query = &language.as_ref().indents_query.as_ref().unwrap();

        buffer_handle.read(&app, |buffer, _| {
            // Check that the top level code is not improperly marked as indented because of the indentation
            // in the string literal.
            assert_eq!(
                indentation_delta(buffer, &tree, Point { row: 0, column: 0 }, query)
                    .unwrap()
                    .delta,
                0
            );
            assert_eq!(
                indentation_delta(buffer, &tree, Point { row: 1, column: 0 }, query)
                    .unwrap()
                    .delta,
                0
            );

            // Indentation level in import statements should be 1.
            assert_eq!(
                indentation_delta(buffer, &tree, Point { row: 3, column: 0 }, query)
                    .unwrap()
                    .delta,
                1
            );

            // Indentation level in type definition should be 1.
            assert_eq!(
                indentation_delta(buffer, &tree, Point { row: 7, column: 0 }, query)
                    .unwrap()
                    .delta,
                1
            );

            // Indentation level in if statement should be 2.
            assert_eq!(
                indentation_delta(buffer, &tree, Point { row: 13, column: 0 }, query)
                    .unwrap()
                    .delta,
                2
            );

            // Indentation level at the start of the if statement closing bracket should be 2.
            assert_eq!(
                indentation_delta(buffer, &tree, Point { row: 14, column: 0 }, query)
                    .unwrap()
                    .delta,
                2
            );
        });
    });
}

#[test]
fn test_indent_query_on_go_bracket_expansion() {
    let language =
        language_by_filename(&test_path("test.go")).expect("Should contain language rule for go");
    let (buffer, tree) = mock_buffer_and_tree(
        r#"func test(){}
        func test() {
            go func() {}
        }"#,
        language.clone(),
    );
    let query = &language.as_ref().indents_query.as_ref().unwrap();

    // Indentation level on first line between parentheses should be 0 (considering the closing bracket).
    assert_eq!(
        indentation_delta(&buffer, &tree, Point { row: 0, column: 10 }, query)
            .unwrap()
            .delta,
        0
    );

    // Indentation level on first line between brackets should be 0 (considering the closing bracket).
    assert_eq!(
        indentation_delta(&buffer, &tree, Point { row: 0, column: 12 }, query)
            .unwrap()
            .delta,
        0
    );

    // Indentation level on the third line between brackets should be 2 since this is a
    // go func nested in another func.
    assert_eq!(
        indentation_delta(&buffer, &tree, Point { row: 2, column: 23 }, query)
            .unwrap()
            .delta,
        2
    );
}

// source: https://peps.python.org/pep-0008/#indentation
#[test]
fn test_indent_query_on_python() {
    let language = language_by_filename(&test_path("test.py"))
        .expect("Should contain language rule for python");
    let (buffer, tree) = mock_buffer_and_tree(
        r#"# Aligned with opening delimiter.
        foo = long_function_name(var_one, var_two,
                                 var_three, var_four)

        # Add 4 spaces (an extra level of indentation) to distinguish arguments from the rest.
        def long_function_name(
                var_one, var_two, var_three,
                var_four):
            print(var_one)"#,
        language.clone(),
    );
    let query = &language.as_ref().indents_query.as_ref().unwrap();

    // Check that the top level code is not improperly marked as indented because of the indentation
    // in the string literal.
    assert_eq!(
        indentation_delta(&buffer, &tree, Point { row: 0, column: 0 }, query)
            .unwrap()
            .delta,
        0
    );
    assert_eq!(
        indentation_delta(&buffer, &tree, Point { row: 1, column: 0 }, query)
            .unwrap()
            .delta,
        0
    );

    // Indentation level argument list split across lines should be 1.
    assert_eq!(
        indentation_delta(&buffer, &tree, Point { row: 2, column: 0 }, query)
            .unwrap()
            .delta,
        1
    );

    // Indentation level in top-level function definition should be 1.
    assert_eq!(
        indentation_delta(&buffer, &tree, Point { row: 8, column: 0 }, query)
            .unwrap()
            .delta,
        1
    );
}

#[test]
fn test_indent_query_on_python_colon() {
    let language = language_by_filename(&test_path("test.py"))
        .expect("Should contain language rule for python");
    let (if_buffer, if_tree) = mock_buffer_and_tree(r#"if x:"#, language.clone());
    let query = &language.as_ref().indents_query.as_ref().unwrap();

    // `if x:|`
    assert_eq!(
        indentation_delta(&if_buffer, &if_tree, Point { row: 0, column: 5 }, query)
            .unwrap()
            .delta,
        1
    );

    let (empty_next_line, empty_next_line_tree) = mock_buffer_and_tree(
        r#"if x:
        "#,
        language.clone(),
    );
    let query = &language.as_ref().indents_query.as_ref().unwrap();

    // `if x:|
    // `
    assert_eq!(
        indentation_delta(
            &empty_next_line,
            &empty_next_line_tree,
            Point { row: 0, column: 5 },
            query
        )
        .unwrap()
        .delta,
        1
    );

    // The `if_statement`'s block would end on line 1, so pressing `Enter` here should go back to
    // indentation level 0
    // `if x:
    // |`
    assert_eq!(
        indentation_delta(
            &empty_next_line,
            &empty_next_line_tree,
            Point { row: 1, column: 0 },
            query
        )
        .unwrap()
        .delta,
        0
    );

    // This is invalid Python syntax because the `pass` statement is not indented.
    let (invalid_syntax_buffer, invalid_syntax_tree) = mock_buffer_and_tree(
        r#"if x:
        pass"#,
        language.clone(),
    );
    let query = &language.as_ref().indents_query.as_ref().unwrap();

    // `if x:|
    // pass`
    assert_eq!(
        indentation_delta(
            &invalid_syntax_buffer,
            &invalid_syntax_tree,
            Point { row: 0, column: 5 },
            query
        )
        .unwrap()
        .delta,
        1
    );

    // `if x:
    // |pass`
    assert_eq!(
        indentation_delta(
            &invalid_syntax_buffer,
            &invalid_syntax_tree,
            Point { row: 1, column: 0 },
            query
        )
        .unwrap()
        .delta,
        1
    );

    // `if x:
    // pass|`
    assert_eq!(
        indentation_delta(
            &invalid_syntax_buffer,
            &invalid_syntax_tree,
            Point { row: 1, column: 4 },
            query
        )
        .unwrap()
        .delta,
        1
    );

    let (valid_non_empty_new_line_buffer, valid_non_empty_new_line_tree) = mock_buffer_and_tree(
        r#"if x:
            pass"#,
        language.clone(),
    );
    let query = &language.as_ref().indents_query.as_ref().unwrap();

    // `if x:|
    //     pass`
    assert_eq!(
        indentation_delta(
            &valid_non_empty_new_line_buffer,
            &valid_non_empty_new_line_tree,
            Point { row: 0, column: 5 },
            query
        )
        .unwrap()
        .delta,
        1
    );

    // `if x:
    //  |   pass`
    assert_eq!(
        indentation_delta(
            &valid_non_empty_new_line_buffer,
            &valid_non_empty_new_line_tree,
            Point { row: 1, column: 0 },
            query
        )
        .unwrap()
        .delta,
        1
    );

    // `if x:
    //     pass|`
    assert_eq!(
        indentation_delta(
            &valid_non_empty_new_line_buffer,
            &valid_non_empty_new_line_tree,
            Point { row: 1, column: 7 },
            query
        )
        .unwrap()
        .delta,
        1
    );

    let (split_line_buffer, split_line_bugger) =
        mock_buffer_and_tree(r#"if x:pass"#, language.clone());
    let query = &language.as_ref().indents_query.as_ref().unwrap();

    // `if x:|pass`
    assert_eq!(
        indentation_delta(
            &split_line_buffer,
            &split_line_bugger,
            Point { row: 0, column: 5 },
            query
        )
        .unwrap()
        .delta,
        1
    );

    // `if x:pass|`
    assert_eq!(
        indentation_delta(
            &split_line_buffer,
            &split_line_bugger,
            Point { row: 0, column: 9 },
            query
        )
        .unwrap()
        .delta,
        1
    );

    let (function_buffer, function_tree) = mock_buffer_and_tree(r#"def foo():"#, language.clone());
    let query = &language.as_ref().indents_query.as_ref().unwrap();

    // `def foo():|`
    assert_eq!(
        indentation_delta(
            &function_buffer,
            &function_tree,
            Point { row: 0, column: 10 },
            query
        )
        .unwrap()
        .delta,
        1
    );

    // Text content is fully outdented to avoid confusion with indentation and Rust raw string
    // literals.
    let (multilevel_buffer, multilevel_tree) = mock_buffer_and_tree(
        r#"
def foo():
    x = True
    if x:"#,
        language.clone(),
    );
    let query = &language.as_ref().indents_query.as_ref().unwrap();

    // `
    // def foo():
    //     x = True|
    //     if x:`
    assert_eq!(
        indentation_delta(
            &multilevel_buffer,
            &multilevel_tree,
            Point { row: 2, column: 11 },
            query
        )
        .unwrap()
        .delta,
        1
    );

    // `
    // def foo():
    //     x = True
    // |   if x:`
    assert_eq!(
        indentation_delta(
            &multilevel_buffer,
            &multilevel_tree,
            Point { row: 3, column: 0 },
            query
        )
        .unwrap()
        .delta,
        1
    );

    // `
    // def foo():
    //      x = True
    //      if x:|`
    assert_eq!(
        indentation_delta(
            &multilevel_buffer,
            &multilevel_tree,
            Point { row: 3, column: 9 },
            query
        )
        .unwrap()
        .delta,
        2
    );

    let (function_buffer, function_tree) = mock_buffer_and_tree(
        r#"
def foo():
    x = True
    if x:
"#,
        language.clone(),
    );
    let query = &language.as_ref().indents_query.as_ref().unwrap();

    // `
    // def foo():
    //     x = True
    //     if x:|
    // `
    assert_eq!(
        indentation_delta(
            &function_buffer,
            &function_tree,
            Point { row: 2, column: 9 },
            query
        )
        .unwrap()
        .delta,
        1
    );

    // `
    // def foo():
    //     x = True
    //     if x:
    // |`
    assert_eq!(
        indentation_delta(
            &function_buffer,
            &function_tree,
            Point { row: 4, column: 0 },
            query
        )
        .unwrap()
        .delta,
        0
    );
}

#[test]
fn test_indent_query_on_javascript() {
    let language = language_by_filename(&test_path("test.js"))
        .expect("Should contain language rule for javascript");
    let (buffer, tree) = mock_buffer_and_tree(
        r#"// Import the 'fs' module, commonly used for file operations
        const fs = require('fs');

        // Function to check if a number is positive
        function checkIfPositive(number) {
            // Check if the number is greater than zero
            if (number > 0) {
                console.log('The number is positive.');
            } else if (number === 0) {
                console.log('The number is zero.');
            } else {
                console.log('The number is negative.');
            }
        }

        // Example usage of the function
        checkIfPositive(5);
        checkIfPositive(-3);
        checkIfPositive(0);"#,
        language.clone(),
    );
    let query = &language.as_ref().indents_query.as_ref().unwrap();

    // Check that the top level code is not improperly marked as indented because of the indentation
    // in the string literal.
    assert_eq!(
        indentation_delta(&buffer, &tree, Point { row: 0, column: 0 }, query)
            .unwrap()
            .delta,
        0
    );
    assert_eq!(
        indentation_delta(&buffer, &tree, Point { row: 1, column: 0 }, query)
            .unwrap()
            .delta,
        0
    );

    // Indentation level inside function definition should be 1.
    assert_eq!(
        indentation_delta(&buffer, &tree, Point { row: 5, column: 0 }, query)
            .unwrap()
            .delta,
        1
    );

    // Indentation level in if statement should be 1 more than its parent.
    assert_eq!(
        indentation_delta(&buffer, &tree, Point { row: 7, column: 0 }, query)
            .unwrap()
            .delta,
        2
    );
}

#[test]
fn test_indent_query_on_typescript() {
    let language = language_by_filename(&test_path("test.ts"))
        .expect("Should contain language rule for typescript");
    let (buffer, tree) = mock_buffer_and_tree(
        r#"import { User } from './types';
    import { validateEmail } from './utils';

    interface ProcessedUser {
        id: string;
        displayName: string;
        emailStatus: 'valid' | 'invalid';
    }

    function processUserData(user: User, includeEmail: boolean = false): ProcessedUser {
        const processedUser: ProcessedUser = {
            id: user.id,
            displayName: '',
            emailStatus: 'invalid'
        };

        if (user.firstName && user.lastName) {
            processedUser.displayName = `${user.firstName} ${user.lastName}`;
        } else if (user.firstName) {
            processedUser.displayName = user.firstName;
        } else {
            processedUser.displayName = 'Anonymous User';
        }

        if (includeEmail && user.email) {
            processedUser.emailStatus = validateEmail(user.email) ? 'valid' : 'invalid';
        }

        return processedUser;
    }

    export { ProcessedUser, processUserData };"#,
        language.clone(),
    );
    let query = &language.as_ref().indents_query.as_ref().unwrap();

    // Check that the top level code is not improperly marked as indented because of the indentation
    // in the string literal.
    assert_eq!(
        indentation_delta(&buffer, &tree, Point { row: 0, column: 0 }, query)
            .unwrap()
            .delta,
        0
    );
    assert_eq!(
        indentation_delta(&buffer, &tree, Point { row: 1, column: 0 }, query)
            .unwrap()
            .delta,
        0
    );

    // Indentation level inside top level interface definition should be 1.
    assert_eq!(
        indentation_delta(&buffer, &tree, Point { row: 4, column: 0 }, query)
            .unwrap()
            .delta,
        1
    );

    // Indentation level in if statement should be 1 more than its parent.
    assert_eq!(
        indentation_delta(&buffer, &tree, Point { row: 17, column: 0 }, query)
            .unwrap()
            .delta,
        2
    );
}
