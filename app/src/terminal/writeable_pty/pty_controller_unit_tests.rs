use super::bytes_for_tmux_command;
use crate::terminal::model::tmux::commands::TmuxCommand;

#[test]
fn drops_tmux_command_when_control_mode_is_inactive() {
    let command = TmuxCommand::RunInBackgroundWindow {
        command_id: "test-command".to_string(),
        current_directory_path: None,
        command: "echo hello".to_string(),
        environment_variables: None,
    };

    assert_eq!(bytes_for_tmux_command(command, false), None);
}

#[test]
fn formats_tmux_command_when_control_mode_is_active() {
    let command = TmuxCommand::UpdateClientSize {
        num_rows: 24,
        num_cols: 80,
    };

    assert_eq!(
        bytes_for_tmux_command(command, true),
        Some(b"refresh-client -C 80,24\n".to_vec())
    );
}
