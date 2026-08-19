use chrono::NaiveDate;

use super::*;

#[test]
fn test_parse_version_string() {
    let version_string = "v0.2023.05.15.08.04.stable_01";
    let parsed_version: ParsedVersion = version_string
        .try_into()
        .expect("version string is parsable");
    assert_eq!(parsed_version.major, 0);
    assert_eq!(
        parsed_version.date,
        NaiveDate::from_ymd_opt(2023, 5, 15)
            .unwrap()
            .and_hms_opt(8, 4, 0)
            .unwrap()
    );
    assert_eq!(parsed_version.patch, 1);
}

#[test]
fn test_major_versions_compare_correctly() {
    let older_version: ParsedVersion = "v0.2023.05.15.08.04.stable_01"
        .try_into()
        .expect("older_version is parsable");
    let newer_version: ParsedVersion = "v1.2023.05.15.08.04.stable_01"
        .try_into()
        .expect("newer_version is parsable");
    assert!(newer_version > older_version);
}

#[test]
fn test_dates_compare_correctly() {
    let older_version: ParsedVersion = "v0.2023.05.15.08.04.stable_01"
        .try_into()
        .expect("older_version is parsable");
    let newer_version: ParsedVersion = "v0.2023.05.22.08.04.stable_00"
        .try_into()
        .expect("newer_version is parsable");
    assert!(newer_version > older_version);
}

#[test]
fn test_patches_compare_correctly() {
    let older_version: ParsedVersion = "v0.2023.05.15.08.04.stable_00"
        .try_into()
        .expect("older_version is parsable");
    let newer_version: ParsedVersion = "v0.2023.05.15.08.04.stable_01"
        .try_into()
        .expect("newer_version is parsable");
    assert!(newer_version > older_version);
}

#[test]
fn test_ignores_unknown_channels() {
    // We no longer support or parse-out beta and canary versions, but we
    // need to be able to parse a JSON file that still contains them.
    let channel_version_string = r#"{
        "beta": {
          "version": "v0.2024.01.30.16.52.beta_00"
        },
        "canary": {
          "version": "v0.2022.09.29.08.08.canary_00"
        },
        "dev": {
          "version": "v0.2024.01.30.20.34.dev_00"
        },
        "preview": {
          "version": "v0.2024.01.30.20.34.preview_00"
        },
        "stable": {
          "version": "v0.2024.01.16.16.31.stable_01"
        }
      }"#;

    let channel_versions: ChannelVersions = serde_json::from_str(channel_version_string)
        .expect("Should be able to parse channel versions");
    assert_eq!(
        channel_versions.stable.version_info().version,
        "v0.2024.01.16.16.31.stable_01"
    );
}

#[test]
fn changelog_tui_updates_are_backward_compatible() {
    let old_payload = r#"{
        "date": "2026-07-30T12:00:00+00:00",
        "sections": [],
        "oz_updates": []
    }"#;
    let changelog: Changelog =
        serde_json::from_str(old_payload).expect("old changelog payload should deserialize");
    assert!(changelog.tui_updates.is_empty());

    let new_payload = r#"{
        "date": "2026-07-30T12:00:00+00:00",
        "sections": [],
        "tui_updates": ["Added inline TUI menus"]
    }"#;
    let changelog: Changelog =
        serde_json::from_str(new_payload).expect("new changelog payload should deserialize");
    assert_eq!(changelog.tui_updates, ["Added inline TUI menus"]);
}
