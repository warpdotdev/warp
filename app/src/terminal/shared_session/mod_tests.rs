use super::{decode_scrollback, SharedSessionScrollbackType};

use crate::ai::blocklist::agent_view::AgentViewState;
use crate::assert_lines_approx_eq;
use crate::channel::ChannelState;
use crate::terminal::color::List;
use crate::terminal::model::test_utils::block_size;
use crate::uri::web_intent_parser::maybe_rewrite_web_url_to_intent;

use crate::terminal::model::ObfuscateSecrets;
use crate::terminal::TerminalModel;
use crate::terminal::{event_listener::ChannelEventListener, model::block::SerializedBlock};
use crate::themes::default_themes::dark_theme;
use serde_json::Value;
use session_sharing_protocol::common::{Scrollback, ScrollbackBlock};
use std::sync::Arc;
use url::Url;
use warpui::r#async::executor::Background;
use warpui::units::Lines;

pub const MAX_BYTES_SHAREABLE: usize = 5000;

#[test]
fn maybe_rewrite_web_url_to_shared_session_intent_rewrites_matching_web_url() {
    let server_root = ChannelState::server_root_url();
    let web_url = Url::parse(&format!(
        "{server_root}/session/00000000-0000-0000-0000-000000000000?pwd=secret&preview=true"
    ))
    .expect("valid shared session web URL");

    let maybe_intent = maybe_rewrite_web_url_to_intent(&web_url)
        .expect("expected shared session web URL to rewrite to an intent URL");

    assert_eq!(maybe_intent.scheme(), ChannelState::url_scheme());
    assert_eq!(maybe_intent.host_str(), Some("shared_session"));
    assert_eq!(maybe_intent.path(), "/00000000-0000-0000-0000-000000000000");
    assert_eq!(maybe_intent.query(), Some("pwd=secret&preview=true"));
}

#[test]
fn maybe_rewrite_web_url_to_shared_session_intent_ignores_non_matching_host() {
    let web_url =
        Url::parse("https://example.com/session/00000000-0000-0000-0000-000000000000?pwd=secret")
            .expect("valid web URL with non-matching host");

    let maybe_intent = maybe_rewrite_web_url_to_intent(&web_url);

    assert!(maybe_intent.is_none());
}

#[test]
fn maybe_rewrite_web_url_to_shared_session_intent_ignores_invalid_session_id() {
    let server_root = ChannelState::server_root_url();
    let web_url = Url::parse(&format!(
        "{server_root}/session/not-a-valid-session-id?pwd=secret&preview=true",
    ))
    .expect("valid web URL with invalid session id path segment");

    let maybe_intent = maybe_rewrite_web_url_to_intent(&web_url);

    assert!(maybe_intent.is_none());
}

pub fn terminal_model_for_viewer(event_proxy: ChannelEventListener) -> TerminalModel {
    TerminalModel::new_for_shared_session_viewer(
        block_size(),
        List::from(&dark_theme().into()),
        event_proxy,
        Arc::new(Background::default()),
        false, /* show_memory_stats */
        false, /* honor_ps1 */
        false, /* is_inverted */
        ObfuscateSecrets::No,
    )
}

#[test]
fn test_get_no_scrollback() {
    let restored_blocks = &[SerializedBlock::new_for_test("a".into(), "b".into()).into()];
    let channel_event_proxy = ChannelEventListener::new_for_test();
    let mut model = TerminalModel::mock(Some(restored_blocks), Some(channel_event_proxy));

    model.simulate_block("block1", "block1");
    model.simulate_block("block2", "block2");

    let scrollback = SharedSessionScrollbackType::None.to_scrollback(&model);
    // Should only contain the active block
    assert_eq!(scrollback.blocks.len(), 1);
}

#[test]
fn test_get_scrollback_starting_at_block() {
    let restored_blocks = &[SerializedBlock::new_for_test("a".into(), "b".into()).into()];
    let channel_event_proxy = ChannelEventListener::new_for_test();
    let mut model = TerminalModel::mock(Some(restored_blocks), Some(channel_event_proxy));

    model.simulate_block("block1", "block1");
    model.simulate_block("block2", "block2");

    let starting_block = model
        .block_list()
        .last_non_hidden_block()
        .expect("there is a non-hidden block");
    let scrollback = SharedSessionScrollbackType::FromBlock {
        block_index: starting_block.index(),
    }
    .to_scrollback(&model);

    // Should contain 1 completed block + active block
    assert_eq!(scrollback.blocks.len(), 2);
}

