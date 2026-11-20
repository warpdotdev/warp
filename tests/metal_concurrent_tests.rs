#![cfg(feature = "metal")]

use candle_core::{Device, Result, Tensor};

// readbacks must wait on the command buffer holding their own blit, not shared device state
#[test]
fn concurrent_readback() -> Result<()> {
    let device = Device::new_metal(0)?;
    std::thread::scope(|scope| {
        for thread in 0..8usize {
            let device = device.clone();
            scope.spawn(move || {
                for iter in 0..100usize {
                    let value = (thread * 1000 + iter) as f64;
                    let a = Tensor::full(value as f32, (64, 64), &device).unwrap();
                    let b = a.affine(2.0, 1.0).unwrap();
                    let values = b.flatten_all().unwrap().to_vec1::<f32>().unwrap();
                    let expected = (2.0 * value + 1.0) as f32;
                    assert!(
                        values.iter().all(|&x| x == expected),
                        "thread {thread} iter {iter}: expected {expected}, got {:?}",
                        &values[..4]
                    );
                }
            });
        }
    });
    Ok(())
}

#[test]
fn concurrent_quantized_data_roundtrip() -> Result<()> {
    use candle_core::quantized::{GgmlDType, QTensor};
    let device = Device::new_metal(0)?;
    std::thread::scope(|scope| {
        for thread in 0..8usize {
            let device = device.clone();
            scope.spawn(move || {
                for iter in 0..25usize {
                    let src = Tensor::rand(-1f32, 1f32, (256, 256), &device).unwrap();
                    let q = QTensor::quantize(&src, GgmlDType::Q8_0).unwrap();
                    let bytes = q.data().unwrap();
                    let q2 = QTensor::quantize(&src, GgmlDType::Q8_0).unwrap();
                    let bytes2 = q2.data().unwrap();
                    assert_eq!(
                        bytes, bytes2,
                        "thread {thread} iter {iter}: data() readback mismatch"
                    );
                }
            });
        }
    });
    Ok(())
}
