use warp_core::ui::appearance::Appearance;
use warpui::{platform::WindowStyle, App, SingletonEntity, ViewHandle};

use std::sync::Arc;

use super::WorkflowModal;
use crate::auth::AuthStateProvider;
use crate::{
    cloud_object::model::persistence::CloudModel,
    editor::PlainTextEditorViewAction as EditorAction,
    server::server_api::team::MockTeamClient,
    server::server_api::workspace::MockWorkspaceClient,
    server::server_api::ServerApiProvider,
    settings_view::keybindings::KeybindingChangedNotifier,
    test_util::settings::initialize_settings_for_tests,
    workflows::workflow::{Argument, Workflow},
    UserWorkspaces,
};

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);

    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());

    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);

    let team_client_mock = Arc::new(MockTeamClient::new());
    let workspace_client_mock = Arc::new(MockWorkspaceClient::new());
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            team_client_mock.clone(),
            workspace_client_mock.clone(),
            vec![],
            ctx,
        )
    });
}

fn create_modal(app: &mut App) -> ViewHandle<WorkflowModal> {
    initialize_app(app);
    let (_, modal_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        let server_api = ServerApiProvider::as_ref(ctx).get();
        WorkflowModal::new(server_api.clone(), ctx)
    });

    modal_view
}

fn build_argument(
    name: impl Into<String>,
    description: impl Into<Option<String>>,
    default_value: impl Into<Option<String>>,
) -> Argument {
    Argument {
        name: name.into(),
        description: description.into(),
        default_value: default_value.into(),
        arg_type: Default::default(),
    }
}

#[test]
fn test_pasting_command_no_argument_overlap_fewer_arguments() {
    App::test((), |mut app| async move {
        let modal_view = create_modal(&mut app);

        modal_view.update(&mut app, |view, ctx| {
            view.content_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text("{{foo_1}} {{foo_2}}", ctx);
            });
        });

        modal_view.read(&app, |view, _| {
            assert_eq!(view.arguments_rows.len(), 2);
        });

        modal_view.update(&mut app, |view, ctx| {
            view.arguments_rows[0]
                .description_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("description for foo_1", ctx);
                });

            view.arguments_rows[0]
                .default_value_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("default value for foo_1", ctx);
                });

            view.arguments_rows[1]
                .description_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("description for foo_2", ctx);
                });

            view.arguments_rows[1]
                .default_value_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("default value for foo_2", ctx);
                });

            view.content_editor.update(ctx, |command_editor, ctx| {
                command_editor.select_all(ctx);
                command_editor.user_initiated_insert("{{bar_1}}", EditorAction::Paste, ctx);
            });
        });

        modal_view.read(&app, |view, app| {
            assert_eq!(view.arguments_rows.len(), 1);

            assert!(view.arguments_rows[0]
                .description_editor
                .as_ref(app)
                .buffer_text(app)
                .is_empty());

            assert!(view.arguments_rows[0]
                .default_value_editor
                .as_ref(app)
                .buffer_text(app)
                .is_empty());
        });
    });
}

#[test]
fn test_pasting_command_no_argument_overlap_more_arguments() {
    App::test((), |mut app| async move {
        let modal_view = create_modal(&mut app);

        modal_view.update(&mut app, |view, ctx| {
            view.content_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text("{{foo_1}}", ctx);
            });
        });

        modal_view.read(&app, |view, _| {
            assert_eq!(view.arguments_rows.len(), 1);
        });

        modal_view.update(&mut app, |view, ctx| {
            view.arguments_rows[0]
                .description_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("description for foo_1", ctx);
                });

            view.arguments_rows[0]
                .default_value_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("default value for foo_1", ctx);
                });

            view.content_editor.update(ctx, |command_editor, ctx| {
                command_editor.select_all(ctx);
                command_editor.user_initiated_insert(
                    "{{bar_1}} {{bar_2}}",
                    EditorAction::Paste,
                    ctx,
                );
            });
        });

        modal_view.read(&app, |view, app| {
            assert_eq!(view.arguments_rows.len(), 2);

            assert!(view.arguments_rows[0]
                .description_editor
                .as_ref(app)
                .buffer_text(app)
                .is_empty());

            assert!(view.arguments_rows[0]
                .default_value_editor
                .as_ref(app)
                .buffer_text(app)
                .is_empty());

            assert!(view.arguments_rows[1]
                .description_editor
                .as_ref(app)
                .buffer_text(app)
                .is_empty());

            assert!(view.arguments_rows[1]
                .default_value_editor
                .as_ref(app)
                .buffer_text(app)
                .is_empty());
        });
    });
}

