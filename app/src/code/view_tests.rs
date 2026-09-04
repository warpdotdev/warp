use lsp::LspManagerModel;
use virtual_fs::{Stub, VirtualFS};
use warp_files::{FileModel, FileModelEvent};
use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity};

use super::{CodeView, TabData};
use crate::ai::skills::{SkillOpenOrigin, SkillReference};
use crate::app_state::{CodePaneSnapShot, CodePaneTabSnapshot, LeafContents};
use crate::code::ImmediateSaveError;
use crate::code::editor_management::CodeSource;
use crate::code::global_buffer_model::GlobalBufferModel;
use crate::code::local_code_editor::LocalCodeEditorEvent;
use crate::editor::InteractionState;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::pane_group::{CodePane, PaneContent};
use crate::terminal::local_shell::LocalShellState;
use crate::test_util::terminal::initialize_app_for_terminal_view;
use crate::vim_registers::VimRegisters;
use crate::workspace::ToastStack;

fn initialize_app(app: &mut App) {
    initialize_app_for_terminal_view(app);
    app.add_singleton_model(|_| LspManagerModel::new());
    app.add_singleton_model(FileModel::new);
    app.add_singleton_model(GlobalBufferModel::new);
    app.add_singleton_model(|_| ToastStack);
    app.add_singleton_model(|_| VimRegisters::new());
    app.add_singleton_model(NotebookKeybindings::new);
    app.add_singleton_model(|_| LocalShellState::NotLoaded);
}

fn snapshots(paths: &[&std::path::Path]) -> Vec<CodePaneTabSnapshot> {
    paths
        .iter()
        .map(|path| CodePaneTabSnapshot {
            path: Some(path.to_path_buf()),
        })
        .collect()
}

fn source(path: &std::path::Path) -> CodeSource {
    CodeSource::Link {
        path: path.to_path_buf(),
        range_start: None,
        range_end: None,
    }
}

#[test]
fn restored_tabs_load_on_activation_and_duplicate_paths_share_a_buffer() {
    VirtualFS::test("lazy_restored_code_tabs", |dirs, mut vfs| {
        vfs.with_files(vec![
            Stub::FileWithContent("first.rs", "fn first() {}\n"),
            Stub::FileWithContent("second.rs", "fn second() {}\n"),
        ]);
        let first = dirs.tests().join("first.rs");
        let second = dirs.tests().join("second.rs");

        App::test((), |mut app| async move {
            initialize_app(&mut app);
            let tabs = snapshots(&[&first, &second, &first]);
            let (_, code_view) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
                CodeView::restore(&tabs, 1, source(&second), ctx)
            });

            code_view.read(&app, |view, _| {
                assert!(view.tab_at(0).unwrap().editor_view().is_none());
                assert!(view.tab_at(1).unwrap().editor_view().is_some());
                assert!(view.tab_at(2).unwrap().editor_view().is_none());
            });

            code_view.update(&mut app, |view, ctx| view.set_active_tab_index(0, ctx));
            let first_file_id = code_view.read(&app, |view, ctx| {
                view.tab_at(0)
                    .and_then(TabData::editor_view)
                    .and_then(|editor| editor.as_ref(ctx).file_id())
                    .unwrap()
            });

            code_view.update(&mut app, |view, ctx| view.set_active_tab_index(2, ctx));
            let duplicate_file_id = code_view.read(&app, |view, ctx| {
                view.tab_at(2)
                    .and_then(TabData::editor_view)
                    .and_then(|editor| editor.as_ref(ctx).file_id())
                    .unwrap()
            });

            assert_eq!(duplicate_file_id, first_file_id);
        });
    });
}

#[test]
fn inactive_file_loaded_does_not_rewrite_active_tab_location() {
    VirtualFS::test("inactive_file_loaded", |dirs, mut vfs| {
        vfs.with_files(vec![
            Stub::FileWithContent("delayed.rs", "fn delayed() {}\n"),
            Stub::FileWithContent("active-editor.rs", "fn active_editor() {}\n"),
            Stub::FileWithContent("active.rs", "fn active() {}\n"),
        ]);
        let delayed = dirs.tests().join("delayed.rs");
        let active_editor = dirs.tests().join("active-editor.rs");
        let active = dirs.tests().join("active.rs");

        App::test((), |mut app| async move {
            initialize_app(&mut app);
            let tabs = snapshots(&[&delayed, &active_editor]);
            let source_path = delayed.clone();
            let (_, code_view) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
                CodeView::restore(&tabs, 0, source(&source_path), ctx)
            });
            let delayed_editor = code_view.read(&app, |view, _| {
                view.tab_at(0).unwrap().editor_view().unwrap().clone()
            });

            code_view.update(&mut app, |view, ctx| {
                view.set_active_tab_index(1, ctx);
                view.tab_group[1].location = Some(active.clone().into());
            });
            delayed_editor.update(&mut app, |_, ctx| {
                ctx.emit(LocalCodeEditorEvent::FileLoaded);
            });

            code_view.read(&app, |view, _| {
                assert_eq!(view.active_tab_index(), 1);
                assert_eq!(
                    view.tab_at(1).unwrap().local_path().as_deref(),
                    Some(active.as_path())
                );
            });
        });
    });
}

