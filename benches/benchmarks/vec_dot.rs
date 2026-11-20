use candle_core::cpu::kernels::VecOps;
use criterion::{criterion_group, BatchSize, Criterion, Throughput};
use half::{bf16, f16};

fn bench_vec_dot_f32(c: &mut Criterion, k: usize) {
    let a: Vec<f32> = (0..k).map(|i| i as f32).collect();
    let b: Vec<f32> = (0..k).map(|i| i as f32 * 0.5).collect();
    let mut out = vec![0.0; k];

    let mut group = c.benchmark_group(format!("cpu_vec_dot_f32_k{k}"));
    group.throughput(Throughput::Elements(k as u64));
    group.bench_function("iter", |bench| {
        bench.iter_batched(
            || (),
            |_| unsafe {
                f32::vec_dot(
                    std::hint::black_box(a.as_ptr()),
                    std::hint::black_box(b.as_ptr()),
                    std::hint::black_box(out.as_mut_ptr()),
                    std::hint::black_box(k),
                )
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

fn bench_vec_dot_f16(c: &mut Criterion, k: usize) {
    let a: Vec<f16> = (0..k).map(|i| f16::from_f32(i as f32)).collect();
    let b: Vec<f16> = (0..k).map(|i| f16::from_f32(i as f32 * 0.5)).collect();

    let mut group = c.benchmark_group(format!("cpu_vec_dot_f16_k{k}"));
    group.throughput(Throughput::Elements(k as u64));
    group.bench_function("iter", |bench| {
        bench.iter_batched(
            || f16::ZERO,
            |mut out| unsafe {
                f16::vec_dot(a.as_ptr(), b.as_ptr(), &mut out, k);
                out
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

fn bench_vec_dot_bf16(c: &mut Criterion, k: usize) {
    let a: Vec<bf16> = (0..k).map(|i| bf16::from_f32(i as f32)).collect();
    let b: Vec<bf16> = (0..k).map(|i| bf16::from_f32(i as f32 * 0.5)).collect();

    let mut group = c.benchmark_group(format!("cpu_vec_dot_bf16_k{k}"));
    group.throughput(Throughput::Elements(k as u64));
    group.bench_function("iter", |bench| {
        bench.iter_batched(
            || bf16::ZERO,
            |mut out| unsafe {
                bf16::vec_dot(a.as_ptr(), b.as_ptr(), &mut out, k);
                out
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

fn criterion_benchmark(c: &mut Criterion) {
    for k in [64, 128, 256, 512] {
        bench_vec_dot_f32(c, k);
        bench_vec_dot_f16(c, k);
        bench_vec_dot_bf16(c, k);
    }
}

criterion_group!(benches, criterion_benchmark);
