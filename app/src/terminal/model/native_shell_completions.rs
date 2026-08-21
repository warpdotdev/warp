use crate::terminal::shell::ShellType;

/// Returns the text to write to the PTY to request native shell completions for `buffer_text`
/// (the portion of the input editor's buffer up to the cursor).
///
/// For zsh, bash, and fish, this is a full generator-command line; the caller runs it as a
/// foreground command (see `PtyController::send_write_to_event_loop`). For PowerShell, this is
/// just the hex-encoded buffer text on its own: the caller instead types it as ordinary
/// characters and immediately triggers a dedicated PSReadLine key handler that reads it back and
/// computes completions directly, never treating it as a command at all -- see
/// `POWERSHELL_NATIVE_COMPLETIONS_TRIGGER`'s doc comment in `pty_controller.rs`.
///
/// In every case the buffer text is hex-encoded so it can be embedded directly without any
/// shell-specific quoting -- a hex string only ever contains `[0-9a-f]` characters, none of which
/// are special to any of the supported shells. Each shell's bootstrap script (or, for PowerShell,
/// the key handler itself) decodes it back to the original bytes before use.
///
/// For the three shells that run this as a command, the invoked function name is chosen so each
/// shell's own bookkeeping recognizes it as a generator command (hidden from history, not treated
/// as a normal foreground command, etc.), matching the naming convention already used by
/// `warp_run_generator_command`:
/// - zsh's `_is_warp_generator_command` and `_warp_zshaddhistory` do a substring match on
///   `warp_run_generator_command`.
/// - bash's `warp_preexec` prefix-matches `warp_run_generator_command*`, as does its
///   `HISTIGNORE` entry (`*warp_run_generator_command*`).
/// - fish's `warp_preexec` prefix-matches `warp_run_generator_command*` (to kill stale generator
///   jobs); the leading space added below is what actually omits it from fish's history file.
pub fn generator_command_for(shell_type: ShellType, buffer_text: &str) -> String {
    let hex_encoded_buffer_text = hex::encode(buffer_text.as_bytes());
    match shell_type {
        // zsh cannot run this through the ordinary (backgrounded) generator command path: it
        // must run in the foreground, in the main shell, with no command substitution around the
        // `select` loop that activates ZLE. See `warp_run_generator_command_native_completions`
        // in zsh_body.sh for the full explanation.
        ShellType::Zsh => {
            format!("warp_run_generator_command_native_completions {hex_encoded_buffer_text}")
        }
        ShellType::Bash => {
            format!("warp_run_generator_command_native_completions {hex_encoded_buffer_text}")
        }
        ShellType::Fish => {
            // Unlike bash (`HISTIGNORE`) and zsh (`hist_ignore_space`/`_warp_zshaddhistory`),
            // fish has no configurable command-pattern history exclusion -- a leading space is
            // the only (default, non-configurable) way to omit a command from its history file.
            // `bytes_to_execute_command`'s bracketed-paste handling already preserves leading
            // whitespace specifically for this reason (see its `ShellType::Fish` case), and
            // `InBandCommandExecutor::execute_command_internal` already does this for the
            // existing `warp_run_generator_command` mechanism -- match that convention here.
            format!(" warp_run_generator_command_native_completions {hex_encoded_buffer_text}")
        }
        // No function call at all: this is typed as ordinary characters and read back by a
        // PSReadLine key handler, never submitted or executed, so there's nothing for any
        // history- or command-exclusion mechanism to need to recognize.
        ShellType::PowerShell => hex_encoded_buffer_text,
    }
}

#[cfg(test)]
#[path = "native_shell_completions_tests.rs"]
mod tests;
