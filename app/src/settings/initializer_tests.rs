use std::sync::Arc;

use settings::Setting as _;
use warpui::{App, SingletonEntity};

use super::SettingsInitializer;
use crate::auth::auth_state::AuthState;
use crate::auth::user::{PrincipalType, User};
use crate::settings::InputSettings;
use crate::settings::input::InputBoxType;
use crate::test_util::settings::initialize_settings_for_tests;

#[test]
fn service_account_does_not_receive_first_user_defaults() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let auth_state = Arc::new(AuthState::new_for_test());
        let mut user = User::test();
        user.is_onboarded = false;
        user.principal_type = PrincipalType::ServiceAccount;
        auth_state.set_user(Some(user));
        app.add_singleton_model(|_| SettingsInitializer::new());

        SettingsInitializer::handle(&app).update(&mut app, |initializer, ctx| {
            initializer.handle_user_fetched(auth_state, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                *InputSettings::as_ref(ctx).input_box_type.value(),
                InputBoxType::Classic
            );
        });
    })
}
