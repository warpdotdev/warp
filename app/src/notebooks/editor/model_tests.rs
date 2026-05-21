use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use super::super::rich_text_styles;
use super::NotebooksEditorModel;
use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{Owner, Revision, ServerMetadata, ServerPermissions, ServerWorkflow};
use crate::editor::InteractionState;
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::notebooks::editor::model::DEBOUNCED_RESIZE_PERIOD;
use crate::notebooks::editor::notebook_command::NotebookCommand;
use crate::notebooks::editor::view::{RichTextEditorConfig, RichTextEditorView};
use crate::notebooks::file::MarkdownDisplayMode;
use crate::notebooks::link::{NotebookLinks, SessionSource};
use crate::search::files::model::FileSearchModel;
use crate::server::ids::{ServerId, SyncId};
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::settings::FontSettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::terminal::keys::TerminalKeybindings;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workflows::workflow::Workflow;
use crate::workflows::{CloudWorkflow, CloudWorkflowModel, WorkflowId};
use crate::workspace::ActiveSession;
use crate::UserWorkspaces;
use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};
use chrono::Utc;
use futures::prelude::*;
use itertools::Itertools;
use markdown_parser::markdown_parser::RUNNABLE_BLOCK_MARKDOWN_LANG;
use markdown_parser::{
    parse_markdown, CodeBlockText, FormattedText, FormattedTextFragment, FormattedTextLine,
};
use pathfinder_geometry::vector::Vector2F;
use string_offset::CharOffset;
use vec1::vec1;
use warp_core::features::FeatureFlag;
use warp_editor::content::buffer::{AutoScrollBehavior, BufferSelectAction, SelectionOffsets};
use warp_editor::content::text::{BlockType, BufferBlockStyle, CodeBlockType, TextStyles};
use warp_editor::model::{CoreEditorModel, RichTextEditorModel};
use warp_editor::render::model::viewport::SizeInfo;
use warp_editor::render::model::BlockItem;
use warp_editor::render::model::RenderEvent;
use warp_editor::selection::{TextDirection, TextUnit};
use warpui::elements::ListIndentLevel;
use warpui::platform::WindowStyle;
use warpui::presenter::ChildView;
use warpui::r#async::{block_on, FutureId};
use warpui::text::word_boundaries::WordBoundariesPolicy;
use warpui::{r#async::Timer, App, Entity, ModelHandle, SingletonEntity, TypedActionView};
use warpui::{AddSingletonModel, AppContext, Element, View, ViewHandle};

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

/// Create a new RTE model with the given Markdown content. Also creates a [`RichTextEditorView`] (and adds relevant dependencies) since that
/// window_id is needed for the model.
fn model_from_markdown(
    markdown: &str,
    app: &mut App,
    should_initialize_cloud_model: bool,
) -> ModelHandle<NotebooksEditorModel> {
    let global_resources = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resources));
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(|_| KeybindingChangedNotifier::new());
    app.add_singleton_model(|_| repo_metadata::repositories::DetectedRepositories::default());
    #[cfg(feature = "local_fs")]
    app.add_singleton_model(repo_metadata::RepoMetadataModel::new);
    app.add_singleton_model(FileSearchModel::new);
    app.add_singleton_model(NotebookKeybindings::new);
    app.add_singleton_model(TerminalKeybindings::new);

    // In some tests, we need to initialize CloudModel first to mock some server data. In those cases, avoid mocking it a second time.
    if should_initialize_cloud_model {
        app.add_singleton_model(CloudModel::mock);
    }

    let (window, _) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
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
    app.add_model(|ctx| {
        let styles = rich_text_styles(Appearance::as_ref(ctx), FontSettings::as_ref(ctx));
        let mut model = NotebooksEditorModel::new(styles, window, ctx);
        model.reset_with_markdown(markdown, ctx);

        model
    })
}

fn initialize_deps(app: &mut App) {
    app.add_singleton_model(|_| Appearance::mock());
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
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);
    initialize_settings_for_tests(app);
}

/// Get the handle for each nested editor model.
fn command_models(
    handle: &ModelHandle<NotebooksEditorModel>,
    app: &mut App,
) -> Vec<ModelHandle<NotebookCommand>> {
    app.read(|ctx| {
        let model = handle.as_ref(ctx);
        model.child_models.model_handles().collect()
    })
}

fn command_range(
    command_handle: &ModelHandle<NotebookCommand>,
    app: &mut App,
) -> Range<CharOffset> {
    app.read(|ctx| {
        let command = command_handle.as_ref(ctx);
        let start = command.start_offset(ctx).expect("Start offset is invalid");
        let end = command.end_offset(ctx).expect("End offset is invalid");
        start..end
    })
}

/// Wait for text layout to finish.
async fn layout_model(app: &mut App, model: &ModelHandle<NotebooksEditorModel>) {
    app.read(|ctx| model.as_ref(ctx).render_state.as_ref(ctx).layout_complete())
        .await;
}

/// Wait for code blocks to finish syntax highlighting.
async fn finish_highlighting(
    model: &ModelHandle<NotebooksEditorModel>,
    expected_blocks: usize,
    seen_futures: &mut HashSet<FutureId>,
    app: &mut App,
) {
    layout_model(app, model).await;
    let commands = command_models(model, app);
    let highlighting_blocks = app.update(|ctx| {
        commands
            .into_iter()
            .filter_map(|command| {
                command
                    .as_ref(ctx)
                    .syntax_highlighting_handle()
                    .and_then(|future| {
                        seen_futures
                            .insert(future.future_id())
                            .then(|| ctx.await_spawned_future(future.future_id()))
                    })
            })
            .collect_vec()
    });
    assert_eq!(
        highlighting_blocks.len(),
        expected_blocks,
        "Expected {expected_blocks} blocks to be running syntax highlighting"
    );

    future::join_all(highlighting_blocks).await;
}

#[test]
fn test_edit_command_submodel() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);

        let model_handle = model_from_markdown("Hello\n```\necho test\n```\nworld", &mut app, true);
        layout_model(&mut app, &model_handle).await;
        let command_model = command_models(&model_handle, &mut app)
            .into_iter()
            .exactly_one()
            .expect("Command model should exist");
        assert_eq!(
            command_range(&command_model, &mut app),
            CharOffset::from(6)..CharOffset::from(16)
        );
        assert_eq!(
            app.read(|ctx| command_model.as_ref(ctx).command(ctx))
                .as_deref(),
            Some("echo test")
        );

        // Editing the buffer should preserve the existing model.
        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(CharOffset::from(12), ctx);
            model.user_insert("edited ", ctx);
        });
        layout_model(&mut app, &model_handle).await;

        let model2 = command_models(&model_handle, &mut app)
            .into_iter()
            .exactly_one()
            .expect("Command model should exist");
        assert_eq!(command_model, model2);

        // The edit should also automatically update the model's range.
        assert_eq!(
            command_range(&command_model, &mut app),
            CharOffset::from(6)..CharOffset::from(23)
        );
        assert_eq!(
            app.read(|ctx| command_model.as_ref(ctx).command(ctx))
                .as_deref(),
            Some("echo edited test")
        );

        // Insert at the very end of the command block.
        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(CharOffset::from(23), ctx);
            model.user_insert("...", ctx);
        });
        layout_model(&mut app, &model_handle).await;
        assert_eq!(
            command_range(&command_model, &mut app),
            CharOffset::from(6)..CharOffset::from(26)
        );
        assert_eq!(
            app.read(|ctx| command_model.as_ref(ctx).command(ctx))
                .as_deref(),
            Some("echo edited test...")
        );
    });
}

#[test]
fn test_delete_command_submodel() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);

        let model_handle = model_from_markdown("Hello\n```\necho test\n```\nworld", &mut app, true);
        layout_model(&mut app, &model_handle).await;
        assert_eq!(command_models(&model_handle, &mut app).len(), 1);

        // Backspacing the start marker should un-style the block.
        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(CharOffset::from(7), ctx);
            model.backspace(ctx);
        });
        layout_model(&mut app, &model_handle).await;

        assert_eq!(
            app.read(|ctx| model_handle.as_ref(ctx).debug_buffer(ctx)),
            "<text>Hello\\necho test\\nworld"
        );
        assert!(command_models(&model_handle, &mut app).is_empty());
    });
}

#[test]
fn test_replace_command_submodel() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);

        let model_handle = model_from_markdown("Hello\n```\necho test\n```\nworld", &mut app, true);
        layout_model(&mut app, &model_handle).await;
        let command1 = command_models(&model_handle, &mut app)
            .into_iter()
            .exactly_one()
            .expect("Command model should exist");

        assert_eq!(
            app.read(|ctx| command1.as_ref(ctx).command(ctx)).as_deref(),
            Some("echo test")
        );

        // Delete the entire command, and replace it with a new one.
        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(6.into(), ctx);
            model.set_last_selection_head(16.into(), ctx);
            model.backspace(ctx);
            model.user_insert("\n", ctx);

            model.insert_formatted_from_paste(
                FormattedText::new([FormattedTextLine::CodeBlock(CodeBlockText {
                    lang: RUNNABLE_BLOCK_MARKDOWN_LANG.into(),
                    code: "also nine".into(),
                })]),
                "also nine",
                ctx,
            );
        });

        assert_eq!(
            app.read(|ctx| model_handle.as_ref(ctx).debug_buffer(ctx)),
            "<text>Hello<code:Shell>also nine<text>world"
        );
        layout_model(&mut app, &model_handle).await;

        let command2 = command_models(&model_handle, &mut app)
            .into_iter()
            .exactly_one()
            .expect("Command model should exist");
        assert_ne!(command1, command2);

        app.read(|ctx| {
            // Even though command1 was removed from the editor model, its anchors will still be
            // valid. That's because, if an anchor is exactly on the deletion boundary, we keep it.

            // command2 should have the same range as command1, even though it's a different model.
            assert_eq!(
                command2.as_ref(ctx).command(ctx).as_deref(),
                Some("also nine")
            );
            assert_eq!(command2.as_ref(ctx).start_offset(ctx), Some(6.into()));
            assert_eq!(command2.as_ref(ctx).end_offset(ctx), Some(16.into()));
        });

        // Now, replace just the command contents. This will logically be the same command.
        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(CharOffset::from(7), ctx);
            model.set_last_selection_head(CharOffset::from(15), ctx);
            model.user_insert("echo four", ctx);
        });
        layout_model(&mut app, &model_handle).await;

        let command3 = command_models(&model_handle, &mut app)
            .into_iter()
            .exactly_one()
            .expect("Command model should exist");
        assert_eq!(command2, command3);
    });
}

