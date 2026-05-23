use dirs::home_dir;

use super::*;

#[test]
fn test_data_dir_path() {
    let home_dir = home_dir().expect("Should be able to compute home directory");
    // ChannelState, by default, is configured for Channel::Oss.
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            assert_eq!(data_dir(), home_dir.join(".black-oss"));
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            assert_eq!(data_dir(), home_dir.join(".local/share/black-oss"));
        } else if #[cfg(windows)] {
            assert_eq!(data_dir(), home_dir.join("AppData\\\\Roaming\\\\blackdagger\\\\Black-Oss\\\\data"));
        } else {
            unimplemented!("Need to update tests for current platform!");
        }
    }
}

#[test]
fn test_config_local_dir_path() {
    let home_dir = home_dir().expect("Should be able to compute home directory");
    // ChannelState, by default, is configured for Channel::Oss.
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            assert_eq!(config_local_dir(), home_dir.join(".black-oss"));
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            assert_eq!(config_local_dir(), home_dir.join(".config/black-oss"));
        } else if #[cfg(windows)] {
            assert_eq!(config_local_dir(), home_dir.join("AppData\\Local\\blackdagger\\Black-Oss\\config"));
        } else {
            unimplemented!("Need to update tests for current platform!");
        }
    }
}

#[test]
fn test_warp_home_config_dir_path() {
    let home_dir = home_dir().expect("Should be able to compute home directory");
    let expected_dir_name = match ChannelState::data_profile() {
        Some(data_profile) => format!(".black-oss-{data_profile}"),
        None => ".black-oss".to_string(),
    };

    assert_eq!(
        warp_home_config_dir(),
        Some(home_dir.join(expected_dir_name))
    );
}

#[test]
fn test_warp_home_skills_and_mcp_paths() {
    let Some(config_dir) = warp_home_config_dir() else {
        panic!("Should be able to compute Black home config directory");
    };

    assert_eq!(warp_home_skills_dir(), Some(config_dir.join("skills")));
    assert_eq!(
        warp_home_mcp_config_file_path(),
        Some(config_dir.join(".mcp.json"))
    );
}
#[test]
fn test_cache_dir_path() {
    let home_dir = home_dir().expect("Should be able to compute home directory");
    // ChannelState, by default, is configured for Channel::Oss.
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            assert_eq!(cache_dir(), home_dir.join("Library/Application Support/io.blackdagger.Black"));
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            assert_eq!(cache_dir(), home_dir.join(".cache/black-oss"));
        } else if #[cfg(windows)] {
            assert_eq!(cache_dir(), home_dir.join("AppData\\Local\\blackdagger\\Black-Oss\\cache"));
        } else {
            unimplemented!("Need to update tests for current platform!");
        }
    }
}

#[test]
fn test_state_dir_path() {
    let home_dir = home_dir().expect("Should be able to compute home directory");
    cfg_if::cfg_if! {
        // ChannelState, by default, is configured for Channel::Oss.
        if #[cfg(target_os = "macos")] {
            assert_eq!(state_dir(), home_dir.join("Library/Application Support/io.blackdagger.Black"));
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            assert_eq!(state_dir(), home_dir.join(".local/state/black-oss"));
        } else if #[cfg(windows)] {
            assert_eq!(state_dir(), home_dir.join("AppData\\Local\\blackdagger\\Black-Oss\\data"));
        } else {
            unimplemented!("Need to update tests for current platform!");
        }
    }
}

#[test]
fn test_project_path_for_warp_app_id() {
    let project_dirs = project_dirs_for_app_id(AppId::new("io", "blackdagger", "Black"), None)
        .expect("should be able to compute project dirs");
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            assert_eq!(project_dirs.project_path(), "io.blackdagger.Black");
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            assert_eq!(project_dirs.project_path(), "black-terminal");
        } else if #[cfg(windows)] {
            assert_eq!(project_dirs.project_path(), "blackdagger\\Black");
        } else {
            unimplemented!("Need to update tests for current platform!");
        }
    }
}

#[test]
fn test_project_path_for_warp_dev_app_id() {
    let project_dirs = project_dirs_for_app_id(AppId::new("io", "blackdagger", "BlackDev"), None)
        .expect("should be able to compute project dirs");
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            assert_eq!(project_dirs.project_path(), "io.blackdagger.BlackDev");
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            assert_eq!(project_dirs.project_path(), "black-terminal-dev");
        } else if #[cfg(windows)] {
            assert_eq!(project_dirs.project_path(), "blackdagger\\BlackDev");
        } else {
            unimplemented!("Need to update tests for current platform!");
        }
    }
}

#[test]
fn test_project_path_for_oss_app_id() {
    let project_dirs = project_dirs_for_app_id(AppId::new("io", "blackdagger", "Black"), None)
        .expect("should be able to compute project dirs");
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            assert_eq!(project_dirs.project_path(), "io.blackdagger.Black");
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            assert_eq!(project_dirs.project_path(), "black-oss");
        } else if #[cfg(windows)] {
            assert_eq!(project_dirs.project_path(), "blackdagger\\Black");
        } else {
            unimplemented!("Need to update tests for current platform!");
        }
    }
}
