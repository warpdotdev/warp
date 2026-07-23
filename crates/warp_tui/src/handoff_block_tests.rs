use warp::settings::AISettings;
use warp::tui_export::{
    AmbientAgentTaskId, BlocklistAIHistoryModel, ImageContext, ParsedSlashCommandInput,
    PendingAttachment, SlashCommandDataSource as _, TuiCloudEnvironment,
    register_tui_session_view_test_singletons, slash_commands,
};
use warp_core::features::FeatureFlag;
use warp_core::settings::Setting as _;
use warp_editor::model::CoreEditorModel;
use warpui::platform::WindowStyle;
use warpui::{
    AddWindowOptions, App, EntityIdMap, SingletonEntity, TuiView as _, TypedActionView as _,
    ViewHandle, WindowInvalidation,
};
use warpui_core::elements::tui::{TuiBufferExt, TuiConstraint, TuiLayoutContext, TuiRect, TuiSize};
use warpui_core::keymap::Keystroke;
use warpui_core::presenter::tui::TuiPresenter;

use super::TuiTerminalSessionView;
use crate::autoupdate::TuiAutoupdater;
use crate::handoff_block::TuiHandoffBlock;
use crate::option_selector::TuiOptionSelectorAction;
use crate::orchestration_model::TuiOrchestrationModel;
use crate::root_view::RootTuiView;
use crate::session_registry::TuiSessions;
use crate::test_fixtures::{add_test_semantic_selection, add_test_terminal_session};

struct Fixture {
    view: ViewHandle<TuiTerminalSessionView>,
    window_id: warpui_core::WindowId,
}

fn fixture(app: &mut App) -> Fixture {
    register_tui_session_view_test_singletons(app);
    add_test_semantic_selection(app);
    app.update(TuiAutoupdater::register);
    app.update(crate::keybindings::init);
    let (window_id, _) = app.update(|ctx| {
        ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| RootTuiView::new(),
        )
    });
    let sessions = app.add_singleton_model(|_| TuiSessions::new_for_test());
    let orchestration = app.update(TuiOrchestrationModel::register);
    app.update(|ctx| TuiSessions::wire_orchestration(&sessions, &orchestration, ctx));
    let (view, manager) = add_test_terminal_session(app, window_id);
    app.update(|ctx| {
        TuiSessions::register_session(&sessions, view.clone(), manager, true, ctx);
    });
    Fixture { view, window_id }
}

fn dispatch(
    app: &mut App,
    window_id: warpui_core::WindowId,
    path: &[warpui_core::EntityId],
    key: &str,
) -> bool {
    app.dispatch_keystroke(
        window_id,
        path,
        &Keystroke::parse(key).expect("valid keystroke"),
        false,
    )
    .expect("keystroke dispatch succeeds")
}

fn submit_handoff(app: &mut App, fixture: &Fixture, text: &str) -> ViewHandle<TuiHandoffBlock> {
    fixture.view.update(app, |view, ctx| {
        view.input_view.update(ctx, |input, ctx| {
            input.set_text(text, ctx);
        });
        ctx.focus(&view.input_view);
    });
    let input_id = fixture.view.read(app, |view, _| view.input_view.id());
    assert!(dispatch(
        app,
        fixture.window_id,
        &[fixture.view.id(), input_id],
        "enter",
    ));
    fixture.view.read(app, |view, ctx| {
        view.blocking_interaction_model
            .as_ref(ctx)
            .handoff_for_test()
            .expect("handoff card is installed")
    })
}

fn input_text(app: &App, fixture: &Fixture) -> String {
    fixture.view.read(app, |view, ctx| {
        view.input_view
            .as_ref(ctx)
            .model()
            .as_ref(ctx)
            .content()
            .as_ref(ctx)
            .text()
            .into_string()
    })
}

