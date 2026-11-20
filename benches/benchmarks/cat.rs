use crate::benchmarks::{BenchDevice, BenchDeviceHandler};
use candle_core::{DType, Device, Tensor};
use criterion::{criterion_group, Criterion, Throughput};
use std::hint::black_box;
use std::time::Instant;

// Shape: [batch, seq, hidden] — representative of transformer KV-cache growth.
const BATCH: usize = 4;
const SEQ: usize = 128;
const HIDDEN: usize = 64;
const N: usize = 8;

fn run_cat(tensors: &[Tensor], dim: usize) {
    Tensor::cat(tensors, dim).unwrap();
}

/// Build `n` contiguous tensors of shape [BATCH, SEQ, HIDDEN].
fn make_contiguous(device: &Device) -> [Tensor; N] {
    std::array::from_fn::<_, N, _>(|_| {
        Tensor::zeros((BATCH, SEQ, HIDDEN), DType::F32, device).unwrap()
    })
}

/// Build `n` non-contiguous tensors by transposing a contiguous tensor so that
/// the underlying storage is no longer a simple row-major slice.
fn make_non_contiguous(device: &Device) -> [Tensor; N] {
    std::array::from_fn::<_, N, _>(|_| {
        Tensor::zeros((HIDDEN, SEQ, BATCH), DType::F32, device)
            .unwrap()
            // transpose dims 0 and 2 -> shape [BATCH, SEQ, HIDDEN], non-contiguous
            .transpose(0, 2)
            .unwrap()
    })
}

fn bench_cat(c: &mut Criterion, device: &Device) {
    let total_bytes = (N * BATCH * SEQ * HIDDEN * DType::F32.size_in_bytes()) as u64;

    // contiguous, dim 0
    {
        let tensors = make_contiguous(device);
        let mut group = c.benchmark_group(device.bench_name("cat_dim0"));
        group.throughput(Throughput::Bytes(total_bytes));
        group.bench_function("iter", move |b| {
            b.iter_custom(|iters| {
                let start = Instant::now();
                for _ in 0..iters {
                    run_cat(black_box(&tensors), 0);
                }
                device.sync().unwrap();
                start.elapsed()
            })
        });
        group.finish();
    }

    // contiguous, dim 1
    {
        let tensors = make_contiguous(device);
        let mut group = c.benchmark_group(device.bench_name("cat_dim1"));
        group.throughput(Throughput::Bytes(total_bytes));
        group.bench_function("iter", move |b| {
            b.iter_custom(|iters| {
                let start = Instant::now();
                for _ in 0..iters {
                    run_cat(black_box(&tensors), 1);
                }
                device.sync().unwrap();
                start.elapsed()
            })
        });
        group.finish();
    }

    // strided, dim 0 -> cat0 path
    {
        let tensors = make_non_contiguous(device);
        let mut group = c.benchmark_group(device.bench_name("cat_strided_dim0"));
        group.throughput(Throughput::Bytes(total_bytes));
        group.bench_function("iter", move |b| {
            b.iter_custom(|iters| {
                let start = Instant::now();
                for _ in 0..iters {
                    run_cat(black_box(&tensors), 0);
                }
                device.sync().unwrap();
                start.elapsed()
            })
        });
        group.finish();
    }

    // non-contiguous, dim 1 -> transpose -> cat0 -> transpose path
    {
        let tensors = make_non_contiguous(device);
        let mut group = c.benchmark_group(device.bench_name("cat_strided_dim1"));
        group.throughput(Throughput::Bytes(total_bytes));
        group.bench_function("iter", move |b| {
            b.iter_custom(|iters| {
                let start = Instant::now();
                for _ in 0..iters {
                    run_cat(black_box(&tensors), 1);
                }
                device.sync().unwrap();
                start.elapsed()
            })
        });
        group.finish();
    }
}

fn criterion_benchmark(c: &mut Criterion) {
    let handler = BenchDeviceHandler::new().unwrap();
    for device in &handler.devices {
        bench_cat(c, device);
    }
}

criterion_group!(benches, criterion_benchmark);
