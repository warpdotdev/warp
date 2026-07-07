use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::{App, ViewHandle};

use crate::auth::AuthStateProvider;
use crate::cloud_object::model::actions::ObjectActions;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::model::view::CloudViewModel;
use crate::env_vars::active_env_var_collection_data::SavingStatus;
use crate::env_vars::view::env_var_collection::EnvVarCollectionView;
use crate::network::NetworkStatus;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspace::ActiveSession;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_profiles::UserProfiles;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);

    let global_resources = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resources));
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| Appearance::mock());

    app.add_singleton_model(UserWorkspaces::default_mock);
    app.add_singleton_model(SyncQueue::mock);
    app.add_singleton_model(TeamTesterStatus::mock);
    app.add_singleton_model(UpdateManager::mock);
    app.add_singleton_model(CloudViewModel::mock);
    app.add_singleton_model(|_| UserProfiles::new(vec![]));
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(|_| ObjectActions::new(Vec::new()));
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());

    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);
}

fn create_env_var_collection_view(app: &mut App) -> ViewHandle<EnvVarCollectionView> {
    initialize_app(app);
    let (_, env_var_collection_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        EnvVarCollectionView::new(ctx)
    });

    env_var_collection_view
}

#[test]
fn test_variable_row_addition_and_removal() {
    App::test((), |mut app| async move {
        let env_var_collection_view = create_env_var_collection_view(&mut app);

        env_var_collection_view.update(&mut app, |view, ctx| {
            view.open_new_env_var_collection(
                crate::cloud_object::Owner::mock_current_user(),
                None,
                ctx,
            );
        });

        // New EVCs should open with a new row
        env_var_collection_view.read(&app, |view, _| {
            assert_eq!(view.variable_rows.len(), 1);
        });

        env_var_collection_view.update(&mut app, |view, ctx| {
            view.add_variable_row(ctx);
        });

        env_var_collection_view.update(&mut app, |view, ctx| {
            view.variable_rows[1]
                .variable_description_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("description for foo_1", ctx);
                });

            view.delete_row(0, ctx);
        });

        env_var_collection_view.read(&app, |view, ctx| {
            assert_eq!(view.variable_rows.len(), 1);
            assert_eq!(
                view.variable_rows[0]
                    .variable_description_editor
                    .as_ref(ctx)
                    .buffer_text(ctx),
                "description for foo_1".to_owned()
            )
        });
    });
}

#[test]
fn test_saving_status() {
    App::test((), |mut app| async move {
        let env_var_collection_view = create_env_var_collection_view(&mut app);

        env_var_collection_view.read(&app, |view, ctx| {
            assert_eq!(
                view.active_env_var_collection_data
                    .as_ref(ctx)
                    .saving_status,
                SavingStatus::Saved
            );
        });

        env_var_collection_view.update(&mut app, |view, ctx| {
            view.add_variable_row(ctx);
        });

        env_var_collection_view.read(&app, |view, ctx| {
            assert_eq!(
                view.active_env_var_collection_data
                    .as_ref(ctx)
                    .saving_status,
                SavingStatus::Unsaved
            );
        });

        env_var_collection_view.update(&mut app, |view, ctx| {
            view.active_env_var_collection_data.update(ctx, |data, _| {
                data.saving_status = SavingStatus::Saved;
            })
        });

        env_var_collection_view.update(&mut app, |view, ctx| {
            view.delete_row(0, ctx);
        });

        env_var_collection_view.read(&app, |view, ctx| {
            assert_eq!(
                view.active_env_var_collection_data
                    .as_ref(ctx)
                    .saving_status,
                SavingStatus::Unsaved
            );
        });
    });
}

#[test]
fn test_should_disable_save() {
    App::test((), |mut app| async move {
        let env_var_collection_view = create_env_var_collection_view(&mut app);

        env_var_collection_view.read(&app, |view, ctx| {
            assert!(view.should_disable_save(ctx));
        });

        env_var_collection_view.update(&mut app, |view, ctx| {
            view.add_variable_row(ctx);
        });

        env_var_collection_view.read(&app, |view, ctx| {
            assert!(view.should_disable_save(ctx));
        });

        env_var_collection_view.update(&mut app, |view, ctx| {
            view.variable_rows[0]
                .variable_name_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("Test", ctx);
                });

            view.variable_rows[0]
                .variable_value_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("Test", ctx);
                })
        });

        env_var_collection_view.read(&app, |view, ctx| {
            assert!(!view.should_disable_save(ctx));
        });

        env_var_collection_view.update(&mut app, |view, ctx| {
            view.variable_rows[0]
                .variable_value_editor
                .update(ctx, |editor, ctx| {
                    editor.clear_buffer(ctx);
                })
        });

        env_var_collection_view.read(&app, |view, ctx| {
            assert!(view.should_disable_save(ctx));
        });
    });
}
