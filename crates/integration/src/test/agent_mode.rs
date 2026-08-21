//! Integration tests for text selection and copying functionality in AI blocks.
//! This module tests AI blocks with markdown **enabled**.
//! There are no tests with markdown disabled because Agent Mode Markdown has been fully rolled out.
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use lazy_static::lazy_static;
use pathfinder_geometry::vector::{Vector2F, vec2f};
use settings::ToggleableSetting;
use warp::cmd_or_ctrl_shift;
use warp::features::FeatureFlag;
use warp::integration_testing::clipboard::assert_clipboard_contains_string;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::{
    assert_view_has_text_selection, clear_blocklist_to_remove_bootstrapped_blocks,
    execute_echo_str, wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::single_terminal_view_for_tab;
use warp::settings::SelectionSettings;
use warp_multi_agent_api as api;
use warpui_core::event::ModifiersState;
use warpui_core::integration::TestStep;
use warpui_core::text::SelectionType;
use warpui_core::{Event, SingletonEntity, async_assert};

use super::new_builder;
use crate::Builder;
use crate::util::skip_if_powershell_core_2303;

cfg_if::cfg_if! {
    if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
        lazy_static! {
            /// Position directly to the left of the first user query.
            static ref START_OF_FIRST_BLOCK_POSITION: Vector2F = vec2f(17.0, 239.0);
            /// Position directly to the right of the last command output.
            static ref END_OF_LAST_BLOCK_POSITION: Vector2F = vec2f(209.0, 668.0);
            /// Position in the middle of the word "mo|de" of the AI block output.
            static ref MIDDLE_OF_MODE_POSITION: Vector2F = vec2f(224.0, 557.0);
        }
    } else {
        lazy_static! {
            /// Position directly to the left of the first user query.
            static ref START_OF_FIRST_BLOCK_POSITION: Vector2F = vec2f(19.097656, 207.80469);
            /// Position directly to the right of the last command output.
            static ref END_OF_LAST_BLOCK_POSITION: Vector2F = vec2f(214.0, 645.0);
            /// Position in the middle of the word "mo|de" of the AI block output.
            static ref MIDDLE_OF_MODE_POSITION: Vector2F = vec2f(222.0, 530.0);
        }
    }
}

/// Sets up the blocklist with the following blocks:
/// ```text
///  _______________________________________________________________________________________
/// | echo "this is the first block"                                                        |
/// | this is the first block                                                               |
/// |_______________________________________________________________________________________|
/// | echo "now its the second block"                                                       |
/// | now its the second block                                                              |
/// |_______________________________________________________________________________________|
/// | ~                                                                                     |
/// | Can you produce some dummy output for me?                                             |
/// | ### This is a dummy title                                                             |
/// | •  Hi, I am agent mode and this is my dummy output. Hope that answers your question.  |
/// | •  This is list item 2                                                                |
/// |_______________________________________________________________________________________|
/// | echo "hello Im the third block"                                                       |
/// | hello Im the third block                                                              |
/// |_______________________________________________________________________________________|
/// ```
fn builder_with_setup() -> Builder {
    new_builder()
        // TODO(CORE-2721): Block count / index Failed b/c of in-band generators
        // TODO(CORE-2303): Some of these also don't work b/c of other positioning issues
        .set_should_run_test(skip_if_powershell_core_2303)
        .with_step(
            wait_until_bootstrapped_single_pane_for_tab(0)
        )
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        // Run three commands
        .with_step(execute_echo_str(0, "this is the first block"))
        .with_step(execute_echo_str(0, "now its the second block"))
        .with_step(new_step_with_default_assertions("Insert dummy AI block")
            .with_action(|app, _, _| {
                let window_id = app.window_ids()[0];
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);

                terminal_view.update(app, |view, ctx| {
                    view.insert_dummy_ai_block(
                        "Can you produce some dummy output for me?".to_owned(),
                        concat!(
                            "### This is a dummy title\n",
                            "* Hi, I am agent mode and this is my dummy output. Hope that answers your question.\n",
                            "* This is list item 2"
                        ).to_owned(),
                        ctx,
                    );
                });
            }))
        .with_step(execute_echo_str(0, "hello Im the third block").add_assertion(|app, window_id| {
            let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
            terminal_view.read(app, |view, _ctx| {
                async_assert!(!view.is_selecting(), "Should not be selecting",)
            })
        }))
}

fn markdown_visuals_fixture_directory() -> String {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../warpui_core/test_data");
    fixture_dir
        .canonicalize()
        .unwrap_or(fixture_dir)
        .to_string_lossy()
        .into_owned()
}

fn restored_user_query_message(task_id: &str, request_id: &str, directory: &str) -> api::Message {
    api::Message {
        id: "restored-user-query".to_string(),
        task_id: task_id.to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::UserQuery(api::message::UserQuery {
            query: "Show me local images and a Mermaid diagram".to_string(),
            context: Some(api::InputContext {
                directory: Some(api::input_context::Directory {
                    pwd: directory.to_string(),
                    home: String::new(),
                    pwd_file_symbols_indexed: false,
                }),
                ..Default::default()
            }),
            referenced_attachments: HashMap::new(),
            mode: None,
            intended_agent: Default::default(),
        })),
        request_id: request_id.to_string(),
        timestamp: None,
        fetched_memories: vec![],
    }
}

fn restored_agent_output_message(task_id: &str, request_id: &str) -> api::Message {
    api::Message {
        id: "restored-agent-output".to_string(),
        task_id: task_id.to_string(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::AgentOutput(
            api::message::AgentOutput {
                text: concat!(
                    "Inline local images:\n",
                    "![One](local.png) ![Two](local.png)\n\n",
                    "```mermaid\n",
                    "graph TD\n",
                    "A[Agent] --> B[Blocklist]\n",
                    "B --> C[Rendered visuals]\n",
                    "```\n"
                )
                .to_string(),
            },
        )),
        request_id: request_id.to_string(),
        timestamp: None,
        fetched_memories: vec![],
    }
}