#[test]
fn merged_bundled_skill_tabs_remain_selectable_when_loaded_lazily() {
    VirtualFS::test("merged_bundled_skill_tabs", |dirs, mut vfs| {
        vfs.with_files(vec![
            Stub::FileWithContent("destination.rs", "fn destination() {}\n"),
            Stub::FileWithContent("first.md", "# First\n"),
            Stub::FileWithContent("second.md", "# Second\n"),
        ]);
        let destination = dirs.tests().join("destination.rs");
        let first = dirs.tests().join("first.md");
        let second = dirs.tests().join("second.md");

        App::test((), |mut app| async move {
            initialize_app(&mut app);
            let destination_source = source(&destination);
            let (_, destination_view) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
                CodeView::new(destination_source, None, ctx)
            });
            let skill_tabs = snapshots(&[&first, &second]);
            let skill_source = CodeSource::Skill {
                reference: SkillReference::BundledSkillId("test-skill".to_string()),
                location: first.clone().into(),
                origin: SkillOpenOrigin::ReadSkill,
            };
            let (_, skill_view) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
                CodeView::restore(&skill_tabs, 0, skill_source, ctx)
            });

            skill_view.read(&app, |view, ctx| {
                let editor = view.tab_at(0).unwrap().editor_view().unwrap();
                assert_eq!(
                    editor
                        .as_ref(ctx)
                        .editor()
                        .as_ref(ctx)
                        .interaction_state(ctx),
                    InteractionState::Selectable
                );
                assert!(view.tab_at(1).unwrap().editor_view().is_none());
            });

            skill_view.update(&mut app, |skill_view, ctx| {
                destination_view.update(ctx, |destination_view, ctx| {
                    destination_view.merge_tabs(skill_view, ctx);
                });
            });
            destination_view.update(&mut app, |view, ctx| {
                view.set_active_tab_index(2, ctx);
            });

            destination_view.read(&app, |view, ctx| {
                for index in [1, 2] {
                    let editor = view.tab_at(index).unwrap().editor_view().unwrap();
                    assert_eq!(
                        editor
                            .as_ref(ctx)
                            .editor()
                            .as_ref(ctx)
                            .interaction_state(ctx),
                        InteractionState::Selectable
                    );
                }
            });
        });
    });
}

#[test]
fn unloaded_tabs_persist_and_can_close_without_loading() {
    VirtualFS::test("unloaded_code_tab_close", |dirs, mut vfs| {
        vfs.with_files(vec![
            Stub::FileWithContent("active.rs", "fn active() {}\n"),
            Stub::FileWithContent("inactive.rs", "fn inactive() {}\n"),
        ]);
        let active = dirs.tests().join("active.rs");
        let inactive = dirs.tests().join("inactive.rs");

        App::test((), |mut app| async move {
            initialize_app(&mut app);
            let tabs = snapshots(&[&active, &inactive]);
            let (_, code_view) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
                CodeView::restore(&tabs, 0, source(&active), ctx)
            });

            code_view.read(&app, |view, ctx| {
                let inactive_tab = view.tab_at(1).unwrap();
                assert_eq!(
                    inactive_tab.local_path().as_deref(),
                    Some(inactive.as_path())
                );
                assert!(!CodeView::has_unsaved_changes(inactive_tab, ctx));
            });
            let code_pane = app.update(|ctx| CodePane::from_view(code_view.clone(), ctx));
            let LeafContents::Code(CodePaneSnapShot::Local { tabs, .. }) =
                app.read(|ctx| code_pane.snapshot(ctx))
            else {
                panic!("expected a local code pane snapshot");
            };
            assert_eq!(tabs[1].path.as_deref(), Some(inactive.as_path()));

            code_view.update(&mut app, |view, ctx| {
                assert!(matches!(
                    view.save_local(1, None, ctx),
                    crate::code::SaveStatus::Failed(ImmediateSaveError::NoActiveFileTab)
                ));
                view.remove_tab_with_confirmation(1, false, ctx);
            });

            code_view.read(&app, |view, _| {
                assert_eq!(view.tab_count(), 1);
                assert!(view.tab_at(0).unwrap().editor_view().is_some());
            });
        });
    });
}

#[test]
fn activating_a_missing_restored_file_keeps_the_tab_loaded() {
    VirtualFS::test("missing_restored_code_tab", |dirs, mut vfs| {
        vfs.with_files(vec![Stub::FileWithContent("active.rs", "fn active() {}\n")]);
        let active = dirs.tests().join("active.rs");
        let missing = dirs.tests().join("missing.rs");

        App::test((), |mut app| async move {
            initialize_app(&mut app);
            let file_events = {
                let (sender, receiver) = async_channel::unbounded();
                app.update(|ctx| {
                    ctx.subscribe_to_model(&FileModel::handle(ctx), move |_, event, _| {
                        sender
                            .try_send(matches!(event, FileModelEvent::FailedToLoad { .. }))
                            .unwrap();
                    });
                });
                receiver
            };
            let tabs = snapshots(&[&active, &missing]);
            let source_path = active.clone();
            let (_, code_view) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
                CodeView::restore(&tabs, 0, source(&source_path), ctx)
            });
            assert!(!file_events.recv().await.unwrap());

            code_view.update(&mut app, |view, ctx| view.set_active_tab_index(1, ctx));
            assert!(file_events.recv().await.unwrap());

            code_view.read(&app, |view, _| {
                assert_eq!(view.active_tab_index(), 1);
                assert!(view.tab_at(1).unwrap().editor_view().is_some());
                assert_eq!(
                    view.tab_at(1).unwrap().local_path().as_deref(),
                    Some(missing.as_path())
                );
            });
        });
    });
}