fn render_session(app: &mut App, fixture: &Fixture) -> Vec<String> {
    let mut presenter = TuiPresenter::new();
    app.update(|ctx| {
        let mut invalidation = WindowInvalidation::default();
        invalidation.updated.insert(fixture.view.id());
        let session = fixture.view.as_ref(ctx);
        invalidation.updated.extend(session.child_view_ids(ctx));
        invalidation
            .updated
            .extend(session.transcript.as_ref(ctx).child_view_ids(ctx));
        if let Some(handoff) = session
            .blocking_interaction_model
            .as_ref(ctx)
            .handoff_for_test()
        {
            invalidation
                .updated
                .extend(handoff.as_ref(ctx).child_view_ids(ctx));
            let selector = handoff.as_ref(ctx).selector_for_test();
            invalidation
                .updated
                .extend(selector.as_ref(ctx).child_view_ids(ctx));
        }
        presenter.invalidate(&invalidation, ctx, fixture.window_id);
        presenter
            .present(ctx, &fixture.view, TuiRect::new(0, 0, 100, 40))
            .buffer
            .to_lines()
    })
}

#[test]
fn slash_menu_selection_inserts_handoff_for_optional_prompt_composition() {
    let _oz_handoff = FeatureFlag::OzHandoff.override_enabled(true);
    let _local_cloud = FeatureFlag::HandoffLocalCloud.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = fixture(&mut app);
        fixture.view.update(&mut app, |view, ctx| {
            view.select_tui_slash_command(&slash_commands::MOVE_TO_CLOUD, ctx);
        });
        assert_eq!(input_text(&app, &fixture), "/handoff ");
    });
}

#[test]
fn fresh_created_card_omits_continue_and_n_starts_a_new_conversation() {
    let _oz_handoff = FeatureFlag::OzHandoff.override_enabled(true);
    let _local_cloud = FeatureFlag::HandoffLocalCloud.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = fixture(&mut app);
        let handoff = submit_handoff(&mut app, &fixture, "/handoff launch");
        handoff.update(&mut app, |block, ctx| {
            block.set_created_for_test(
                "https://app.warp.dev/agent/fresh-run".to_owned(),
                false,
                ctx,
            );
        });

        let lines = render_session(&mut app, &fixture).join("\n");
        assert!(lines.contains("Enter open cloud run"), "{lines}");
        assert!(!lines.contains("C continue locally"), "{lines}");
        assert!(lines.contains("N new conversation"), "{lines}");
        assert!(dispatch(
            &mut app,
            fixture.window_id,
            &[fixture.view.id(), handoff.id()],
            "n",
        ));
        fixture.view.read(&app, |view, ctx| {
            assert!(
                view.blocking_interaction_model
                    .as_ref(ctx)
                    .handoff_for_test()
                    .is_none()
            );
            assert!(
                view.terminal_model
                    .lock()
                    .block_list()
                    .rich_content_row_range(handoff.id())
                    .is_none()
            );
        });
    });
}

#[test]
fn local_conversation_with_task_id_remains_handoff_eligible() {
    let _oz_handoff = FeatureFlag::OzHandoff.override_enabled(true);
    let _local_cloud = FeatureFlag::HandoffLocalCloud.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = fixture(&mut app);
        let conversation_id = fixture.view.read(&app, |view, ctx| {
            view.conversation_selection
                .as_ref(ctx)
                .selected_conversation_id(ctx)
                .expect("fixture eagerly selects a local conversation")
        });
        let task_id: AmbientAgentTaskId = "00000000-0000-0000-0000-000000000001"
            .parse()
            .expect("valid task id");
        BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, _| {
            history
                .conversation_mut(&conversation_id)
                .expect("selected conversation exists")
                .set_task_id(task_id);
        });

        fixture.view.read(&app, |view, ctx| {
            assert!(
                !view
                    .slash_commands_source
                    .as_ref(ctx)
                    .conversation_is_cloud_agent_run(conversation_id, ctx),
                "a local conversation remains local even when it has a task id"
            );
        });
        let handoff = submit_handoff(&mut app, &fixture, "/handoff keep going");
        assert!(handoff.read(&app, |block, _| block.is_active()));
    });
}

