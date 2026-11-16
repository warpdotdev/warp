use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use warp_tui::benchmark_support::{
    ZeroStateBenchmark, ZeroStateBenchmarkShape, ZeroStateProjectionBenchmark,
};

fn benchmark_zero_state_frame(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("tui_zero_state/retained_frame");
    for (width, height) in [(80, 24), (120, 40), (240, 80)] {
        for shape in [
            ZeroStateBenchmarkShape::BuiltIn,
            ZeroStateBenchmarkShape::Ascii,
        ] {
            let mut benchmark = ZeroStateBenchmark::new(shape, width, height);
            group.bench_with_input(
                BenchmarkId::new(format!("{shape:?}"), format!("{width}x{height}")),
                &(width, height),
                |b, _| b.iter(|| black_box(benchmark.present())),
            );
        }
    }
    group.finish();
}

fn benchmark_logo_projection(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("tui_zero_state/logo_projection");
    for (width, height) in [(32, 24), (32, 40), (32, 80)] {
        for shape in [
            ZeroStateBenchmarkShape::BuiltIn,
            ZeroStateBenchmarkShape::Ascii,
        ] {
            let mut benchmark = ZeroStateProjectionBenchmark::new(shape, width, height);
            group.bench_with_input(
                BenchmarkId::new(format!("{shape:?}"), format!("{width}x{height}")),
                &(width, height),
                |b, _| b.iter(|| black_box(benchmark.project())),
            );
        }
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(1));
    targets = benchmark_zero_state_frame, benchmark_logo_projection
}
criterion_main!(benches);
