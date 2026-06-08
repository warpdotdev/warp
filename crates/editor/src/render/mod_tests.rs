//! End-to-end editor tests.

use string_offset::CharOffset;
use warp_core::features::FeatureFlag;
use warpui_core::{App, ModelHandle, ReadModel};

use super::model::test_utils::{TEST_STYLES, init_logging};
use super::model::{BlockItem, RenderEvent, RenderState};
use crate::content::buffer::{
    AutoScrollBehavior, Buffer, BufferEditAction, BufferEvent, BufferSelectAction, EditOrigin,
    InitialBufferState, ShouldAutoscroll,
};
use crate::content::selection_model::BufferSelectionModel;
use crate::content::text::{BlockType, BufferBlockItem, IndentBehavior, TextStyles};
use crate::content::version::BufferVersion;

#[test]
fn test_simple_edit() {
    init_logging();
    App::test((), |mut app| async move {
        let state = TestState::new(&mut app);

        state
            .edit(
                BufferEditAction::Insert {
                    text: "x",
                    style: Default::default(),
                    override_text_style: None,
                },
                EditOrigin::UserTyped,
                &mut app,
            )
            .await;
        // See comments in EditDelta::layout_delta on why this paragraph has two characters even
        // though it (a) doesn't include the initial `<text>` marker and (b) doesn't end in an
        // explicit newline.
        state.assert_rendered(
            &app,
            r#"
-------- 0.00px / 0 characters --------
Paragraph (2 characters, 1 lines, 24.00px tall)
"#,
        );
    });
}

#[test]
fn test_edit_many_lines() {
    init_logging();
    App::test((), |mut app| async move {
        let state = TestState::new(&mut app);

        // Reset with several lines of Markdown at once.
        state
            .markdown(
                r#"a
bb
ccc
dddd
eeeee
ffffff
ggggggg
hhhhhhhh
iiiiiiiii
jjjjjjjjjj
kkkkkkkkkkk
llllllllllll
mmmmmmmmmmmmm
nnnnnnnnnnnnnn
ooooooooooooooo
pppppppppppppppp
qqqqqqqqqqqqqqqqq
rrrrrrrrrrrrrrrrrr
sssssssssssssssssss
tttttttttttttttttttt
uuuuuuuuuuuuuuuuuuuuu
vvvvvvvvvvvvvvvvvvvvvv
wwwwwwwwwwwwwwwwwwwwwww
xxxxxxxxxxxxxxxxxxxxxxxx
yyyyyyyyyyyyyyyyyyyyyyyyy
zzzzzzzzzzzzzzzzzzzzzzzzzz"#,
                &mut app,
            )
            .await;

        // Assert that paragraphs are laid out in the correct order.
        state.assert_rendered(
            &app,
            r#"
-------- 0.00px / 0 characters --------
Paragraph (2 characters, 1 lines, 24.00px tall)
-------- 24.00px / 2 characters --------
Paragraph (3 characters, 1 lines, 24.00px tall)
-------- 48.00px / 5 characters --------
Paragraph (4 characters, 1 lines, 24.00px tall)
-------- 72.00px / 9 characters --------
Paragraph (5 characters, 1 lines, 24.00px tall)
-------- 96.00px / 14 characters --------
Paragraph (6 characters, 1 lines, 24.00px tall)
-------- 120.00px / 20 characters --------
Paragraph (7 characters, 1 lines, 24.00px tall)
-------- 144.00px / 27 characters --------
Paragraph (8 characters, 1 lines, 24.00px tall)
-------- 168.00px / 35 characters --------
Paragraph (9 characters, 1 lines, 24.00px tall)
-------- 192.00px / 44 characters --------
Paragraph (10 characters, 1 lines, 24.00px tall)
-------- 216.00px / 54 characters --------
Paragraph (11 characters, 1 lines, 24.00px tall)
-------- 240.00px / 65 characters --------
Paragraph (12 characters, 1 lines, 24.00px tall)
-------- 264.00px / 77 characters --------
Paragraph (13 characters, 1 lines, 24.00px tall)
-------- 288.00px / 90 characters --------
Paragraph (14 characters, 1 lines, 24.00px tall)
-------- 312.00px / 104 characters --------
Paragraph (15 characters, 1 lines, 24.00px tall)
-------- 336.00px / 119 characters --------
Paragraph (16 characters, 1 lines, 24.00px tall)
-------- 360.00px / 135 characters --------
Paragraph (17 characters, 1 lines, 24.00px tall)
-------- 384.00px / 152 characters --------
Paragraph (18 characters, 1 lines, 24.00px tall)
-------- 408.00px / 170 characters --------
Paragraph (19 characters, 1 lines, 24.00px tall)
-------- 432.00px / 189 characters --------
Paragraph (20 characters, 1 lines, 24.00px tall)
-------- 456.00px / 209 characters --------
Paragraph (21 characters, 1 lines, 24.00px tall)
-------- 480.00px / 230 characters --------
Paragraph (22 characters, 1 lines, 24.00px tall)
-------- 504.00px / 252 characters --------
Paragraph (23 characters, 1 lines, 24.00px tall)
-------- 528.00px / 275 characters --------
Paragraph (24 characters, 1 lines, 24.00px tall)
-------- 552.00px / 299 characters --------
Paragraph (25 characters, 1 lines, 24.00px tall)
-------- 576.00px / 324 characters --------
Paragraph (26 characters, 1 lines, 24.00px tall)
-------- 600.00px / 350 characters --------
Paragraph (27 characters, 1 lines, 24.00px tall)
"#,
        );
    });
}

