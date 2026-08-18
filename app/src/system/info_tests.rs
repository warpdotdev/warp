use byte_unit::Byte;

use super::*;
use crate::terminal::model::test_utils::TestBlockBuilder;
use crate::test_util::mock_blockgrid;

const THRESHOLD_BYTES: u64 = 10 * 1024 * 1024 * 1024;

#[test]
fn decide_memory_warning_ignores_a_footprint_under_threshold() {
    let (pending, decision) = decide_memory_warning(None, THRESHOLD_BYTES - 1, THRESHOLD_BYTES);
    assert_eq!(pending, None);
    assert_eq!(decision, MemoryWarningDecision::NoAction);
}

#[test]
fn decide_memory_warning_starts_tracking_a_first_crossing_without_reporting() {
    let (pending, decision) = decide_memory_warning(None, THRESHOLD_BYTES, THRESHOLD_BYTES);
    assert_eq!(pending, Some(THRESHOLD_BYTES));
    assert_eq!(decision, MemoryWarningDecision::NoAction);
}

#[test]
fn decide_memory_warning_confirms_a_sustained_crossing() {
    let (pending, decision) = decide_memory_warning(
        Some(THRESHOLD_BYTES),
        THRESHOLD_BYTES + 1024,
        THRESHOLD_BYTES,
    );
    assert_eq!(pending, None);
    assert_eq!(decision, MemoryWarningDecision::Confirmed);
}

#[test]
fn decide_memory_warning_reports_a_transient_spike_and_clears_the_pending_state() {
    let (pending, decision) = decide_memory_warning(
        Some(THRESHOLD_BYTES + 4096),
        THRESHOLD_BYTES - 1024,
        THRESHOLD_BYTES,
    );
    assert_eq!(pending, None);
    assert_eq!(
        decision,
        MemoryWarningDecision::Transient {
            triggering_footprint_bytes: THRESHOLD_BYTES + 4096,
            confirmation_footprint_bytes: THRESHOLD_BYTES - 1024,
        }
    );
}

#[test]
fn decide_memory_warning_can_start_a_fresh_cycle_after_a_transient_skip() {
    // A skip must not permanently disable detection: after a transient spike
    // is dismissed, a later sustained crossing should still be trackable and
    // eventually confirmed.
    let (pending, decision) = decide_memory_warning(None, THRESHOLD_BYTES, THRESHOLD_BYTES);
    assert_eq!(pending, Some(THRESHOLD_BYTES));
    assert_eq!(decision, MemoryWarningDecision::NoAction);

    let (pending, decision) = decide_memory_warning(pending, THRESHOLD_BYTES - 1, THRESHOLD_BYTES);
    assert_eq!(pending, None);
    assert!(matches!(decision, MemoryWarningDecision::Transient { .. }));

    // A brand new crossing after the skip starts a fresh cycle.
    let (pending, decision) = decide_memory_warning(pending, THRESHOLD_BYTES, THRESHOLD_BYTES);
    assert_eq!(pending, Some(THRESHOLD_BYTES));
    assert_eq!(decision, MemoryWarningDecision::NoAction);

    let (pending, decision) = decide_memory_warning(pending, THRESHOLD_BYTES, THRESHOLD_BYTES);
    assert_eq!(pending, None);
    assert_eq!(decision, MemoryWarningDecision::Confirmed);
}

#[test]
fn test_memory_usage_stats_construction() {
    let total_application_usage_bytes = 1024;
    let mut stats = MemoryUsageStats::new(Byte::from_u64(total_application_usage_bytes));

    let now = Local::now();

    let mut block_with_content = TestBlockBuilder::new().build();
    block_with_content.set_prompt_and_command_grid(mock_blockgrid("line1\nline2"));
    block_with_content.set_output_grid(mock_blockgrid("line3"));
    block_with_content.update_last_painted_at(now);

    let inactive_5m_block = TestBlockBuilder::new().build();
    inactive_5m_block.update_last_painted_at(now - chrono::Duration::minutes(10));

    let inactive_1h_block1 = TestBlockBuilder::new().build();
    inactive_1h_block1.update_last_painted_at(now - chrono::Duration::minutes(70));

    let inactive_1h_block2 = TestBlockBuilder::new().build();
    inactive_1h_block2.update_last_painted_at(now - chrono::Duration::minutes(70));

    let blocks = [
        block_with_content,
        inactive_5m_block,
        inactive_1h_block1,
        inactive_1h_block2,
        TestBlockBuilder::new().build(),
    ];

    stats.add_blocks(now, blocks.iter());

    assert_eq!(
        stats.total_application_usage_bytes,
        total_application_usage_bytes as usize
    );
    assert_eq!(stats.total_blocks, 5);
    assert_eq!(stats.total_lines, 3);

    assert_eq!(stats.active_block_stats.num_blocks, 1);
    assert_eq!(stats.active_block_stats.num_lines, 3);

    assert_eq!(stats.inactive_5m_stats.num_blocks, 1);
    assert_eq!(stats.inactive_5m_stats.num_lines, 0);

    assert_eq!(stats.inactive_1h_stats.num_blocks, 2);
    assert_eq!(stats.inactive_1h_stats.num_lines, 0);

    assert_eq!(stats.inactive_24h_stats.num_blocks, 1);
    assert_eq!(stats.inactive_24h_stats.num_lines, 0);
}