#[test]
fn test_inline_markdown() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("First **bold", &mut app, true);

        editor.update(&mut app, |editor, ctx| {
            editor.user_insert("*", ctx);
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.content.as_ref(ctx).debug(), "<text>First **bold*");
        });

        editor.update(&mut app, |editor, ctx| {
            editor.user_insert("*", ctx);
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.active_text_style, TextStyles::default());
            assert_eq!(
                editor.content.as_ref(ctx).debug(),
                "<text>First <b_s>bold<b_e>"
            );
        });

        editor.update(&mut app, |editor, ctx| {
            editor.cursor_at(CharOffset::from(6), ctx);
            editor.active_text_style = TextStyles::default();
            editor.user_insert("[abc](https://warp.dev", ctx);
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor.content.as_ref(ctx).debug(),
                "<text>First[abc](https://warp.dev <b_s>bold<b_e>"
            );
        });

        editor.update(&mut app, |editor, ctx| {
            editor.user_insert(")", ctx);
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.active_text_style, TextStyles::default());
            assert_eq!(
                editor.content.as_ref(ctx).debug(),
                "<text>First<a_https://warp.dev>abc<a> <b_s>bold<b_e>"
            );
        });

        layout_model(&mut app, &editor).await; // move_to_line_end relies on soft-wrapped points.
        editor.update(&mut app, |editor, ctx| {
            editor.move_to_line_end(ctx);
        });

        editor.update(&mut app, |editor, ctx| {
            editor.user_insert("`abc", ctx);

            assert_eq!(
                editor.content.as_ref(ctx).debug(),
                "<text>First<a_https://warp.dev>abc<a> <b_s>bold`abc<b_e>"
            );

            editor.user_insert("`", ctx);
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.active_text_style, TextStyles::default().bold());
            assert_eq!(
                editor.content.as_ref(ctx).debug(),
                "<text>First<a_https://warp.dev>abc<a> <b_s>bold<b_e><c_s>abc<c_e>"
            );
        });
    })
}

#[test]
fn test_inline_markdown_italic_underscores() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("Start _italic", &mut app, true);

        // Typing the trailing `_` in `_italic_` coerces to italic.
        editor.update(&mut app, |editor, ctx| {
            editor.user_insert("_", ctx);
        });
        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.active_text_style, TextStyles::default());
            assert_eq!(
                editor.content.as_ref(ctx).debug(),
                "<text>Start <i_s>italic<i_e>"
            );
        });
    })
}

#[test]
fn test_inline_markdown_intra_word_underscore_ignored() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("foo_bar", &mut app, true);

        // Intra-word underscores should not be coerced to italic.
        editor.update(&mut app, |editor, ctx| {
            editor.user_insert("_", ctx);
        });
        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.content.as_ref(ctx).debug(), "<text>foo_bar_");
        });
    })
}

#[test]
fn test_inline_markdown_double_leading_underscore_not_italic() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("__w", &mut app, true);

        // Typing a trailing `_` after `__w` should NOT coerce to italic,
        // because the opening delimiter is `__` (double-underscore), not `_`.
        editor.update(&mut app, |editor, ctx| {
            editor.user_insert("_", ctx);
        });
        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.content.as_ref(ctx).debug(), "<text>__w_");
        });
    })
}

#[test]
fn test_find_matching_header_simple() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("- [Goal](#goal)\n\n## Goal\nBody", &mut app, true);

        editor.read(&app, |editor, ctx| {
            let range = editor
                .find_matching_header("#goal", ctx)
                .expect("Fragment should match heading");
            let heading = editor
                .content
                .as_ref(ctx)
                .text_in_range(range.start + 1..range.end)
                .into_string();

            assert_eq!(heading, "Goal");
        });
    })
}

#[test]
fn test_find_matching_header_case_insensitive() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("## My Bold Goal\nBody", &mut app, true);

        editor.read(&app, |editor, ctx| {
            assert!(editor.find_matching_header("#my bold goal", ctx).is_some());
            assert!(editor.find_matching_header("#MY BOLD GOAL", ctx).is_some());
        });
    })
}

#[test]
fn test_find_matching_header_percent_decoded() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("## Hello World\nBody", &mut app, true);

        editor.read(&app, |editor, ctx| {
            assert!(editor.find_matching_header("#Hello%20World", ctx).is_some());
        });
    })
}

#[test]
fn test_find_matching_header_returns_first_match() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("## Goal\nFirst\n\n## Goal\nSecond", &mut app, true);

        editor.read(&app, |editor, ctx| {
            let range = editor
                .find_matching_header("#goal", ctx)
                .expect("Should match first heading");
            let heading_text = editor
                .content
                .as_ref(ctx)
                .text_in_range(range.start + 1..range.end)
                .into_string();
            assert_eq!(heading_text, "Goal");
        });
    })
}

#[test]
fn test_find_matching_header_missing_returns_none() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("## Goal\nBody", &mut app, true);

        editor.read(&app, |editor, ctx| {
            assert!(editor.find_matching_header("#nonexistent", ctx).is_none());
            assert!(editor.find_matching_header("#", ctx).is_none());
            assert!(editor.find_matching_header("", ctx).is_none());
        });
    })
}

#[test]
fn test_cursor_bias_editing() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("First`inline", &mut app, true);

        editor.update(&mut app, |editor, ctx| {
            editor.user_insert("`", ctx);
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor.content.as_ref(ctx).debug(),
                "<text>First<c_s>inline<c_e>"
            );
        });

        editor.update(&mut app, |editor, ctx| {
            editor.move_left(ctx);
        });

        editor.update(&mut app, |editor, ctx| {
            editor.user_insert("in", ctx);
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor.content.as_ref(ctx).debug(),
                "<text>First<c_s>inlinein<c_e>"
            );
            assert_eq!(
                editor.active_text_style,
                TextStyles::default().inline_code()
            );
        });

        // We need soft-wrapped points for select_to_line_start.
        layout_model(&mut app, &editor).await;

        // Changing selection head should not impact the selection bias.
        editor.update(&mut app, |editor, ctx| {
            editor.select_to_line_start(ctx);
        });

        editor.update(&mut app, |editor, ctx| {
            editor.move_right(ctx);
        });

        editor.update(&mut app, |editor, ctx| {
            editor.backspace(ctx);
        });

        editor.update(&mut app, |editor, ctx| {
            editor.user_insert("b", ctx);
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor.content.as_ref(ctx).debug(),
                "<text>First<c_s>inlineib<c_e>"
            );
            assert_eq!(
                editor.active_text_style,
                TextStyles::default().inline_code()
            );
        });
    })
}

#[test]
fn test_markdown_shortcuts_require_single_cursor() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("First *line", &mut app, true);

        editor.update(&mut app, |editor, ctx| {
            // Type a Markdown trigger with a text selection.
            editor.select_left(ctx);
            editor.user_insert("*", ctx);

            assert_eq!(editor.debug_buffer(ctx), "<text>First *lin*");
        })
    });
}

#[test]
fn test_plain_text_pasting() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("First text\nSecond line", &mut app, true);
        let clipboard_content = "text";

        layout_model(&mut app, &editor).await;

        editor.update(&mut app, |editor, ctx| {
            editor.cursor_at(CharOffset::from(1), ctx);
            editor.select_to_line_end(ctx);
        });

        let markdown = parse_markdown(clipboard_content).expect("Should parse plain text");
        editor.update(&mut app, |editor, ctx| {
            editor.insert_formatted_from_paste(markdown, clipboard_content, ctx);

            assert_eq!(editor.debug_buffer(ctx), "<text>text\\nSecond line");
        });
    });
}

#[test]
fn test_delete_inside_raw_mermaid_block_edits_text_without_removing_block() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        let _editable_flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let markdown = "Text
```mermaid
graph TD
A --> B
```
More text";
        let original_char_count = markdown.chars().count();

        let model_handle = model_from_markdown(markdown, &mut app, true);
        model_handle.update(&mut app, |model, ctx| {
            model.set_interaction_state(InteractionState::Editable, ctx);
        });
        layout_model(&mut app, &model_handle).await;
        let cursor_offset = CharOffset::from(
            markdown
                .find("graph TD")
                .expect("Mermaid source should exist")
                + 3,
        );

        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(cursor_offset, ctx);
            assert!(!model.has_command_selection(ctx));
            model.backspace(ctx);
        });
        layout_model(&mut app, &model_handle).await;

        model_handle.read(&app, |model, ctx| {
            let updated_markdown = model.markdown(ctx);
            let updated_mermaid_command = model
                .child_models
                .model_handles::<NotebookCommand>()
                .exactly_one()
                .ok()
                .expect("Mermaid command should still exist after backspace");
            let updated_mermaid_command = updated_mermaid_command.as_ref(ctx);
            let updated_mermaid_range = updated_mermaid_command
                .start_offset(ctx)
                .expect("Mermaid command should still have a start offset")
                ..updated_mermaid_command
                    .end_offset(ctx)
                    .expect("Mermaid command should still have an end offset");
            let cursor = model.selection.as_ref(ctx).cursors(ctx)[0];

            assert!(updated_markdown.contains("```mermaid"));
            assert!(updated_markdown.contains("More text"));
            assert_eq!(updated_markdown.chars().count(), original_char_count - 1);
            assert!(model.selection_is_single_cursor(ctx));
            assert!(cursor > updated_mermaid_range.start && cursor < updated_mermaid_range.end);
            assert!(!model.has_command_selection(ctx));
        });

        model_handle.update(&mut app, |model, ctx| model.undo(ctx));
        layout_model(&mut app, &model_handle).await;

        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(cursor_offset, ctx);
            assert!(!model.has_command_selection(ctx));
            model.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
        });
        layout_model(&mut app, &model_handle).await;

        model_handle.read(&app, |model, ctx| {
            let updated_markdown = model.markdown(ctx);
            let updated_mermaid_command = model
                .child_models
                .model_handles::<NotebookCommand>()
                .exactly_one()
                .ok()
                .expect("Mermaid command should still exist after delete");
            let updated_mermaid_command = updated_mermaid_command.as_ref(ctx);
            let updated_mermaid_range = updated_mermaid_command
                .start_offset(ctx)
                .expect("Mermaid command should still have a start offset")
                ..updated_mermaid_command
                    .end_offset(ctx)
                    .expect("Mermaid command should still have an end offset");
            let cursor = model.selection.as_ref(ctx).cursors(ctx)[0];

            assert!(updated_markdown.contains("```mermaid"));
            assert!(updated_markdown.contains("More text"));
            assert_eq!(updated_markdown.chars().count(), original_char_count - 1);
            assert!(model.selection_is_single_cursor(ctx));
            assert!(cursor > updated_mermaid_range.start && cursor < updated_mermaid_range.end);
            assert!(!model.has_command_selection(ctx));
        });
    });
}

#[test]
fn test_pasting_link_on_selected_text() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("First text\nSecond line", &mut app, true);
        let clipboard_content = "https://warp.dev";

        layout_model(&mut app, &editor).await;

        editor.update(&mut app, |editor, ctx| {
            editor.cursor_at(CharOffset::from(1), ctx);
            editor.select_to_line_end(ctx);
        });

        let markdown = parse_markdown(clipboard_content).expect("Should parse link");
        editor.update(&mut app, |editor, ctx| {
            editor.insert_formatted_from_paste(markdown, clipboard_content, ctx);

            assert_eq!(
                editor.debug_buffer(ctx),
                "<text><a_https://warp.dev>First text<a>\\nSecond line"
            );
        });
    });
}

