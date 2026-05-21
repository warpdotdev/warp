use crate::features::FeatureFlag;
use async_channel::TryRecvError;
use parking_lot::Mutex;
use std::{path::PathBuf, sync::Arc};
use string_offset::CharOffset;
use tempfile::tempdir;
use warp_editor::{
    content::mermaid_diagram::mermaid_asset_source,
    render::{
        element::RichTextAction,
        model::{
            BlockItem, BlockSpacing, HitTestBlockType, ImageBlockConfig, Location, RenderEvent,
        },
    },
};
use warp_util::user_input::UserInput;
use warpui::assets::asset_cache::{AssetCache, AssetState};

use warpui::event::ModifiersState;
use warpui::image_cache::ImageType;
use warpui::r#async::block_on;
use warpui::units::Pixels;
use warpui::windowing::WindowManager;
use warpui::{platform::WindowStyle, presenter::ChildView, App, Element, Entity, View, ViewHandle};
use warpui::{SingletonEntity, TypedActionView, WindowId};

use super::{EditorViewAction, LayoutAffectingAssetLoad, RichTextEditorConfig, RichTextEditorView};
use crate::appearance::Appearance;
use crate::editor::InteractionState;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::notebooks::editor::link_editor::LinkEditorAction;
use crate::notebooks::editor::model::NotebooksEditorModel;
use crate::notebooks::editor::rich_text_styles;
use crate::notebooks::file::MarkdownDisplayMode;
use crate::notebooks::link::{LinkEvent, NotebookLinks, SessionSource};
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;

use crate::settings::FontSettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;

use crate::auth::AuthStateProvider;
use crate::terminal::keys::TerminalKeybindings;
use crate::terminal::{model::session::Session, shell::ShellType, ShellLaunchData};
use crate::test_util::assert_eventually;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspace::ActiveSession;
use crate::UserWorkspaces;
use crate::{
    cloud_object::model::persistence::CloudModel, search::files::model::FileSearchModel,
    GlobalResourceHandles, GlobalResourceHandlesProvider,
};

/// Container for a [`RichTextEditorView`] in unit tests.
struct TestView {
    editor: ViewHandle<RichTextEditorView>,
}

impl Entity for TestView {
    type Event = ();
}

impl View for TestView {
    fn ui_name() -> &'static str {
        "TestView"
    }

    fn render(&self, _app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        ChildView::new(&self.editor).finish()
    }
}
impl TypedActionView for TestView {
    type Action = ();
}

fn initialize_editor(
    app: &mut App,
) -> (
    WindowId,
    ViewHandle<RichTextEditorView>,
    ViewHandle<TestView>,
) {
    initialize_settings_for_tests(app);

    let global_resources = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resources));
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());
    app.add_singleton_model(|_| repo_metadata::repositories::DetectedRepositories::default());
    #[cfg(feature = "local_fs")]
    app.add_singleton_model(repo_metadata::RepoMetadataModel::new);
    app.add_singleton_model(FileSearchModel::new);
    app.add_singleton_model(NotebookKeybindings::new);
    app.add_singleton_model(TerminalKeybindings::new);
    app.add_singleton_model(CloudModel::mock);
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

    let (window, test_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        let window_id = ctx.window_id();
        let links = ctx.add_model(|ctx| NotebookLinks::new(SessionSource::Active(window_id), ctx));
        let editor_model = ctx.add_model(|ctx| {
            let styles = rich_text_styles(Appearance::as_ref(ctx), FontSettings::as_ref(ctx));
            NotebooksEditorModel::new(styles, window_id, ctx)
        });
        let editor = ctx.add_typed_action_view(|ctx| {
            RichTextEditorView::new(
                String::new(),
                editor_model,
                links,
                RichTextEditorConfig::default(),
                ctx,
            )
        });
        TestView { editor }
    });

    let editor_view = app.read(|ctx| test_view.as_ref(ctx).editor.clone());
    (window, editor_view, test_view)
}

async fn reset_editor_with_markdown(
    app: &mut App,
    editor_view: &ViewHandle<RichTextEditorView>,
    markdown: &str,
) {
    editor_view.update(app, |editor, ctx| {
        editor.reset_with_markdown(markdown, ctx);
        editor.set_interaction_state(InteractionState::Editable, ctx);
    });
    let render_state = editor_view.read(app, |editor, ctx| {
        editor.model.as_ref(ctx).render_state().clone()
    });
    app.read(|ctx| render_state.as_ref(ctx).layout_complete())
        .await;
}

