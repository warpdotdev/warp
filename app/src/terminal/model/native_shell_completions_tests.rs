use super::*;

#[test]
fn hex_encodes_the_buffer_text_argument() {
    let command = generator_command_for(ShellType::Bash, "git ch");
    assert_eq!(
        command,
        "warp_run_generator_command_native_completions 676974206368"
    );
}

#[test]
fn fish_command_has_a_leading_space_to_omit_it_from_history() {
    // A leading space is fish's only (default, non-configurable) mechanism for omitting a
    // command from its history file -- unlike bash's HISTIGNORE or zsh's hist_ignore_space.
    // Without it, every native-completions request would leak into the user's fish history.
    let command = generator_command_for(ShellType::Fish, "git ch");
    assert_eq!(
        command,
        " warp_run_generator_command_native_completions 676974206368"
    );

    // The other three shells have their own, different exclusion mechanisms and must not gain
    // an unnecessary leading space.
    for shell_type in [ShellType::Zsh, ShellType::Bash, ShellType::PowerShell] {
        let command = generator_command_for(shell_type, "git ch");
        assert!(
            !command.starts_with(' '),
            "expected {shell_type:?}'s command not to have a leading space, got {command:?}"
        );
    }
}

#[test]
fn hex_argument_matches_the_contract_the_four_shell_decoders_rely_on() {
    // Characters that would otherwise require shell-specific quoting: single quotes,
    // double quotes, backslashes, a partially-typed unbalanced quote, and non-ASCII text.
    let inputs = [
        "git ch",
        "echo 'hello world'",
        "echo \"hi\\there\"",
        "echo 'unterminated",
        "caf\u{e9}",
        "",
    ];
    for input in inputs {
        // Fish's command has a leading space (see `generator_command_for`'s `ShellType::Fish`
        // case), so trim it before stripping the name prefix.
        let hex = generator_command_for(ShellType::Fish, input)
            .trim_start()
            .strip_prefix("warp_run_generator_command_native_completions ")
            .expect("command has the expected prefix")
            .to_owned();

        // Each shell decoder (zsh/bash's `${hex:$i:2}` slicing, fish's `string sub`, and
        // PowerShell's `Substring`) assumes exactly this shape: lowercase hex digits, no
        // separators, and an even count so every two-character slice is a complete byte.
        // Round-tripping through the `hex` crate wouldn't catch a change to this shape
        // (its decoder is more permissive than any of the four), so decode by hand here,
        // the same way the shell scripts do.
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "expected only lowercase hex digits with no separators, got {hex:?}"
        );
        assert_eq!(
            hex.len() % 2,
            0,
            "expected an even number of hex digits, got {hex:?}"
        );

        let decoded: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex pair"))
            .collect();
        assert_eq!(String::from_utf8(decoded).unwrap(), input);
    }
}

#[test]
fn each_shell_uses_a_generator_command_recognized_name() {
    for (shell_type, expected_prefix) in [
        (
            ShellType::Zsh,
            "warp_run_generator_command_foreground_completions ",
        ),
        (
            ShellType::Bash,
            "warp_run_generator_command_native_completions ",
        ),
        (
            ShellType::Fish,
            "warp_run_generator_command_native_completions ",
        ),
    ] {
        let command = generator_command_for(shell_type, "x");
        // Fish's command has a leading space (see `generator_command_for`'s `ShellType::Fish`
        // case, which omits it from fish's history file), so trim before checking the prefix.
        assert!(
            command.trim_start().starts_with(expected_prefix),
            "expected {command:?} to start with {expected_prefix:?}"
        );
    }
}

#[test]
fn powershell_command_is_just_the_hex_text_with_no_function_call() {
    // PowerShell never calls a named function at all: the caller types this directly as
    // ordinary characters and reads it back via a PSReadLine key handler (see
    // `POWERSHELL_NATIVE_COMPLETIONS_TRIGGER`'s doc comment in `pty_controller.rs`), so unlike
    // the other three shells there's no generator-command name for anything to recognize.
    let command = generator_command_for(ShellType::PowerShell, "git ch");
    assert_eq!(command, "676974206368");
}
