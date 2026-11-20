use crate::benchmarks::{BenchDevice, BenchDeviceHandler};
use candle_core::{DType, Device, Tensor};
use criterion::{criterion_group, Criterion, Throughput};
use std::hint::black_box;
use std::time::Instant;

fn run(w: &Tensor, bias: &Tensor) {
    w.broadcast_add(bias).unwrap();
}

fn run_bias_benchmark(c: &mut Criterion, device: &Device, dtype: DType, name: &str) {
    // We simulate a candle-nn style conv2d + bias forward pass.
    let batch_size = 1;
    let ch = 1;
    let m = 126;
    let bias_size = 128;

    let x = Tensor::ones((batch_size, ch, m, m), dtype, device).unwrap();
    let bias = Tensor::ones((1, bias_size, 1, 1), dtype, device).unwrap();

    let output_size = batch_size * bias_size * m * m;

    let flops = output_size * dtype.size_in_bytes();

    let mut group = c.benchmark_group(device.bench_name(name));
    group.throughput(Throughput::Bytes(flops as u64));
    group.bench_function("iter", move |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _i in 0..iters {
                run(black_box(&x), black_box(&bias));
            }
            device.sync().unwrap();
            start.elapsed()
        })
    });
    group.finish();
}

fn run_scalar_broadcast_benchmark(c: &mut Criterion, device: &Device, dtype: DType, name: &str) {
    let x = Tensor::ones((1, 256, 64), dtype, device).unwrap();
    let bias = Tensor::ones((1,), dtype, device).unwrap();

    let flops = 256 * 64 * dtype.size_in_bytes();

    let mut group = c.benchmark_group(device.bench_name(name));
    group.throughput(Throughput::Bytes(flops as u64));
    group.bench_function("iter", move |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _i in 0..iters {
                run(black_box(&x), black_box(&bias));
            }
            device.sync().unwrap();
            start.elapsed()
        })
    });
    group.finish();
}

fn run_contiguous_add_benchmark(c: &mut Criterion, device: &Device, dtype: DType, name: &str) {
    let bias_size = 128;
    let m = 126;

    let a = Tensor::ones((bias_size, m, m), dtype, device).unwrap();
    let b = Tensor::ones((bias_size, m, m), dtype, device).unwrap();

    let flops = 3 * bias_size * m * m * dtype.size_in_bytes();

    let mut group = c.benchmark_group(device.bench_name(name));
    group.throughput(Throughput::Bytes(flops as u64));
    group.bench_function("iter", move |b_| {
        b_.iter_custom(|iters| {
            let start = Instant::now();
            for _i in 0..iters {
                run(black_box(&a), black_box(&b));
            }
            device.sync().unwrap();
            start.elapsed()
        })
    });
    group.finish();
}

fn criterion_benchmark(c: &mut Criterion) {
    let handler = BenchDeviceHandler::new().unwrap();
    for device in handler.devices {
        run_contiguous_add_benchmark(c, &device, DType::F32, "broadcast_add_contiguous_f32");
        run_contiguous_add_benchmark(c, &device, DType::F16, "broadcast_add_contiguous_f16");
        run_contiguous_add_benchmark(c, &device, DType::BF16, "broadcast_add_contiguous_bf16");
        run_bias_benchmark(c, &device, DType::F32, "broadcast_add_f32");
        run_bias_benchmark(c, &device, DType::F16, "broadcast_add_f16");
        run_bias_benchmark(c, &device, DType::BF16, "broadcast_add_bf16");
        run_scalar_broadcast_benchmark(c, &device, DType::F32, "broadcast_scalar_add_f32");
        run_scalar_broadcast_benchmark(c, &device, DType::F16, "broadcast_scalar_add_f16");
        run_scalar_broadcast_benchmark(c, &device, DType::BF16, "broadcast_scalar_add_bf16");
    }
}

criterion_group!(benches, criterion_benchmark);