fn link_offset(
    editor: &RichTextEditorView,
    link_url: &str,
    ctx: &warpui::AppContext,
) -> CharOffset {
    let max_offset = editor.markdown(ctx).chars().count();
    (0..=max_offset)
        .map(CharOffset::from)
        .find(|offset| {
            editor
                .model
                .as_ref(ctx)
                .link_url_at(*offset, ctx)
                .as_deref()
                == Some(link_url)
        })
        .expect("Expected link URL to exist in editor")
}

fn rendered_mermaid_block_range(
    editor: &RichTextEditorView,
    ctx: &warpui::AppContext,
) -> Option<std::ops::Range<CharOffset>> {
    let render_state = editor.model.as_ref(ctx).render_state().clone();
    let render_state = render_state.as_ref(ctx);
    let content = render_state.content();
    let mut block_start = CharOffset::zero();

    for block in content.block_items() {
        let block_end = block_start + block.content_length();
        if matches!(
            block,
            warp_editor::render::model::BlockItem::MermaidDiagram { .. }
        ) {
            return Some(block_start..block_end);
        }
        block_start = block_end;
    }

    None
}

#[test]
fn test_loaded_mermaid_diagram_with_placeholder_height_needs_relayout() {
    App::test((), |app| async move {
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        let contents = "graph TD\nA[Start] --> B[Finish]\n";
        let asset_source = mermaid_asset_source(contents);

        let pending = app.read(|ctx| {
            let asset_cache = AssetCache::as_ref(ctx);
            match asset_cache.load_asset::<ImageType>(asset_source.clone()) {
                AssetState::Loading { handle } => handle.when_loaded(asset_cache),
                AssetState::Loaded { .. } => None,
                AssetState::Evicted => panic!("Mermaid asset should not be evicted during test"),
                AssetState::FailedToLoad(err) => {
                    panic!("Mermaid asset should load successfully: {err}")
                }
            }
        });
        if let Some(future) = pending {
            future.await;
        }

        app.read(|ctx| {
            let config = ImageBlockConfig {
                width: Pixels::new(640.),
                height: Pixels::new(120.),
                spacing: BlockSpacing::default(),
            };
            let block = BlockItem::MermaidDiagram {
                content_length: CharOffset::from(contents.chars().count()),
                asset_source,
                config,
            };
            let asset_cache = AssetCache::as_ref(ctx);

            assert!(matches!(
                RichTextEditorView::layout_affecting_asset_load(&block, asset_cache),
                Some(LayoutAffectingAssetLoad::LoadedNeedsRelayout)
            ));
        });
    })
}

#[test]
fn layout_affecting_asset_loads_rebuild_selectable_and_editable_layouts() {
    assert!(
        RichTextEditorView::should_rebuild_layout_after_layout_affecting_asset_load(
            InteractionState::Selectable,
        )
    );
    assert!(
        RichTextEditorView::should_rebuild_layout_after_layout_affecting_asset_load(
            InteractionState::Editable,
        )
    );
    assert!(
        RichTextEditorView::should_rebuild_layout_after_layout_affecting_asset_load(
            InteractionState::EditableWithInvalidSelection,
        )
    );
}

#[test]
fn test_focus() {
    App::test((), |mut app| async move {
        let (window, editor_view, test_view) = initialize_editor(&mut app);

        // The editor isn't focused, so it should ignore the typed characters.
        editor_view.update(&mut app, |editor, ctx| {
            editor.handle_action(&EditorViewAction::UserTyped(UserInput::new("abc")), ctx);
        });
        editor_view.read(&app, |editor, ctx| assert!(editor.markdown(ctx).is_empty()));

        // Once the editor gains focus, it should start dispatching key events.
        editor_view.update(&mut app, |_, ctx| {
            ctx.focus_self();
        });

        editor_view.update(&mut app, |editor, ctx| {
            editor.handle_action(&EditorViewAction::UserTyped(UserInput::new("abc")), ctx);
        });
        editor_view.read(&app, |editor, ctx| assert_eq!(&editor.markdown(ctx), "abc"));

        // Focus the root view to ensure that the editor is not focused at the framework level.
        test_view.update(&mut app, |_, ctx| ctx.focus_self());
        assert_ne!(app.focused_view_id(window), Some(editor_view.id()));

        // Clicking into the editor should restore focus.
        editor_view.update(&mut app, |editor, ctx| {
            editor.selection_start(CharOffset::from(2), false, ctx);
        });
        assert_eq!(app.focused_view_id(window), Some(editor_view.id()));
    })
}

