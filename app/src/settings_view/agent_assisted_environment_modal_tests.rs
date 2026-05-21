use super::*;

use crate::server::server_api::ServerApiProvider;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspace::ToastStack;
use ai::index::full_source_code_embedding::manager::CodebaseIndexManager;
use std::path::PathBuf;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{ChildView, Empty};
use warpui::platform::WindowStyle;
use warpui::{App, AppContext, Element, Entity, TypedActionView, View, ViewContext, ViewHandle};

fn init_modal_test_models(app: &mut App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| ToastStack);

    // The modal queries CodebaseIndexManager for locally indexed repos.
    // Register a test instance so `available_indexed_repos(...)` doesn't panic.
    app.add_singleton_model(|ctx| {
        CodebaseIndexManager::new_for_test(ServerApiProvider::as_ref(ctx).get(), ctx)
    });
}

#[derive(Default)]
// Simple view that owns the modal and records its emitted events.
struct ModalHarness {
    modal: Option<ViewHandle<AgentAssistedEnvironmentModal>>,
    events: Vec<AgentAssistedEnvironmentModalEvent>,
}

impl ModalHarness {
    fn new(ctx: &mut ViewContext<Self>) -> Self {
        let modal = ctx.add_typed_action_view(AgentAssistedEnvironmentModal::new);
        ctx.subscribe_to_view(&modal, |me, _, event, ctx| {
            me.events.push(event.clone());
            ctx.notify();
        });

        Self {
            modal: Some(modal),
            events: Vec::new(),
        }
    }

    fn modal(&self) -> ViewHandle<AgentAssistedEnvironmentModal> {
        self.modal
            .clone()
            .expect("ModalHarness.modal should be initialized")
    }
}

impl Entity for ModalHarness {
    type Event = ();
}

impl View for ModalHarness {
    fn ui_name() -> &'static str {
        "AgentAssistedEnvironmentModalTestHarness"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        let Some(modal) = &self.modal else {
            return Empty::new().finish();
        };

        ChildView::new(modal).finish()
    }
}

impl TypedActionView for ModalHarness {
    type Action = ();
}

#[test]
fn test_modal_default_render_is_empty() {
    // When not visible, the modal renders as an Empty element.
    App::test((), |mut app| async move {
        init_modal_test_models(&mut app);

        let (_window_id, harness) = app.add_window(WindowStyle::NotStealFocus, ModalHarness::new);

        harness.read(&app, |harness, ctx| {
            let modal = harness.modal();
            let element = modal.as_ref(ctx).render(ctx);
            let text_content = element.debug_text_content().unwrap_or_default();
            assert!(
                text_content.is_empty(),
                "Expected empty modal render when hidden, got: {}",
                text_content
            );
        });
    })
}

#[test]
fn test_modal_show_renders_expected_copy_with_empty_repos_message() {
    // We validate the copy via the section renderers (selected/available) rather than the full dialog,
    // because the dialog includes icons/buttons that may rely on asset providers in unit tests.
    App::test((), |mut app| async move {
        init_modal_test_models(&mut app);

        let (_window_id, harness) = app.add_window(WindowStyle::NotStealFocus, ModalHarness::new);

        // Show modal, then force it into the non-loading empty state so the content is deterministic.
        harness.update(&mut app, |harness, ctx| {
            harness.events.clear();
            let modal = harness.modal();
            modal.update(ctx, |modal, ctx| {
                modal.show(ctx);
                modal.stop_available_repos_loading();
                modal.available_repos.clear();
            });
        });

        harness.read(&app, |harness, ctx| {
            let appearance = Appearance::as_ref(ctx);
            let modal = harness.modal();
            let modal = modal.as_ref(ctx);

            let selected_section = modal.render_selected_section(appearance);
            let selected_text = selected_section.debug_text_content().unwrap_or_default();
            assert!(
                selected_text.contains("Selected repos"),
                "Expected selected section title in rendered content: {}",
                selected_text
            );
            assert!(
                selected_text.contains("No repos selected yet"),
                "Expected selected empty-state message in rendered content: {}",
                selected_text
            );

            let available_section = modal.render_available_section(appearance);
            let available_text = available_section.debug_text_content().unwrap_or_default();
            assert!(
                available_text.contains("Available indexed repos"),
                "Expected available section title in rendered content: {}",
                available_text
            );
            assert!(
                available_text
                    .contains("No locally indexed repos found yet. Index a repo, then try again."),
                "Expected available empty-state message in rendered content: {}",
                available_text
            );
        });
    })
}