fn restored_markdown_visuals_conversation_data() -> api::ConversationData {
    let task_id = "restored-markdown-visuals-task";
    let request_id = "restored-markdown-visuals-request";
    api::ConversationData {
        tasks: vec![api::Task {
            id: task_id.to_string(),
            messages: vec![
                restored_user_query_message(
                    task_id,
                    request_id,
                    &markdown_visuals_fixture_directory(),
                ),
                restored_agent_output_message(task_id, request_id),
            ],
            dependencies: None,
            description: String::new(),
            summary: String::new(),
            server_data: String::new(),
        }],
        ..Default::default()
    }
}

pub fn test_restored_ai_block_renders_mermaid_and_local_images() -> Builder {
    FeatureFlag::BlocklistMarkdownImages.set_enabled(true);
    FeatureFlag::MarkdownMermaid.set_enabled(true);

    new_builder()
        .with_real_display()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(
            new_step_with_default_assertions(
                "Restore AI conversation with local images and Mermaid",
            )
            .with_action(|app, window_id, _| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                terminal_view.update(app, |view, ctx| {
                    view.load_conversation_from_tasks(
                        restored_markdown_visuals_conversation_data(),
                        ctx,
                    );
                });
            }),
        )
        .with_step(
            TestStep::new("Wait for restored markdown visuals and capture screenshot")
                .set_timeout(Duration::from_secs(20))
                .set_post_step_pause(Duration::from_secs(3))
                .with_take_screenshot("restored_ai_block_markdown_visuals.png")
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        async_assert!(
                            view.last_ai_block().is_some(),
                            "Restored AI block should exist"
                        )
                    })
                }),
        )
}

/// Renders an orchestrate card whose `run_agents` tool call was still streaming
/// when the conversation was cancelled. The call never reaches the action
/// queue, so it has no action status and the card must fall back to its
/// terminal cancelled state instead of the "Configuring agents…" placeholder.
pub fn test_cancelled_run_agents_card_renders_cancelled_state() -> Builder {
    new_builder()
        .with_real_display()
        // A dummy AI block is not attached to an agent view conversation, so with
        // `AgentView` on it is filtered out of the terminal transcript and never
        // renders. The user preference is the only override that wins over the
        // flag state the app installs during startup.
        .with_step(
            TestStep::new("Render AI blocks inline in the blocklist").with_action(|_, _, _| {
                FeatureFlag::AgentView.set_user_preference(false);
            }),
        )
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(execute_echo_str(0, "orchestrate card repro"))
        .with_step(
            new_step_with_default_assertions("Insert cancelled orchestrate AI block").with_action(
                |app, window_id, _| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.update(app, |view, ctx| {
                        view.insert_dummy_cancelled_run_agents_ai_block(
                            "Can you parallelize the migration?".to_owned(),
                            "Splitting the migration across three agents.".to_owned(),
                            vec![
                                "schema-migration".to_owned(),
                                "api-handlers".to_owned(),
                                "integration-tests".to_owned(),
                            ],
                            ctx,
                        );
                    });
                },
            ),
        )
        .with_step(
            TestStep::new("Capture the cancelled orchestrate card")
                .set_timeout(Duration::from_secs(20))
                .set_post_step_pause(Duration::from_secs(3))
                .with_take_screenshot("run_agents_cancelled_card.png")
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, _ctx| {
                        async_assert!(
                            view.last_ai_block().is_some(),
                            "Cancelled orchestrate AI block should exist"
                        )
                    })
                }),
        )
}

fn select_first_to_last_through_ai_simple(is_copy_on_select: bool) -> Builder {
    let mut builder = builder_with_setup();
    // TODO(INT-339): There should be a "T" to the left of the query "Can you produce some dummy output for me?"
    // because of the dummy user avatar having a first initial instead of an image.
    // However, it appears next to "This is a dummy title" because it's organized as a flex row
    // with two flex column elements, and flex row selections read from children from left to right.
    // The flex element needs to be smarter about handling selections for this case.
    let expected_clipboard = "echo \"this is the first block\"
this is the first block
echo \"now its the second block\"
now its the second block
~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode and this is my dummy output. Hope that answers your question.
•  This is list item 2
echo \"hello Im the third block\"
hello Im the third block";

    let mut end_selecting_step = new_step_with_default_assertions("end selecting")
        .with_event(Event::LeftMouseUp {
            position: *END_OF_LAST_BLOCK_POSITION,
            modifiers: Default::default(),
        })
        .add_assertion(assert_view_has_text_selection(false))
        .add_assertion(|app, window_id| {
            let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
            terminal_view.read(app, |terminal_view, ctx| {
                let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                ai_block.read(ctx, |ai_block, _| {
                    let is_simple_selection =
                        matches!(ai_block.selection_type(), SelectionType::Simple);
                    let is_selected_text_correct =
                        ai_block.selected_text(ctx).is_some_and(|selected_text| {
                            selected_text
                                == "~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode and this is my dummy output. Hope that answers your question.
•  This is list item 2"
                        });
                    async_assert!(
                        is_simple_selection && is_selected_text_correct,
                        "AI block has expected selection"
                    )
                })
            })
        });

    if is_copy_on_select {
        // For some reason, dispatching FeaturesPageAction::ToggleCopyOnSelect using the toggle_setting fn
        // doesn't work because the action doesn't get processed.
        builder = builder.with_step(
            new_step_with_default_assertions("Enable copy on select").add_assertion(|app, _| {
                SelectionSettings::handle(app).update(app, |settings, ctx| {
                    settings
                        .copy_on_select
                        .toggle_and_save_value(ctx)
                        .expect("can toggle copy_on_select");
                    async_assert!(settings.copy_on_select_enabled())
                })
            }),
        );
        end_selecting_step = end_selecting_step.add_assertion(assert_clipboard_contains_string(
            expected_clipboard.to_owned(),
        ));
    }

    builder = builder
        .with_step(
            // Drag from the top left to the bottom right.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(end_selecting_step);

    if !is_copy_on_select {
        builder = builder.with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                .add_assertion(assert_clipboard_contains_string(
                    expected_clipboard.to_owned(),
                )),
        );
    }
    builder
}

/// Text we mark as selected within the AI block for the copy regression test.
const AI_BLOCK_SELECTED_TEXT: &str = "agent mode and this is my dummy output";