#[test]
fn test_window_focus() {
    App::test((), |mut app| async move {
        let (window_id, editor_view, _) = initialize_editor(&mut app);

        // Initially, the editor is not focused.
        editor_view.read(&app, |editor, ctx| assert!(!editor.is_focused(ctx)));

        // If the editor is focused, but not the window, it's still not considered focused.
        editor_view.update(&mut app, |editor, ctx| editor.focus(ctx));
        editor_view.read(&app, |editor, ctx| assert!(!editor.is_focused(ctx)));

        // Once the window is focused, we treat the editor as focused too.
        WindowManager::handle(&app).update(&mut app, |windowing_state, _| {
            windowing_state.overwrite_for_test(windowing_state.stage(), Some(window_id));
        });

        editor_view.read(&app, |editor, ctx| assert!(editor.is_focused(ctx)));
    })
}

#[test]
fn test_appearance_changes() {
    App::test((), |mut app| async move {
        let (_, editor_view, _) = initialize_editor(&mut app);

        let render_model = editor_view.read(&app, |editor, ctx| {
            editor.model.as_ref(ctx).render_state().clone()
        });

        // Subscribe to layout updates from the render model to verify edits.
        let layouts = {
            let (tx, rx) = async_channel::unbounded();
            app.update(|ctx| {
                ctx.subscribe_to_model(&render_model, move |_, event, _| {
                    if let RenderEvent::LayoutUpdated = event {
                        block_on(tx.send(*event)).unwrap();
                    }
                })
            });
            rx
        };

        // Wait for initial layout.
        assert!(layouts.recv().await.is_ok());

        // First, focus the editor so it is editable.
        editor_view.update(&mut app, |_, ctx| ctx.focus_self());
        editor_view.update(&mut app, |editor, ctx| {
            editor.user_typed("ABC", ctx);
        });

        // Wait for the typed text to lay out.
        assert!(layouts.recv().await.is_ok());

        // Simulate an appearance change.
        Appearance::handle(&app).update(&mut app, |appearance, ctx| {
            appearance.set_monospace_font_family(warpui::fonts::FamilyId(123), ctx);
            ctx.notify()
        });

        // The appearance change should cause a re-layout.
        assert!(layouts.recv().await.is_ok());

        render_model.update(&mut app, |model, _| {
            // The render model's style should be updated.
            assert_eq!(
                model.styles().code_text.font_family,
                warpui::fonts::FamilyId(123)
            );
        });

        assert_eq!(layouts.try_recv().unwrap_err(), TryRecvError::Empty);
    });
}

#[test]
fn test_omnibar_is_hidden_for_rendered_mermaid_selection() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        let _editable_flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let (_, editor_view, _) = initialize_editor(&mut app);
        let markdown = "Before\n```mermaid\ngraph TD\nA --> B\n```\nAfter";
        reset_editor_with_markdown(&mut app, &editor_view, markdown).await;

        // Blocks default to Raw; explicitly enable Rendered mode for the Mermaid block.
        let render_state = editor_view.read(&app, |editor, ctx| {
            editor.model.as_ref(ctx).render_state().clone()
        });
        editor_view.update(&mut app, |editor, ctx| {
            editor.model.update(ctx, |model, ctx| {
                model.set_mermaid_render_mode(
                    CharOffset::from(7),
                    MarkdownDisplayMode::Rendered,
                    ctx,
                );
            });
        });
        app.read(|ctx| render_state.as_ref(ctx).layout_complete())
            .await;

        editor_view.update(&mut app, |editor, ctx| {
            let mermaid_block_range =
                rendered_mermaid_block_range(editor, ctx).expect("Expected rendered Mermaid block");
            editor.selection_start(mermaid_block_range.start, false, ctx);
            editor.selection_update(mermaid_block_range.end, ctx);
            editor.selection_end(ctx);
        });

        editor_view.read(&app, |editor, ctx| {
            assert!(!editor.should_show_omnibar(ctx));
        });
    });
}