#[test]
fn no_environment_card_has_top_padding_and_ctrl_c_restores_prompt_and_images() {
    let _oz_handoff = FeatureFlag::OzHandoff.override_enabled(true);
    let _local_cloud = FeatureFlag::HandoffLocalCloud.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = fixture(&mut app);
        fixture.view.update(&mut app, |view, ctx| {
            view.ai_context_model.update(ctx, |context, ctx| {
                context.append_pending_attachments(
                    vec![PendingAttachment::Image(ImageContext {
                        data: "aW1hZ2U=".to_owned(),
                        mime_type: "image/png".to_owned(),
                        file_name: "context.png".to_owned(),
                        is_figma: false,
                    })],
                    ctx,
                );
            });
        });
        let handoff = submit_handoff(&mut app, &fixture, "/handoff finish the task");

        assert_eq!(input_text(&app, &fixture), "");
        fixture.view.read(&app, |view, ctx| {
            assert!(
                view.ai_context_model
                    .as_ref(ctx)
                    .pending_attachments()
                    .is_empty()
            );
        });
        let lines = render_session(&mut app, &fixture).join("\n");
        assert!(lines.contains("Hand off to cloud"), "{lines}");
        assert!(lines.contains("A cloud environment is required"), "{lines}");
        assert!(lines.contains("Enter open setup guide"), "{lines}");
        assert!(!lines.contains("finish the task"), "{lines}");
        let title_row = lines
            .lines()
            .position(|line| line.contains("Hand off to cloud"))
            .expect("handoff title renders");
        assert!(
            lines
                .lines()
                .nth(title_row.saturating_sub(1))
                .is_some_and(|line| line.trim().is_empty()),
            "the handoff card has a blank row above it:\n{lines}"
        );

        assert!(dispatch(
            &mut app,
            fixture.window_id,
            &[fixture.view.id(), handoff.id()],
            "ctrl-c",
        ));
        assert_eq!(input_text(&app, &fixture), "finish the task");
        fixture.view.read(&app, |view, ctx| {
            assert_eq!(
                view.ai_context_model
                    .as_ref(ctx)
                    .pending_attachments()
                    .len(),
                1
            );
            assert!(
                view.terminal_model
                    .lock()
                    .block_list()
                    .rich_content_row_range(handoff.id())
                    .is_none()
            );
            assert!(
                view.blocking_interaction_model
                    .as_ref(ctx)
                    .handoff_for_test()
                    .is_none()
            );
        });
    });
}