#[test]
fn test_enter_before_horizontal_rule() {
    init_logging();
    App::test((), |mut app| async move {
        let app = &mut app;
        let state = TestState::new(app);
        state.markdown("First line\n---\nSecond line", app).await;
        state.set_cursor(11, app); // At the end of "First line".

        state
            .edit(
                BufferEditAction::Enter {
                    force_newline: false,
                    style: Default::default(),
                },
                EditOrigin::UserTyped,
                app,
            )
            .await;
        state.assert_rendered(
            app,
            r#"
-------- 0.00px / 0 characters --------
Paragraph (11 characters, 1 lines, 24.00px tall)
-------- 24.00px / 11 characters --------
Paragraph (1 characters, 1 lines, 24.00px tall)
-------- 48.00px / 12 characters --------
Horizontal Rule (1 characters, 1 lines, 10.00px tall)
-------- 58.00px / 13 characters --------
Paragraph (12 characters, 1 lines, 24.00px tall)
"#,
        );
    })
}

#[test]
fn test_enter_after_horizontal_rule() {
    init_logging();
    App::test((), |mut app| async move {
        let app = &mut app;
        let state = TestState::new(app);
        state.markdown("First line\n---\nSecond line", app).await;
        state.set_cursor(13, app); // At the end of "First line".

        state
            .edit(
                BufferEditAction::Enter {
                    force_newline: false,
                    style: Default::default(),
                },
                EditOrigin::UserTyped,
                app,
            )
            .await;
        state.assert_rendered(
            app,
            r#"
-------- 0.00px / 0 characters --------
Paragraph (11 characters, 1 lines, 24.00px tall)
-------- 24.00px / 11 characters --------
Horizontal Rule (1 characters, 1 lines, 10.00px tall)
-------- 34.00px / 12 characters --------
Paragraph (1 characters, 1 lines, 24.00px tall)
-------- 58.00px / 13 characters --------
Paragraph (12 characters, 1 lines, 24.00px tall)
"#,
        );
    })
}

#[test]
fn test_edit_at_horizontal_rule_end() {
    init_logging();
    App::test((), |mut app| async move {
        let app = &mut app;
        let state = TestState::new(app);
        state.markdown("First line\n---\nSecond line", app).await;
        state.set_cursor(12, app); // At the end of "First line".

        state
            .edit(
                BufferEditAction::Insert {
                    text: "x",
                    style: Default::default(),
                    override_text_style: None,
                },
                EditOrigin::UserTyped,
                app,
            )
            .await;
        state.assert_rendered(
            app,
            r#"
-------- 0.00px / 0 characters --------
Paragraph (11 characters, 1 lines, 24.00px tall)
-------- 24.00px / 11 characters --------
Horizontal Rule (1 characters, 1 lines, 10.00px tall)
-------- 34.00px / 12 characters --------
Paragraph (2 characters, 1 lines, 24.00px tall)
-------- 58.00px / 14 characters --------
Paragraph (12 characters, 1 lines, 24.00px tall)
"#,
        );
    })
}

#[test]
fn test_edit_after_style() {
    init_logging();
    App::test((), |mut app| async move {
        let app = &mut app;
        let state = TestState::new(app);
        state
            .markdown("Some **styled** text\nAnd `more`", app)
            .await;

        // Set the cursor to just after the bold text.
        state.set_cursor(12, app);

        // Insert some new text with style inheritance.
        state
            .edit(
                BufferEditAction::Insert {
                    text: "!",
                    style: TextStyles::default().bold(),
                    override_text_style: None,
                },
                EditOrigin::UserTyped,
                app,
            )
            .await;

        state.assert_rendered(
            app,
            r#"
-------- 0.00px / 0 characters --------
Paragraph (18 characters, 1 lines, 24.00px tall)
-------- 24.00px / 18 characters --------
Paragraph (9 characters, 1 lines, 24.00px tall)
        "#,
        );
    })
}

