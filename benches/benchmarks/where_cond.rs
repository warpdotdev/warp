use crate::benchmarks::{BenchDevice, BenchDeviceHandler};
use candle_core::{DType, Device, Tensor};
use criterion::{criterion_group, Criterion, Throughput};
use std::hint::black_box;
use std::time::Instant;

fn run(a: &Tensor, b: &Tensor, c: &Tensor) {
    a.where_cond(b, c).unwrap();
}

const fn create_cond_arr<const N: usize>() -> [u8; N] {
    let mut arr = [0u8; N];
    let mut i = 0;
    while i < N {
        arr[i] = (i % 2) as u8;
        i += 1;
    }
    arr
}

const fn create_causal_arr<const S: usize, const N: usize>() -> [u8; N] {
    let mut arr = [0u8; N];
    let mut i = 0;
    while i < N {
        let row = (i / S) % S;
        let col = i % S;
        arr[i] = if col > row { 1 } else { 0 };
        i += 1;
    }
    arr
}

const B: usize = 1;
const M: usize = 1024;
const K: usize = 1024;
const SIZE: usize = B * M * K;

static DATA: [u8; SIZE] = create_cond_arr::<SIZE>();

fn run_where_cond_benchmark(c: &mut Criterion, device: &Device, dtype: DType, name: &str) {
    let tensor = Tensor::from_slice(DATA.as_slice(), (B, M, K), device).unwrap();
    let on_true = Tensor::ones((B, M, K), dtype, device).unwrap();
    let on_false = Tensor::zeros((B, M, K), dtype, device).unwrap();

    let elements = B * M * K;
    // E.g. 2 f32 tensors + 1 u8 tensor
    let flops = (2 * elements * dtype.size_in_bytes()) + elements;

    let mut group = c.benchmark_group(device.bench_name(name));
    group.throughput(Throughput::Bytes(flops as u64));
    group.bench_function("iter", move |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _i in 0..iters {
                run(
                    black_box(&tensor),
                    black_box(&on_true),
                    black_box(&on_false),
                );
            }
            device.sync().unwrap();
            start.elapsed()
        })
    });
    group.finish();
}

const AB: usize = 1;
const AH: usize = 4;
const AS: usize = 512;

static CAUSAL: [u8; AB * AH * AS * AS] = create_causal_arr::<AS, { AB * AH * AS * AS }>();

fn run_masked_fill_benchmark(c: &mut Criterion, device: &Device, dtype: DType, name: &str) {
    let shape = (AB, AH, AS, AS);
    let tensor = Tensor::from_slice(CAUSAL.as_slice(), shape, device).unwrap();
    let on_true = Tensor::new(1.0, device)
        .unwrap()
        .to_dtype(dtype)
        .unwrap()
        .broadcast_as(shape)
        .unwrap();
    let on_false = Tensor::zeros(shape, dtype, device).unwrap();

    let elements = AB * AH * AS * AS;
    let flops = (2 * elements * dtype.size_in_bytes()) + elements;

    let mut group = c.benchmark_group(device.bench_name(name));
    group.throughput(Throughput::Bytes(flops as u64));
    group.bench_function("iter", move |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _i in 0..iters {
                run(
                    black_box(&tensor),
                    black_box(&on_true),
                    black_box(&on_false),
                );
            }
            device.sync().unwrap();
            start.elapsed()
        })
    });
    group.finish();
}

fn run_masked_fill_bcast_benchmark(c: &mut Criterion, device: &Device, dtype: DType, name: &str) {
    let shape = (AB, AH, AS, AS);
    let tensor = Tensor::from_slice(&CAUSAL[..AS * AS], (1, 1, AS, AS), device)
        .unwrap()
        .broadcast_as(shape)
        .unwrap();
    let on_true = Tensor::new(1.0, device)
        .unwrap()
        .to_dtype(dtype)
        .unwrap()
        .broadcast_as(shape)
        .unwrap();
    let on_false = Tensor::zeros(shape, dtype, device).unwrap();

    let elements = AB * AH * AS * AS;
    let flops = (2 * elements * dtype.size_in_bytes()) + elements;

    let mut group = c.benchmark_group(device.bench_name(name));
    group.throughput(Throughput::Bytes(flops as u64));
    group.bench_function("iter", move |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _i in 0..iters {
                run(
                    black_box(&tensor),
                    black_box(&on_true),
                    black_box(&on_false),
                );
            }
            device.sync().unwrap();
            start.elapsed()
        })
    });
    group.finish();
}