#[test]
fn test_modal_cancel_emits_event() {
    App::test((), |mut app| async move {
        init_modal_test_models(&mut app);

        let (_window_id, harness) = app.add_window(WindowStyle::NotStealFocus, ModalHarness::new);

        harness.update(&mut app, |harness, ctx| {
            harness.events.clear();
            let modal = harness.modal();
            modal.update(ctx, |modal, ctx| {
                modal.handle_action(&AgentAssistedEnvironmentModalAction::Cancel, ctx);
            });
        });

        harness.read(&app, |harness, _ctx| {
            assert!(
                matches!(
                    harness.events.as_slice(),
                    [AgentAssistedEnvironmentModalEvent::Cancelled]
                ),
                "Expected a single Cancelled event"
            );
        });
    })
}

#[test]
fn test_modal_confirm_only_emits_event_when_repos_selected() {
    App::test((), |mut app| async move {
        init_modal_test_models(&mut app);

        let (_window_id, harness) = app.add_window(WindowStyle::NotStealFocus, ModalHarness::new);

        // Confirm with no selection should not emit.
        harness.update(&mut app, |harness, ctx| {
            harness.events.clear();
            let modal = harness.modal();
            modal.update(ctx, |modal, ctx| {
                modal.selected_repo_paths.clear();
                modal.selected_row_mouse_states.clear();
                modal.handle_action(&AgentAssistedEnvironmentModalAction::Confirm, ctx);
            });
        });

        harness.read(&app, |harness, _ctx| {
            assert!(
                harness.events.is_empty(),
                "Did not expect any events when confirming with no selected repos"
            );
        });

        // Confirm with a selected repo should emit Confirmed.
        let selected_repo = PathBuf::from("/tmp/repo-a");
        harness.update(&mut app, |harness, ctx| {
            harness.events.clear();
            let modal = harness.modal();
            modal.update(ctx, |modal, ctx| {
                modal.selected_repo_paths = vec![selected_repo.clone()];
                modal.selected_row_mouse_states = vec![MouseStateHandle::default()];
                modal.handle_action(&AgentAssistedEnvironmentModalAction::Confirm, ctx);
            });
        });

        harness.read(&app, |harness, _ctx| {
            assert!(
                matches!(
                    harness.events.as_slice(),
                    [AgentAssistedEnvironmentModalEvent::Confirmed { repo_paths }]
                        if repo_paths == &vec!["/tmp/repo-a".to_string()]
                ),
                "Expected a single Confirmed event with selected repo path"
            );
        });
    })
}

#[test]
fn test_modal_show_clears_selection() {
    App::test((), |mut app| async move {
        init_modal_test_models(&mut app);

        let (_window_id, harness) = app.add_window(WindowStyle::NotStealFocus, ModalHarness::new);

        harness.update(&mut app, |harness, ctx| {
            let modal = harness.modal();
            modal.update(ctx, |modal, ctx| {
                modal.selected_repo_paths = vec![PathBuf::from("/tmp/should-clear")];
                modal.selected_row_mouse_states = vec![MouseStateHandle::default()];

                modal.show(ctx);

                assert!(modal.selected_repo_paths.is_empty());
                assert!(modal.selected_row_mouse_states.is_empty());
            });
        });
    })
}