#[test]
fn test_get_all_scrollback() {
    let restored_blocks = &[SerializedBlock::new_for_test("a".into(), "b".into()).into()];
    let channel_event_proxy = ChannelEventListener::new_for_test();
    let mut model = TerminalModel::mock(Some(restored_blocks), Some(channel_event_proxy));

    // Restored blocks and bootstrap blocks don't count towards scrollback,
    let scrollback = SharedSessionScrollbackType::All.to_scrollback(&model);
    // Only active block
    assert_eq!(scrollback.blocks.len(), 1);

    model.simulate_block("block1", "block1");
    let scrollback = SharedSessionScrollbackType::All.to_scrollback(&model);
    assert_eq!(scrollback.blocks.len(), 2);

    model.simulate_block("block2", "block2");
    let scrollback = SharedSessionScrollbackType::All.to_scrollback(&model);

    // Should contain 2 completed blocks + active block
    assert_eq!(scrollback.blocks.len(), 3);
}

#[test]
fn test_scrollback_round_trip() {
    let channel_event_proxy = ChannelEventListener::new_for_test();
    let mut model = TerminalModel::mock(None, Some(channel_event_proxy));

    model.simulate_block("hello", "world");

    // Capture the expected stylized bytes from the completed block before serialization.
    let completed_block = model.block_list().block_at(1.into()).unwrap();
    let expected: SerializedBlock = completed_block.into();

    let scrollback = SharedSessionScrollbackType::All.to_scrollback(&model);
    let decoded = decode_scrollback(&scrollback);

    // The completed block is first; the active (empty) block is second.
    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0].stylized_command, expected.stylized_command);
    assert_eq!(decoded[0].stylized_output, expected.stylized_output);
}

#[test]
fn test_scrollback_serialization() {
    let channel_event_proxy = ChannelEventListener::new_for_test();
    let mut model = TerminalModel::mock(None, Some(channel_event_proxy));

    model.simulate_block("hello", "world");

    let scrollback = SharedSessionScrollbackType::All.to_scrollback(&model);
    let first_block = scrollback
        .blocks
        .first()
        .expect("expected first scrollback block");
    let json: Value = serde_json::from_slice(&first_block.raw).expect("valid scrollback json");

    // Capture the expected bytes from the model so we can assert exact JSON array contents.
    let completed_block = model.block_list().block_at(1.into()).unwrap();
    let expected: SerializedBlock = completed_block.into();

    let expected_command: Vec<Value> = expected
        .stylized_command
        .iter()
        .map(|&b| Value::from(b))
        .collect();
    let expected_output: Vec<Value> = expected
        .stylized_output
        .iter()
        .map(|&b| Value::from(b))
        .collect();

    assert_eq!(
        json.get("stylized_command"),
        Some(&Value::Array(expected_command)),
    );
    assert_eq!(
        json.get("stylized_output"),
        Some(&Value::Array(expected_output)),
    );
}

#[test]
fn test_scrollback_deserialization() {
    let raw = serde_json::json!({
        "id": "00000000-0000-0000-0000-000000000000",
        "stylized_command": [104, 101, 108, 108, 111],
        "stylized_output": [119, 111, 114, 108, 100],
        "pwd": null,
        "git_head": null,
        "virtual_env": null,
        "conda_env": null,
        "node_version": null,
        "exit_code": 0,
        "did_execute": true,
        "completed_ts": null,
        "start_ts": null,
        "ps1": null,
        "rprompt": null,
        "honor_ps1": false,
        "is_background": false,
        "session_id": null,
        "shell_host": null,
        "prompt_snapshot": null,
        "ai_metadata": null
    });

    let scrollback = Scrollback {
        blocks: vec![ScrollbackBlock {
            raw: serde_json::to_vec(&raw).expect("serialize scrollback json"),
        }],
        is_alt_screen_active: false,
    };

    let decoded = decode_scrollback(&scrollback);

    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].stylized_command, b"hello");
    assert_eq!(decoded[0].stylized_output, b"world");
}