#[test]
fn test_shift_click_on_rendered_mermaid_dispatches_selection_update_to_block_boundary() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        let _editable_flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let (_, editor_view, _) = initialize_editor(&mut app);
        let markdown = "Before\n```mermaid\ngraph TD\nA --> B\n```\nAfter";
        reset_editor_with_markdown(&mut app, &editor_view, markdown).await;

        // Blocks default to Raw; explicitly enable Rendered mode for the Mermaid block.
        let render_state = editor_view.read(&app, |editor, ctx| {
            editor.model.as_ref(ctx).render_state().clone()
        });
        editor_view.update(&mut app, |editor, ctx| {
            editor.model.update(ctx, |model, ctx| {
                model.set_mermaid_render_mode(
                    CharOffset::from(7),
                    MarkdownDisplayMode::Rendered,
                    ctx,
                );
            });
        });
        app.read(|ctx| render_state.as_ref(ctx).layout_complete())
            .await;

        editor_view.update(&mut app, |editor, ctx| {
            editor.selection_start(CharOffset::from(2), false, ctx);
            editor.selection_end(ctx);
        });

        editor_view.read(&app, |editor, ctx| {
            let mermaid_block_range =
                rendered_mermaid_block_range(editor, ctx).expect("Expected rendered Mermaid block");
            let mermaid_block_start = mermaid_block_range.start;
            let mermaid_block_end = mermaid_block_range.end;

            let action = <EditorViewAction as RichTextAction<RichTextEditorView>>::left_mouse_down(
                Location::Block {
                    start_offset: mermaid_block_start,
                    end_offset: mermaid_block_end,
                    block_type: HitTestBlockType::MermaidDiagram,
                },
                ModifiersState {
                    shift: true,
                    ..Default::default()
                },
                1,
                false,
                &editor_view.downgrade(),
                ctx,
            );

            assert_eq!(
                action,
                Some(EditorViewAction::SelectionUpdate(mermaid_block_end))
            );
        });

        editor_view.update(&mut app, |editor, ctx| {
            let mermaid_block_end = rendered_mermaid_block_range(editor, ctx)
                .expect("Expected rendered Mermaid block")
                .end;
            editor.selection_start(mermaid_block_end + 2, false, ctx);
            editor.selection_end(ctx);
        });

        editor_view.read(&app, |editor, ctx| {
            let mermaid_block_range =
                rendered_mermaid_block_range(editor, ctx).expect("Expected rendered Mermaid block");
            let mermaid_block_start = mermaid_block_range.start;
            let mermaid_block_end = mermaid_block_range.end;

            let action = <EditorViewAction as RichTextAction<RichTextEditorView>>::left_mouse_down(
                Location::Block {
                    start_offset: mermaid_block_start,
                    end_offset: mermaid_block_end,
                    block_type: HitTestBlockType::MermaidDiagram,
                },
                ModifiersState {
                    shift: true,
                    ..Default::default()
                },
                1,
                false,
                &editor_view.downgrade(),
                ctx,
            );

            assert_eq!(
                action,
                Some(EditorViewAction::SelectionUpdate(mermaid_block_start))
            );
        });
    });
}