#[test]
fn test_modal_directory_picked_adds_repo_and_confirm_emits_event() {
    App::test((), |mut app| async move {
        init_modal_test_models(&mut app);

        let (_window_id, harness) = app.add_window(WindowStyle::NotStealFocus, ModalHarness::new);

        let tmp_dir = tempfile::TempDir::new().expect("TempDir should be creatable");
        git2::Repository::init(tmp_dir.path()).expect("git repo should be init-able");
        let selected_repo = tmp_dir.path().to_path_buf();
        let expected_repo_string = dunce::canonicalize(tmp_dir.path())
            .unwrap_or_else(|_| tmp_dir.path().to_path_buf())
            .to_string_lossy()
            .to_string();

        harness.update(&mut app, |harness, ctx| {
            harness.events.clear();
            let modal = harness.modal();
            modal.update(ctx, |modal, ctx| {
                modal.show(ctx);
                modal.stop_available_repos_loading();

                modal.handle_action(
                    &AgentAssistedEnvironmentModalAction::DirectoryPicked(
                        Ok(selected_repo.clone()),
                    ),
                    ctx,
                );
                modal.handle_action(&AgentAssistedEnvironmentModalAction::Confirm, ctx);
            });
        });

        harness.read(&app, |harness, _ctx| {
            let [AgentAssistedEnvironmentModalEvent::Confirmed { repo_paths }] =
                harness.events.as_slice()
            else {
                panic!(
                    "Expected a single Confirmed event with selected repo path, got: {:?}",
                    harness.events
                );
            };

            assert_eq!(repo_paths.len(), 1);
            let actual = dunce::canonicalize(PathBuf::from(&repo_paths[0]))
                .unwrap_or_else(|_| PathBuf::from(&repo_paths[0]))
                .to_string_lossy()
                .to_string();
            assert_eq!(actual, expected_repo_string);
        });
    })
}

#[test]
fn test_modal_directory_picked_dedupes_paths() {
    App::test((), |mut app| async move {
        init_modal_test_models(&mut app);

        let (_window_id, harness) = app.add_window(WindowStyle::NotStealFocus, ModalHarness::new);

        let tmp_dir = tempfile::TempDir::new().expect("TempDir should be creatable");
        git2::Repository::init(tmp_dir.path()).expect("git repo should be init-able");
        let selected_repo = tmp_dir.path().to_path_buf();

        harness.update(&mut app, |harness, ctx| {
            let modal = harness.modal();
            modal.update(ctx, |modal, ctx| {
                modal.show(ctx);

                modal.handle_action(
                    &AgentAssistedEnvironmentModalAction::DirectoryPicked(
                        Ok(selected_repo.clone()),
                    ),
                    ctx,
                );
                modal.handle_action(
                    &AgentAssistedEnvironmentModalAction::DirectoryPicked(
                        Ok(selected_repo.clone()),
                    ),
                    ctx,
                );

                assert_eq!(modal.selected_repo_paths.len(), 1);
                assert_eq!(modal.selected_row_mouse_states.len(), 1);
            });
        });
    })
}

#[test]
fn test_modal_directory_picked_rejects_non_repos() {
    App::test((), |mut app| async move {
        init_modal_test_models(&mut app);

        let (_window_id, harness) = app.add_window(WindowStyle::NotStealFocus, ModalHarness::new);

        let tmp_dir = tempfile::TempDir::new().expect("TempDir should be creatable");
        let selected_dir = tmp_dir.path().to_path_buf();

        harness.update(&mut app, |harness, ctx| {
            let modal = harness.modal();
            modal.update(ctx, |modal, ctx| {
                modal.show(ctx);

                modal.handle_action(
                    &AgentAssistedEnvironmentModalAction::DirectoryPicked(Ok(selected_dir.clone())),
                    ctx,
                );

                assert!(modal.selected_repo_paths.is_empty());
                assert!(modal.selected_row_mouse_states.is_empty());
            });
        });
    })
}