/// Regression test for copying a selection that lives *entirely within* an AI
/// block (select text in an AI response, then copy). This is the case broken by
/// #12079's `mouse_down` `if !handled` guard: the AI block's `SelectableArea`
/// consumes the mouse-down, so the terminal model selection is never started and
/// `selection_to_string` returns nothing on copy.
///
/// Unlike the `*_through_ai_*` tests, the selection here does NOT start in a
/// command block, so there is no point-based model selection to fall back on —
/// it exercises the AI-block-only path that the fix repairs.
///
/// We simulate the in-AI selection the same way the `SelectableArea` does for a
/// pure in-block drag (write the block-level selected text and notify the
/// terminal view), rather than relying on layout-sensitive pixel coordinates
/// (which is why the `*_through_ai_*` tests are currently ignored).
pub fn test_copy_selection_within_ai_block() -> Builder {
    builder_with_setup()
        .with_step(
            new_step_with_default_assertions("Select text within the AI block").with_action(
                |app, _, _| {
                    let window_id = app.window_ids()[0];
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    let ai_block = terminal_view
                        .read(app, |view, _| view.last_ai_block())
                        .expect("AI block exists");
                    ai_block.update(app, |block, ctx| {
                        block.simulate_text_selection_for_test(
                            Some(AI_BLOCK_SELECTED_TEXT.to_owned()),
                            ctx,
                        );
                    });
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Copy the in-AI-block selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                .add_assertion(assert_clipboard_contains_string(
                    AI_BLOCK_SELECTED_TEXT.to_owned(),
                )),
        )
}

pub fn test_selection_first_to_last_through_ai_simple() -> Builder {
    select_first_to_last_through_ai_simple(false)
}

pub fn test_copy_on_select_first_to_last_through_ai_simple() -> Builder {
    select_first_to_last_through_ai_simple(true)
}

pub fn test_selection_first_to_last_through_ai_semantic() -> Builder {
    builder_with_setup()
        .with_step(
            // Drag from the top left to the bottom right.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                        ai_block.read(ctx, |ai_block, _| {
                            let is_simple_selection =
                                matches!(ai_block.selection_type(), SelectionType::Simple);
                            let is_selected_text_correct =
                                ai_block.selected_text(ctx).is_some_and(|selected_text| {
                                    selected_text
                                        == "~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode and this is my dummy output. Hope that answers your question.
•  This is list item 2"
                                });
                            async_assert!(
                                is_simple_selection && is_selected_text_correct,
                                "AI block has expected selection"
                            )
                        })
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                // TODO(INT-339): There should be a "T" to the left of the query "Can you produce some dummy output for me?"
                // because of the dummy user avatar having a first initial instead of an image.
                // However, it appears next to "This is a dummy title" because it's organized as a flex row
                // with two flex column elements, and flex row selections read from children from left to right.
                // The flex element needs to be smarter about handling selections for this case.
                .add_assertion(assert_clipboard_contains_string(
                    "echo \"this is the first block\"
this is the first block
echo \"now its the second block\"
now its the second block
~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode and this is my dummy output. Hope that answers your question.
•  This is list item 2
echo \"hello Im the third block\"
hello Im the third block"
                        .into(),
                )),
        )
}

pub fn test_selection_first_to_last_through_ai_lines() -> Builder {
    builder_with_setup()
        .with_step(
            // Drag from the top left to the bottom right.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                    click_count: 3,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                        ai_block.read(ctx, |ai_block, _| {
                            let is_lines_selection =
                                matches!(ai_block.selection_type(), SelectionType::Lines);
                            let is_selected_text_correct =
                                ai_block.selected_text(ctx).is_some_and(|selected_text| {
                                    selected_text
                                        == "~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode and this is my dummy output. Hope that answers your question.
•  This is list item 2"
                                });
                            async_assert!(
                                is_lines_selection && is_selected_text_correct,
                                "AI block has expected selection"
                            )
                        })
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                // TODO(INT-339): There should be a "T" to the left of the query "Can you produce some dummy output for me?"
                // because of the dummy user avatar having a first initial instead of an image.
                // However, it appears next to "This is a dummy title" because it's organized as a flex row
                // with two flex column elements, and flex row selections read from children from left to right.
                // The flex element needs to be smarter about handling selections for this case.
                .add_assertion(assert_clipboard_contains_string(
                    "echo \"this is the first block\"
this is the first block
echo \"now its the second block\"
now its the second block
~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode and this is my dummy output. Hope that answers your question.
•  This is list item 2
echo \"hello Im the third block\"
hello Im the third block"
                        .into(),
                )),
        )
}

pub fn test_selection_last_to_first_through_ai_simple() -> Builder {
    builder_with_setup()
        .with_step(
            // Drag from the bottom right to the top left.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                        ai_block.read(ctx, |ai_block, _| {
                            let is_simple_selection =
                                matches!(ai_block.selection_type(), SelectionType::Simple);
                            let is_selected_text_correct =
                                ai_block.selected_text(ctx).is_some_and(|selected_text| {
                                    selected_text
                                        == "~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode and this is my dummy output. Hope that answers your question.
•  This is list item 2"
                                });
                            async_assert!(
                                is_simple_selection && is_selected_text_correct,
                                "AI block has expected selection"
                            )
                        })
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                // TODO(INT-339): There should be a "T" to the left of the query "Can you produce some dummy output for me?"
                // because of the dummy user avatar having a first initial instead of an image.
                // However, it appears next to "This is a dummy title" because it's organized as a flex row
                // with two flex column elements, and flex row selections read from children from left to right.
                // The flex element needs to be smarter about handling selections for this case.
                .add_assertion(assert_clipboard_contains_string(
                    "echo \"this is the first block\"
this is the first block
echo \"now its the second block\"
now its the second block
~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode and this is my dummy output. Hope that answers your question.
•  This is list item 2
echo \"hello Im the third block\"
hello Im the third block"
                        .into(),
                )),
        )
}