#[test]
fn test_drag_on_rendered_mermaid_dispatches_selection_update_to_block_boundary() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        let _editable_flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let (_, editor_view, _) = initialize_editor(&mut app);
        let markdown = "Before\n```mermaid\ngraph TD\nA --> B\n```\nAfter";
        reset_editor_with_markdown(&mut app, &editor_view, markdown).await;

        // Blocks default to Raw; explicitly enable Rendered mode for the Mermaid block.
        let render_state = editor_view.read(&app, |editor, ctx| {
            editor.model.as_ref(ctx).render_state().clone()
        });
        editor_view.update(&mut app, |editor, ctx| {
            editor.model.update(ctx, |model, ctx| {
                model.set_mermaid_render_mode(
                    CharOffset::from(7),
                    MarkdownDisplayMode::Rendered,
                    ctx,
                );
            });
        });
        app.read(|ctx| render_state.as_ref(ctx).layout_complete())
            .await;

        editor_view.update(&mut app, |editor, ctx| {
            editor.selection_start(CharOffset::from(2), false, ctx);
        });

        editor_view.read(&app, |editor, ctx| {
            let mermaid_block_range =
                rendered_mermaid_block_range(editor, ctx).expect("Expected rendered Mermaid block");
            let mermaid_block_start = mermaid_block_range.start;
            let mermaid_block_end = mermaid_block_range.end;

            let action =
                <EditorViewAction as RichTextAction<RichTextEditorView>>::left_mouse_dragged(
                    Location::Block {
                        start_offset: mermaid_block_start,
                        end_offset: mermaid_block_end,
                        block_type: HitTestBlockType::MermaidDiagram,
                    },
                    false,
                    false,
                    &editor_view.downgrade(),
                    ctx,
                );

            assert_eq!(
                action,
                Some(EditorViewAction::SelectionUpdate(mermaid_block_end))
            );
        });

        editor_view.update(&mut app, |editor, ctx| {
            editor.selection_end(ctx);
            let mermaid_block_end = rendered_mermaid_block_range(editor, ctx)
                .expect("Expected rendered Mermaid block")
                .end;
            editor.selection_start(mermaid_block_end + 2, false, ctx);
        });

        editor_view.read(&app, |editor, ctx| {
            let mermaid_block_range =
                rendered_mermaid_block_range(editor, ctx).expect("Expected rendered Mermaid block");
            let mermaid_block_start = mermaid_block_range.start;
            let mermaid_block_end = mermaid_block_range.end;

            let action =
                <EditorViewAction as RichTextAction<RichTextEditorView>>::left_mouse_dragged(
                    Location::Block {
                        start_offset: mermaid_block_start,
                        end_offset: mermaid_block_end,
                        block_type: HitTestBlockType::MermaidDiagram,
                    },
                    false,
                    false,
                    &editor_view.downgrade(),
                    ctx,
                );

            assert_eq!(
                action,
                Some(EditorViewAction::SelectionUpdate(mermaid_block_start))
            );
        });
    });
}

#[test]
fn test_link_editing() {
    App::test((), |mut app| async move {
        let (_, editor_view, _) = initialize_editor(&mut app);
        // First, focus the editor so it is editable.
        editor_view.update(&mut app, |_, ctx| ctx.focus_self());

        // Select some text and open the link editor. This must be split across several updates so
        // that model changes don't close the link editor.
        editor_view.update(&mut app, |editor, ctx| {
            editor.user_typed("Some text", ctx);
            editor.handle_action(&EditorViewAction::SelectBackwardsByWord, ctx);
        });
        editor_view.update(&mut app, |editor, ctx| {
            editor.handle_action(&EditorViewAction::CreateOrEditLink, ctx);
        });

        // Populate the link editor to create a hyperlink.
        editor_view.update(&mut app, |editor, ctx| {
            assert!(editor.link_editor_open);
            let link_editor = editor.link_editor.as_ref(ctx);
            assert!(link_editor.url_editor().is_focused(ctx));
            assert_eq!(
                link_editor.tag_editor().as_ref(ctx).buffer_text(ctx),
                "text"
            );

            link_editor
                .url_editor()
                .clone()
                .update(ctx, |url_editor, ctx| {
                    url_editor.user_insert("https://warp.dev", ctx);
                });

            editor.link_editor.update(ctx, |link_editor, ctx| {
                link_editor.handle_action(&LinkEditorAction::ApplyLink, ctx)
            });
        });

        // Ensure that the link was created.
        editor_view.read(&app, |editor, ctx| {
            assert_eq!(
                editor.model.as_ref(ctx).debug_buffer(ctx),
                "<text>Some <a_https://warp.dev>text<a>"
            );
        });

        // Create a separate link after the first one, with no initial text selection.
        editor_view.update(&mut app, |editor, ctx| {
            editor.handle_action(&EditorViewAction::MoveToLineEnd, ctx);
        });
        editor_view.update(&mut app, |editor, ctx| {
            editor.handle_action(&EditorViewAction::CreateOrEditLink, ctx);
        });
        editor_view.update(&mut app, |editor, ctx| {
            assert!(editor.link_editor_open);
            let tag_editor = editor.link_editor.as_ref(ctx).tag_editor().clone();
            let url_editor = editor.link_editor.as_ref(ctx).url_editor().clone();

            url_editor.update(ctx, |url_editor, ctx| {
                url_editor.user_insert("https://example.com", ctx);
            });
            tag_editor.update(ctx, |tag_editor, ctx| {
                assert!(tag_editor.is_empty(ctx));
                tag_editor.user_insert("new link", ctx)
            });

            editor.link_editor.update(ctx, |link_editor, ctx| {
                link_editor.handle_action(&LinkEditorAction::ApplyLink, ctx)
            });
        });
        editor_view.read(&app, |editor, ctx| {
            assert_eq!(
                editor.model.as_ref(ctx).debug_buffer(ctx),
                "<text>Some <a_https://warp.dev>text<a><a_https://example.com>new link<a>"
            );
        });
    });
}