#[test]
fn environment_projection_transitions_the_same_card_and_selector_dispatches_real_keys() {
    let _oz_handoff = FeatureFlag::OzHandoff.override_enabled(true);
    let _local_cloud = FeatureFlag::HandoffLocalCloud.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = fixture(&mut app);
        let handoff = submit_handoff(&mut app, &fixture, "/handoff build it");
        let environments = handoff.read(&app, |block, _| block.environments_for_test());
        environments.update(&mut app, |projection, ctx| {
            projection.replace_for_test(
                vec![
                    TuiCloudEnvironment::new_for_test(1, "Alpha"),
                    TuiCloudEnvironment::new_for_test(2, "Beta"),
                ],
                ctx,
            );
        });
        assert!(handoff.read(&app, |block, _| block.is_accepting_for_test()));
        let lines = render_session(&mut app, &fixture).join("\n");
        assert!(
            lines.contains("Environment: Select an environment"),
            "{lines}"
        );
        assert!(lines.contains("Model:"), "{lines}");

        assert!(dispatch(
            &mut app,
            fixture.window_id,
            &[fixture.view.id(), handoff.id()],
            "ctrl-e",
        ));
        let selector = handoff.read(&app, |block, _| block.selector_for_test());
        assert!(app.read(|ctx| selector.is_focused(ctx)));
        let search = selector.read(&app, |selector, _| {
            selector
                .search_field_for_test()
                .expect("environment page is searchable")
        });
        let lines = render_session(&mut app, &fixture).join("\n");
        assert!(
            lines.contains("Which environment should run this conversation?"),
            "{lines}"
        );
        selector.update(&mut app, |selector, ctx| {
            selector.handle_action(&TuiOptionSelectorAction::FocusSearchAndInsert('b'), ctx);
        });
        assert!(search.read(&app, |search, _| search.is_focused()));
        let lines = render_session(&mut app, &fixture).join("\n");
        assert!(lines.contains("Beta"), "{lines}");
        assert!(!lines.contains("Alpha"), "{lines}");
        assert!(dispatch(
            &mut app,
            fixture.window_id,
            &[fixture.view.id(), handoff.id(), selector.id(), search.id()],
            "backspace",
        ));
        assert_eq!(search.read(&app, |search, ctx| search.text(ctx)), "");
        assert!(search.read(&app, |search, _| search.is_focused()));
        let lines = render_session(&mut app, &fixture).join("\n");
        assert!(lines.contains("Alpha"), "{lines}");
        assert!(lines.contains("Beta"), "{lines}");
        selector.update(&mut app, |selector, ctx| {
            selector.handle_action(&TuiOptionSelectorAction::FocusSearchAndInsert('b'), ctx);
        });
        selector.update(&mut app, |selector, ctx| {
            selector.handle_action(&TuiOptionSelectorAction::SelectItem(0), ctx);
        });
        let lines = render_session(&mut app, &fixture).join("\n");
        assert!(
            lines.contains("Which model should run this conversation?"),
            "{lines}"
        );
        assert!(lines.contains("2 of 2"), "{lines}");
        assert!(app.read(|ctx| selector.is_focused(ctx)));

        assert!(dispatch(
            &mut app,
            fixture.window_id,
            &[fixture.view.id(), handoff.id(), selector.id()],
            "left",
        ));
        let lines = render_session(&mut app, &fixture).join("\n");
        assert!(
            lines.contains("Which environment should run this conversation?"),
            "{lines}"
        );
        assert!(lines.contains("1 of 2"), "{lines}");

        assert!(dispatch(
            &mut app,
            fixture.window_id,
            &[fixture.view.id(), handoff.id(), selector.id()],
            "escape",
        ));
        let lines = render_session(&mut app, &fixture).join("\n");
        assert!(lines.contains("Environment: Beta"), "{lines}");
        assert!(lines.contains("Ctrl + E to edit"), "{lines}");
    });
}

#[test]
fn incompatible_model_blocks_confirmation_until_a_compatible_model_is_selected() {
    let _oz_handoff = FeatureFlag::OzHandoff.override_enabled(true);
    let _local_cloud = FeatureFlag::HandoffLocalCloud.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = fixture(&mut app);
        let handoff = submit_handoff(&mut app, &fixture, "/handoff build it");
        handoff
            .read(&app, |block, _| block.environments_for_test())
            .update(&mut app, |projection, ctx| {
                projection
                    .replace_for_test(vec![TuiCloudEnvironment::new_for_test(1, "Alpha")], ctx);
            });
        handoff.update(&mut app, |block, ctx| {
            block.select_first_environment_for_test(ctx);
            block.set_model_for_test("custom-router:local:test".to_owned(), ctx);
        });
        let lines = render_session(&mut app, &fixture).join("\n");
        assert!(lines.contains("(incompatible)"), "{lines}");

        assert!(dispatch(
            &mut app,
            fixture.window_id,
            &[fixture.view.id(), handoff.id()],
            "enter",
        ));
        assert!(handoff.read(&app, |block, _| block.is_accepting_for_test()));
        let lines = render_session(&mut app, &fixture).join("\n");
        assert!(lines.contains("cannot run in Oz cloud"), "{lines}");
    });
}

#[test]
fn settings_invalidation_restores_the_draft_and_repeated_submission_keeps_one_card() {
    let _oz_handoff = FeatureFlag::OzHandoff.override_enabled(true);
    let _local_cloud = FeatureFlag::HandoffLocalCloud.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = fixture(&mut app);
        let handoff = submit_handoff(&mut app, &fixture, "/handoff preserve me");
        fixture.view.update(&mut app, |view, ctx| {
            view.execute_tui_slash_command(
                &slash_commands::MOVE_TO_CLOUD,
                Some(&"second".to_owned()),
                ctx,
            );
        });
        fixture.view.read(&app, |view, ctx| {
            assert_eq!(
                view.blocking_interaction_model
                    .as_ref(ctx)
                    .handoff_for_test()
                    .map(|view| view.id()),
                Some(handoff.id())
            );
        });

        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .should_force_disable_cloud_handoff
                .set_value(true, ctx)
                .expect("test setting persists");
        });
        assert_eq!(input_text(&app, &fixture), "preserve me");
        fixture.view.read(&app, |view, ctx| {
            assert!(
                view.blocking_interaction_model
                    .as_ref(ctx)
                    .handoff_for_test()
                    .is_none()
            );
        });
    });
}

