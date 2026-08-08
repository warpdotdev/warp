use byte_unit::Byte;

use super::*;
use crate::terminal::model::test_utils::TestBlockBuilder;
use crate::test_util::mock_blockgrid;

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

/// Regression test for the Windows DPC_WATCHDOG_VIOLATION caused by
/// enumerating the full process table with per-process CPU sampling.
///
/// The full-table sweep ([`SystemInfo::refresh_all_processes`], used only to
/// check whether a process with a given name is running) must NOT request CPU
/// or memory data: on Windows, per-process CPU sampling issues
/// `NtQueryInformationProcess(ProcessCycleTime)` for every process, each
/// forcing an all-core `KeFlushProcessWriteBuffers` IPI. The single-PID
/// self-poll ([`SystemInfo::refresh_kind`]) legitimately still samples both.
#[test]
fn all_processes_refresh_kind_does_not_sample_cpu_or_memory() {
    let all = SystemInfo::all_processes_refresh_kind();
    assert!(
        !all.cpu(),
        "full process-table sweep must not sample per-process CPU (forces \
         NtQueryInformationProcess(ProcessCycleTime) -> all-core IPI per process)"
    );
    assert!(
        !all.memory(),
        "full process-table sweep only needs process names, not memory"
    );

    // The single-PID self-poll should still gather CPU/memory (cheap: one PID).
    let self_poll = SystemInfo::refresh_kind();
    assert!(
        self_poll.cpu() && self_poll.memory(),
        "current-process self-poll should still sample CPU and memory"
    );
}