#[test]
fn test_undo_at_block_boundary() {
    // This is a regression test for CLD-1178.
    init_logging();
    App::test((), |mut app| async move {
        let app = &mut app;
        let state = TestState::new(app);
        state
            .markdown("- [x] A\n- [x] B\n- [ ] C\n- [ ] D", app)
            .await;
        state.assert_buffer(app, "<cl0:true>A<cl0:true>B<cl0:false>C<cl0:false>D<text>");

        // Select from the start of item C up through A and B.
        state.set_cursor(5, app);
        state.select(BufferSelectAction::SetLastHead { offset: 1.into() }, app);

        // Press backspace, deleting A and B.
        state
            .edit(BufferEditAction::Backspace, EditOrigin::UserTyped, app)
            .await;
        state.assert_buffer(app, "<cl0:true>C<cl0:false>D<text>");
        state.assert_rendered(
            app,
            r#"
-------- 0.00px / 0 characters --------
Task List @ 1 [X] (2 characters, 1 lines, 18.00px tall)
-------- 18.00px / 2 characters --------
Task List @ 1 [ ] (2 characters, 1 lines, 18.00px tall)
-------- 36.00px / 4 characters --------
Trailing Newline (1 characters, 1 lines, 24.00px tall)
        "#,
        );

        // Undo that change and ensure we revert to the original contents.
        state
            .edit(BufferEditAction::Undo, EditOrigin::UserInitiated, app)
            .await;
        state.assert_buffer(app, "<cl0:true>A<cl0:true>B<cl0:false>C<cl0:false>D<text>");
        state.assert_rendered(
            app,
            r#"
-------- 0.00px / 0 characters --------
Task List @ 1 [X] (2 characters, 1 lines, 18.00px tall)
-------- 18.00px / 2 characters --------
Task List @ 1 [X] (2 characters, 1 lines, 18.00px tall)
-------- 36.00px / 4 characters --------
Task List @ 1 [ ] (2 characters, 1 lines, 18.00px tall)
-------- 54.00px / 6 characters --------
Task List @ 1 [ ] (2 characters, 1 lines, 18.00px tall)
-------- 72.00px / 8 characters --------
Trailing Newline (1 characters, 1 lines, 24.00px tall)
        "#,
        )
    });
}

#[test]
fn test_convert_first_line() {
    // This is a full-stack analogue to test_remove_prefix_and_insert_block_item.
    init_logging();
    App::test((), |mut app| async move {
        let app = &mut app;
        let state = TestState::new(app);
        // This only uses 2 dashes so it's not parsed as Markdown yet.
        state.markdown("--\n```\ncode\n```\n", app).await;
        state.assert_buffer(app, "<text>--<code:Shell>code<text>");

        // Mimic a Markdown shortcut on the first line.
        state.set_cursor(3, app);
        state
            .edit(
                BufferEditAction::RemovePrefixAndStyleBlocks(BlockType::Item(
                    BufferBlockItem::HorizontalRule,
                )),
                EditOrigin::UserInitiated,
                app,
            )
            .await;
        state.assert_buffer(app, "<hr><code:Shell>code<text>");
        state.assert_rendered(
            app,
            r#"
-------- 0.00px / 0 characters --------
Horizontal Rule (1 characters, 1 lines, 10.00px tall)
-------- 10.00px / 1 characters --------
Code Block - Shell (5 characters, 1 lines, 84.00px tall)
-------- 94.00px / 6 characters --------
Trailing Newline (1 characters, 1 lines, 24.00px tall)
"#,
        );

        // Undo that change and ensure we revert to the original contents.
        state
            .edit(BufferEditAction::Undo, EditOrigin::UserInitiated, app)
            .await;
        state.assert_buffer(app, "<text>--<code:Shell>code<text>");
        state.assert_rendered(
            app,
            r#"
-------- 0.00px / 0 characters --------
Paragraph (3 characters, 1 lines, 24.00px tall)
-------- 24.00px / 3 characters --------
Code Block - Shell (5 characters, 1 lines, 84.00px tall)
-------- 108.00px / 8 characters --------
Trailing Newline (1 characters, 1 lines, 24.00px tall)
"#,
        )
    });
}

/// Helper for testing edits end-to-end. This is essentially a stripped-down editor model.
struct TestState {
    content: ModelHandle<Buffer>,
    selection: ModelHandle<BufferSelectionModel>,
    render: ModelHandle<RenderState>,
    layout_updates: async_channel::Receiver<()>,
}

impl TestState {
    fn new(app: &mut App) -> Self {
        let content = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(content.clone()));
        let render = app.add_model(|ctx| RenderState::new(TEST_STYLES, false, None, ctx));