pub fn test_selection_last_to_first_through_ai_semantic() -> Builder {
    builder_with_setup()
        .with_step(
            // Drag from the bottom right to the top left.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                        ai_block.read(ctx, |ai_block, _| {
                            let is_simple_selection =
                                matches!(ai_block.selection_type(), SelectionType::Simple);
                            let is_selected_text_correct =
                                ai_block.selected_text(ctx).is_some_and(|selected_text| {
                                    selected_text
                                        == "~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode and this is my dummy output. Hope that answers your question.
•  This is list item 2"
                                });
                            async_assert!(
                                is_simple_selection && is_selected_text_correct,
                                "AI block has expected selection"
                            )
                        })
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                // TODO(INT-339): There should be a "T" to the left of the query "Can you produce some dummy output for me?"
                // because of the dummy user avatar having a first initial instead of an image.
                // However, it appears next to "This is a dummy title" because it's organized as a flex row
                // with two flex column elements, and flex row selections read from children from left to right.
                // The flex element needs to be smarter about handling selections for this case.
                .add_assertion(assert_clipboard_contains_string(
                    "echo \"this is the first block\"
this is the first block
echo \"now its the second block\"
now its the second block
~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode and this is my dummy output. Hope that answers your question.
•  This is list item 2
echo \"hello Im the third block\"
hello Im the third block"
                        .into(),
                )),
        )
}

pub fn test_selection_last_to_first_through_ai_lines() -> Builder {
    builder_with_setup()
        .with_step(
            // Drag from the bottom right to the top left.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                    click_count: 3,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                        ai_block.read(ctx, |ai_block, _| {
                            let is_lines_selection =
                                matches!(ai_block.selection_type(), SelectionType::Lines);
                            let is_selected_text_correct =
                                ai_block.selected_text(ctx).is_some_and(|selected_text| {
                                    selected_text
                                        == "~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode and this is my dummy output. Hope that answers your question.
•  This is list item 2"
                                });
                            async_assert!(
                                is_lines_selection && is_selected_text_correct,
                                "AI block has expected selection"
                            )
                        })
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                // TODO(INT-339): There should be a "T" to the left of the query "Can you produce some dummy output for me?"
                // because of the dummy user avatar having a first initial instead of an image.
                // However, it appears next to "This is a dummy title" because it's organized as a flex row
                // with two flex column elements, and flex row selections read from children from left to right.
                // The flex element needs to be smarter about handling selections for this case.
                .add_assertion(assert_clipboard_contains_string(
                    "echo \"this is the first block\"
this is the first block
echo \"now its the second block\"
now its the second block
~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode and this is my dummy output. Hope that answers your question.
•  This is list item 2
echo \"hello Im the third block\"
hello Im the third block"
                        .into(),
                )),
        )
}

pub fn test_selection_last_to_ai_simple() -> Builder {
    builder_with_setup()
        .with_step(
            // Drag from the bottom right to the middle of the word "mo|de" in the ai block output.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                        ai_block.read(ctx, |ai_block, _| {
                            let is_simple_selection = matches!(ai_block.selection_type(), SelectionType::Simple);
                            let is_selected_text_correct = ai_block.selected_text(ctx).is_some_and(
                                |selected_text| selected_text == "de and this is my dummy output. Hope that answers your question.
•  This is list item 2"
                            );
                            async_assert!(is_simple_selection && is_selected_text_correct, "AI block has expected selection")
                        })
                    })
                })
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                .add_assertion(assert_clipboard_contains_string(
"de and this is my dummy output. Hope that answers your question.
•  This is list item 2
echo \"hello Im the third block\"
hello Im the third block".into()
                )
            ),
        )
}

pub fn test_selection_last_to_ai_semantic() -> Builder {
    builder_with_setup()
        .with_step(
            // Drag from the bottom right to the middle of the word "mo|de" in the ai block output.
            // Double click is semantic selection.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                    click_count: 2,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                        ai_block.read(ctx, |ai_block, _| {
                            let is_semantic_selection = matches!(ai_block.selection_type(), SelectionType::Semantic);
                            let is_selected_text_correct = ai_block.selected_text(ctx).is_some_and(
                                |selected_text| selected_text == "mode and this is my dummy output. Hope that answers your question.\n•  This is list item 2"
                            );
                            async_assert!(is_semantic_selection && is_selected_text_correct, "AI block has expected selection")
                        })
                    })
                })
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                .add_assertion(assert_clipboard_contains_string(
"mode and this is my dummy output. Hope that answers your question.
•  This is list item 2
echo \"hello Im the third block\"
hello Im the third block".into()
                )
            ),
        )
}

pub fn test_selection_last_to_ai_lines() -> Builder {
    builder_with_setup()
        .with_step(
            // Drag from the bottom right to the middle of the word "mo|de" in the ai block output.
            // Triple click is lines selection.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                    click_count: 3,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                        ai_block.read(ctx, |ai_block, _| {
                            let is_lines_selection = matches!(ai_block.selection_type(), SelectionType::Lines);
                            let is_selected_text_correct = ai_block.selected_text(ctx).is_some_and(|selected_text|
                                selected_text == "•  Hi, I am agent mode and this is my dummy output. Hope that answers your question.\n•  This is list item 2"
                            );
                            async_assert!(is_lines_selection && is_selected_text_correct, "AI block has expected selection")
                        })
                    })
                })
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                .add_assertion(assert_clipboard_contains_string(
"•  Hi, I am agent mode and this is my dummy output. Hope that answers your question.
•  This is list item 2
echo \"hello Im the third block\"
hello Im the third block".into()
                )
            ),
        )
}

pub fn test_selection_ai_to_last_simple() -> Builder {
    builder_with_setup()
        .with_step(
            // Drag the middle of the word "mo|de" in the ai block output to the end of the last block.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                        ai_block.read(ctx, |ai_block, _| {
                            let is_simple_selection = matches!(ai_block.selection_type(), SelectionType::Simple);
                            let is_selected_text_correct = ai_block.selected_text(ctx).is_some_and(
                                |selected_text| selected_text == "de and this is my dummy output. Hope that answers your question.
•  This is list item 2"
                            );
                            async_assert!(is_simple_selection && is_selected_text_correct, "AI block has expected selection")
                        })
                    })
                })
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                .add_assertion(assert_clipboard_contains_string(
"de and this is my dummy output. Hope that answers your question.
•  This is list item 2
echo \"hello Im the third block\"
hello Im the third block".into()
                )
            ),
        )
}