#[test]
fn privacy_invalidation_restores_the_draft_and_removes_handoff_from_commands() {
    let _oz_handoff = FeatureFlag::OzHandoff.override_enabled(true);
    let _local_cloud = FeatureFlag::HandoffLocalCloud.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = fixture(&mut app);
        submit_handoff(&mut app, &fixture, "/handoff preserve privacy draft");

        warp::settings::PrivacySettings::handle(&app).update(&mut app, |privacy_settings, ctx| {
            privacy_settings.is_cloud_conversation_storage_enabled = false;
            ctx.emit(
                warp::settings::PrivacySettingsChangedEvent::UpdateIsCloudConversationStorageEnabled {
                    old_value: true,
                    new_value: false,
                },
            );
        });

        assert_eq!(input_text(&app, &fixture), "preserve privacy draft");
        fixture.view.read(&app, |view, ctx| {
            assert!(
                view.blocking_interaction_model
                    .as_ref(ctx)
                    .handoff_for_test()
                    .is_none()
            );
            assert!(!matches!(
                view.slash_commands_source
                    .as_ref(ctx)
                    .parse_input("/handoff another", ctx),
                ParsedSlashCommandInput::SlashCommand(_)
            ));
        });
    });
}

#[test]
fn missing_token_after_eager_cancellation_restores_only_trimmed_argument() {
    let argument = "  keep this prompt  ".to_owned();
    assert_eq!(
        TuiTerminalSessionView::input_after_handoff_prepare_error(
            &warp::tui_export::HandoffPrepareError::MissingServerConversationToken,
            true,
            Some(&argument),
        )
        .as_deref(),
        Some("keep this prompt")
    );
    assert_eq!(
        TuiTerminalSessionView::input_after_handoff_prepare_error(
            &warp::tui_export::HandoffPrepareError::MissingServerConversationToken,
            true,
            None,
        )
        .as_deref(),
        Some("")
    );
    assert!(
        TuiTerminalSessionView::input_after_handoff_prepare_error(
            &warp::tui_export::HandoffPrepareError::LongRunningCommand,
            true,
            Some(&argument),
        )
        .is_none(),
        "pre-cancellation guard failures keep the full slash command draft"
    );
    assert!(
        TuiTerminalSessionView::input_after_handoff_prepare_error(
            &warp::tui_export::HandoffPrepareError::MissingServerConversationToken,
            false,
            Some(&argument),
        )
        .is_none(),
        "idle missing-token failures did not eagerly cancel the source"
    );
}

