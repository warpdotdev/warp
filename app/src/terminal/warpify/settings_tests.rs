use settings::Setting;
use warpui::{App, SingletonEntity};

use super::WarpifySettings;
use crate::test_util::settings::initialize_settings_for_tests;

#[test]
fn test_parsed_subshell_commands_updated_via_self_subscription() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        app.read(|ctx| {
            assert!(WarpifySettings::as_ref(ctx)
                .parsed_added_subshell_commands
                .is_empty());
        });

        WarpifySettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .added_subshell_commands
                .set_value(vec!["^my-custom-shell$".to_string()], ctx)
                .unwrap();
        });

        // The parsed field must now contain the compiled regex.
        app.read(|ctx| {
            let parsed = &WarpifySettings::as_ref(ctx).parsed_added_subshell_commands;
            assert_eq!(
                parsed.len(),
                1,
                "self-subscription should have updated parsed field"
            );
            let regex = parsed[0].as_ref().expect("regex should compile");
            assert!(
                regex.is_match("my-custom-shell"),
                "compiled regex should match the command pattern"
            );
        });
    });
}

#[cfg(windows)]
#[test]
fn test_wsl_subshell_detection_success() {
    [
        "wsl",
        "wsl.exe",
        "wsl -d Ubuntu",
        "wsl --distribution Ubuntu",
        "wsl -u user",
        "wsl --cd /home/user",
        "wsl --system",
        "wsl --shell-type login",
        "wsl -d Ubuntu --cd /home/user -u username",
        "wsl.exe -d Ubuntu --cd /home/user -u username",
    ]
    .iter()
    .for_each(|cmd| {
        assert!(
            WarpifySettings::is_built_in_subshell_match(cmd),
            "{} failed to match",
            *cmd
        )
    });
}

#[cfg(windows)]
#[test]
fn test_wsl_subshell_detection_fail() {
    [
        "wsl --install",
        "wsl --status",
        "wsl --list",
        "wsl --export Ubuntu file.tar",
        "wsl --uninstall",
        "wsl --shutdown",
        "wslfetch",
        "nowsl",
        "wsl --help",
        "wsl --version",
        "wsl --terminate Ubuntu",
        "wsl --unregister Ubuntu",
        "wsl --update",
        "wsl --import-in-place Ubuntu",
        "wsl --default-user root",
        "wsl --mount \\device",
    ]
    .iter()
    .for_each(|cmd| {
        assert!(
            !WarpifySettings::is_built_in_subshell_match(cmd),
            "{} accidentally matched",
            *cmd
        )
    });
}