pub fn test_selection_ai_to_last_semantic() -> Builder {
    builder_with_setup()
        .with_step(
            // Drag the middle of the word "mo|de" in the ai block output to the end of the last block.
            // Double click is semantic selection.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                    click_count: 2,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                        ai_block.read(ctx, |ai_block, _| {
                            let is_semantic_selection = matches!(ai_block.selection_type(), SelectionType::Semantic);
                            let is_selected_text_correct = ai_block.selected_text(ctx).is_some_and(
                                |selected_text| selected_text ==
                                    "mode and this is my dummy output. Hope that answers your question.\n•  This is list item 2"
                            );
                            async_assert!(is_semantic_selection && is_selected_text_correct, "AI block has expected selection")
                        })
                    })
                })
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                .add_assertion(assert_clipboard_contains_string(
"mode and this is my dummy output. Hope that answers your question.
•  This is list item 2
echo \"hello Im the third block\"
hello Im the third block".into()
                )
            ),
        )
}

pub fn test_selection_ai_to_last_lines() -> Builder {
    builder_with_setup()
        .with_step(
            // Drag the middle of the word "mo|de" in the ai block output to the end of the last block.
            // Triple click is lines selection.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                    click_count: 3,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                        ai_block.read(ctx, |ai_block, _| {
                            let is_lines_selection = matches!(ai_block.selection_type(), SelectionType::Lines);
                            let is_selected_text_correct = ai_block.selected_text(ctx).is_some_and(
                                |selected_text| selected_text ==
                                    "•  Hi, I am agent mode and this is my dummy output. Hope that answers your question.\n•  This is list item 2"
                            );
                            async_assert!(is_lines_selection && is_selected_text_correct, "AI block has expected selection")
                        })
                    })
                })
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                .add_assertion(assert_clipboard_contains_string(
"•  Hi, I am agent mode and this is my dummy output. Hope that answers your question.
•  This is list item 2
echo \"hello Im the third block\"
hello Im the third block".into()
                )
            ),
        )
}

pub fn test_selection_first_to_ai_simple() -> Builder {
    builder_with_setup()
        .with_step(
            // Drag from the top left to the middle of the word "mo|de" in the ai block output.
            // Single click is simple selection.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                        ai_block.read(ctx, |ai_block, _| {
                            let is_simple_selection =
                                matches!(ai_block.selection_type(), SelectionType::Simple);
                            let is_selected_text_correct =
                                ai_block.selected_text(ctx).is_some_and(|selected_text| {
                                    selected_text
                                        == "~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mo"
                                });
                            async_assert!(
                                is_simple_selection && is_selected_text_correct,
                                "AI block has expected selection"
                            )
                        })
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                // TODO(INT-339): There should be a "T" to the left of the query "Can you produce some dummy output for me?"
                // because of the dummy user avatar having a first initial instead of an image.
                // However, it appears next to "This is a dummy title" because it's organized as a flex row
                // with two flex column elements, and flex row selections read from children from left to right.
                // The flex element needs to be smarter about handling selections for this case.
                .add_assertion(assert_clipboard_contains_string(
                    "echo \"this is the first block\"
this is the first block
echo \"now its the second block\"
now its the second block
~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mo"
                        .into(),
                )),
        )
}

pub fn test_selection_first_to_ai_semantic() -> Builder {
    builder_with_setup()
        .with_step(
            // Drag from the top left to the middle of the word "mo|de" in the ai block output.
            // Double click is semantic selection.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                    click_count: 2,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                        ai_block.read(ctx, |ai_block, _| {
                            let is_semantic_selection =
                                matches!(ai_block.selection_type(), SelectionType::Semantic);
                            let is_selected_text_correct =
                                ai_block.selected_text(ctx).is_some_and(|selected_text| {
                                    selected_text
                                        == "~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode"
                                });
                            async_assert!(
                                is_semantic_selection && is_selected_text_correct,
                                "AI block has expected selection"
                            )
                        })
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                // TODO(INT-339): There should be a "T" to the left of the query "Can you produce some dummy output for me?"
                // because of the dummy user avatar having a first initial instead of an image.
                // However, it appears next to "This is a dummy title" because it's organized as a flex row
                // with two flex column elements, and flex row selections read from children from left to right.
                // The flex element needs to be smarter about handling selections for this case.
                .add_assertion(assert_clipboard_contains_string(
                    "echo \"this is the first block\"
this is the first block
echo \"now its the second block\"
now its the second block
~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode"
                        .into(),
                )),
        )
}

pub fn test_selection_first_to_ai_lines() -> Builder {
    builder_with_setup()
        .with_step(
            // Drag from the top left to the middle of the word "mo|de" in the ai block output.
            // Triple click is lines selection.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                    click_count: 3,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                        ai_block.read(ctx, |ai_block, _| {
                            let is_lines_selection =
                                matches!(ai_block.selection_type(), SelectionType::Lines);
                            let is_selected_text_correct =
                                ai_block.selected_text(ctx).is_some_and(|selected_text| {
                                    selected_text
                                        == "~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode and this is my dummy output. Hope that answers your question."
                                });
                            async_assert!(
                                is_lines_selection && is_selected_text_correct,
                                "AI block has expected selection"
                            )
                        })
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                // TODO(INT-339): There should be a "T" to the left of the query "Can you produce some dummy output for me?"
                // because of the dummy user avatar having a first initial instead of an image.
                // However, it appears next to "This is a dummy title" because it's organized as a flex row
                // with two flex column elements, and flex row selections read from children from left to right.
                // The flex element needs to be smarter about handling selections for this case.
                .add_assertion(assert_clipboard_contains_string(
                    "echo \"this is the first block\"
this is the first block
echo \"now its the second block\"
now its the second block
~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode and this is my dummy output. Hope that answers your question."
                        .into(),
                )),
        )
}

