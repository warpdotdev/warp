//! Integration tests for text selection and copying functionality in AI blocks.
//! This module tests AI blocks with markdown **enabled**.
//! There are no tests with markdown disabled because Agent Mode Markdown has been fully rolled out.
use std::{collections::HashMap, path::PathBuf, time::Duration};

use super::new_builder;
use crate::{util::skip_if_powershell_core_2303, Builder};
use lazy_static::lazy_static;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use settings::ToggleableSetting;
use warp::{
    cmd_or_ctrl_shift,
    features::FeatureFlag,
    integration_testing::{
        clipboard::assert_clipboard_contains_string,
        step::new_step_with_default_assertions,
        terminal::{
            assert_view_has_text_selection, clear_blocklist_to_remove_bootstrapped_blocks,
            execute_echo_str, wait_until_bootstrapped_single_pane_for_tab,
        },
        view_getters::single_terminal_view_for_tab,
    },
    settings::SelectionSettings,
};
use warp_multi_agent_api as api;
use warpui::{async_assert, integration::TestStep, text::SelectionType, Event, SingletonEntity};

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
