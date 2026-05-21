use chrono::{DateTime, Utc};

use crate::{ChannelVersion, ChannelVersions};

use super::*;

#[test]
fn test_only_first_override_is_applied() {
    #[cfg(target_os = "macos")]
    let predicate = OverridePredicate::TargetOS(TargetOS::MacOS);
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let predicate = OverridePredicate::TargetOS(TargetOS::Linux);
    #[cfg(target_os = "windows")]
    let predicate = OverridePredicate::TargetOS(TargetOS::Windows);

    let version = ChannelVersion {
        version_info: VersionInfo {
            version: "base_version".to_string(),
            update_by: None,
            soft_cutoff: None,
            last_prominent_update: None,
            is_rollback: None,
            version_for_new_users: None,
            cli_version: None,
        },
        overrides: vec![
            VersionOverride {
                predicate: predicate.clone(),
                version_info: VersionInfo {
                    version: "override_version".to_string(),
                    update_by: Some(DateTime::<Utc>::MIN_UTC.fixed_offset()),
                    soft_cutoff: Some("override_cutoff".to_string()),
                    last_prominent_update: None,
                    is_rollback: None,
                    version_for_new_users: None,
                    cli_version: None,
                },
            },
            VersionOverride {
                predicate,
                version_info: VersionInfo {
                    // This should not be applied; as we only apply the first
                    // matching override.
                    version: "second_override_version".to_string(),
                    update_by: None,
                    soft_cutoff: None,
                    last_prominent_update: None,
                    is_rollback: None,
                    version_for_new_users: None,
                    cli_version: None,
                },
            },
        ],
    };

    let version_info = version.version_info.clone();
    let version_info_with_overrides = version.version_info();
    assert_ne!(version_info.version, version_info_with_overrides.version);
    assert_eq!(version_info_with_overrides.version, "override_version");
    assert_ne!(
        version_info.update_by,
        version_info_with_overrides.update_by
    );
    assert_eq!(
        version_info_with_overrides.update_by,
        Some(DateTime::<Utc>::MIN_UTC.fixed_offset())
    );
    assert_ne!(
        version_info.soft_cutoff,
        version_info_with_overrides.soft_cutoff
    );
    assert_eq!(
        version_info_with_overrides.soft_cutoff,
        Some("override_cutoff".to_string())
    );
}

#[test]
fn test_unknown_target_is_ignored() {
    let channel_version_string = r#"{
        "beta": {
          "version": "v0.2024.01.30.16.52.beta_00"
        },
        "canary": {
          "version": "v0.2022.09.29.08.08.canary_00"
        },
        "dev": {
          "soft_cutoff": "v0.2023.05.12.08.03.dev_00",
          "version": "v0.2024.01.30.20.34.dev_00"
        },
        "preview": {
          "version": "v0.2024.01.30.20.34.preview_00"
        },
        "stable": {
          "soft_cutoff": "v0.2023.11.28.08.02.stable_00",
          "version": "v0.2024.01.16.16.31.stable_01",
          "overrides": [
            {
              "predicate": {
                "target_os": "gibberish"
              },
              "version_info": {
                "version": "v0.2024.01.30.16.52.stable_00"
              }
            }
          ]
        }
      }"#;

    // We should still be able to deserialize even if the target OS isn't recognized.
    let channel_versions: ChannelVersions = serde_json::from_str(channel_version_string)
        .expect("Should be able to parse channel versions");
    assert_eq!(
        channel_versions.stable.version_info().version,
        "v0.2024.01.16.16.31.stable_01"
    );

    // The override should have no effect, as the target OS is gibberish.
    let version_with_overrides = channel_versions
        .stable
        .version_info()
        .with_overrides_applied(&channel_versions.stable.overrides, &Context::from_env());
    assert_eq!(
        version_with_overrides.version,
        "v0.2024.01.16.16.31.stable_01"
    );
}

#[test]
fn test_cli_version_override_is_applied() {
    #[cfg(target_os = "macos")]
    let predicate = OverridePredicate::TargetOS(TargetOS::MacOS);
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let predicate = OverridePredicate::TargetOS(TargetOS::Linux);
    #[cfg(target_os = "windows")]
    let predicate = OverridePredicate::TargetOS(TargetOS::Windows);

    let version = ChannelVersion {
        version_info: VersionInfo {
            version: "base_version".to_string(),
            update_by: None,
            soft_cutoff: None,
            last_prominent_update: None,
            is_rollback: None,
            version_for_new_users: None,
            cli_version: Some("base_cli_version".to_string()),
        },
        overrides: vec![VersionOverride {
            predicate,
            version_info: VersionInfo {
                version: "override_version".to_string(),
                update_by: None,
                soft_cutoff: None,
                last_prominent_update: None,
                is_rollback: None,
                version_for_new_users: None,
                cli_version: Some("override_cli_version".to_string()),
            },
        }],
    };

    let version_info_with_overrides = version.version_info();
    assert_eq!(
        version_info_with_overrides.cli_version,
        Some("override_cli_version".to_string())
    );
    assert_eq!(
        version_info_with_overrides.cli_version(),
        "override_cli_version"
    );
}

#[test]
fn test_cli_version_preserved_when_override_omits_it() {
    #[cfg(target_os = "macos")]
    let predicate = OverridePredicate::TargetOS(TargetOS::MacOS);
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let predicate = OverridePredicate::TargetOS(TargetOS::Linux);
    #[cfg(target_os = "windows")]
    let predicate = OverridePredicate::TargetOS(TargetOS::Windows);

    let version = ChannelVersion {
        version_info: VersionInfo {
            version: "base_version".to_string(),
            update_by: None,
            soft_cutoff: None,
            last_prominent_update: None,
            is_rollback: None,
            version_for_new_users: None,
            cli_version: Some("base_cli_version".to_string()),
        },
        overrides: vec![VersionOverride {
            predicate,
            version_info: VersionInfo {
                version: "override_version".to_string(),
                update_by: None,
                soft_cutoff: None,
                last_prominent_update: None,
                is_rollback: None,
                version_for_new_users: None,
                cli_version: None,
            },
        }],
    };

    let version_info_with_overrides = version.version_info();
    // cli_version should be preserved from the base since the override doesn't set it.
    assert_eq!(
        version_info_with_overrides.cli_version,
        Some("base_cli_version".to_string())
    );
    assert_eq!(
        version_info_with_overrides.cli_version(),
        "base_cli_version"
    );
}

#[test]
fn test_cli_version_falls_back_to_version() {
    let info = VersionInfo::new("app_version".to_string());
    assert_eq!(info.cli_version, None);
    assert_eq!(info.cli_version(), "app_version");
}