pub fn test_selection_ai_to_first_simple() -> Builder {
    builder_with_setup()
        .with_step(
            // Drag from the middle of the word "mo|de" in the ai block output to the top left of the first block.
            // Single click is simple selection.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                        ai_block.read(ctx, |ai_block, _| {
                            let is_simple_selection =
                                matches!(ai_block.selection_type(), SelectionType::Simple);
                            let is_selected_text_correct =
                                ai_block.selected_text(ctx).is_some_and(|selected_text| {
                                    selected_text == AI_BLOCK_TEXT_UP_TO_MIDDLE_OF_MODE
                                });
                            async_assert!(
                                is_simple_selection && is_selected_text_correct,
                                "AI block has expected selection"
                            )
                        })
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                // TODO(INT-339): There should be a "T" to the left of the query "Can you produce some dummy output for me?"
                // because of the dummy user avatar having a first initial instead of an image.
                // However, it appears next to "This is a dummy title" because it's organized as a flex row
                // with two flex column elements, and flex row selections read from children from left to right.
                // The flex element needs to be smarter about handling selections for this case.
                .add_assertion(assert_clipboard_contains_string(
                    "echo \"this is the first block\"
this is the first block
echo \"now its the second block\"
now its the second block
"
                    .to_owned()
                        + AI_BLOCK_TEXT_UP_TO_MIDDLE_OF_MODE,
                )),
        )
}

pub fn test_selection_ai_to_first_semantic() -> Builder {
    builder_with_setup()
        .with_step(
            // Drag from the middle of the word "mo|de" in the ai block output to the top left of the first block.
            // Double click is semantic selection.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                    click_count: 2,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                        ai_block.read(ctx, |ai_block, _| {
                            let is_semantic_selection =
                                matches!(ai_block.selection_type(), SelectionType::Semantic);
                            let is_selected_text_correct =
                                ai_block.selected_text(ctx).is_some_and(|selected_text| {
                                    selected_text
                                        == "~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode"
                                });
                            async_assert!(
                                is_semantic_selection && is_selected_text_correct,
                                "AI block has expected selection"
                            )
                        })
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                // TODO(INT-339): There should be a "T" to the left of the query "Can you produce some dummy output for me?"
                // because of the dummy user avatar having a first initial instead of an image.
                // However, it appears next to "This is a dummy title" because it's organized as a flex row
                // with two flex column elements, and flex row selections read from children from left to right.
                // The flex element needs to be smarter about handling selections for this case.
                .add_assertion(assert_clipboard_contains_string(
                    "echo \"this is the first block\"
this is the first block
echo \"now its the second block\"
now its the second block
~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode"
                        .into(),
                )),
        )
}

/// Text of the full dummy AI block, exactly as `select_first_to_last_through_ai_simple`
/// verifies for a drag that fully crosses it: `~`, the query, the title, and both output lines.
/// A direct (non-drag) Shift+click extension fully selects any rich-content block it passes
/// through entirely, but not one its destination lands inside of: that block's tail moves to
/// the exact clicked position instead (see `TerminalView::extend_block_text_selection`).
const FULL_AI_BLOCK_TEXT: &str = "~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode and this is my dummy output. Hope that answers your question.
•  This is list item 2";

/// Prefix of [`FULL_AI_BLOCK_TEXT`] selected by a simple (single-click) selection ending at
/// `MIDDLE_OF_MODE_POSITION`, exactly as `test_selection_ai_to_first_simple` verifies for an
/// equivalent drag: the cursor lands in the middle of "mo|de" in the output's second line, so
/// the selection is cut off right after "...agent mo".
const AI_BLOCK_TEXT_UP_TO_MIDDLE_OF_MODE: &str = "~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mo";

lazy_static! {
    /// The complement of [`AI_BLOCK_TEXT_UP_TO_MIDDLE_OF_MODE`] within [`FULL_AI_BLOCK_TEXT`]:
    /// what a selection extending backward *from* `MIDDLE_OF_MODE_POSITION` to the end of the
    /// AI block selects. Derived from the same two independently-verified constants rather than
    /// a separately hand-copied literal, so the two can never silently drift apart.
    static ref AI_BLOCK_TEXT_FROM_MIDDLE_OF_MODE_TO_END: &'static str =
        &FULL_AI_BLOCK_TEXT[AI_BLOCK_TEXT_UP_TO_MIDDLE_OF_MODE.len()..];
}

/// A small in-place drag-and-release within the first command block, anchored at
/// `START_OF_FIRST_BLOCK_POSITION`. Leaves a non-empty, completed point-based selection with its
/// fixed head at `START_OF_FIRST_BLOCK_POSITION` — the precondition for a later Shift+click to
/// extend rather than begin a new selection (PRODUCT rule 1).
fn drag_and_release_small_selection_in_first_block(name: &str) -> TestStep {
    let tail = *START_OF_FIRST_BLOCK_POSITION + vec2f(50., 0.);
    new_step_with_default_assertions(name)
        .with_event(Event::LeftMouseDown {
            position: *START_OF_FIRST_BLOCK_POSITION,
            modifiers: Default::default(),
            click_count: 1,
            is_first_mouse: false,
        })
        .with_event(Event::LeftMouseDragged {
            position: tail,
            modifiers: Default::default(),
        })
        .with_event(Event::LeftMouseUp {
            position: tail,
            modifiers: Default::default(),
        })
        .add_assertion(assert_view_has_text_selection(false))
}

/// A Shift+click (mouse down and up at the same position, no drag) at `position`.
fn shift_click(name: &str, position: Vector2F) -> TestStep {
    let shift = ModifiersState {
        shift: true,
        ..Default::default()
    };
    new_step_with_default_assertions(name)
        .with_event(Event::LeftMouseDown {
            position,
            modifiers: shift,
            click_count: 1,
            is_first_mouse: false,
        })
        .with_event(Event::LeftMouseUp {
            position,
            modifiers: shift,
        })
}

/// Direct (non-drag) Shift+click extension from a command block, across the intervening AI
/// block, to another command block (PRODUCT rules 1 and 4; finding 2's "direct command→command"
/// case). The fixed head is anchored at the same point `test_selection_first_to_last_through_ai_simple`
/// uses for an equivalent drag, so the two gestures produce the same final selection and copied
/// text — demonstrating parity between the click-only and drag paths.
pub fn test_shift_click_extends_through_ai_block_to_last_block() -> Builder {
    builder_with_setup()
        .with_step(drag_and_release_small_selection_in_first_block(
            "start a small selection in the first block",
        ))
        .with_step(
            shift_click(
                "Shift+click at the end of the last block",
                *END_OF_LAST_BLOCK_POSITION,
            )
            .add_assertion(assert_view_has_text_selection(false))
            .add_assertion(|app, window_id| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                terminal_view.read(app, |terminal_view, ctx| {
                    let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                    ai_block.read(ctx, |ai_block, ctx| {
                        let is_selected_text_correct = ai_block
                            .selected_text(ctx)
                            .is_some_and(|selected_text| selected_text == FULL_AI_BLOCK_TEXT);
                        async_assert!(
                            is_selected_text_correct,
                            "AI block should be fully selected"
                        )
                    })
                })
            }),
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                .add_assertion(assert_clipboard_contains_string(
                    "echo \"this is the first block\"
this is the first block
echo \"now its the second block\"
now its the second block
"
                    .to_owned()
                        + FULL_AI_BLOCK_TEXT
                        + "
echo \"hello Im the third block\"
hello Im the third block",
                )),
        )
}