#[test]
fn test_pasting_command_some_argument_overlap_fewer_arguments() {
    App::test((), |mut app| async move {
        let modal_view = create_modal(&mut app);

        modal_view.update(&mut app, |view, ctx| {
            view.content_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text("{{foo_1}} {{foo_2}} {{foo_3}}", ctx);
            });
        });

        modal_view.read(&app, |view, _| {
            assert_eq!(view.arguments_rows.len(), 3);
        });

        modal_view.update(&mut app, |view, ctx| {
            view.arguments_rows[0]
                .description_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("description for foo_1", ctx);
                });

            view.arguments_rows[0]
                .default_value_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("default value for foo_1", ctx);
                });

            view.arguments_rows[1]
                .description_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("description for foo_2", ctx);
                });

            view.arguments_rows[1]
                .default_value_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("default value for foo_2", ctx);
                });

            view.arguments_rows[2]
                .description_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("description for foo_3", ctx);
                });

            view.arguments_rows[2]
                .default_value_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("default value for foo_3", ctx);
                });

            view.content_editor.update(ctx, |command_editor, ctx| {
                command_editor.select_all(ctx);
                command_editor.user_initiated_insert(
                    "{{foo_3}} {{bar_1}}",
                    EditorAction::Paste,
                    ctx,
                );
            });
        });

        modal_view.read(&app, |view, app| {
            assert_eq!(view.arguments_rows.len(), 2);

            assert_eq!(
                view.arguments_rows[0]
                    .description_editor
                    .as_ref(app)
                    .buffer_text(app)
                    .as_str(),
                "description for foo_3"
            );

            assert_eq!(
                view.arguments_rows[0]
                    .default_value_editor
                    .as_ref(app)
                    .buffer_text(app)
                    .as_str(),
                "default value for foo_3"
            );

            assert!(view.arguments_rows[1]
                .description_editor
                .as_ref(app)
                .buffer_text(app)
                .is_empty());

            assert!(view.arguments_rows[1]
                .default_value_editor
                .as_ref(app)
                .buffer_text(app)
                .is_empty());
        });
    });
}

#[test]
fn test_pasting_command_some_argument_overlap_more_arguments() {
    App::test((), |mut app| async move {
        let modal_view = create_modal(&mut app);

        modal_view.update(&mut app, |view, ctx| {
            view.content_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text("{{foo_1}} {{foo_2}}", ctx);
            });
        });

        modal_view.read(&app, |view, _| {
            assert_eq!(view.arguments_rows.len(), 2);
        });

        modal_view.update(&mut app, |view, ctx| {
            view.arguments_rows[0]
                .description_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("description for foo_1", ctx);
                });

            view.arguments_rows[0]
                .default_value_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("default value for foo_1", ctx);
                });

            view.arguments_rows[1]
                .description_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("description for foo_2", ctx);
                });

            view.arguments_rows[1]
                .default_value_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("default value for foo_2", ctx);
                });

            view.content_editor.update(ctx, |command_editor, ctx| {
                command_editor.select_all(ctx);
                command_editor.user_initiated_insert(
                    "{{bar_1}} {{bar_2}} {{foo_2}} {{bar_3}}",
                    EditorAction::Paste,
                    ctx,
                );
            });
        });

        modal_view.read(&app, |view, app| {
            assert_eq!(view.arguments_rows.len(), 4);

            assert!(view.arguments_rows[0]
                .description_editor
                .as_ref(app)
                .buffer_text(app)
                .is_empty());

            assert!(view.arguments_rows[0]
                .default_value_editor
                .as_ref(app)
                .buffer_text(app)
                .is_empty());

            assert!(view.arguments_rows[1]
                .description_editor
                .as_ref(app)
                .buffer_text(app)
                .is_empty());

            assert!(view.arguments_rows[1]
                .default_value_editor
                .as_ref(app)
                .buffer_text(app)
                .is_empty());

            assert_eq!(
                view.arguments_rows[2]
                    .description_editor
                    .as_ref(app)
                    .buffer_text(app)
                    .as_str(),
                "description for foo_2"
            );

            assert_eq!(
                view.arguments_rows[2]
                    .default_value_editor
                    .as_ref(app)
                    .buffer_text(app)
                    .as_str(),
                "default value for foo_2"
            );

            assert!(view.arguments_rows[3]
                .description_editor
                .as_ref(app)
                .buffer_text(app)
                .is_empty());

            assert!(view.arguments_rows[3]
                .default_value_editor
                .as_ref(app)
                .buffer_text(app)
                .is_empty());
        });
    });
}

