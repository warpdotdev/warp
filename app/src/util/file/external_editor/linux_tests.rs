use std::path::PathBuf;

use warp_util::path::LineAndColumnArg;

use super::{tokenize_exec, DesktopExecError, EditorMetadata};

#[cfg(test)]
fn with_files(tag: &str, contents: &str, cb: impl FnOnce(PathBuf, PathBuf) -> anyhow::Result<()>) {
    use crate::test_util::{Stub, VirtualFS};

    VirtualFS::test(tag, |dirs, mut sandbox| {
        sandbox.with_files(vec![
            Stub::FileWithContent("bar.desktop", contents),
            Stub::EmptyFile("foo.txt"),
        ]);

        let desktop_file_path = dirs.tests().join("bar.desktop");
        let content_file_path = dirs.tests().join("foo.txt");

        match cb(desktop_file_path, content_file_path) {
            Ok(_) => {}
            Err(err) => panic!("{err:?}"),
        };
    })
}

// ---------- tokenize_exec unit tests ----------

#[test]
fn test_tokenize_simple() {
    let tokens = tokenize_exec("echo hello world").unwrap();
    assert_eq!(tokens, vec!["echo", "hello", "world"]);
}

#[test]
fn test_tokenize_quoted_argument() {
    let tokens = tokenize_exec(r#""/path/with spaces/editor" %f"#).unwrap();
    assert_eq!(tokens, vec!["/path/with spaces/editor", "%f"]);
}

#[test]
fn test_tokenize_escape_sequences_in_quotes() {
    let tokens = tokenize_exec(r#""a\"b\\c\$d\`e""#).unwrap();
    assert_eq!(tokens, vec!["a\"b\\c$d`e"]);
}

#[test]
fn test_tokenize_unrecognized_escape_in_quotes_keeps_backslash() {
    let tokens = tokenize_exec(r#""foo\nbar""#).unwrap();
    assert_eq!(tokens, vec!["foo\\nbar"]);
}

#[test]
fn test_tokenize_unterminated_quote_errors() {
    let result = tokenize_exec(r#""unterminated"#);
    assert!(matches!(result, Err(DesktopExecError::UnterminatedQuote)));
}

#[test]
fn test_tokenize_multiple_whitespace() {
    let tokens = tokenize_exec("a   b\tc\n d").unwrap();
    assert_eq!(tokens, vec!["a", "b", "c", "d"]);
}

#[test]
fn test_tokenize_empty_string() {
    let tokens = tokenize_exec("").unwrap();
    assert!(tokens.is_empty());
}

#[test]
fn test_tokenize_quoted_empty_string_produces_token() {
    let tokens = tokenize_exec(r#"cmd """#).unwrap();
    assert_eq!(tokens, vec!["cmd", ""]);
}

// ---------- build_command tests ----------

#[test]
fn test_missing_exec_command_errors() {
    with_files(
        "test_missing_exec_command_errors",
        "",
        |desktop, _content| {
            let result = EditorMetadata::try_new(desktop);

            assert!(matches!(result, Err(DesktopExecError::NoExec)));
            Ok(())
        },
    )
}

#[test]
fn test_unterminated_quote_errors() {
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec="unterminated %f
    "#;
    with_files(
        "test_unterminated_quote_errors",
        data,
        |desktop, content| {
            let metadata = EditorMetadata::try_new(desktop)?;
            let result = metadata.build_default_command(&content);
            assert!(matches!(result, Err(DesktopExecError::UnterminatedQuote)));
            Ok(())
        },
    )
}

#[test]
fn test_basic_exec_no_field_codes() {
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=/usr/bin/editor --flag
    "#;
    with_files(
        "test_basic_exec_no_field_codes",
        data,
        |desktop, content| {
            let metadata = EditorMetadata::try_new(desktop)?;
            let result = metadata.build_default_command(&content);
            assert!(result.is_ok());
            let cmd = result.unwrap();
            assert_eq!(cmd.get_program(), "/usr/bin/editor");
            assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["--flag"]);
            Ok(())
        },
    )
}

#[test]
fn test_file_path_substitution() {
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=cat %f
    "#;
    with_files("test_file_path_substitution", data, |desktop, content| {
        let metadata = EditorMetadata::try_new(desktop)?;
        let file_name = content.display().to_string();
        let result = metadata.build_default_command(&content);

        assert!(result.is_ok());
        let cmd = result.unwrap();
        assert_eq!(cmd.get_program(), "cat");
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), [file_name.as_str()]);
        Ok(())
    });

    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=cat %F
    "#;
    with_files("test_file_path_substitution", data, |desktop, content| {
        let metadata = EditorMetadata::try_new(desktop)?;
        let file_name = content.display().to_string();
        let result = metadata.build_default_command(&content);

        assert!(result.is_ok());
        let cmd = result.unwrap();
        assert_eq!(cmd.get_program(), "cat");
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), [file_name.as_str()]);
        Ok(())
    });
}

