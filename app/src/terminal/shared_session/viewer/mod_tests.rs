use settings::Setting;
use warpui::{App, SingletonEntity};

use crate::{
    terminal::{
        safe_mode_settings::SafeModeSettings, shared_session::SharedSessionStatus, TerminalModel,
    },
    test_util::settings::initialize_settings_for_tests,
};

#[test]
fn test_viewer_secret_obfuscation_disabled() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        app.update(|ctx| {
            SafeModeSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .safe_mode_enabled
                    .set_value(true, ctx)
                    .expect("Can update safe mode setting");
            });
        });

        let mut model = TerminalModel::mock(None, None);
        model.set_shared_session_status(SharedSessionStatus::ActiveViewer {
            role: Default::default(),
        });
        model.simulate_block("echo 1.1.1.1", "");
        for block in model.block_list().blocks() {
            assert_eq!(
                block
                    .prompt_and_command_grid()
                    .grid_handler()
                    .num_secrets_obfuscated(),
                0
            );
        }
    });
}