#[test]
fn test_pasting_on_command_selection() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let model_handle = model_from_markdown(
            r#"Text
```
first command
```
More text
```
echo command
```"#,
            &mut app,
            true,
        );
        // Wait for layout and syntax highlighting, to reduce flakiness.
        finish_highlighting(&model_handle, 2, &mut Default::default(), &mut app).await;
        let clipboard_content = "pasteboard";

        model_handle.update(&mut app, |editor, ctx| {
            editor.cursor_at(CharOffset::from(1), ctx);

            editor.select_command_down(ctx);
            assert_eq!(selected_commands(editor, ctx), vec![CharOffset::from(5)]);
        });

        let markdown = parse_markdown(clipboard_content).expect("Should parse content");
        model_handle.update(&mut app, |editor, ctx| {
            editor.insert_formatted_from_paste(markdown, clipboard_content, ctx);

            assert_eq!(
                editor.debug_buffer(ctx),
                "<text>Text<code:Shell>pasteboard<text>More text<code:Shell><c_#b4fa72>echo<c> command<text>"
            );
        });
    });
}

#[test]
fn test_markdown_block_conversion() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown(
            "This is a list:\n1. First\n2. Second\n3. Third",
            &mut app,
            true,
        );

        editor.update(&mut app, |editor, ctx| {
            assert_eq!(
                editor.debug_buffer(ctx),
                "<text>This is a list:<ol0@1>First<ol0>Second<ol0>Third<text>"
            );

            editor.cursor_at(17.into(), ctx);
            editor.user_insert("#", ctx);
            editor.user_insert(" ", ctx);

            assert_eq!(
                editor.debug_buffer(ctx),
                "<text>This is a list:<header1>First<ol0>Second<ol0>Third<text>"
            );

            editor.undo(ctx);
            assert_eq!(
                editor.debug_buffer(ctx),
                "<text>This is a list:<ol0@1># First<ol0>Second<ol0>Third<text>"
            );
        });
    });
}

#[test]
fn test_conversion_preserves_indent_level() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown(
            "1. Level 1\n    1. Level 2\n        1. Level 3",
            &mut app,
            true,
        );
        editor.update(&mut app, |editor, ctx| {
            assert_eq!(
                editor.debug_buffer(ctx),
                "<ol0@1>Level 1<ol1@1>Level 2<ol2@1>Level 3<text>"
            );

            // Convert the level-2 item to a task list by mocking the conversion menu.
            editor.select_line_at(10.into(), false, ctx);
            editor.convert_block(
                BufferBlockStyle::TaskList {
                    // The default for the conversion menu is level 1.
                    indent_level: ListIndentLevel::One,
                    complete: false,
                },
                ctx,
            );
            assert_eq!(
                editor.debug_buffer(ctx),
                "<ol0@1>Level 1<cl1:false>Level 2<ol2@1>Level 3<text>"
            );

            editor.undo(ctx);
            assert_eq!(
                editor.debug_buffer(ctx),
                "<ol0@1>Level 1<ol1@1>Level 2<ol2@1>Level 3<text>"
            );
            editor.redo(ctx);

            // Convert the level-3 item to an unordered list using Markdown shortcuts.
            editor.cursor_at(17.into(), ctx);
            editor.user_insert("-", ctx);
            editor.user_insert(" ", ctx);

            assert_eq!(
                editor.debug_buffer(ctx),
                "<ol0@1>Level 1<cl1:false>Level 2<ul2>Level 3<text>"
            );

            editor.undo(ctx);
            assert_eq!(
                editor.debug_buffer(ctx),
                "<ol0@1>Level 1<cl1:false>Level 2<ol2@1>- Level 3<text>"
            );
            editor.redo(ctx);
            assert_eq!(
                editor.debug_buffer(ctx),
                "<ol0@1>Level 1<cl1:false>Level 2<ul2>Level 3<text>"
            );
        });
    });
}

#[test]
fn test_task_list_toggling() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("Some text \nMore", &mut app, true);
        layout_model(&mut app, &editor).await;

        editor.update(&mut app, |editor, ctx| {
            assert_eq!(editor.debug_buffer(ctx), "<text>Some text \\nMore");

            editor.cursor_at(1.into(), ctx);
            editor.select_to_line_end(ctx);
            editor.set_block_style(
                BufferBlockStyle::TaskList {
                    indent_level: ListIndentLevel::One,
                    complete: false,
                },
                ctx,
            );

            assert_eq!(editor.debug_buffer(ctx), "<cl0:false>Some text <text>More");

            editor.toggle_task_list(CharOffset::from(0), ctx);
            assert_eq!(editor.debug_buffer(ctx), "<cl0:true>Some text <text>More");
        });
    });
}

#[test]
fn test_ordered_list_shortcut_within_line() {
    // This is a regression test for a bug where typing `1. ` in the middle of a line would trigger
    // the ordered list Markdown shortcut.

    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("Some text \nMore", &mut app, true);

        editor.update(&mut app, |editor, ctx| {
            assert_eq!(editor.debug_buffer(ctx), "<text>Some text \\nMore");

            editor.cursor_at(11.into(), ctx);
            editor.user_insert("1", ctx);
            editor.user_insert(".", ctx);
            editor.user_insert(" ", ctx);

            // When the cursor is in the middle of a line, the shortcut doesn't trigger.
            assert_eq!(editor.debug_buffer(ctx), "<text>Some text 1. \\nMore");

            // At the start of a line, however, the shortcut can still trigger.
            editor.cursor_at(15.into(), ctx);
            editor.user_insert("1", ctx);
            editor.user_insert(".", ctx);
            editor.user_insert(" ", ctx);
            assert_eq!(
                editor.debug_buffer(ctx),
                "<text>Some text 1. <ol0@1>More<text>"
            );
        });
    });
}

#[test]
/// This tests a similar regression to [`test_ordered_list_shortcut_within_line`]. If a line starts
/// with a potential ordered list shortcut, and we type elsewhere in the line, the shortcut should
/// not trigger.
fn test_ordered_list_shortcut_anchored() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("2\\. Not a list", &mut app, true);

        editor.update(&mut app, |editor, ctx| {
            // First, make sure the buffer has an untriggered potential shortcut. A more likely way
            // to get into this situation is from typing `2. ` and then undoing the shortcut.
            assert_eq!(editor.debug_buffer(ctx), "<text>2. Not a list");

            editor.cursor_at(5.into(), ctx);

            // This space should not trigger the shortcut.
            editor.user_insert(" ", ctx);
            assert_eq!(editor.debug_buffer(ctx), "<text>2. N ot a list");

            // However, a space right after the `2.` can trigger it.
            editor.cursor_at(3.into(), ctx);
            editor.user_insert(" ", ctx);
            assert_eq!(editor.debug_buffer(ctx), "<ol0@2> N ot a list<text>");
        });
    });
}

#[test]
fn test_ordered_list_start_number() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("", &mut app, true);
        editor.update(&mut app, |editor, ctx| {
            editor.user_insert("3", ctx);
            assert_eq!(editor.debug_buffer(ctx), "<text>3");
            editor.user_insert(".", ctx);
            assert_eq!(editor.debug_buffer(ctx), "<text>3.");
            editor.user_insert(" ", ctx);
            assert_eq!(editor.debug_buffer(ctx), "<ol0@3><text>");
        })
    });
}

#[test]
fn test_ordered_list_renumbering() {
    // This test covers ordered list shortcut behavior when the cursor is already in a numbered
    // list:
    // - If it would change the number, we apply the shortcut
    // - If it would not, we don't apply the shortcut
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("1. First\n    1. Nested\n2. Second", &mut app, true);
        editor.update(&mut app, |editor, ctx| {
            assert_eq!(
                editor.debug_buffer(ctx),
                "<ol0@1>First<ol1@1>Nested<ol0>Second<text>"
            );

            // The Markdown shortcut will renumber either of the two starting list items.
            editor.cursor_at(1.into(), ctx);
            editor.user_insert("5", ctx);
            editor.user_insert(".", ctx);
            editor.user_insert(" ", ctx);
            assert_eq!(
                editor.debug_buffer(ctx),
                "<ol0@5>First<ol1@1>Nested<ol0>Second<text>"
            );

            editor.cursor_at(7.into(), ctx);
            editor.user_insert("3", ctx);
            editor.user_insert(".", ctx);
            editor.user_insert(" ", ctx);
            assert_eq!(
                editor.debug_buffer(ctx),
                "<ol0@5>First<ol1@3>Nested<ol0>Second<text>"
            );

            // The second item in the outer list can't be renumbered.
            editor.cursor_at(14.into(), ctx);
            editor.user_insert("4", ctx);
            editor.user_insert(".", ctx);
            editor.user_insert(" ", ctx);
            assert_eq!(
                editor.debug_buffer(ctx),
                "<ol0@5>First<ol1@3>Nested<ol0>4. Second<text>"
            );
        })
    })
}

#[test]
fn test_select_paragraph() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown(
            "First line\nSecond line\n```\nCode block\n```",
            &mut app,
            true,
        );

        // Select the second line of text.
        editor.update(&mut app, |editor, ctx| {
            editor.select_line_at(13.into(), false, ctx);
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor
                    .buffer_selection_model()
                    .as_ref(ctx)
                    .first_selection_tail(),
                CharOffset::from(12)
            );
            // The cursor should be at the end of the line.
            assert_eq!(
                editor
                    .buffer_selection_model()
                    .as_ref(ctx)
                    .first_selection_head(),
                CharOffset::from(23)
            );
        });

        // Now, select the first line. This makes sure we replace the selection rather than expand it.
        editor.update(&mut app, |editor, ctx| {
            editor.select_line_at(11.into(), false, ctx);
        });
        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor
                    .buffer_selection_model()
                    .as_ref(ctx)
                    .first_selection_tail(),
                CharOffset::from(1)
            );
            assert_eq!(
                editor
                    .buffer_selection_model()
                    .as_ref(ctx)
                    .first_selection_head(),
                CharOffset::from(11)
            );
        });
    })
}

#[test]
fn test_select_line_out_of_bounds() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown(
            "First line\nSecond line\n```\nCode block\n```",
            &mut app,
            true,
        );

        // Ensure there's an initial selection.
        editor.update(&mut app, |editor, ctx| editor.cursor_at(4.into(), ctx));

        // Now, select out of bounds.
        editor.update(&mut app, |editor, ctx| {
            editor.select_line_at(100.into(), false, ctx);
        });

        // The last line should be selected instead (the empty one after the code block end marker).
        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor
                    .buffer_selection_model()
                    .as_ref(ctx)
                    .first_selection_tail(),
                CharOffset::from(35)
            );
            assert_eq!(
                editor
                    .buffer_selection_model()
                    .as_ref(ctx)
                    .first_selection_head(),
                CharOffset::from(35)
            );
        });
    })
}

#[test]
fn test_select_line_in_block() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown(
            "```\nFirst line\nSecond line\nThird line\n```",
            &mut app,
            true,
        );

        // Select one of the code block lines.
        editor.update(&mut app, |editor, ctx| {
            editor.select_line_at(14.into(), false, ctx);
        });

        // Only the second line should be selected, not the whole block.
        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor
                    .buffer_selection_model()
                    .as_ref(ctx)
                    .first_selection_tail(),
                CharOffset::from(12)
            );
            assert_eq!(
                editor
                    .buffer_selection_model()
                    .as_ref(ctx)
                    .first_selection_head(),
                CharOffset::from(23)
            );
        });
    })
}