#[test]
fn test_editable_markdown_anchor_click_opens_link_tooltip() {
    App::test((), |mut app| async move {
        let (_, editor_view, _) = initialize_editor(&mut app);
        reset_editor_with_markdown(&mut app, &editor_view, "- [Goal](#goal)\n\n## Goal").await;

        let offset = editor_view.read(&app, |editor, ctx| link_offset(editor, "#goal", ctx));
        editor_view.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &EditorViewAction::MaybeOpenFileOrUrl {
                    offset,
                    link_in_text: None,
                    cmd: false,
                },
                ctx,
            );
        });

        editor_view.read(&app, |editor, _ctx| {
            let open_link = editor
                .open_link
                .as_ref()
                .expect("Editable anchor click should show the link tooltip");
            assert_eq!(open_link.url, "#goal");
            assert!(open_link.editable);
        });
    });
}

#[test]
fn test_cmd_click_markdown_anchor_navigates_without_link_tooltip() {
    App::test((), |mut app| async move {
        let (_, editor_view, _) = initialize_editor(&mut app);
        reset_editor_with_markdown(&mut app, &editor_view, "- [Goal](#goal)\n\n## Goal").await;

        let offset = editor_view.read(&app, |editor, ctx| link_offset(editor, "#goal", ctx));
        editor_view.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &EditorViewAction::MaybeOpenFileOrUrl {
                    offset,
                    link_in_text: None,
                    cmd: true,
                },
                ctx,
            );
        });

        editor_view.read(&app, |editor, _ctx| {
            assert!(
                editor.open_link.is_none(),
                "Cmd-click anchor navigation should not show the link tooltip"
            );
        });
    });
}

#[test]
fn test_cmd_click_missing_markdown_anchor_falls_back_to_link_resolution() {
    App::test((), |mut app| async move {
        let (window_id, editor_view, _) = initialize_editor(&mut app);
        let base = tempdir().expect("Expected temp dir");
        let fallback_path = base.path().join("#missing.png");
        std::fs::File::create(&fallback_path).expect("Expected fallback file");
        let session = Arc::new(Session::test().with_shell_launch_data(
            ShellLaunchData::Executable {
                executable_path: PathBuf::from("/bin/bash"),
                shell_type: ShellType::Bash,
            },
        ));

        ActiveSession::handle(&app).update(&mut app, |active_session, ctx| {
            active_session.set_session_for_test(
                window_id,
                session.clone(),
                Some(base.path()),
                None,
                ctx,
            );
        });

        reset_editor_with_markdown(
            &mut app,
            &editor_view,
            "- [Missing](#missing.png)\n\n## Goal",
        )
        .await;

        let events = Arc::new(Mutex::new(Vec::<LinkEvent>::new()));
        let links = editor_view.read(&app, |editor, _ctx| editor.links.clone());
        {
            let events = events.clone();
            app.update(|ctx| {
                ctx.subscribe_to_model(&links, move |_, event, _| {
                    events.lock().push(event.clone());
                })
            });
        }

        let offset = editor_view.read(&app, |editor, ctx| link_offset(editor, "#missing.png", ctx));
        editor_view.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &EditorViewAction::MaybeOpenFileOrUrl {
                    offset,
                    link_in_text: None,
                    cmd: true,
                },
                ctx,
            );
        });

        assert_eventually!(
            events.lock().iter().any(|event| {
                matches!(
                    event,
                    LinkEvent::OpenFileWithTarget { path, .. } if path == &fallback_path
                )
            }),
            "Missing anchor click should fall back to link resolution: {:?}",
            events.lock().clone()
        );
    });
}
#[test]
fn test_run_command_from_text_selection() {
    // This tests that, starting from a text selection, we can still run a command.
    App::test((), |mut app| async move {
        let (_, editor_view, _) = initialize_editor(&mut app);
        let (tx, has_layout) = futures::channel::oneshot::channel();
        app.update(|ctx| {
            let mut tx = Some(tx);
            let render_state = editor_view
                .as_ref(ctx)
                .model
                .as_ref(ctx)
                .render_state()
                .clone();
            ctx.subscribe_to_model(&render_state, move |_, event, _ctx| {
                if let RenderEvent::LayoutUpdated = event {
                    if let Some(tx) = tx.take() {
                        tx.send(()).unwrap();
                    }
                }
            });
        });

        editor_view.update(&mut app, |editor, ctx| {
            editor.reset_with_markdown("Text\n```\necho hi\n```\n```\necho hello\n```", ctx);
        });
        has_layout.await.expect("Model was not laid out");

        editor_view.update(&mut app, |editor, ctx| {
            // Simulate cmd-enter in a non-text block, which should be a no-op.
            editor.selection_start(3.into(), false, ctx);
            editor.run_selected_commands(ctx);
            assert!(!editor.model.as_ref(ctx).has_command_selection(ctx));

            // If the cursor is in a command block, cmd-enter should auto-select it.
            editor.selection_start(8.into(), false, ctx);
            editor.run_selected_commands(ctx);
            let selected_command = editor
                .model
                .as_ref(ctx)
                .selected_command_workflow(ctx)
                .unwrap();
            assert_eq!(
                selected_command
                    .workflow
                    .as_workflow()
                    .command()
                    .expect("Workflow is Command Workflow"),
                "echo hi"
            );

            // If the text cursor was in one command block, but another is selected, cmd-enter
            // should run the selected command.
            editor.command_down(ctx);
            editor.run_selected_commands(ctx);
            let selected_command = editor
                .model
                .as_ref(ctx)
                .selected_command_workflow(ctx)
                .unwrap();
            assert_eq!(
                selected_command
                    .workflow
                    .as_workflow()
                    .command()
                    .expect("Workflow is Command Workflow"),
                "echo hello"
            );
        });
    })
}