#[test]
fn test_pasting_command_same_number_of_arguments() {
    App::test((), |mut app| async move {
        let modal_view = create_modal(&mut app);

        modal_view.update(&mut app, |view, ctx| {
            view.content_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text("{{foo_1}} {{foo_2}}", ctx);
            });
        });

        modal_view.read(&app, |view, _| {
            assert_eq!(view.arguments_rows.len(), 2);
        });

        modal_view.update(&mut app, |view, ctx| {
            view.arguments_rows[0]
                .description_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("description for foo_1", ctx);
                });

            view.arguments_rows[0]
                .default_value_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("default value for foo_1", ctx);
                });

            view.arguments_rows[1]
                .description_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("description for foo_2", ctx);
                });

            view.arguments_rows[1]
                .default_value_editor
                .update(ctx, |editor, ctx| {
                    editor.set_buffer_text("default value for foo_2", ctx);
                });

            view.content_editor.update(ctx, |command_editor, ctx| {
                command_editor.select_all(ctx);
                command_editor.user_initiated_insert(
                    "{{bar_1}} {{bar_2}}",
                    EditorAction::Paste,
                    ctx,
                );
            });
        });

        modal_view.read(&app, |view, app| {
            assert_eq!(view.arguments_rows.len(), 2);

            // if we have the same # of args before/after, it's a coin toss as to whether the args
            // are semantically the same or not. err on the side of not blowing away the associated
            // descriptions / default values.
            assert_eq!(
                view.arguments_rows[0]
                    .description_editor
                    .as_ref(app)
                    .buffer_text(app)
                    .as_str(),
                "description for foo_1"
            );

            assert_eq!(
                view.arguments_rows[0]
                    .default_value_editor
                    .as_ref(app)
                    .buffer_text(app)
                    .as_str(),
                "default value for foo_1"
            );

            assert_eq!(
                view.arguments_rows[1]
                    .description_editor
                    .as_ref(app)
                    .buffer_text(app)
                    .as_str(),
                "description for foo_2"
            );

            assert_eq!(
                view.arguments_rows[1]
                    .default_value_editor
                    .as_ref(app)
                    .buffer_text(app)
                    .as_str(),
                "default value for foo_2"
            );
        });
    });
}

#[test]
fn test_populating_missing_fields_with_suggestion() {
    App::test((), |mut app| async move {
        let modal_view = create_modal(&mut app);

        modal_view.update(&mut app, |view, ctx| {
            view.content_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text("git {{foo_1}} {{foo_2}}", ctx);
            });

            view.title_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text("Title foo", ctx);
            });
        });

        modal_view.read(&app, |view, _| {
            assert_eq!(view.arguments_rows.len(), 2);
        });

        modal_view.update(&mut app, |view, ctx| {
            let workflow = Workflow::Command {
                name: "New Title".to_string(),
                description: Some("New description".to_string()),
                command: "git foo_1 foo_2".to_string(),
                arguments: vec![],
                tags: vec![],
                source_url: None,
                author: None,
                author_url: None,
                shells: vec![],
                environment_variables: None,
            };
            view.populate_missing_field_with_suggestion(workflow, ctx)
        });

        modal_view.read(&app, |view, app| {
            assert_eq!(view.arguments_rows.len(), 2);

            assert_eq!(
                view.content_editor.as_ref(app).buffer_text(app).as_str(),
                "git {{foo_1}} {{foo_2}}"
            );

            assert_eq!(
                view.title_editor.as_ref(app).buffer_text(app).as_str(),
                "Title foo"
            );

            assert_eq!(
                view.description_editor
                    .as_ref(app)
                    .buffer_text(app)
                    .as_str(),
                "New description"
            );
        });
    });
}

#[test]
fn test_populating_with_sanitization() {
    App::test((), |mut app| async move {
        let modal_view = create_modal(&mut app);

        modal_view.update(&mut app, |view, ctx| {
            view.content_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(
                    "tar -czvf {{9output_(file).tar.gz}} {{input_directory}} {{.file9!_zip}}",
                    ctx,
                );
            });

            view.title_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text("Title foo", ctx);
            });
        });

        modal_view.update(&mut app, |view, ctx| {
            let workflow = Workflow::Command {
                name: "New Title".to_string(),
                description: Some("New description".to_string()),
                command: "tar -czvf {{9output_(file).tar.gz}} {{input_directory}} {{.file9!_zip}}"
                    .to_string(),
                arguments: vec![
                    build_argument("9output_(file).tar.gz", None, None),
                    build_argument("input_directory", None, None),
                    build_argument(".file9!_zip", None, None),
                ],
                tags: vec![],
                source_url: None,
                author: None,
                author_url: None,
                shells: vec![],
                environment_variables: None,
            };
            view.populate(workflow, ctx)
        });

        modal_view.read(&app, |view, app| {
            assert_eq!(view.arguments_rows.len(), 3);

            assert_eq!(
                view.content_editor.as_ref(app).buffer_text(app).as_str(),
                "tar -czvf {{output_file_tar_gz}} {{input_directory}} {{_file9_zip}}"
            );
        });
    });
}