#[test]
fn test_select_to_end_of_last_line() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("First line\nLast line", &mut app, true);
        layout_model(&mut app, &editor).await;
        editor.update(&mut app, |editor, ctx| {
            // Position the cursor in the middle of the last line.
            editor.cursor_at(14.into(), ctx);

            // This should clamp to the end of the buffer.
            editor.select_to_line_end(ctx);
            assert_eq!(
                editor
                    .buffer_selection_model()
                    .as_ref(ctx)
                    .first_selection_tail(),
                14.into()
            );
            assert_eq!(
                editor
                    .buffer_selection_model()
                    .as_ref(ctx)
                    .first_selection_head(),
                21.into()
            );
        });
    })
}

#[test]
fn test_move_within_line() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("First line\nMiddle line\nLast line", &mut app, true);
        layout_model(&mut app, &editor).await;
        editor.update(&mut app, |editor, ctx| {
            // Position the cursor in the middle of the middle line.
            editor.cursor_at(16.into(), ctx);

            editor.move_to_line_start(ctx);
            assert_eq!(
                editor
                    .buffer_selection_model()
                    .as_ref(ctx)
                    .first_selection_head(),
                12.into()
            );

            // Reset the cursor and move to the end.
            editor.cursor_at(16.into(), ctx);
            editor.move_to_line_end(ctx);
            assert_eq!(
                editor
                    .buffer_selection_model()
                    .as_ref(ctx)
                    .first_selection_head(),
                23.into()
            );
        });
    })
}

#[test]
fn test_move_to_start_of_first_line() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("First line\nLast line", &mut app, true);
        layout_model(&mut app, &editor).await;
        editor.update(&mut app, |editor, ctx| {
            // Position the cursor in the middle of the first line.
            editor.cursor_at(3.into(), ctx);

            editor.move_to_line_start(ctx);
            assert!(editor
                .buffer_selection_model()
                .as_ref(ctx)
                .first_selection_is_single_cursor());
            assert_eq!(
                editor
                    .buffer_selection_model()
                    .as_ref(ctx)
                    .first_selection_head(),
                1.into()
            );
        });
    })
}

#[test]
fn test_move_up_on_first_line() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("First line\nLast line", &mut app, true);
        editor.update(&mut app, |editor, ctx| {
            // Position the cursor in the middle of the first line.
            editor.cursor_at(3.into(), ctx);

            editor.move_up(ctx);
            assert!(editor
                .buffer_selection_model()
                .as_ref(ctx)
                .first_selection_is_single_cursor());
            assert_eq!(
                editor
                    .buffer_selection_model()
                    .as_ref(ctx)
                    .first_selection_head(),
                1.into()
            );
        });
    })
}

#[test]
fn test_move_down_on_last_line() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("First line\nLast line", &mut app, true);
        layout_model(&mut app, &editor).await;
        editor.update(&mut app, |editor, ctx| {
            // Position the cursor in the middle of the last line.
            editor.cursor_at(14.into(), ctx);

            editor.move_down(ctx);
            assert!(editor
                .buffer_selection_model()
                .as_ref(ctx)
                .first_selection_is_single_cursor());
            assert_eq!(
                editor
                    .buffer_selection_model()
                    .as_ref(ctx)
                    .first_selection_head(),
                21.into()
            );
        });
    })
}

#[test]
fn test_enter_on_first_code_block() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("```\nFirst\n```", &mut app, true);
        let render_state = app.read(|ctx| editor.as_ref(ctx).render_state().clone());
        layout_model(&mut app, &editor).await;

        // Expected blocks:
        // 1. The code block
        // 2. An empty text block to end the buffer
        app.read(|ctx| {
            assert_eq!(render_state.as_ref(ctx).blocks(), 2);
            assert_eq!(render_state.as_ref(ctx).max_offset(), 6.into());
        });

        editor.update(&mut app, |editor, ctx| {
            // Position the cursor at the start of the first line.
            editor.cursor_at(1.into(), ctx);
            editor.enter(ctx);
        });

        layout_model(&mut app, &editor).await;

        render_state.update(&mut app, |render_state, _ctx| {
            // Expected blocks:
            // 1. An empty text block, inserted by enter
            // 2. The code block
            // 3. An empty text block to end the buffer
            assert_eq!(render_state.blocks(), 3);
            assert_eq!(render_state.max_offset(), 7.into());
            assert_eq!(render_state.max_line(), 3.into());
        });
    })
}

#[test]
fn test_enter_on_first_header() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("# Hello", &mut app, true);
        let render_state = app.read(|ctx| editor.as_ref(ctx).render_state().clone());
        layout_model(&mut app, &editor).await;

        // Expected blocks:
        // 1. The header
        // 2. An empty text block to end the buffer
        app.read(|ctx| {
            assert_eq!(render_state.as_ref(ctx).blocks(), 2);
            assert_eq!(render_state.as_ref(ctx).max_offset(), 6.into());
        });

        editor.update(&mut app, |editor, ctx| {
            editor.cursor_at(1.into(), ctx);
            editor.enter(ctx);
        });

        layout_model(&mut app, &editor).await;

        app.read(|ctx| {
            let render_state = render_state.as_ref(ctx);
            // Expected blocks:
            // 1. An empty text block, inserted by enter
            // 2. The header
            // 3. An empty text block to end the buffer
            assert_eq!(render_state.blocks(), 3);
            assert_eq!(render_state.max_offset(), 7.into());
            assert_eq!(render_state.max_line(), 3.into());
        })
    });
}

/// Stub model for subscribing to RTE model events.
struct Observer {}

impl Entity for Observer {
    type Event = ();
}

#[test]
fn test_debounced_resizes() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);

        let model_handle = model_from_markdown("This is resizable text", &mut app, true);

        let (events_tx, events_rx) = async_channel::unbounded();
        let render_state = app.read(|ctx| model_handle.as_ref(ctx).render_state().clone());
        let model2 = render_state.clone();
        let _observer = app.add_model::<Observer, _>(move |ctx| {
            ctx.subscribe_to_model(&model2, move |_, event, _| {
                block_on(events_tx.send(*event)).unwrap();
            });
            Observer {}
        });

        let size_info = SizeInfo {
            viewport_size: Vector2F::zero(),
            needs_layout: true,
        };

        // Simulate several resizes, spread across updates.
        render_state.update(&mut app, |render_state, ctx| {
            render_state.set_viewport_size(size_info, ctx)
        });
        render_state.update(&mut app, |render_state, ctx| {
            render_state.set_viewport_size(size_info, ctx)
        });
        render_state.update(&mut app, |render_state, ctx| {
            render_state.set_viewport_size(size_info, ctx)
        });

        // The resizes should lead to a single re-layout.
        // There is an extra layout updated here which we don't quite understand. It is coming from a rebuild
        // layout call and doesn't increase / decrease with the number of consecutive calls. This also doesn't
        // happen on a local build when tested manually.
        // TODO(kevin): Figure out why this is happening.
        assert_eq!(events_rx.recv().await, Ok(RenderEvent::NeedsResize));
        assert_eq!(events_rx.recv().await, Ok(RenderEvent::NeedsResize));
        assert_eq!(events_rx.recv().await, Ok(RenderEvent::NeedsResize));
        assert_eq!(events_rx.recv().await, Ok(RenderEvent::LayoutUpdated));
        assert_eq!(events_rx.recv().await, Ok(RenderEvent::LayoutUpdated));

        // Resize again after the debounce period.
        Timer::after(DEBOUNCED_RESIZE_PERIOD).await;
        render_state.update(&mut app, |render_state, ctx| {
            render_state.set_viewport_size(size_info, ctx)
        });

        // This should cause another re-layout.
        assert_eq!(events_rx.recv().await, Ok(RenderEvent::NeedsResize));
        assert_eq!(events_rx.recv().await, Ok(RenderEvent::LayoutUpdated));

        // Drop the model and force an update, so that the stream completes.
        drop(model_handle);
        drop(render_state);
        app.update(|_| {});
        assert!(events_rx.is_empty() && events_rx.is_closed());
    });
}

/// The starting offset of each selected command.
fn selected_commands(model: &NotebooksEditorModel, ctx: &AppContext) -> Vec<CharOffset> {
    model
        .selected_commands(ctx)
        .map(|(start, _)| start)
        .collect_vec()
}