#[test]
fn test_file_url_substitution() {
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=open %u
    "#;
    with_files("test_file_url_substitution", data, |desktop, content| {
        let metadata = EditorMetadata::try_new(desktop)?;
        let file_name = content.display().to_string();
        let expected_file_uri = format!("file://{file_name}");
        let result = metadata.build_default_command(&content);

        assert!(result.is_ok());

        let cmd = result.unwrap();
        assert_eq!(cmd.get_program(), "open");
        assert_eq!(
            cmd.get_args().collect::<Vec<_>>(),
            [expected_file_uri.as_str()]
        );
        Ok(())
    });

    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=open %U
    "#;
    with_files("test_file_url_substitution", data, |desktop, content| {
        let metadata = EditorMetadata::try_new(desktop)?;
        let file_name = content.display().to_string();
        let expected_file_uri = format!("file://{file_name}");
        let result = metadata.build_default_command(&content);

        assert!(result.is_ok());

        let cmd = result.unwrap();
        assert_eq!(cmd.get_program(), "open");
        assert_eq!(
            cmd.get_args().collect::<Vec<_>>(),
            [expected_file_uri.as_str()]
        );
        Ok(())
    });
}

#[test]
fn test_field_code_substitutions() {
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=/usr/bin/app %c %i %k %%
    Name=Warp Test Application
    Icon=/foo/bar/icon.png
    "#;
    with_files("test_field_code_substitutions", data, |desktop, content| {
        let desktop_file_path = desktop.display().to_string();
        let metadata = EditorMetadata::try_new(desktop)?;
        let result = metadata.build_default_command(&content);

        assert!(result.is_ok());

        let cmd = result.unwrap();
        assert_eq!(cmd.get_program(), "/usr/bin/app");
        // %i expands to TWO arguments per the spec: --icon and the icon path.
        assert_eq!(
            cmd.get_args().collect::<Vec<_>>(),
            [
                "Warp Test Application",
                "--icon",
                "/foo/bar/icon.png",
                desktop_file_path.as_str(),
                "%",
            ]
        );
        Ok(())
    });
}

#[test]
fn test_jetbrains_command_no_line_numbers() {
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=/snap/bin/phpstorm %f
    "#;

    with_files(
        "test_jetbrains_command_no_line_numbers",
        data,
        |desktop, content| {
            let metadata = EditorMetadata::try_new(desktop)?;
            let file_path = content.display().to_string();
            let result = metadata.build_jetbrains_command(&content, None);

            assert!(result.is_ok());

            let cmd = result.unwrap();
            assert_eq!(cmd.get_program(), "/snap/bin/phpstorm");
            assert_eq!(cmd.get_args().collect::<Vec<_>>(), [file_path.as_str()]);
            Ok(())
        },
    );
}

#[test]
fn test_jetbrains_command_line_numbers() {
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=/snap/bin/phpstorm %f
    "#;

    with_files(
        "test_jetbrains_command_line_numbers",
        data,
        |desktop, content| {
            let metadata = EditorMetadata::try_new(desktop)?;
            let file_path = content.display().to_string();
            let result = metadata.build_jetbrains_command(
                &content,
                Some(LineAndColumnArg {
                    line_num: 42,
                    column_num: None,
                }),
            );

            assert!(result.is_ok());

            let cmd = result.unwrap();
            assert_eq!(cmd.get_program(), "/snap/bin/phpstorm");
            assert_eq!(
                cmd.get_args().collect::<Vec<_>>(),
                ["--line", "42", file_path.as_str()]
            );
            Ok(())
        },
    );
}

#[test]
fn test_jetbrains_command_line_and_col_numbers() {
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=/snap/bin/phpstorm %f
    "#;
    with_files(
        "test_jetbrains_command_line_and_col_numbers",
        data,
        |desktop, content| {
            let metadata = EditorMetadata::try_new(desktop)?;
            let file_path = content.display().to_string();
            let result = metadata.build_jetbrains_command(
                &content,
                Some(LineAndColumnArg {
                    line_num: 42,
                    column_num: Some(25),
                }),
            );

            assert!(result.is_ok());

            let cmd = result.unwrap();
            assert_eq!(cmd.get_program(), "/snap/bin/phpstorm");
            assert_eq!(
                cmd.get_args().collect::<Vec<_>>(),
                ["--line", "42", "--column", "25", file_path.as_str()]
            );
            Ok(())
        },
    );
}

