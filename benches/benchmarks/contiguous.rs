use crate::benchmarks::{BenchDevice, BenchDeviceHandler};
use candle_core::{DType, Device, Tensor};
use criterion::{criterion_group, Criterion, Throughput};
use std::hint::black_box;
use std::time::Instant;

// Representative transformer KV-cache shape: [batch, seq, head_dim].
const BATCH: usize = 8;
const SEQ: usize = 512;
const HIDDEN: usize = 64;
const PAD: usize = 16;

fn run_contiguous(t: &Tensor) {
    t.contiguous().unwrap();
}

fn bench_contiguous(c: &mut Criterion, device: &Device) {
    let bytes = (BATCH * SEQ * HIDDEN * DType::F32.size_in_bytes()) as u64;

    // Contiguous baseline:
    // Close to theoretical best case / single optimised copy.
    // This is for comparison with the other benches. We use `force_contigious`, because we're
    // not interested in the performance of `Arc::clone()`
    {
        let t = Tensor::zeros((BATCH, SEQ, HIDDEN), DType::F32, device).unwrap();
        let mut group = c.benchmark_group(device.bench_name("contiguous"));
        group.throughput(Throughput::Bytes(bytes));
        group.bench_function("iter", |b| {
            b.iter_custom(|iters| {
                let start = Instant::now();
                for _ in 0..iters {
                    black_box(t.force_contiguous().unwrap());
                }
                device.sync().unwrap();
                start.elapsed()
            })
        });
        group.finish();
    }

    // UniformBlocks
    // Narrow a padded tensor so only the outer stride differs.
    // narrow(1, 0, SEQ) on [BATCH, SEQ + PAD, HIDDEN] gives strides
    // [HIDDEN * (SEQ + PAD), HIDDEN, 1]. Inner block of SEQ * HIDDEN is fully contiguous.
    // This is the layout that copy2d targets.
    {
        let base = Tensor::zeros((BATCH, SEQ + PAD, HIDDEN), DType::F32, device).unwrap();
        let t = base.narrow(1, 0, SEQ).unwrap();
        assert!(
            !t.is_contiguous(),
            "expected non-contiguous tensor for benchmark"
        );

        let mut group = c.benchmark_group(device.bench_name("contiguous_uniform"));
        group.throughput(Throughput::Bytes(bytes));
        group.bench_function("iter", |b| {
            b.iter_custom(|iters| {
                let start = Instant::now();
                for _ in 0..iters {
                    run_contiguous(black_box(&t));
                }
                device.sync().unwrap();
                start.elapsed()
            })
        });
        group.finish();
    }

    // MultipleBlocks
    // Transpose mixes dimensions so the innermost stride is no longer 1.
    // `contiguous()` must use the general strided kernel.
    {
        let base = Tensor::zeros((BATCH, SEQ, HIDDEN), DType::F32, device).unwrap();
        // transpose(0, 1) - shape: [SEQ, BATCH, HIDDEN], strides: [HIDDEN, SEQ * HIDDEN, 1]
        // aka MultipleBlocks { block_len = HIDDEN } (SEQ * BATCH blocks).
        let t = base.transpose(0, 1).unwrap();
        assert!(
            !t.is_contiguous(),
            "expected non-contiguous tensor for benchmark"
        );

        let mut group = c.benchmark_group(device.bench_name("contiguous_strided"));
        group.throughput(Throughput::Bytes(bytes));
        group.bench_function("iter", |b| {
            b.iter_custom(|iters| {
                let start = Instant::now();
                for _ in 0..iters {
                    run_contiguous(black_box(&t));
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
        bench_contiguous(c, device);
    }
}

criterion_group!(benches, criterion_benchmark);