#[test]
fn committed_ctrl_c_is_consumed_and_created_banner_persists_in_the_transcript() {
    let _oz_handoff = FeatureFlag::OzHandoff.override_enabled(true);
    let _local_cloud = FeatureFlag::HandoffLocalCloud.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = fixture(&mut app);
        let handoff = submit_handoff(&mut app, &fixture, "/handoff launch");
        handoff.update(&mut app, |block, ctx| {
            block.set_committed_for_test(ctx);
        });
        assert!(dispatch(
            &mut app,
            fixture.window_id,
            &[fixture.view.id(), handoff.id()],
            "ctrl-c",
        ));
        fixture.view.read(&app, |view, ctx| {
            assert!(
                view.blocking_interaction_model
                    .as_ref(ctx)
                    .handoff_for_test()
                    .is_some()
            );
        });

        handoff.update(&mut app, |block, ctx| {
            block.set_created_for_test("https://app.warp.dev/agent/test-run".to_owned(), true, ctx);
        });
        fixture.view.read(&app, |view, ctx| {
            assert!(
                view.blocking_interaction_model
                    .as_ref(ctx)
                    .handoff_for_test()
                    .is_some()
            );
        });
        let lines = render_session(&mut app, &fixture).join("\n");
        assert!(lines.contains("Cloud run created."), "{lines}");
        assert!(lines.contains("Enter open cloud run"), "{lines}");
        assert!(lines.contains("C continue locally"), "{lines}");
        assert!(lines.contains("N new conversation"), "{lines}");
        assert!(dispatch(
            &mut app,
            fixture.window_id,
            &[fixture.view.id(), handoff.id()],
            "c",
        ));
        fixture.view.read(&app, |view, ctx| {
            assert!(
                view.blocking_interaction_model
                    .as_ref(ctx)
                    .handoff_for_test()
                    .is_none()
            );
        });
        assert_eq!(input_text(&app, &fixture), "");
        handoff.read(&app, |block, ctx| {
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            let mut element = block.render(ctx);
            let size = element.layout(
                TuiConstraint::loose(TuiSize::new(100, u16::MAX)),
                &mut layout_ctx,
                ctx,
            );
            assert_eq!(size.width, 100);
        });
        fixture.view.update(&mut app, |view, ctx| {
            view.input_view.update(ctx, |input, ctx| {
                input.set_text("continue locally", ctx);
            });
        });
        let lines = render_session(&mut app, &fixture).join("\n");
        assert!(
            lines.contains("Conversation forked to cloud; continuing locally"),
            "{lines}"
        );
        let banner_row = lines
            .lines()
            .position(|line| line.contains("Conversation forked to cloud; continuing locally"))
            .expect("persistent handoff banner renders");
        assert!(
            lines
                .lines()
                .nth(banner_row.saturating_sub(1))
                .is_some_and(|line| line.trim().is_empty()),
            "the persistent handoff banner has a blank row above it:\n{lines}"
        );
        assert!(
            lines
                .lines()
                .nth(banner_row + 1)
                .is_some_and(|line| line.contains("https://app.warp.dev/agent/test-run")),
            "the cloud-run link renders on the row below the banner copy:\n{lines}"
        );
        assert!(
            lines.contains("https://app.warp.dev/agent/test-run"),
            "{lines}"
        );
        assert!(lines.contains("continue locally"), "{lines}");
        fixture.view.read(&app, |view, _| {
            assert!(
                view.terminal_model
                    .lock()
                    .block_list()
                    .rich_content_row_range(handoff.id())
                    .is_some()
            );
        });
        fixture.view.update(&mut app, |view, ctx| {
            assert!(view.start_new_conversation(None, ctx));
        });
        fixture.view.read(&app, |view, _| {
            assert!(
                view.terminal_model
                    .lock()
                    .block_list()
                    .rich_content_row_range(handoff.id())
                    .is_none()
            );
        });
        let lines = render_session(&mut app, &fixture).join("\n");
        assert!(!lines.contains("Conversation forked to cloud"), "{lines}");
    });
}

#[test]
fn long_running_command_rejection_preserves_the_full_local_draft() {
    let _oz_handoff = FeatureFlag::OzHandoff.override_enabled(true);
    let _local_cloud = FeatureFlag::HandoffLocalCloud.override_enabled(true);
    App::test((), |mut app| async move {
        let fixture = fixture(&mut app);
        fixture.view.update(&mut app, |view, ctx| {
            view.terminal_model
                .lock()
                .simulate_long_running_block("sleep 30", "");
            view.input_view.update(ctx, |input, ctx| {
                input.set_text("/handoff keep this prompt", ctx);
            });
            view.execute_tui_slash_command(
                &slash_commands::MOVE_TO_CLOUD,
                Some(&"keep this prompt".to_owned()),
                ctx,
            );
        });
        assert_eq!(input_text(&app, &fixture), "/handoff keep this prompt");
        fixture.view.read(&app, |view, ctx| {
            assert!(
                view.blocking_interaction_model
                    .as_ref(ctx)
                    .handoff_for_test()
                    .is_none()
            );
            assert!(
                view.transient_hint
                    .current()
                    .is_some_and(|(message, _)| message.contains("command is running"))
            );
        });
    });
}