#[test]
fn test_sublime_command_no_line_numbers() {
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=/snap/bin/subl %f
    "#;
    with_files(
        "test_sublime_command_no_line_numbers",
        data,
        |desktop, content| {
            let metadata = EditorMetadata::try_new(desktop)?;
            let file_path = content.display().to_string();
            let result: Result<command::blocking::Command, DesktopExecError> =
                metadata.build_sublime_command(&content, None);

            assert!(result.is_ok());

            let cmd = result.unwrap();
            assert_eq!(cmd.get_program(), "/snap/bin/subl");
            assert_eq!(cmd.get_args().collect::<Vec<_>>(), [file_path.as_str()]);
            Ok(())
        },
    );
}

#[test]
fn test_sublime_command_line_numbers() {
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=/snap/bin/subl %f
    "#;
    with_files(
        "test_sublime_command_line_numbers",
        data,
        |desktop, content| {
            let metadata = EditorMetadata::try_new(desktop)?;
            let file_path = content.display().to_string();
            let result = metadata.build_sublime_command(
                &content,
                Some(LineAndColumnArg {
                    line_num: 42,
                    column_num: None,
                }),
            );

            assert!(result.is_ok());

            let cmd = result.unwrap();
            assert_eq!(cmd.get_program(), "/snap/bin/subl");
            assert_eq!(
                cmd.get_args().collect::<Vec<_>>(),
                [format!("{file_path}:42").as_str()]
            );
            Ok(())
        },
    );
}

#[test]
fn test_sublime_command_line_and_col_numbers() {
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=/snap/bin/subl %f
    "#;
    with_files(
        "test_sublime_command_line_and_col_numbers",
        data,
        |desktop, content| {
            let metadata = EditorMetadata::try_new(desktop)?;
            let file_path = content.display().to_string();
            let result = metadata.build_sublime_command(
                &content,
                Some(LineAndColumnArg {
                    line_num: 42,
                    column_num: Some(25),
                }),
            );

            assert!(result.is_ok());

            let cmd = result.unwrap();
            assert_eq!(cmd.get_program(), "/snap/bin/subl");
            assert_eq!(
                cmd.get_args().collect::<Vec<_>>(),
                [format!("{file_path}:42:25").as_str()]
            );
            Ok(())
        },
    );
}

// ---------- Injection prevention ----------

#[test]
fn test_file_path_with_shell_metacharacters_is_single_arg() {
    // Verify that shell metacharacters in file paths are treated as literal
    // characters, not interpreted by a shell.
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=/usr/bin/editor %f
    "#;

    let malicious_path = PathBuf::from("/tmp/foo; rm -rf /");
    with_files(
        "test_file_path_with_shell_metacharacters",
        data,
        |desktop, _content| {
            let metadata = EditorMetadata::try_new(desktop)?;
            let result = metadata.build_default_command(&malicious_path);

            assert!(result.is_ok());
            let cmd = result.unwrap();
            // The program is the editor, not "sh".
            assert_eq!(cmd.get_program(), "/usr/bin/editor");
            // The malicious path is a single argument, not split by shell.
            assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["/tmp/foo; rm -rf /"]);
            Ok(())
        },
    );
}

#[test]
fn test_file_path_with_spaces_is_single_arg() {
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=/usr/bin/editor %f
    "#;

    let path_with_spaces = PathBuf::from("/home/user/my documents/file.txt");
    with_files("test_file_path_with_spaces", data, |desktop, _content| {
        let metadata = EditorMetadata::try_new(desktop)?;
        let result = metadata.build_default_command(&path_with_spaces);

        assert!(result.is_ok());
        let cmd = result.unwrap();
        assert_eq!(cmd.get_program(), "/usr/bin/editor");
        assert_eq!(
            cmd.get_args().collect::<Vec<_>>(),
            ["/home/user/my documents/file.txt"]
        );
        Ok(())
    });
}

// ---------- Quoted exec string ----------

#[test]
fn test_quoted_executable_path() {
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec="/opt/My App/editor" --flag %f
    "#;
    with_files("test_quoted_executable_path", data, |desktop, content| {
        let metadata = EditorMetadata::try_new(desktop)?;
        let file_path = content.display().to_string();
        let result = metadata.build_default_command(&content);

        assert!(result.is_ok());
        let cmd = result.unwrap();
        assert_eq!(cmd.get_program(), "/opt/My App/editor");
        assert_eq!(
            cmd.get_args().collect::<Vec<_>>(),
            ["--flag", file_path.as_str()]
        );
        Ok(())
    });
}