#[test]
fn test_cursor_to_command_selection() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);

        let model_handle = model_from_markdown(
            r#"```
A command
```

Text"#,
            &mut app,
            true,
        );
        layout_model(&mut app, &model_handle).await;

        model_handle.update(&mut app, |model, ctx| {
            // For each position within the command, put a cursor there and convert it to a command selection.
            for i in 1..10 {
                model.cursor_at(i.into(), ctx);
                model.select_command_at_cursor(ctx);
                assert_eq!(selected_commands(model, ctx), vec![CharOffset::zero()]);

                model.clear_command_selections(ctx);
            }

            // If plain text is selected, it should not convert to a command selection.
            model.cursor_at(14.into(), ctx);
            model.select_command_at_cursor(ctx);
            assert!(!model.has_command_selection(ctx));
        });
    });
}

#[test]
fn semantic_selection_clears_command_selection_and_opposite() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);

        let model_handle = model_from_markdown(
            r#"```
A command
```

Text"#,
            &mut app,
            true,
        );
        layout_model(&mut app, &model_handle).await;

        model_handle.update(&mut app, |model, ctx| {
            // Select command — note that select_command_at takes the block start,
            // not an arbitrary position in the block (use select_command_at_cursor)
            model.select_command_at(0.into(), ctx);
            assert!(model.has_command_selection(ctx));

            // Select word, ensuring command selection is cleared
            model.select_word_at(14.into(), false, ctx);
            assert!(!model.has_command_selection(ctx));
            assert!(model.selected_text(ctx) == *"Text");

            // Select command again, ensuring semantic selection is cleared
            model.select_command_at(0.into(), ctx);
            assert!(model.has_command_selection(ctx));
            assert!(model.selected_text(ctx) == String::new());
        })
    })
}

#[test]
fn test_syntax_highlighting_in_command() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let mut seen_futures = HashSet::new();

        let model_handle = model_from_markdown(
            r#"Text
```
git checkout
```
More text
```
cargo run
```"#,
            &mut app,
            true,
        );

        finish_highlighting(&model_handle, 2, &mut seen_futures, &mut app).await;
        model_handle.update(&mut app, |model, ctx| {
            assert_eq!(
                model.debug_buffer(ctx),
                "<text>Text<code:Shell><c_#b4fa72>git<c> <c_#a5d5fe>checkout<c><text>More text<code:Shell><c_#b4fa72>cargo<c> <c_#a5d5fe>run<c><text>"
            );

            model.cursor_at(CharOffset::from(18), ctx);
            model.user_insert(" -b", ctx);
        });

        finish_highlighting(&model_handle, 1, &mut seen_futures, &mut app).await;
        model_handle.update(&mut app, |model, ctx| {
            assert_eq!(
                model.debug_buffer(ctx),
                "<text>Text<code:Shell><c_#b4fa72>git<c> <c_#a5d5fe>checkout<c> <c_#fefdc2>-b<c><text>More text<code:Shell><c_#b4fa72>cargo<c> <c_#a5d5fe>run<c><text>"
            );

            model.undo(ctx);
        });

        finish_highlighting(&model_handle, 1, &mut seen_futures, &mut app).await;
        model_handle.update(&mut app, |model, ctx| {
            assert_eq!(
                model.debug_buffer(ctx),
                "<text>Text<code:Shell><c_#b4fa72>git<c> <c_#a5d5fe>checkout<c><text>More text<code:Shell><c_#b4fa72>cargo<c> <c_#a5d5fe>run<c><text>"
            );

            model.redo(ctx);
        });

        finish_highlighting(&model_handle, 1, &mut seen_futures, &mut app).await;
        model_handle.update(&mut app, |model, ctx| {
            assert_eq!(
                model.debug_buffer(ctx),
                "<text>Text<code:Shell><c_#b4fa72>git<c> <c_#a5d5fe>checkout<c> <c_#fefdc2>-b<c><text>More text<code:Shell><c_#b4fa72>cargo<c> <c_#a5d5fe>run<c><text>"
            );
        });
    });
}

// Regression test for CLD-1257.
#[test]
fn test_delete_block_after_highlighted_block() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let mut seen_futures = HashSet::new();

        let model_handle = model_from_markdown(
            r#"Text
```
git checkout
```
a
```
cargo run
```"#,
            &mut app,
            true,
        );

        finish_highlighting(&model_handle, 2, &mut seen_futures, &mut app).await;
        model_handle.update(&mut app, |model, ctx| {
            assert_eq!(
                model.debug_buffer(ctx),
                "<text>Text<code:Shell><c_#b4fa72>git<c> <c_#a5d5fe>checkout<c><text>a<code:Shell><c_#b4fa72>cargo<c> <c_#a5d5fe>run<c><text>"
            );

            model.cursor_at(CharOffset::from(20), ctx);
            model.backspace(ctx);
            model.backspace(ctx);
        });

        // We shouldn't need to wait for highlighting here since the cached color should be applied right away.
        model_handle.update(&mut app, |model, ctx| {
            assert_eq!(
                model.debug_buffer(ctx),
                "<text>Text<code:Shell><c_#b4fa72>git<c> <c_#a5d5fe>checkout<c><code:Shell><c_#b4fa72>cargo<c> <c_#a5d5fe>run<c><text>"
            );
        });
    });
}

#[test]
fn test_text_to_command_selection() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);

        let model_handle = model_from_markdown(
            r#"Text
```
First command
```
More text
```
Second command
```"#,
            &mut app,
            true,
        );
        layout_model(&mut app, &model_handle).await;

        // From the first line of text, we can't select a command above, but we can select a command below.
        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(CharOffset::from(2), ctx);

            model.select_command_up(ctx);
            assert!(!model.has_command_selection(ctx));

            model.select_command_down(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(5)]);
        });

        // From within a command, selecting up should target that command.
        model_handle.update(&mut app, |model, ctx| {
            model.clear_command_selections(ctx);
            model.cursor_at(CharOffset::from(7), ctx);

            model.select_command_up(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(5)]);
        });

        // Selecting down in that command targets the _next_ command.
        model_handle.update(&mut app, |model, ctx| {
            model.clear_command_selections(ctx);
            model.cursor_at(CharOffset::from(7), ctx);

            model.select_command_down(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(29)]);
        });

        // From the middle line of text, we can select up to the first command or down to the second.
        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(CharOffset::from(20), ctx);

            model.select_command_up(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(5)]);

            model.clear_command_selections(ctx);
            model.cursor_at(CharOffset::from(20), ctx);

            model.select_command_down(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(29)]);
        });
    });
}

#[test]
fn test_command_to_text_selection() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);

        let model_handle = model_from_markdown(
            r#"Text
```
First command
```
More text
```
Second command
```"#,
            &mut app,
            true,
        );
        layout_model(&mut app, &model_handle).await;

        // Clearing command selections should put a cursor at the end of the selected command.
        // We should update this test once command multi-selection is supported.
        model_handle.update(&mut app, |model, ctx| {
            // Select the first command.
            model.cursor_at(CharOffset::zero(), ctx);
            model.select_command_down(ctx);
            assert!(model.has_command_selection(ctx));

            model.exit_command_selection(ctx);
            assert!(!model.has_command_selection(ctx));
            let buffer_selection = model.buffer_selection_model().as_ref(ctx);
            assert!(buffer_selection.first_selection_is_single_cursor());
            assert_eq!(
                buffer_selection.first_selection_head(),
                CharOffset::from(19)
            );
        });
    });
}

#[test]
fn test_move_command_selection() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);

        let model_handle = model_from_markdown(
            r#"Text
```
First command
```
More text
```
Second command
```"#,
            &mut app,
            true,
        );
        layout_model(&mut app, &model_handle).await;

        model_handle.update(&mut app, |model, ctx| {
            // Select the first command.
            model.cursor_at(CharOffset::zero(), ctx);
            model.select_command_down(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(5)]);

            // Moving up has no effect, since there's no previous command.
            model.select_command_up(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(5)]);

            // However, we can move down to the next command.
            model.select_command_down(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(29)]);

            // From the last command, moving down again has no effect.
            model.select_command_down(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(29)]);

            // We can move back up to the first command.
            model.select_command_up(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(5)]);
        });
    });
}

// Mock out a server workflow with the given i64 ID.
fn mock_server_workflow(id: i64, app: &mut App) {
    let server_id: ServerId = id.into();
    let workflow_id: WorkflowId = server_id.into();
    let sync_id = SyncId::ServerId(workflow_id.into());
    let ts = Utc::now();

    let server_metadata = ServerMetadata {
        uid: server_id,
        revision: Revision::now(),
        metadata_last_updated_ts: ts.into(),
        trashed_ts: None,
        folder_id: None,
        is_welcome_object: false,
        creator_uid: None,
        last_editor_uid: None,
        current_editor_uid: None,
    };

    let workflow = ServerWorkflow {
        id: SyncId::ServerId(workflow_id.into()),
        metadata: server_metadata,
        permissions: ServerPermissions {
            space: Owner::mock_current_user(),
            guests: Vec::new(),
            permissions_last_updated_ts: ts.into(),
            anyone_link_sharing: None,
        },
        model: CloudWorkflowModel::new(Workflow::new(format!("w{id}"), format!("c{id}"))),
    };

    CloudModel::handle(app).update(app, |cloud_model, _| {
        cloud_model.add_object(sync_id, CloudWorkflow::new_from_server(workflow));
    });
}

#[test]
fn test_interleaving_command_and_embedding() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        app.add_singleton_model(CloudModel::mock);

        // IDs are padded to be length 22.
        mock_server_workflow(123, &mut app);
        mock_server_workflow(245, &mut app);

        let model_handle = model_from_markdown(
            r#"Text
```warp-embedded-object
id: Workflow-test_uid00000000000123
```
More text
```warp-embedded-object
id: Workflow-test_uid00000000000245
```
```Python
def
```
```
First command
```"#,
            &mut app,
            false,
        );
        layout_model(&mut app, &model_handle).await;

        // From the first line of text, we can't select a command above, but we can select a command below.
        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(CharOffset::from(2), ctx);

            model.select_command_up(ctx);
            assert!(!model.has_command_selection(ctx));

            // Embedded workflows should be selectable.
            model.select_command_down(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(5)]);

            // Should jump to the next embedded workflow.
            model.select_command_down(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(16)]);

            // Should do the python block next
            model.select_command_down(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(17)]);

            // Now the last block
            model.select_command_down(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(21)]);
        });

        model_handle.update(&mut app, |model, ctx| {
            model.clear_command_selections(ctx);
            model.cursor_at(CharOffset::from(25), ctx);

            model.select_command_up(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(21)]);

            model.select_command_up(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(17)]);

            model.select_command_up(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(16)]);

            model.select_command_up(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(5)]);
        });

        // Test command selection behavior on edge of block offsets.
        model_handle.update(&mut app, |model, ctx| {
            model.clear_command_selections(ctx);
            // Place the cursor right before an embedded block.
            model.cursor_at(CharOffset::from(5), ctx);

            // Visually the cursor position is before the embedded block, this operaton should be a no-op.
            model.select_command_up(ctx);
            assert!(!model.has_command_selection(ctx));

            // Visually the cursor position is before the embedded block, this should select embedded block at offset.
            model.select_command_down(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(5)]);
        });

        model_handle.update(&mut app, |model, ctx| {
            model.clear_command_selections(ctx);
            model.cursor_at(CharOffset::from(6), ctx);

            // Visually the cursor position is after the embedded block, this operaton should select the embedded block.
            model.select_command_up(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(5)]);
        });
    })
}

#[test]
fn test_toggle_style_at_cursor() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let model_handle = model_from_markdown("Hello", &mut app, true);

        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(2.into(), ctx);

            // Toggle to bold, then insert text.
            model.toggle_style(TextStyles::default().bold(), ctx);
            model.user_insert("b", ctx);
            assert_eq!(model.debug_buffer(ctx), "<text>H<b_s>b<b_e>ello");

            // Now, toggle the style off (still with no selection).
            model.toggle_style(TextStyles::default().bold(), ctx);
            model.user_insert("x", ctx);
            assert_eq!(model.debug_buffer(ctx), "<text>H<b_s>b<b_e>xello");

            // Cursor-toggled styles should stack.
            model.toggle_style(TextStyles::default().bold(), ctx);
            model.user_insert("a", ctx);
            model.toggle_style(TextStyles::default().italic(), ctx);
            model.user_insert("b", ctx);
            model.toggle_style(TextStyles::default().bold(), ctx);
            model.user_insert("c", ctx);
            assert_eq!(
                model.debug_buffer(ctx),
                "<text>H<b_s>b<b_e>x<b_s>a<i_s>b<b_e>c<i_e>ello"
            );
        })
    })
}

#[test]
fn test_moving_resets_cursor_styles() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let model_handle = model_from_markdown("Hello", &mut app, true);

        // First, make sure the active style at the cursor is italic.
        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(2.into(), ctx);
            model.toggle_style(TextStyles::default().italic(), ctx);
            assert!(model.active_text_style.is_italic());
        });

        // Now, move to a non-italic character.
        model_handle.update(&mut app, |model, ctx| model.move_right(ctx));

        model_handle.update(&mut app, |model, ctx| {
            assert!(!model.active_text_style.is_italic());
            model.user_insert("t", ctx);
            assert_eq!(model.debug_buffer(ctx), "<text>Hetllo");
        });
    })
}

#[test]
fn test_movement_cut() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let model_handle: ModelHandle<NotebooksEditorModel> =
            model_from_markdown("**First** line\nSecond line\n", &mut app, true);
        layout_model(&mut app, &model_handle).await;

        model_handle.update(&mut app, |model, ctx| {
            // Position the cursor part of the way through styled text, and use a movement-based cut.
            // This tests that we cut the right range, both from the buffer and into the clipboard.
            model.cursor_at(4.into(), ctx);

            model.delete(TextDirection::Forwards, TextUnit::LineBoundary, true, ctx);
            assert_eq!(model.debug_buffer(ctx), "<text><b_s>Fir<b_e>\\nSecond line");
            let clipboard = ctx.clipboard().read();
            assert_eq!(&clipboard.plain_text, "st line");
            assert_eq!(
                clipboard.html.as_deref(),
                Some("<p><strong>st</strong> line</p>")
            );
            assert!(model.selection_is_single_cursor(ctx));
            assert_eq!(
                model.selection.as_ref(ctx).cursors(ctx)[0],
                CharOffset::from(4)
            );
        });

        // Undoing should preserve the original cursor position as well.
        model_handle.update(&mut app, |model, ctx| {
            model.undo(ctx);
            assert_eq!(
                model.debug_buffer(ctx),
                "<text><b_s>First<b_e> line\\nSecond line"
            );
            assert!(model.selection_is_single_cursor(ctx));
            assert_eq!(
                model.selection.as_ref(ctx).cursors(ctx)[0],
                CharOffset::from(4)
            );
        })
    });
}

// Regression test for CLD-624.
#[test]
fn test_paste_multiline_into_code_blocks() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let model_handle: ModelHandle<NotebooksEditorModel> =
            model_from_markdown("text\n```\ncode\n```\n", &mut app, true);

        model_handle.update(&mut app, |model, ctx| {
            // Position the cursor at the end of the code block.
            model.cursor_at(10.into(), ctx);

            model.insert_formatted_from_paste(
                FormattedText::new([FormattedTextLine::Line(vec![
                    FormattedTextFragment::plain_text("abc"),
                    FormattedTextFragment::plain_text("def"),
                ])]),
                "abc\ndef",
                ctx,
            );
            assert_eq!(
                model.debug_buffer(ctx),
                "<text>text<code:Shell>codeabc\\ndef<text>"
            );
        });

        // Undoing should preserve the original cursor position as well.
        model_handle.update(&mut app, |model, ctx| {
            model.undo(ctx);
            assert_eq!(model.debug_buffer(ctx), "<text>text<code:Shell>code<text>");
        })
    });
}

#[test]
fn test_delete_word_backwards() {
    // This tests that deleting backwards relative to the cursor works correctly.
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let model_handle: ModelHandle<NotebooksEditorModel> =
            model_from_markdown("**First** line\nSecond line\n", &mut app, true);

        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(15.into(), ctx);
            model.delete(
                TextDirection::Backwards,
                TextUnit::Word(WordBoundariesPolicy::Default),
                false,
                ctx,
            );
            assert_eq!(
                model.debug_buffer(ctx),
                "<text><b_s>First<b_e> line\\nond line"
            );

            assert!(model.selection_is_single_cursor(ctx));
            assert_eq!(
                model.selection.as_ref(ctx).cursors(ctx)[0],
                CharOffset::from(12)
            );

            // If cut is false, this should not touch the clipboard.
            let clipboard = ctx.clipboard().read();
            assert_eq!(&clipboard.plain_text, "");
            assert_eq!(clipboard.html, None);
        });

        // Undoing should shift the cursor to its original location.
        model_handle.update(&mut app, |model, ctx| {
            model.undo(ctx);
            assert_eq!(
                model.debug_buffer(ctx),
                "<text><b_s>First<b_e> line\\nSecond line"
            );
            assert!(model.selection_is_single_cursor(ctx));
            assert_eq!(
                model.selection.as_ref(ctx).cursors(ctx)[0],
                CharOffset::from(15)
            );
        })
    });
}

#[test]
fn test_delete_with_selection() {
    // If text is selected, any delete operation should instead act as backspace.
    // This tests that deleting backwards relative to the cursor works correctly.
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let model_handle: ModelHandle<NotebooksEditorModel> =
            model_from_markdown("**First** line\nSecond line\n", &mut app, true);

        model_handle.update(&mut app, |model, ctx| {
            // Select `irs`.
            model.cursor_at(CharOffset::from(2), ctx);
            model.select_right(ctx);
            model.select_right(ctx);
            model.select_right(ctx);

            // Without a selection, this would delete to the end of the line. Instead, it should
            // delete just the selected text.
            model.delete(TextDirection::Forwards, TextUnit::LineBoundary, false, ctx);

            assert_eq!(
                model.debug_buffer(ctx),
                "<text><b_s>Ft<b_e> line\\nSecond line"
            );

            assert!(model.selection_is_single_cursor(ctx));
            assert_eq!(
                model.selection.as_ref(ctx).cursors(ctx)[0],
                CharOffset::from(2)
            );
        });
    });
}

#[test]
fn test_backspace_with_command_selection() {
    App::test((), |mut app| async move {
        let mut highlighting_futures = Default::default();
        initialize_deps(&mut app);

        let model_handle = model_from_markdown(
            r#"Text
```
command
```
More text"#,
            &mut app,
            true,
        );
        // Ensure the code block is highlighted to prevent flakiness.
        finish_highlighting(&model_handle, 1, &mut highlighting_futures, &mut app).await;

        // Select and delete the command block.
        model_handle.update(&mut app, |model, ctx| {
            model.select_command_at(5.into(), ctx);
            assert!(model.has_command_selection(ctx));
            model.backspace(ctx);
        });
        layout_model(&mut app, &model_handle).await;

        // Afterwards, there should be no command selection and the cursor should be at the
        // deletion location.
        model_handle.read(&app, |model, ctx| {
            assert_eq!(model.debug_buffer(ctx), "<text>Text\\nMore text");
            assert!(model.selection_is_single_cursor(ctx));
            assert_eq!(
                model.selection.as_ref(ctx).cursors(ctx)[0],
                CharOffset::from(5)
            );
            assert!(!model.has_command_selection(ctx));
        });

        // Undo restores the command block and places the cursor back at the deletion location.
        model_handle.update(&mut app, |model, ctx| model.undo(ctx));
        finish_highlighting(&model_handle, 1, &mut highlighting_futures, &mut app).await;

        model_handle.read(&app, |model, ctx| {
            assert_eq!(
                model.debug_buffer(ctx),
                "<text>Text<code:Shell><c_#b4fa72>command<c><text>More text"
            );
            assert!(!model.has_command_selection(ctx));
            let selection = model.selection.as_ref(ctx);
            assert_eq!(selection.selection_start(ctx), CharOffset::from(5));
            assert_eq!(selection.selection_end(ctx), CharOffset::from(5));
        });
    });
}

#[test]
fn test_delete_with_mermaid_command_selection() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        let _editable_flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let model_handle = model_from_markdown(
            "Text\n```mermaid\ngraph TD\nA --> B\n```\nMore text",
            &mut app,
            true,
        );
        layout_model(&mut app, &model_handle).await;

        model_handle.update(&mut app, |model, ctx| {
            model.select_command_at(5.into(), ctx);
            assert!(model.has_command_selection(ctx));
            model.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
        });
        layout_model(&mut app, &model_handle).await;

        model_handle.read(&app, |model, ctx| {
            assert_eq!(model.debug_buffer(ctx), "<text>Text\\nMore text");
            assert!(model.selection_is_single_cursor(ctx));
            assert_eq!(
                model.selection.as_ref(ctx).cursors(ctx)[0],
                CharOffset::from(5)
            );
            assert!(!model.has_command_selection(ctx));
        });
    });
}

#[test]
fn test_adjacent_delete_with_rendered_mermaid_block_is_atomic() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        let _editable_flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let markdown = "Text
```mermaid
graph TD
A --> B
```
More text";

        let model_handle = model_from_markdown(markdown, &mut app, true);
        model_handle.update(&mut app, |model, ctx| {
            model.set_interaction_state(InteractionState::Editable, ctx);
        });
        layout_model(&mut app, &model_handle).await;

        model_handle.update(&mut app, |model, ctx| {
            model.set_mermaid_render_mode(CharOffset::from(5), MarkdownDisplayMode::Rendered, ctx);
        });
        layout_model(&mut app, &model_handle).await;

        let mermaid_command = command_models(&model_handle, &mut app)
            .into_iter()
            .exactly_one()
            .expect("Mermaid command should exist");
        let mermaid_range = command_range(&mermaid_command, &mut app);
        let cursor_after_mermaid = mermaid_range.end + CharOffset::from(1);

        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(cursor_after_mermaid, ctx);
            assert!(!model.has_command_selection(ctx));
            model.backspace(ctx);
        });
        layout_model(&mut app, &model_handle).await;

        model_handle.read(&app, |model, ctx| {
            assert_eq!(model.debug_buffer(ctx), "<text>Text\\nMore text");
            assert!(model.selection_is_single_cursor(ctx));
            assert_eq!(
                model.selection.as_ref(ctx).cursors(ctx)[0],
                mermaid_range.start
            );
            assert!(!model.has_command_selection(ctx));
        });

        model_handle.update(&mut app, |model, ctx| model.undo(ctx));
        layout_model(&mut app, &model_handle).await;
        model_handle.update(&mut app, |model, ctx| {
            model.set_mermaid_render_mode(CharOffset::from(5), MarkdownDisplayMode::Rendered, ctx);
        });
        layout_model(&mut app, &model_handle).await;

        let mermaid_command = command_models(&model_handle, &mut app)
            .into_iter()
            .exactly_one()
            .expect("Mermaid command should exist after undo");
        let mermaid_range = command_range(&mermaid_command, &mut app);

        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(mermaid_range.start, ctx);
            assert!(!model.has_command_selection(ctx));
            model.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
        });
        layout_model(&mut app, &model_handle).await;

        model_handle.read(&app, |model, ctx| {
            assert_eq!(model.debug_buffer(ctx), "<text>Text\\nMore text");
            assert!(model.selection_is_single_cursor(ctx));
            assert_eq!(
                model.selection.as_ref(ctx).cursors(ctx)[0],
                mermaid_range.start
            );
            assert!(!model.has_command_selection(ctx));
        });
    });
}

#[test]
fn test_backspace_with_cursor_inside_rendered_mermaid_block_is_atomic() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        let _editable_flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let markdown = "Text
```mermaid
graph TD
A --> B
```
More text";

        let model_handle = model_from_markdown(markdown, &mut app, true);
        model_handle.update(&mut app, |model, ctx| {
            model.set_interaction_state(InteractionState::Editable, ctx);
        });
        layout_model(&mut app, &model_handle).await;

        model_handle.update(&mut app, |model, ctx| {
            model.set_mermaid_render_mode(CharOffset::from(5), MarkdownDisplayMode::Rendered, ctx);
        });
        layout_model(&mut app, &model_handle).await;

        let mermaid_command = command_models(&model_handle, &mut app)
            .into_iter()
            .exactly_one()
            .expect("Mermaid command should exist");
        let mermaid_range = command_range(&mermaid_command, &mut app);
        let cursor_offset = CharOffset::from(
            markdown
                .find("graph TD")
                .expect("Mermaid source should exist")
                + 3,
        );

        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(cursor_offset, ctx);
            assert!(!model.has_command_selection(ctx));
            model.backspace(ctx);
        });
        layout_model(&mut app, &model_handle).await;

        model_handle.read(&app, |model, ctx| {
            assert_eq!(model.debug_buffer(ctx), "<text>Text\\nMore text");
            assert!(model.selection_is_single_cursor(ctx));
            assert_eq!(
                model.selection.as_ref(ctx).cursors(ctx)[0],
                mermaid_range.start
            );
            assert!(!model.has_command_selection(ctx));
        });
    });
}

#[test]
fn test_move_up_from_below_rendered_mermaid_block_lands_on_block_start() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        let _editable_flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let markdown = "Before\n```mermaid\ngraph TD\nA --> B\n```\nAfter";

        let model_handle = model_from_markdown(markdown, &mut app, true);
        model_handle.update(&mut app, |model, ctx| {
            model.set_interaction_state(InteractionState::Editable, ctx);
        });
        layout_model(&mut app, &model_handle).await;

        // Blocks default to Raw; explicitly enable Rendered mode so the navigation
        // code treats the block as an atomic rendered unit.
        model_handle.update(&mut app, |model, ctx| {
            model.set_mermaid_render_mode(CharOffset::from(7), MarkdownDisplayMode::Rendered, ctx);
        });
        layout_model(&mut app, &model_handle).await;

        let mermaid_command = command_models(&model_handle, &mut app)
            .into_iter()
            .exactly_one()
            .expect("Mermaid command should exist");
        let mermaid_range = command_range(&mermaid_command, &mut app);
        let cursor_after_mermaid = mermaid_range.end + CharOffset::from(1);

        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(cursor_after_mermaid, ctx);
            model.move_up(ctx);
        });

        model_handle.read(&app, |model, ctx| {
            assert!(model.selection_is_single_cursor(ctx));
            assert_eq!(
                model.selection.as_ref(ctx).cursors(ctx)[0],
                mermaid_range.start
            );
        });
    });
}

#[test]
fn test_shift_select_across_rendered_mermaid_block_is_reversible_from_below() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        let _editable_flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let markdown = "Before\n```mermaid\ngraph TD\nA --> B\n```\nAfter";

        let model_handle = model_from_markdown(markdown, &mut app, true);
        model_handle.update(&mut app, |model, ctx| {
            model.set_interaction_state(InteractionState::Editable, ctx);
        });
        layout_model(&mut app, &model_handle).await;

        // Blocks default to Raw; explicitly enable Rendered mode so selection
        // normalization treats the block as an atomic rendered unit.
        model_handle.update(&mut app, |model, ctx| {
            model.set_mermaid_render_mode(CharOffset::from(7), MarkdownDisplayMode::Rendered, ctx);
        });
        layout_model(&mut app, &model_handle).await;

        let mermaid_command = command_models(&model_handle, &mut app)
            .into_iter()
            .exactly_one()
            .expect("Mermaid command should exist");
        let mermaid_range = command_range(&mermaid_command, &mut app);
        let cursor_after_mermaid = mermaid_range.end + CharOffset::from(1);

        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(cursor_after_mermaid, ctx);
            model.select_up(ctx);

            assert_eq!(
                model
                    .buffer_selection_model()
                    .as_ref(ctx)
                    .selection_offsets(),
                vec1![SelectionOffsets {
                    head: mermaid_range.start,
                    tail: cursor_after_mermaid,
                }]
            );

            model.select_down(ctx);
        });

        model_handle.read(&app, |model, ctx| {
            assert!(model.selection_is_single_cursor(ctx));
            assert_eq!(
                model.selection.as_ref(ctx).cursors(ctx)[0],
                cursor_after_mermaid
            );
        });
    });
}

#[test]
fn test_move_down_from_rendered_mermaid_block_start_returns_below_block() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        let _editable_flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let markdown = "Before\n```mermaid\ngraph TD\nA --> B\n```\nAfter";

        let model_handle = model_from_markdown(markdown, &mut app, true);
        model_handle.update(&mut app, |model, ctx| {
            model.set_interaction_state(InteractionState::Editable, ctx);
        });
        layout_model(&mut app, &model_handle).await;

        // Blocks default to Raw; explicitly enable Rendered mode so navigation
        // treats the block as an atomic rendered unit.
        model_handle.update(&mut app, |model, ctx| {
            model.set_mermaid_render_mode(CharOffset::from(7), MarkdownDisplayMode::Rendered, ctx);
        });
        layout_model(&mut app, &model_handle).await;

        let mermaid_command = command_models(&model_handle, &mut app)
            .into_iter()
            .exactly_one()
            .expect("Mermaid command should exist");
        let mermaid_range = command_range(&mermaid_command, &mut app);
        let cursor_after_mermaid = mermaid_range.end + CharOffset::from(1);

        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(mermaid_range.start, ctx);
            model.move_down(ctx);
        });

        model_handle.read(&app, |model, ctx| {
            assert!(model.selection_is_single_cursor(ctx));
            assert_eq!(
                model.selection.as_ref(ctx).cursors(ctx)[0],
                cursor_after_mermaid
            );
        });
    });
}

#[test]
fn test_cut_text() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let model_handle = model_from_markdown("* First\n* Second **line**\n", &mut app, true);
        layout_model(&mut app, &model_handle).await;

        // Cutting with no selection is a no-op.
        model_handle.update(&mut app, |model, ctx| {
            model.cut(ctx);
            assert_eq!(
                model.debug_buffer(ctx),
                "<ul0>First<ul0>Second <b_s>line<b_e><text>"
            );
            assert!(ctx.clipboard().read().plain_text.is_empty());
        });

        // If part of the second list item is cut, we keep its formatting.
        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(11.into(), ctx);
            model.set_last_selection_head(16.into(), ctx);
            model.cut(ctx);

            assert_eq!(
                model.debug_buffer(ctx),
                "<ul0>First<ul0>Seco<b_s>ne<b_e><text>"
            );
            let clipboard = ctx.clipboard().read();
            assert_eq!(clipboard.plain_text, "nd li");
            assert_eq!(
                clipboard.html.unwrap(),
                "<ul><li>nd <strong>li</strong></li></ul>"
            );
        })
    });
}

#[test]
fn test_cut_code_block() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let model_handle = model_from_markdown("Text\n```\ncommand\n```\n* List", &mut app, true);
        layout_model(&mut app, &model_handle).await;

        model_handle.update(&mut app, |model, ctx| {
            model.select_command_at(5.into(), ctx);
            assert!(model.has_command_selection(ctx));

            model.cut(ctx);

            // Since a command is selected, its entire block should be cut.
            assert_eq!(model.debug_buffer(ctx), "<text>Text<ul0>List<text>");
            let clipboard = ctx.clipboard().read();
            assert_eq!(clipboard.plain_text, "command");
            assert_eq!(
                clipboard.html.unwrap(),
                r#"<pre><code class="language-warp-runnable-command">command</code></pre>"#
            );
        })
    });
}

#[test]
fn test_cut_mermaid_code_block_uses_fenced_markdown_plain_text() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        let _editable_flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let model_handle = model_from_markdown(
            "Text\n```mermaid\ngraph TD\nA --> B\n```\n* List",
            &mut app,
            true,
        );
        layout_model(&mut app, &model_handle).await;

        model_handle.update(&mut app, |model, ctx| {
            model.select_command_at(5.into(), ctx);
            assert!(model.has_command_selection(ctx));

            model.cut(ctx);

            assert_eq!(model.debug_buffer(ctx), "<text>Text<ul0>List<text>");
            let clipboard = ctx.clipboard().read();
            assert_eq!(clipboard.plain_text, "```mermaid\ngraph TD\nA --> B\n```");
            assert!(clipboard
                .html
                .as_deref()
                .is_some_and(|html| html.contains("language-mermaid")));
            assert!(clipboard
                .html
                .as_deref()
                .is_some_and(|html| html.contains("data:image/svg+xml;base64,")));
            assert!(clipboard.images.is_none());
        });
    });
}

#[test]
fn test_copy_mermaid_code_block_adds_html_without_image_clipboard_data() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let _flag = FeatureFlag::MarkdownMermaid.override_enabled(true);
        let _editable_flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let model_handle = model_from_markdown(
            "Text\n```mermaid\ngraph TD\nA --> B\n```\n* List",
            &mut app,
            true,
        );
        layout_model(&mut app, &model_handle).await;

        model_handle.update(&mut app, |model, ctx| {
            model.select_command_at(5.into(), ctx);
            assert!(model.has_command_selection(ctx));

            model.copy(ctx);

            let clipboard = ctx.clipboard().read();
            assert_eq!(clipboard.plain_text, "```mermaid\ngraph TD\nA --> B\n```");
            assert!(clipboard
                .html
                .as_deref()
                .is_some_and(|html| html.contains("language-mermaid")));
            assert!(clipboard
                .html
                .as_deref()
                .is_some_and(|html| html.contains("data:image/svg+xml;base64,")));
            assert!(clipboard.images.is_none());
        })
    });
}

#[test]
fn test_copy_selection_with_markdown_image_omits_image_clipboard_data() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);

        let model_handle =
            model_from_markdown("Before\n![Alt text](diagram.png)\nAfter", &mut app, true);
        layout_model(&mut app, &model_handle).await;

        model_handle.update(&mut app, |model, ctx| {
            model.selection.update(ctx, |selection, ctx| {
                selection.update_selection(
                    BufferSelectAction::SelectAll,
                    AutoScrollBehavior::None,
                    ctx,
                );
            });

            model.copy(ctx);

            let clipboard = ctx.clipboard().read();
            assert!(clipboard.plain_text.contains("![Alt text](diagram.png)"));
            assert!(clipboard
                .html
                .as_deref()
                .is_some_and(|html| html.contains("<img")));
            assert!(clipboard.images.is_none());
        })
    });
}

#[test]
fn test_mermaid_feature_flag_disables_rendering_and_toggle() {
    // With the flag disabled, Mermaid blocks never render as diagrams.
    // With the flag enabled, blocks default to Raw mode — no auto-rendering.
    // Diagram rendering only happens when the user explicitly selects Rendered.
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let markdown = "```mermaid\ngraph TD\nA --> B\n```";

        let _disabled = FeatureFlag::MarkdownMermaid.override_enabled(false);
        let model_handle = model_from_markdown(markdown, &mut app, true);
        model_handle.update(&mut app, |model, ctx| {
            model.set_interaction_state(InteractionState::Selectable, ctx);
        });
        layout_model(&mut app, &model_handle).await;

        let disabled_is_mermaid_diagram = model_handle.read(&app, |model, ctx| {
            matches!(
                model
                    .render_state
                    .as_ref(ctx)
                    .content()
                    .block_at_height(0.)
                    .map(|item| item.item),
                Some(BlockItem::MermaidDiagram { .. })
            )
        });
        assert!(!disabled_is_mermaid_diagram);

        drop(_disabled);

        let _enabled = FeatureFlag::MarkdownMermaid.override_enabled(true);
        model_handle.update(&mut app, |model, ctx| {
            model.reset_with_markdown(markdown, ctx);
            model.set_interaction_state(InteractionState::Selectable, ctx);
        });
        layout_model(&mut app, &model_handle).await;

        // Even with the flag enabled, blocks default to Raw mode — not auto-rendered.
        let enabled_defaults_to_raw = model_handle.read(&app, |model, ctx| {
            !matches!(
                model
                    .render_state
                    .as_ref(ctx)
                    .content()
                    .block_at_height(0.)
                    .map(|item| item.item),
                Some(BlockItem::MermaidDiagram { .. })
            )
        });
        assert!(enabled_defaults_to_raw);
    });
}

#[test]
fn test_default_mermaid_display_mode_renders_initial_mermaid_blocks() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let _enabled = FeatureFlag::MarkdownMermaid.override_enabled(true);
        let markdown = "```mermaid\ngraph TD\nA --> B\n```";

        let model_handle = model_from_markdown(markdown, &mut app, true);
        model_handle.update(&mut app, |model, ctx| {
            model.set_default_mermaid_display_mode(MarkdownDisplayMode::Rendered, ctx);
            model.set_interaction_state(InteractionState::Selectable, ctx);
        });
        layout_model(&mut app, &model_handle).await;
        layout_model(&mut app, &model_handle).await;

        let command = command_models(&model_handle, &mut app)
            .into_iter()
            .exactly_one()
            .expect("Mermaid command should exist");
        command.read(&app, |command, _| {
            assert_eq!(command.mermaid_display_mode, MarkdownDisplayMode::Rendered);
        });

        let is_mermaid_diagram = model_handle.read(&app, |model, ctx| {
            matches!(
                model
                    .render_state
                    .as_ref(ctx)
                    .content()
                    .block_at_height(0.)
                    .map(|item| item.item),
                Some(BlockItem::MermaidDiagram { .. })
            )
        });
        assert!(is_mermaid_diagram);
    });
}
#[test]
fn test_dont_invalidate_command_selection() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);

        let model_handle = model_from_markdown("Hello\n```\necho test\n```\nworld", &mut app, true);
        layout_model(&mut app, &model_handle).await;
        let command_model = command_models(&model_handle, &mut app)
            .into_iter()
            .exactly_one()
            .expect("Command model should exist");
        let start_anchor = command_model.read(&app, |model, _| model.start_anchor());

        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(CharOffset::from(1), ctx);
            model.select_command_down(ctx);
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(6)]);
        });

        model_handle.update(&mut app, |model, ctx| {
            model.update_code_block_type_at_offset(
                &CodeBlockType::Code {
                    lang: "Python".to_string(),
                },
                start_anchor,
                ctx,
            )
        });
        layout_model(&mut app, &model_handle).await;

        // Updating block type to code should not invalidate selection.
        model_handle.read(&app, |model, ctx| {
            assert_eq!(selected_commands(model, ctx), vec![CharOffset::from(6)]);
        });
    })
}

#[test]
fn test_insert_block_after_single_cursor() {
    // Ensure that after inserting a block, there is only a single cursor in the correct spot.
    App::test((), |mut app| async move {
        initialize_deps(&mut app);

        let model_handle = model_from_markdown("Hello\n```\necho test\n```\nworld", &mut app, true);

        assert_eq!(
            model_handle.read(&app, |model, ctx| model.content().as_ref(ctx).debug()),
            "<text>Hello<code:Shell>echo test<text>world"
        );

        model_handle.update(&mut app, |model, ctx| {
            model.cursor_at(1.into(), ctx);
            model.add_cursor_at(7.into(), ctx);
        });

        assert_eq!(
            model_handle.read(&app, |model, ctx| model
                .selection_model()
                .as_ref(ctx)
                .selections(ctx)
                .len()),
            2
        );

        model_handle.update(&mut app, |model, ctx| {
            let new_block = BlockType::Text(BufferBlockStyle::CodeBlock {
                code_block_type: CodeBlockType::Shell,
            });
            model.insert_block_after(17.into(), new_block, ctx);
        });

        // Content should be correct and the only cursor should be in the inserted code block.
        assert_eq!(
            model_handle.read(&app, |model, ctx| model.content().as_ref(ctx).debug()),
            "<text>Hello<code:Shell>echo test<text>world<code:Shell><text>"
        );
        assert_eq!(
            model_handle.read(&app, |model, ctx| model
                .selection_model()
                .as_ref(ctx)
                .selections(ctx)),
            vec1![SelectionOffsets {
                head: 23.into(),
                tail: 23.into()
            }]
        );
    });
}

#[test]
fn test_multiselect_markdown_block_conversion() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown(
            "This is a list:\n1. First\n2. Second\n3. Third",
            &mut app,
            true,
        );

        editor.update(&mut app, |editor, ctx| {
            assert_eq!(
                editor.debug_buffer(ctx),
                "<text>This is a list:<ol0@1>First<ol0>Second<ol0>Third<text>"
            );

            // Add two cursors.
            editor.cursor_at(17.into(), ctx);
            editor.add_cursor_at(23.into(), ctx);
            editor.user_insert("#", ctx);
            editor.user_insert(" ", ctx);

            assert_eq!(
                editor.debug_buffer(ctx),
                "<text>This is a list:<header1>First<header1>Second<ol0>Third<text>"
            );

            editor.undo(ctx);
            assert_eq!(
                editor.debug_buffer(ctx),
                "<text>This is a list:<ol0@1># First<ol0># Second<ol0>Third<text>"
            );
        });
    });
}

#[test]
fn test_multiselect_inline_markdown() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown(
            "This is a list:\n1. First\n2. Second\n3. Third",
            &mut app,
            true,
        );

        editor.update(&mut app, |editor, ctx| {
            assert_eq!(
                editor.debug_buffer(ctx),
                "<text>This is a list:<ol0@1>First<ol0>Second<ol0>Third<text>"
            );

            // Add two cursors.
            editor.cursor_at(17.into(), ctx);
            editor.add_cursor_at(23.into(), ctx);
            editor.user_insert("**this is bold*", ctx);
            editor.user_insert("*", ctx);

            assert_eq!(
                editor.debug_buffer(ctx),
                "<text>This is a list:<ol0@1><b_s>this is bold<b_e>First<ol0><b_s>this is bold<b_e>Second<ol0>Third<text>"
            );

            editor.undo(ctx);
            assert_eq!(
                editor.debug_buffer(ctx),
                "<text>This is a list:<ol0@1>**this is bold**First<ol0>**this is bold**Second<ol0>Third<text>"
            );
        });
    });
}

#[test]
fn test_multiselect_pasting() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown(
            "First text\nSecond line\nThird line\n```\ncode\n```",
            &mut app,
            true,
        );
        finish_highlighting(&editor, 1, &mut HashSet::new(), &mut app).await;

        editor.update(&mut app, |editor, ctx| {
            assert_eq!(
                editor.debug_buffer(ctx),
                "<text>First text\\nSecond line\\nThird line<code:Shell><c_#b4fa72>code<c><text>"
            );
        });

        let plain_clipboard_content = "text";

        layout_model(&mut app, &editor).await;

        // Pasting plain text should replace each selection.
        editor.update(&mut app, |editor, ctx| {
            editor.cursor_at(CharOffset::from(1), ctx);
            editor.add_cursor_at(24.into(), ctx);
            editor.select_to_line_end(ctx);
        });

        let markdown = parse_markdown(plain_clipboard_content).expect("Should parse plain text");
        editor.update(&mut app, |editor, ctx| {
            editor.insert_formatted_from_paste(markdown, plain_clipboard_content, ctx);
            assert_eq!(
                editor.debug_buffer(ctx),
                "<text>text\\nSecond line\\ntext<code:Shell><c_#b4fa72>code<c><text>"
            );
        });

        // Pasting a URL for multiple selections should paste as text.
        let url_clipboard_content = "https://warp.dev";

        editor.update(&mut app, |editor, ctx| {
            editor.cursor_at(CharOffset::from(1), ctx);
            editor.set_last_selection_head(5.into(), ctx);
            editor.add_cursor_at(18.into(), ctx);
            editor.set_last_selection_head(22.into(), ctx);
        });

        let markdown = parse_markdown(url_clipboard_content).expect("Should parse link");
        editor.update(&mut app, |editor, ctx| {
            editor.insert_formatted_from_paste(markdown, url_clipboard_content, ctx);
            assert_eq!(
                editor.debug_buffer(ctx),
                "<text><a_https://warp.dev>https://warp.dev<a>\\nSecond line\\n<a_https://warp.dev>https://warp.dev<a><code:Shell><c_#b4fa72>code<c><text>"
            );
        });

        editor.update(&mut app, |editor, ctx| {
            editor.cursor_at(CharOffset::from(1), ctx);
            editor.add_cursor_at(49.into(), ctx);
        });

        // Pasting into text and code block should paste plain text only.
        let code_clipboard_content = "echo *test*";
        let markdown = parse_markdown(code_clipboard_content).expect("Should parse link");

        editor.update(&mut app, |editor, ctx| {
            editor.insert_formatted_from_paste(markdown, "echo test", ctx);
            assert_eq!(
                editor.debug_buffer(ctx),
                "<text>echo test<a_https://warp.dev>https://warp.dev<a>\\nSecond line\\n<a_https://warp.dev>https://warp.dev<a><code:Shell><c_#b4fa72>coecho testde<c><text>"
            );
        });
    });
}

#[test]
fn test_multiselect_delete() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = model_from_markdown("First text\nSecond line\nThird line", &mut app, true);
        layout_model(&mut app, &editor).await;

        // Deleting selections should delete the selected text.
        editor.update(&mut app, |editor, ctx| {
            editor.cursor_at(CharOffset::from(1), ctx);
            editor.set_last_selection_head(6.into(), ctx);
            editor.add_cursor_at(12.into(), ctx);
            editor.set_last_selection_head(18.into(), ctx);

            editor.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
            assert_eq!(editor.debug_buffer(ctx), "<text> text\\n line\\nThird line");
        });

        // Delete backwards by word
        // Deleting selections should delete the selected text.
        editor.update(&mut app, |editor, ctx| {
            editor.cursor_at(CharOffset::from(6), ctx);
            editor.add_cursor_at(18.into(), ctx);

            editor.delete(
                TextDirection::Backwards,
                TextUnit::Word(WordBoundariesPolicy::Default),
                false,
                ctx,
            );
            assert_eq!(editor.debug_buffer(ctx), "<text> \\n line\\n line");
        });

        // Cut backwards by word.
        editor.update(&mut app, |editor, ctx| {
            editor.cursor_at(1.into(), ctx);
            editor.user_insert("First", ctx);
            editor.cursor_at(8.into(), ctx);
            editor.user_insert("Second", ctx);

            editor.add_cursor_at(6.into(), ctx);

            editor.delete(
                TextDirection::Backwards,
                TextUnit::Word(WordBoundariesPolicy::Default),
                true,
                ctx,
            );
            assert_eq!(editor.debug_buffer(ctx), "<text> \\n line\\n line");
            let clipboard = ctx.clipboard().read();
            assert_eq!(&clipboard.plain_text, "Second\nFirst");
            assert_eq!(clipboard.html.as_deref(), Some("<p>Second</p><p>First</p>"));
        });
    });
}