#[test]
fn test_link_editing_disabled_for_multiselect() {
    // Ensure that if multiple selections are made, that the link editor is not opened.
    App::test((), |mut app| async move {
        let (_, editor_view, _) = initialize_editor(&mut app);
        // First, focus the editor so it is editable.
        editor_view.update(&mut app, |_, ctx| ctx.focus_self());

        // Select some text and open the link editor. This must be split across several updates so
        // that model changes don't close the link editor.
        editor_view.update(&mut app, |editor, ctx| {
            editor.user_typed("Some text", ctx);
            editor.handle_action(&EditorViewAction::SelectBackwardsByWord, ctx);
        });

        editor_view.update(&mut app, |editor, ctx| {
            assert_eq!(editor.model().as_ref(ctx).selected_text(ctx), "text");
        });
        editor_view.update(&mut app, |editor, ctx| {
            editor.handle_action(&EditorViewAction::CreateOrEditLink, ctx);
        });

        // Populate the link editor to create a hyperlink.
        editor_view.update(&mut app, |editor, ctx| {
            assert!(editor.link_editor_open);
            let link_editor = editor.link_editor.as_ref(ctx);
            assert!(link_editor.url_editor().is_focused(ctx));
            assert_eq!(
                link_editor.tag_editor().as_ref(ctx).buffer_text(ctx),
                "text"
            );

            link_editor
                .url_editor()
                .clone()
                .update(ctx, |url_editor, ctx| {
                    url_editor.user_insert("https://warp.dev", ctx);
                });

            editor.link_editor.update(ctx, |link_editor, ctx| {
                link_editor.handle_action(&LinkEditorAction::ApplyLink, ctx)
            });
        });

        // Add another selection.
        editor_view.update(&mut app, |editor, ctx| {
            editor.handle_action(
                &EditorViewAction::SelectionStart {
                    offset: 1.into(),
                    multiselect: true,
                },
                ctx,
            );
        });

        // Try to open the link editor.
        editor_view.update(&mut app, |editor, ctx| {
            editor.handle_action(&EditorViewAction::CreateOrEditLink, ctx);
        });

        // Ensure that the link editor was not opened.
        editor_view.read(&app, |editor, _ctx| {
            assert!(!editor.link_editor_open);
        });
    });
}
