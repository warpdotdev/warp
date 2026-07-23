use chrono::{Local, TimeZone as _};

use super::{MenuEntry, MenuItem, interleave_conversations};
use crate::input_suggestions::HistoryOrder;

#[test]
fn interleave_conversations_only_inserts_into_current_session_segment() {
    let t10 = Local.with_ymd_and_hms(2024, 1, 1, 0, 0, 10).unwrap();
    let t20 = Local.with_ymd_and_hms(2024, 1, 1, 0, 0, 20).unwrap();
    let t50 = Local.with_ymd_and_hms(2024, 1, 1, 0, 0, 50).unwrap();
    let t100 = Local.with_ymd_and_hms(2024, 1, 1, 0, 1, 40).unwrap();
    let t150 = Local.with_ymd_and_hms(2024, 1, 1, 0, 2, 30).unwrap();
    let t200 = Local.with_ymd_and_hms(2024, 1, 1, 0, 3, 20).unwrap();

    let base = vec![
        MenuEntry {
            order: HistoryOrder::DifferentSession,
            sort_timestamp: t10,
            item: MenuItem::Command {
                command: "d10".to_string(),
                display_timestamp: t10,
                linked_workflow_data: None,
                prefix_match_len: 0,
            },
        },
        MenuEntry {
            order: HistoryOrder::DifferentSession,
            sort_timestamp: t20,
            item: MenuItem::Command {
                command: "d20".to_string(),
                display_timestamp: t20,
                linked_workflow_data: None,
                prefix_match_len: 0,
            },
        },
        MenuEntry {
            order: HistoryOrder::CurrentSession,
            sort_timestamp: t100,
            item: MenuItem::Command {
                command: "c100".to_string(),
                display_timestamp: t100,
                linked_workflow_data: None,
                prefix_match_len: 0,
            },
        },
        MenuEntry {
            order: HistoryOrder::CurrentSession,
            sort_timestamp: t200,
            item: MenuItem::Command {
                command: "c200".to_string(),
                display_timestamp: t200,
                linked_workflow_data: None,
                prefix_match_len: 0,
            },
        },
    ];

    let conversations = vec![
        MenuEntry {
            order: HistoryOrder::CurrentSession,
            sort_timestamp: t150,
            item: MenuItem::Command {
                command: "conv150".to_string(),
                display_timestamp: t150,
                linked_workflow_data: None,
                prefix_match_len: 0,
            },
        },
        MenuEntry {
            order: HistoryOrder::CurrentSession,
            sort_timestamp: t50,
            item: MenuItem::Command {
                command: "conv50".to_string(),
                display_timestamp: t50,
                linked_workflow_data: None,
                prefix_match_len: 0,
            },
        },
    ];

    let merged = interleave_conversations(base, conversations);

    let commands = merged
        .iter()
        .map(|e| match &e.item {
            MenuItem::Command { command, .. } => command.as_str(),
            MenuItem::Conversation { title, .. } => title.as_str(),
        })
        .collect::<Vec<_>>();

    // DifferentSession entries stay at the top; conversations are inserted into the CurrentSession segment.
    assert_eq!(
        commands,
        vec!["d10", "d20", "conv50", "c100", "conv150", "c200"]
    );
}
