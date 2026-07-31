use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use warp_tui::benchmark_support::{
    ClippedTerminalBlockBenchmark, TranscriptBenchmark, TranscriptDataset,
};

fn benchmark_clipped_terminal_block(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("tui_terminal_block/clipped_content");
    for rows in [100, 1_000] {
        let mut benchmark = ClippedTerminalBlockBenchmark::new(rows, 120, 50);
        group.bench_with_input(BenchmarkId::new("end_frame", rows), &rows, |b, _| {
            b.iter(|| black_box(benchmark.present()))
        });
    }
    group.finish();
}

fn benchmark_many_small_blocks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("tui_transcript/many_small_blocks");
    for blocks in [100, 1_000, 10_000] {
        let mut benchmark =
            TranscriptBenchmark::new(TranscriptDataset::ManySmallBlocks { blocks }, 120, 50);
        group.bench_with_input(
            BenchmarkId::new("retained_end_frame", blocks),
            &blocks,
            |b, _| b.iter(|| black_box(benchmark.present())),
        );
        benchmark.scroll_to_row(blocks / 2);
        group.bench_with_input(
            BenchmarkId::new("retained_middle_frame", blocks),
            &blocks,
            |b, _| b.iter(|| black_box(benchmark.present())),
        );
    }
    group.finish();
}

fn benchmark_long_agent_response(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("tui_transcript/long_agent_response");
    for rows in [1_000, 10_000] {
        let mut benchmark =
            TranscriptBenchmark::new(TranscriptDataset::LongAgentResponse { rows }, 120, 50);
        group.bench_with_input(
            BenchmarkId::new("retained_end_frame", rows),
            &rows,
            |b, _| b.iter(|| black_box(benchmark.present())),
        );
        benchmark.scroll_to_row(rows / 2);
        group.bench_with_input(
            BenchmarkId::new("retained_middle_frame", rows),
            &rows,
            |b, _| b.iter(|| black_box(benchmark.present())),
        );
        group.bench_with_input(
            BenchmarkId::new("invalidated_middle_frame", rows),
            &rows,
            |b, _| {
                b.iter(|| {
                    benchmark.invalidate_all();
                    black_box(benchmark.present())
                })
            },
        );
        benchmark.scroll_to_end();
    }
    group.finish();
}

fn benchmark_offscreen_streaming_tail(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("tui_transcript/offscreen_streaming_tail");
    for tail_rows in [1_000, 10_000] {
        let mut benchmark = TranscriptBenchmark::new(
            TranscriptDataset::OffscreenStreamingTail {
                preceding_rows: 100,
                tail_rows,
            },
            120,
            50,
        );
        group.bench_with_input(
            BenchmarkId::new("retained_end_frame", tail_rows),
            &tail_rows,
            |b, _| b.iter(|| black_box(benchmark.present())),
        );
        benchmark.scroll_to_row(0);
        group.bench_with_input(
            BenchmarkId::new("dirty_fixed_viewport_frame", tail_rows),
            &tail_rows,
            |b, _| {
                b.iter(|| {
                    benchmark.invalidate_streaming_tail();
                    black_box(benchmark.present())
                })
            },
        );
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(1));
    targets =
        benchmark_clipped_terminal_block,
        benchmark_many_small_blocks,
        benchmark_long_agent_response,
        benchmark_offscreen_streaming_tail
}
criterion_main!(benches);