        let (layout_tx, layout_rx) = async_channel::unbounded();
        app.update(|ctx| {
            let render2 = render.clone();
            ctx.subscribe_to_model(&content, move |_, event, ctx| match event {
                BufferEvent::SelectionChanged { .. } => (),
                BufferEvent::ContentChanged {
                    delta,
                    should_autoscroll,
                    ..
                } => render2.update(ctx, |render_state, _| {
                    render_state.add_pending_edit(delta.clone(), BufferVersion::new());
                    if matches!(should_autoscroll, ShouldAutoscroll::Yes) {
                        render_state.request_autoscroll();
                    }
                }),
                BufferEvent::AnchorUpdated { .. } | BufferEvent::ContentReplaced { .. } => (),
            });

            let content2 = content.clone();
            ctx.subscribe_to_model(&render, move |render_state, event, ctx| match event {
                RenderEvent::NeedsResize => {
                    let delta = content2.as_ref(ctx).invalidate_layout();
                    render_state.update(ctx, |render_state, _| {
                        render_state.add_pending_edit(delta, BufferVersion::new())
                    });
                }
                RenderEvent::LayoutUpdated => {
                    let _ = layout_tx.try_send(());
                }
                _ => (),
            });
        });

        Self {
            content,
            selection,
            render,
            layout_updates: layout_rx,
        }
    }

    /// Move the cursor to an offset.
    fn set_cursor(&self, location: impl Into<CharOffset>, app: &mut App) {
        self.select(
            BufferSelectAction::AddCursorAt {
                offset: location.into(),
                clear_selections: true,
            },
            app,
        );
    }

    fn select(&self, action: BufferSelectAction, app: &mut App) {
        self.content.update(app, |buffer, ctx| {
            buffer.update_selection(
                self.selection.clone(),
                action,
                AutoScrollBehavior::Selection,
                ctx,
            );
        });
    }

    /// Apply an edit to the buffer and wait for it to be laid out.
    async fn edit(&self, action: BufferEditAction<'_>, origin: EditOrigin, app: &mut App) {
        self.content.update(app, |buffer, ctx| {
            buffer.update_content(action, origin, self.selection.clone(), ctx)
        });
        self.layout_updates
            .recv()
            .await
            .expect("Layout channel should not be closed");
    }

    /// Replace the buffer with the given Markdown.
    async fn markdown(&self, markdown: &str, app: &mut App) {
        let state = InitialBufferState::markdown(markdown);
        self.edit(
            BufferEditAction::ReplaceWith(state),
            EditOrigin::SystemEdit,
            app,
        )
        .await
    }

    /// Assert that the render state has the expected contents, as produced by describing its
    /// `SumTree` of `BlockItem`s.
    #[track_caller]
    fn assert_rendered(&self, ctx: &impl ReadModel, expected: &str) {
        let rendered = self.render.read(ctx, |render_state, _| {
            let content = render_state.content();
            let described_content = content.describe_content();
            described_content.to_string()
        });
        // TODO: Consider using https://github.com/rust-analyzer/expect-test.
        let rendered = rendered.trim();
        let expected = expected.trim();

        if rendered != expected {
            panic!(
                "\nExpected:
====
{expected}
====

Actual:
====
{rendered}
===="
            );
        }
    }

    /// Assert that the buffer has the expected contents.
    #[track_caller]
    fn assert_buffer(&self, ctx: &impl ReadModel, expected: &str) {
        let buffer = self.content.read(ctx, |buffer, _| buffer.debug());

        let buffer = buffer.trim();
        let expected = expected.trim();

        if buffer != expected {
            panic!(
                "\nExpected:
====
{expected}
====

Actual:
====
{buffer}
===="
            );
        }
    }
}

#[test]
fn test_markdown_table_render_starts_at_zero_offset() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::MarkdownTables.override_enabled(true);
        let state = TestState::new(&mut app);
        state
            .markdown("| Name | Age |\n| --- | --- |\n| Alice | 30 |\n", &mut app)
            .await;

        state.render.read(&app, |render_state, _| {
            let content = render_state.content();
            let block = content
                .block_at_offset(CharOffset::zero())
                .expect("table block should exist at offset 0");
            assert_eq!(block.start_char_offset, CharOffset::zero());
            assert!(matches!(block.item, BlockItem::Table(_)));
        });
    });
}

#[test]
fn test_markdown_table_count_counts_rendered_tables() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::MarkdownTables.override_enabled(true);
        let state = TestState::new(&mut app);
        state
            .markdown("| Name | Age |\n| --- | --- |\n| Alice | 30 |\n", &mut app)
            .await;

        let count = state
            .render
            .read(&app, |render_state, _| render_state.markdown_table_count());
        assert_eq!(count, 1);
    });
}
