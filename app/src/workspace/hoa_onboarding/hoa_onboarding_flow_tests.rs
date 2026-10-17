use settings::Setting;
use warpui::App;
use warpui::platform::WindowStyle;

use super::*;
use crate::auth::AuthStateProvider;
use crate::features::FeatureFlag;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspace::header_toolbar_item::HeaderToolbarItemKind;
use crate::workspace::tab_settings::HeaderToolbarChipSelection;

#[test]
fn onboarding_skips_inbox_when_default_toolbar_hides_it() {
    App::test((), |mut app| async move {
        let _notifications = FeatureFlag::HOANotifications.override_enabled(true);
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(|_| Appearance::mock());
        let (_, flow) = app.add_window(WindowStyle::NotStealFocus, HoaOnboardingFlow::new);

        flow.update(&mut app, |flow, ctx| {
            flow.advance(ctx);
            flow.advance(ctx);
            assert_eq!(flow.step(), HoaOnboardingStep::TabConfig);
            assert_eq!(flow.total_dots(), 2);
        });
    });
}

#[test]
fn onboarding_skips_inbox_when_notifications_are_hidden() {
    App::test((), |mut app| async move {
        let _notifications = FeatureFlag::HOANotifications.override_enabled(true);
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(|_| Appearance::mock());
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .header_toolbar_chip_selection
                    .set_value(
                        HeaderToolbarChipSelection::Custom {
                            left: vec![],
                            right: vec![HeaderToolbarItemKind::NotificationsMailbox],
                        },
                        ctx,
                    )
                    .expect("customize toolbar");
            });
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .show_agent_notifications
                    .set_value(false, ctx)
                    .expect("hide notifications");
            });
        });
        let (_, flow) = app.add_window(WindowStyle::NotStealFocus, HoaOnboardingFlow::new);

        flow.update(&mut app, |flow, ctx| {
            flow.advance(ctx);
            flow.advance(ctx);
            assert_eq!(flow.step(), HoaOnboardingStep::TabConfig);
            assert_eq!(flow.total_dots(), 2);
        });
    });
}

#[test]
fn onboarding_shows_inbox_when_custom_toolbar_contains_it() {
    App::test((), |mut app| async move {
        let _notifications = FeatureFlag::HOANotifications.override_enabled(true);
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(|_| Appearance::mock());
        app.update(|ctx| {
            TabSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .header_toolbar_chip_selection
                    .set_value(
                        HeaderToolbarChipSelection::Custom {
                            left: vec![],
                            right: vec![HeaderToolbarItemKind::NotificationsMailbox],
                        },
                        ctx,
                    )
                    .expect("customize toolbar");
            });
        });
        let (_, flow) = app.add_window(WindowStyle::NotStealFocus, HoaOnboardingFlow::new);

        flow.update(&mut app, |flow, ctx| {
            flow.advance(ctx);
            flow.advance(ctx);
            assert_eq!(flow.step(), HoaOnboardingStep::AgentInboxCallout);
            assert_eq!(flow.total_dots(), 3);
        });
    });
}