/// Direct (non-drag) Shift+click extension from a command block landing inside the AI block
/// (PRODUCT rule 1's third bullet; finding 2's "direct command→rich" case). The selection ends
/// exactly where the user clicked (PRODUCT rule 11: simple cell/character extension), not the
/// whole block — the destination block's tail is moved to the click position via the same
/// mechanism a real drag into it would use, instead of being fully selected.
pub fn test_shift_click_extends_from_first_block_into_ai_block() -> Builder {
    builder_with_setup()
        .with_step(drag_and_release_small_selection_in_first_block(
            "start a small selection in the first block",
        ))
        .with_step(
            shift_click(
                "Shift+click in the middle of the AI block",
                *MIDDLE_OF_MODE_POSITION,
            )
            .add_assertion(assert_view_has_text_selection(false))
            .add_assertion(|app, window_id| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                terminal_view.read(app, |terminal_view, ctx| {
                    let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                    ai_block.read(ctx, |ai_block, ctx| {
                        let is_selected_text_correct =
                            ai_block.selected_text(ctx).is_some_and(|selected_text| {
                                selected_text == AI_BLOCK_TEXT_UP_TO_MIDDLE_OF_MODE
                            });
                        async_assert!(
                            is_selected_text_correct,
                            "AI block should be selected up to the click position, not the \
                             whole block"
                        )
                    })
                })
            }),
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                .add_assertion(assert_clipboard_contains_string(
                    "echo \"this is the first block\"
this is the first block
echo \"now its the second block\"
now its the second block
"
                    .to_owned()
                        + AI_BLOCK_TEXT_UP_TO_MIDDLE_OF_MODE,
                ))
                // Extending only as far as the click position must not also pull in the
                // trailing command block.
                .add_named_assertion("does not include the last block", |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, ctx| {
                        let contains_last_block = view
                            .selected_text(ctx)
                            .is_some_and(|text| text.contains("hello Im the third block"));
                        async_assert!(
                            !contains_last_block,
                            "Selection should not include the last block"
                        )
                    })
                }),
        )
}

/// Reverse-direction direct Shift+click extension: the fixed head starts in the last block and
/// the click extends backward across the AI block into the first block (finding 2's "reverse
/// crossings" case; PRODUCT rule 3 also covers reversal past the fixed endpoint).
pub fn test_shift_click_extends_backward_through_ai_block_to_first_block() -> Builder {
    builder_with_setup()
        .with_step(
            new_step_with_default_assertions("start a small selection in the last block")
                .with_event(Event::LeftMouseDown {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *END_OF_LAST_BLOCK_POSITION - vec2f(50., 0.),
                    modifiers: Default::default(),
                })
                .with_event(Event::LeftMouseUp {
                    position: *END_OF_LAST_BLOCK_POSITION - vec2f(50., 0.),
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false)),
        )
        .with_step(
            shift_click(
                "Shift+click at the start of the first block",
                *START_OF_FIRST_BLOCK_POSITION,
            )
            .add_assertion(assert_view_has_text_selection(false))
            .add_assertion(|app, window_id| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                terminal_view.read(app, |terminal_view, ctx| {
                    let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                    ai_block.read(ctx, |ai_block, ctx| {
                        let is_selected_text_correct = ai_block
                            .selected_text(ctx)
                            .is_some_and(|selected_text| selected_text == FULL_AI_BLOCK_TEXT);
                        async_assert!(
                            is_selected_text_correct,
                            "AI block should be fully selected"
                        )
                    })
                })
            }),
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                .add_assertion(assert_clipboard_contains_string(
                    "echo \"this is the first block\"
this is the first block
echo \"now its the second block\"
now its the second block
"
                    .to_owned()
                        + FULL_AI_BLOCK_TEXT
                        + "
echo \"hello Im the third block\"
hello Im the third block",
                )),
        )
}

/// Regression test for the review finding on `SelectableArea::on_mouse_down`: a completed
/// point-based selection leaves each rich-content block it crossed with an external
/// `SelectionBound` but `is_selecting = false`. A later Shift+click landing *inside* that same
/// rich-content block must still re-extend the point-based selection (shrinking it, since the
/// new endpoint is closer to the fixed head than the old one), rather than being swallowed by
/// the AI block's own clear-and-begin path — which would leave the terminal model's selection
/// stale (still including the last block) while also starting an unrelated local selection
/// inside the AI block.
pub fn test_shift_click_reextends_within_a_previously_crossed_ai_block() -> Builder {
    builder_with_setup()
        .with_step(
            new_step_with_default_assertions(
                "drag from the first block through the AI block to the last block",
            )
            .with_event(Event::LeftMouseDown {
                position: *START_OF_FIRST_BLOCK_POSITION,
                modifiers: Default::default(),
                click_count: 1,
                is_first_mouse: false,
            })
            .with_event(Event::LeftMouseDragged {
                position: *END_OF_LAST_BLOCK_POSITION,
                modifiers: Default::default(),
            })
            .with_event(Event::LeftMouseUp {
                position: *END_OF_LAST_BLOCK_POSITION,
                modifiers: Default::default(),
            })
            .add_assertion(assert_view_has_text_selection(false))
            .add_assertion(|app, window_id| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                terminal_view.read(app, |view, ctx| {
                    let contains_last_block = view
                        .selected_text(ctx)
                        .is_some_and(|text| text.contains("hello Im the third block"));
                    async_assert!(
                        contains_last_block,
                        "Initial drag should select through the last block"
                    )
                })
            }),
        )
        .with_step(
            shift_click(
                "Shift+click back inside the already-crossed AI block",
                *MIDDLE_OF_MODE_POSITION,
            )
            .add_assertion(assert_view_has_text_selection(false))
            .add_assertion(|app, window_id| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                terminal_view.read(app, |terminal_view, ctx| {
                    let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                    ai_block.read(ctx, |ai_block, ctx| {
                        let is_selected_text_correct =
                            ai_block.selected_text(ctx).is_some_and(|selected_text| {
                                selected_text == AI_BLOCK_TEXT_UP_TO_MIDDLE_OF_MODE
                            });
                        async_assert!(
                            is_selected_text_correct,
                            "AI block should be selected up to the click position after \
                             re-extending into it, not the whole block"
                        )
                    })
                })
            })
            .add_named_assertion(
                "no longer includes the last block",
                |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, ctx| {
                        let contains_last_block = view
                            .selected_text(ctx)
                            .is_some_and(|text| text.contains("hello Im the third block"));
                        async_assert!(
                            !contains_last_block,
                            "Re-extending into the AI block should shrink the selection so it no \
                         longer includes the last block"
                        )
                    })
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                .add_assertion(assert_clipboard_contains_string(
                    "echo \"this is the first block\"
this is the first block
echo \"now its the second block\"
now its the second block
"
                    .to_owned()
                        + AI_BLOCK_TEXT_UP_TO_MIDDLE_OF_MODE,
                )),
        )
}