#[test]
fn test_loading_scrollback() {
    let session_id = 42.into();
    let mut active_block = SerializedBlock::new_active_block_for_test();
    active_block.session_id = Some(session_id);

    let scrollback_blocks = &[
        SerializedBlock::new_for_test("block1".into(), "block1".into()),
        SerializedBlock::new_for_test("block2".into(), "block2".into()),
        // We expect the active block as part of scrollback to get the prompt.
        active_block,
    ];
    let channel_event_proxy = ChannelEventListener::new_for_test();
    let mut model = terminal_model_for_viewer(channel_event_proxy);
    model.load_shared_session_scrollback(scrollback_blocks);

    // 4 blocks: first is the bootstrap block, the next two are completed scrollback blocks.
    // The last is the active block, whose prompt came from the last scrollback.
    assert_eq!(model.block_list().blocks().len(), 4);

    assert_eq!(
        model
            .block_list()
            .block_at(1.into())
            .unwrap()
            .command_to_string(),
        "block1"
    );
    assert_eq!(
        model
            .block_list()
            .block_at(1.into())
            .unwrap()
            .output_to_string(),
        "block1"
    );

    assert_eq!(
        model
            .block_list()
            .block_at(2.into())
            .unwrap()
            .command_to_string(),
        "block2"
    );
    assert_eq!(
        model
            .block_list()
            .block_at(2.into())
            .unwrap()
            .output_to_string(),
        "block2"
    );

    // The last scrollback block is the active block and contains the prompt.
    assert_eq!(model.block_list().active_block_index(), 3.into());
    assert_eq!(
        model
            .block_list()
            .active_block()
            .height(&AgentViewState::Inactive),
        Lines::zero()
    );
    assert!(!model.block_list().active_block().started());
    assert_eq!(
        model.block_list().active_block().session_id(),
        Some(session_id)
    );
}

#[test]
fn test_loading_scrollback_with_completed_last_block_creates_active_block() {
    let scrollback_blocks = &[
        SerializedBlock::new_for_test("block1".into(), "block1".into()),
        SerializedBlock::new_for_test("block2".into(), "block2".into()),
    ];
    let channel_event_proxy = ChannelEventListener::new_for_test();
    let mut model = terminal_model_for_viewer(channel_event_proxy);
    model.load_shared_session_scrollback(scrollback_blocks);

    // 4 blocks: first is the bootstrap block, the next two are completed scrollback blocks.
    // Since no active block was serialized, restore creates a fresh active block.
    assert_eq!(model.block_list().blocks().len(), 4);

    assert_eq!(
        model
            .block_list()
            .block_at(1.into())
            .unwrap()
            .command_to_string(),
        "block1"
    );
    assert_eq!(
        model
            .block_list()
            .block_at(2.into())
            .unwrap()
            .command_to_string(),
        "block2"
    );

    assert_eq!(model.block_list().active_block_index(), 3.into());
    assert_eq!(
        model
            .block_list()
            .active_block()
            .height(&AgentViewState::Inactive),
        Lines::zero()
    );
    assert!(!model.block_list().active_block().started());
}

#[test]
fn test_loading_scrollback_in_alt_screen() {
    let scrollback_blocks = &[
        SerializedBlock::new_for_test("block1".into(), "block1".into()),
        // We expect the active block as part of scrollback to get the prompt.
        SerializedBlock::new_active_block_for_test(),
    ];
    let channel_event_proxy = ChannelEventListener::new_for_test();
    let mut model = terminal_model_for_viewer(channel_event_proxy);
    model.load_shared_session_scrollback(scrollback_blocks);
    model.enter_alt_screen(true);

    // 3 blocks: first is the bootstrap block, the second is the completed scrollback blocks.
    // The last is the active block, whose prompt came from the last scrollback.
    assert_eq!(model.block_list().blocks().len(), 3);

    assert_eq!(
        model
            .block_list()
            .block_at(1.into())
            .unwrap()
            .command_to_string(),
        "block1"
    );
    assert_eq!(
        model
            .block_list()
            .block_at(1.into())
            .unwrap()
            .output_to_string(),
        "block1"
    );

    // The last scrollback block is the active block and contains the prompt.
    assert_lines_approx_eq!(
        model
            .block_list()
            .block_at(2.into())
            .unwrap()
            .height(&AgentViewState::Inactive),
        0.
    );
    assert!(!model.block_list().block_at(2.into()).unwrap().started());

    // Make sure we're in the alt screen.
    assert!(model.is_alt_screen_active());
}