// ---------- Quoted exec string edge cases ----------

#[test]
fn test_mixed_quoted_and_unquoted_in_single_token() {
    // Adjacent quoted and unquoted text without whitespace forms one token.
    let tokens = tokenize_exec(r#"foo"bar baz"qux"#).unwrap();
    assert_eq!(tokens, vec!["foobar bazqux"]);
}

#[test]
fn test_quoted_field_code_is_still_expanded() {
    // The spec says field codes must not be used inside a quoted argument and
    // the result is undefined. Our implementation expands them anyway since
    // quotes are stripped before field code processing.
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=/usr/bin/editor "%f"
    "#;
    with_files(
        "test_quoted_field_code_is_still_expanded",
        data,
        |desktop, content| {
            let metadata = EditorMetadata::try_new(desktop)?;
            let file_path = content.display().to_string();
            let result = metadata.build_default_command(&content);

            assert!(result.is_ok());
            let cmd = result.unwrap();
            assert_eq!(cmd.get_program(), "/usr/bin/editor");
            assert_eq!(cmd.get_args().collect::<Vec<_>>(), [file_path.as_str()]);
            Ok(())
        },
    );
}

// ---------- Malformed field codes ----------

#[test]
fn test_bare_percent_at_end_errors() {
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=/usr/bin/editor %
    "#;
    with_files(
        "test_bare_percent_at_end_errors",
        data,
        |desktop, content| {
            let metadata = EditorMetadata::try_new(desktop)?;
            let result = metadata.build_default_command(&content);
            assert!(matches!(result, Err(DesktopExecError::MalformedFieldCode)));
            Ok(())
        },
    );
}

// ---------- Field code edge cases ----------

#[test]
fn test_localized_name_with_spaces_is_single_arg() {
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=/usr/bin/app --title %c %f
    Name=My Cool Application
    "#;
    with_files(
        "test_localized_name_with_spaces",
        data,
        |desktop, content| {
            let metadata = EditorMetadata::try_new(desktop)?;
            let file_path = content.display().to_string();
            let result = metadata.build_default_command(&content);

            assert!(result.is_ok());
            let cmd = result.unwrap();
            assert_eq!(cmd.get_program(), "/usr/bin/app");
            // %c expands to a single arg even though the name contains spaces.
            assert_eq!(
                cmd.get_args().collect::<Vec<_>>(),
                ["--title", "My Cool Application", file_path.as_str()]
            );
            Ok(())
        },
    );
}

// ---------- Shell metacharacters in Exec tokens ----------

#[test]
fn test_shell_constructs_in_exec_are_literal() {
    // Subcommand syntax and backticks in the Exec string itself are not
    // interpreted because we execute directly, not via sh -c.
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=/usr/bin/app $(whoami) `id` %f
    "#;
    with_files(
        "test_shell_constructs_in_exec_are_literal",
        data,
        |desktop, content| {
            let metadata = EditorMetadata::try_new(desktop)?;
            let file_path = content.display().to_string();
            let result = metadata.build_default_command(&content);

            assert!(result.is_ok());
            let cmd = result.unwrap();
            assert_eq!(cmd.get_program(), "/usr/bin/app");
            assert_eq!(
                cmd.get_args().collect::<Vec<_>>(),
                ["$(whoami)", "`id`", file_path.as_str()]
            );
            Ok(())
        },
    );
}

// ---------- Deprecated / unknown field codes ----------

#[test]
fn test_deprecated_field_codes_are_dropped() {
    let data = r#"
    [Desktop Entry]
    Version=1.0
    Type=Application
    Exec=/usr/bin/app %d %D %n %N %v %m %f
    "#;
    with_files(
        "test_deprecated_field_codes_are_dropped",
        data,
        |desktop, content| {
            let metadata = EditorMetadata::try_new(desktop)?;
            let file_path = content.display().to_string();
            let result = metadata.build_default_command(&content);

            assert!(result.is_ok());
            let cmd = result.unwrap();
            assert_eq!(cmd.get_program(), "/usr/bin/app");
            // All deprecated codes are silently dropped; only %f remains.
            assert_eq!(cmd.get_args().collect::<Vec<_>>(), [file_path.as_str()]);
            Ok(())
        },
    );
}