/// Reverse-direction counterpart to `test_shift_click_extends_from_first_block_into_ai_block`:
/// the fixed head starts in the last block and the click lands inside the AI block instead of
/// passing through it, exercising `TerminalView::prime_rich_content_selections_for_cross_block_selection`'s
/// `is_before_head` branch (`AIBlock::extend_selection_from_max_point_to`), which none of the
/// forward-direction tests above reach.
pub fn test_shift_click_extends_backward_from_last_block_into_ai_block() -> Builder {
    builder_with_setup()
        .with_step(
            new_step_with_default_assertions("start a small selection in the last block")
                .with_event(Event::LeftMouseDown {
                    position: *END_OF_LAST_BLOCK_POSITION,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *END_OF_LAST_BLOCK_POSITION - vec2f(50., 0.),
                    modifiers: Default::default(),
                })
                .with_event(Event::LeftMouseUp {
                    position: *END_OF_LAST_BLOCK_POSITION - vec2f(50., 0.),
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false)),
        )
        .with_step(
            shift_click(
                "Shift+click backward into the middle of the AI block",
                *MIDDLE_OF_MODE_POSITION,
            )
            .add_assertion(assert_view_has_text_selection(false))
            .add_assertion(|app, window_id| {
                let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                terminal_view.read(app, |terminal_view, ctx| {
                    let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                    ai_block.read(ctx, |ai_block, ctx| {
                        let is_selected_text_correct =
                            ai_block.selected_text(ctx).is_some_and(|selected_text| {
                                selected_text == *AI_BLOCK_TEXT_FROM_MIDDLE_OF_MODE_TO_END
                            });
                        async_assert!(
                            is_selected_text_correct,
                            "AI block should be selected from the click position onward, not \
                             the whole block"
                        )
                    })
                })
            })
            .add_named_assertion(
                "does not include the first block",
                |app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |view, ctx| {
                        let contains_first_block = view
                            .selected_text(ctx)
                            .is_some_and(|text| text.contains("this is the first block"));
                        async_assert!(
                            !contains_first_block,
                            "Selection should not include the first block"
                        )
                    })
                },
            ),
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                .add_assertion(assert_clipboard_contains_string(
                    AI_BLOCK_TEXT_FROM_MIDDLE_OF_MODE_TO_END.to_string(),
                )),
        )
}

pub fn test_selection_ai_to_first_lines() -> Builder {
    builder_with_setup()
        .with_step(
            // Drag from the middle of the word "mo|de" in the ai block output to the top left of the first block.
            // Triple click is lines selection.
            new_step_with_default_assertions("start selecting")
                .with_event(Event::LeftMouseDown {
                    position: *MIDDLE_OF_MODE_POSITION,
                    modifiers: Default::default(),
                    click_count: 3,
                    is_first_mouse: false,
                })
                .with_event(Event::LeftMouseDragged {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(true)),
        )
        .with_step(
            new_step_with_default_assertions("end selecting")
                .with_event(Event::LeftMouseUp {
                    position: *START_OF_FIRST_BLOCK_POSITION,
                    modifiers: Default::default(),
                })
                .add_assertion(assert_view_has_text_selection(false))
                .add_assertion(|app, window_id| {
                    let terminal_view = single_terminal_view_for_tab(app, window_id, 0);
                    terminal_view.read(app, |terminal_view, ctx| {
                        let ai_block = terminal_view.last_ai_block().expect("AI block exists");
                        ai_block.read(ctx, |ai_block, _| {
                            let is_lines_selection =
                                matches!(ai_block.selection_type(), SelectionType::Lines);
                            let is_selected_text_correct =
                                ai_block.selected_text(ctx).is_some_and(|selected_text| {
                                    selected_text
                                        == "~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode and this is my dummy output. Hope that answers your question."
                                });
                            async_assert!(
                                is_lines_selection && is_selected_text_correct,
                                "AI block has expected selection"
                            )
                        })
                    })
                }),
        )
        .with_step(
            new_step_with_default_assertions("Copy selection")
                .with_keystrokes(&[cmd_or_ctrl_shift("c")])
                // TODO(INT-339): There should be a "T" to the left of the query "Can you produce some dummy output for me?"
                // because of the dummy user avatar having a first initial instead of an image.
                // However, it appears next to "This is a dummy title" because it's organized as a flex row
                // with two flex column elements, and flex row selections read from children from left to right.
                // The flex element needs to be smarter about handling selections for this case.
                .add_assertion(assert_clipboard_contains_string(
                    "echo \"this is the first block\"
this is the first block
echo \"now its the second block\"
now its the second block
~
Can you produce some dummy output for me?
T This is a dummy title
•  Hi, I am agent mode and this is my dummy output. Hope that answers your question."
                        .into(),
                )),
        )
}