fn run_transposed_benchmark(c: &mut Criterion, device: &Device, dtype: DType, name: &str) {
    let tensor = Tensor::from_slice(DATA.as_slice(), (M, K), device).unwrap();
    let on_true = Tensor::ones((M, K), dtype, device).unwrap();
    let on_false = Tensor::zeros((K, M), dtype, device)
        .unwrap()
        .transpose(0, 1)
        .unwrap();

    let elements = M * K;
    let flops = (2 * elements * dtype.size_in_bytes()) + elements;

    let mut group = c.benchmark_group(device.bench_name(name));
    group.throughput(Throughput::Bytes(flops as u64));
    group.bench_function("iter", move |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _i in 0..iters {
                run(
                    black_box(&tensor),
                    black_box(&on_true),
                    black_box(&on_false),
                );
            }
            device.sync().unwrap();
            start.elapsed()
        })
    });
    group.finish();
}

const NB: usize = 256;
const NS: usize = 512;
const ND_PAD: usize = 8;
const ND: usize = 3;

fn run_narrow_inner_benchmark(c: &mut Criterion, device: &Device, dtype: DType, name: &str) {
    let padded = (NB, NS, ND_PAD);
    let tensor = Tensor::from_slice(DATA.as_slice(), padded, device)
        .unwrap()
        .narrow(2, 0, ND)
        .unwrap();
    let on_true = Tensor::new(0f32, device)
        .unwrap()
        .to_dtype(dtype)
        .unwrap()
        .broadcast_as((NB, NS, ND))
        .unwrap();
    let on_false = Tensor::zeros(padded, dtype, device)
        .unwrap()
        .narrow(2, 0, ND)
        .unwrap();

    let elements = NB * NS * ND;
    let flops = (2 * elements * dtype.size_in_bytes()) + elements;

    let mut group = c.benchmark_group(device.bench_name(name));
    group.throughput(Throughput::Bytes(flops as u64));
    group.bench_function("iter", move |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _i in 0..iters {
                run(
                    black_box(&tensor),
                    black_box(&on_true),
                    black_box(&on_false),
                );
            }
            device.sync().unwrap();
            start.elapsed()
        })
    });
    group.finish();
}

fn criterion_benchmark(c: &mut Criterion) {
    let device = BenchDeviceHandler::new().unwrap();
    for d in device.devices {
        run_where_cond_benchmark(c, &d, DType::F32, "where_cond_f32");
        run_where_cond_benchmark(c, &d, DType::BF16, "where_cond_bf16");
        run_where_cond_benchmark(c, &d, DType::F16, "where_cond_f16");

        run_masked_fill_benchmark(c, &d, DType::F32, "where_cond_masked_fill_f32");
        run_masked_fill_benchmark(c, &d, DType::BF16, "where_cond_masked_fill_bf16");
        run_masked_fill_benchmark(c, &d, DType::F16, "where_cond_masked_fill_f16");

        run_masked_fill_bcast_benchmark(c, &d, DType::F32, "where_cond_masked_fill_bcast_f32");
        run_masked_fill_bcast_benchmark(c, &d, DType::BF16, "where_cond_masked_fill_bcast_bf16");
        run_masked_fill_bcast_benchmark(c, &d, DType::F16, "where_cond_masked_fill_bcast_f16");

        run_transposed_benchmark(c, &d, DType::F32, "where_cond_transposed_f32");
        run_transposed_benchmark(c, &d, DType::BF16, "where_cond_transposed_bf16");
        run_transposed_benchmark(c, &d, DType::F16, "where_cond_transposed_f16");

        run_narrow_inner_benchmark(c, &d, DType::F32, "where_cond_narrow_inner_f32");
        run_narrow_inner_benchmark(c, &d, DType::BF16, "where_cond_narrow_inner_bf16");
        run_narrow_inner_benchmark(c, &d, DType::F16, "where_cond_narrow_inner_f16");
    }
}

criterion_group!(benches, criterion_benchmark);
