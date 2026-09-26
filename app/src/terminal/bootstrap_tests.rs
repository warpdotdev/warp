use std::collections::{HashMap, HashSet};

use warp_core::session_id::SessionId;

use super::*;
use crate::terminal::ShellLaunchData;
use crate::terminal::model::session::{HostInfo, IsSSHWrapperSession};
use crate::terminal::model::terminal_model::SubshellInitializationInfo;
use crate::terminal::shell::Shell;

fn session_info_for_test(
    launch_data: Option<ShellLaunchData>,
    subshell_info: Option<SubshellInitializationInfo>,
) -> SessionInfo {
    SessionInfo {
        session_id: SessionId::from(1),
        shell: Shell::new(ShellType::Bash, None, None, HashSet::new(), None),
        launch_data,
        histfile: None,
        user: "test-user".to_owned(),
        hostname: "test-host".to_owned(),
        subshell_info,
        path: None,
        environment_variable_names: HashSet::new(),
        aliases: HashMap::new(),
        abbreviations: HashMap::new(),
        function_names: HashSet::new(),
        builtins: HashSet::new(),
        keywords: Vec::new(),
        is_ssh_wrapper_session: IsSSHWrapperSession::No,
        home_dir: None,
        cdpath: None,
        editor: None,
        session_type: BootstrapSessionType::Local,
        host_info: HostInfo::default(),
        wsl_name: None,
        spawning_session_id: None,
    }
}

fn subshell_info_for_command(spawning_command: &str) -> SubshellInitializationInfo {
    SubshellInitializationInfo {
        spawning_command: spawning_command.to_owned(),
        was_triggered_by_rc_file_snippet: false,
        env_var_collection_name: None,
        ssh_connection_info: None,
    }
}

#[test]
fn dev_container_top_level_session_is_container_exec_relayed() {
    let session_info = session_info_for_test(
        Some(ShellLaunchData::DevContainer {
            workspace_folder: "/home/user/project".into(),
            docker_path: "/usr/bin/docker".into(),
            container_id: "abc123".to_owned(),
            remote_user: None,
            remote_workspace_folder: "/workspaces/project".to_owned(),
            sandbox_id: "deadbeef".to_owned(),
            session_id: SessionId::from(1),
        }),
        None,
    );
    assert!(is_container_exec_relayed_session(&session_info));
}

#[test]
fn detected_docker_exec_subshell_is_container_exec_relayed() {
    let session_info = session_info_for_test(
        None,
        Some(subshell_info_for_command(
            "docker exec -it my-container bash",
        )),
    );
    assert!(is_container_exec_relayed_session(&session_info));
}

#[test]
fn plain_local_session_is_not_container_exec_relayed() {
    let session_info = session_info_for_test(
        Some(ShellLaunchData::Executable {
            executable_path: "/bin/bash".into(),
            shell_type: ShellType::Bash,
        }),
        None,
    );
    assert!(!is_container_exec_relayed_session(&session_info));
}
