use warpui::assets::asset_cache::AssetSource;
use warpui::App;

use super::*;
use crate::terminal::ssh::util::convert_script_to_one_line;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::ASSETS;

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);
}

/// We write a couple extra bytes at the beginning of a script to clear the line, so we limit the
/// script to 1020 bytes here.
fn assert_script_is_short_enough_mac(script: &str, script_name: &str, convert_to_one_line: bool) {
    let script = if convert_to_one_line {
        convert_script_to_one_line(script)
    } else {
        script.to_string()
    };
    assert!(
        script.len() <= 1020,
        "{} script too long: {} bytes",
        script_name,
        script.len()
    );
}

fn get_script(asset_source: AssetSource, ctx: &AppContext) -> String {
    match AssetCache::as_ref(ctx).load_asset::<String>(asset_source) {
        AssetState::Loaded { data } => data.to_string(),
        _ => panic!("install tmux script should be available as a string"),
    }
}

#[cfg_attr(windows, ignore = "TODO(CORE-3626)")]
#[test]
/// See [assert_script_is_short_enough_mac] for more information.
fn test_mac_warpification_script_size() {
    App::test(ASSETS, |mut app| async move {
        initialize_app(&mut app);

        app.read(|ctx| {
            assert_script_is_short_enough_mac(
                &begin_warpify_ssh_session_command(ctx, SessionId::from(u64::MAX)),
                "unknown_init_subshell.sh",
                false,
            );

            assert_script_is_short_enough_mac(
                &get_script(
                    bundled_asset!("ssh/bash_zsh/install_tmux_and_warpify_brew.sh"),
                    ctx,
                ),
                "install_tmux_and_warpify_brew.sh",
                false,
            );
            assert_script_is_short_enough_mac(
                &get_script(
                    bundled_asset!("ssh/fish/install_tmux_and_warpify_brew.sh"),
                    ctx,
                ),
                "fish/install_tmux_and_warpify_brew.sh",
                false,
            );

            assert_script_is_short_enough_mac(
                &warpify_ssh_session_command("Darwin", ShellType::Zsh, ctx)
                    .expect("Should get Darwin zsh script"),
                "zsh warpify",
                true,
            );
            assert_script_is_short_enough_mac(
                &warpify_ssh_session_command("Darwin", ShellType::Bash, ctx)
                    .expect("Should get Darwin bash script"),
                "bash warpify",
                true,
            );
            assert_script_is_short_enough_mac(
                &warpify_ssh_session_command("Darwin", ShellType::Fish, ctx)
                    .expect("Should get Darwin fish script"),
                "fish warpify",
                true,
            )
        });
    });
}
