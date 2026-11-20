#![cfg(feature = "cutile")]

use candle_core::cutile;
use candle_core::{CpuStorage, CudaStorage, CustomOp2, Device, Layout, Result, Shape, Tensor};
use cutile::cuda_async::device_operation::DeviceOp;
use cutile::tile_kernel::TileKernel;
use std::sync::Mutex;

const BLOCK_SIZE: usize = 32;
const OUTPUT_SEED: f32 = -7.;
const WARMUP_SIZE: usize = BLOCK_SIZE * 2;

static GPU_TEST_LOCK: Mutex<()> = Mutex::new(());

#[cutile::module]
mod kernels {
    use candle_core::cutile;
    use cutile::core::*;

    unsafe fn tensor(ptr: *mut f32, len: i32) -> Tensor<f32, { [-1] }> {
        let shape: Shape<{ [-1] }> = Shape::<{ [-1] }> { dims: &[len] };
        let strides: Array<{ [-1] }> = Array::<{ [-1] }> { dims: &[1] };
        let pointer: PointerTile<*mut f32, { [] }> = pointer_to_tile(ptr);
        make_tensor_view(pointer, shape, strides, new_token_unordered())
    }

    #[cutile::entry(unchecked_accesses = true)]
    unsafe fn add(out: *mut f32, lhs: *mut f32, rhs: *mut f32, len: i32) {
        let mut out = tensor(out, len);
        let lhs = tensor(lhs, len);
        let rhs = tensor(rhs, len);
        let block = const_shape![32];
        let block_id = get_tile_block_id().0;
        let value = lhs.partition(block).load([block_id]) + rhs.partition(block).load([block_id]);
        out.partition_mut(block).store(value, [block_id]);
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Compile,
    Launch,
}

struct Add(Mode);

impl CustomOp2 for Add {
    fn name(&self) -> &'static str {
        "cutile-test-add"
    }

    fn cpu_fwd(
        &self,
        _lhs: &CpuStorage,
        _lhs_layout: &Layout,
        _rhs: &CpuStorage,
        _rhs_layout: &Layout,
    ) -> Result<(CpuStorage, Shape)> {
        candle_core::bail!("cutile-test-add requires CUDA")
    }

    fn cuda_fwd(
        &self,
        lhs: &CudaStorage,
        lhs_layout: &Layout,
        rhs: &CudaStorage,
        rhs_layout: &Layout,
    ) -> Result<(CudaStorage, Shape)> {
        if !lhs_layout.is_contiguous() || !rhs_layout.is_contiguous() {
            candle_core::bail!("cutile-test-add requires contiguous tensors")
        }
        if lhs_layout.dims() != rhs_layout.dims() {
            candle_core::bail!("cutile-test-add requires equal shapes")
        }

        let len = lhs_layout.shape().elem_count();
        if !len.is_multiple_of(BLOCK_SIZE) {
            candle_core::bail!("cutile-test-add requires a multiple of {BLOCK_SIZE} elements")
        }
        let len_i32 = i32::try_from(len)?;
        let device = &lhs.device;
        let context = cutile::CutileContext::new(device)?;
        let lhs = context.read_storage::<f32>(lhs, lhs_layout)?;
        let rhs = context.read_storage::<f32>(rhs, rhs_layout)?;
        let seed = vec![OUTPUT_SEED; len];
        let mut output = device.clone_htod(&seed)?;
        let out = context.write(&mut output, 0)?;
        let launcher = unsafe {
            kernels::add(
                out.device_pointer(),
                lhs.device_pointer(),
                rhs.device_pointer(),
                len_i32,
            )
        }
        .grid(((len / BLOCK_SIZE) as u32, 1, 1));

        match self.0 {
            Mode::Compile => {
                cutile::kernel("test add warmup", || launcher.compile_on(context.stream()))?
            }
            Mode::Launch => {
                cutile::kernel("test add launch", || unsafe {
                    launcher.async_on(context.stream())
                })?;
            }
        }
        drop((out, lhs, rhs));
        Ok((
            CudaStorage::wrap_cuda_slice(output, device.clone()),
            lhs_layout.shape().clone(),
        ))
    }
}

fn cuda_device() -> Result<Option<Device>> {
    if candle_core::utils::cuda_is_available() {
        Ok(Some(Device::new_cuda(0)?))
    } else {
        Ok(None)
    }
}

#[test]
fn compile_only_warmup_does_not_launch() -> Result<()> {
    let _lock = GPU_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(device) = cuda_device()? else {
        return Ok(());
    };
    let lhs = Tensor::arange(0f32, (WARMUP_SIZE + 1) as f32, &device)?.narrow(0, 1, WARMUP_SIZE)?;
    let rhs = Tensor::ones(WARMUP_SIZE + 1, candle_core::DType::F32, &device)?.narrow(
        0,
        1,
        WARMUP_SIZE,
    )?;

    let compile_count = cutile::tile_kernel::jit_compile_count();
    let output = lhs.apply_op2_no_bwd(&rhs, &Add(Mode::Compile))?;
    assert_eq!(output.to_vec1::<f32>()?, vec![OUTPUT_SEED; WARMUP_SIZE]);
    let warmed_count = cutile::tile_kernel::jit_compile_count();
    assert_eq!(warmed_count, compile_count + 1);

    let output = lhs.apply_op2_no_bwd(&rhs, &Add(Mode::Launch))?;
    assert_eq!(cutile::tile_kernel::jit_compile_count(), warmed_count);
    let expected = (0..WARMUP_SIZE)
        .map(|index| index as f32 + 2.)
        .collect::<Vec<_>>();
    assert_eq!(output.to_vec1::<f32>()?, expected);
    Ok(())
}

#[test]
fn custom_op_preserves_candle_stream_ordering() -> Result<()> {
    let _lock = GPU_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(device) = cuda_device()? else {
        return Ok(());
    };
    let input = Tensor::arange(0f32, BLOCK_SIZE as f32, &device)?;
    let lhs = (&input + 1.)?;
    let rhs = (&input * 2.)?;

    let output = lhs.apply_op2_no_bwd(&rhs, &Add(Mode::Launch))?;
    drop((input, lhs, rhs));
    let output = (&output + 3.)?;
    let expected = (0..BLOCK_SIZE)
        .map(|index| 3. * index as f32 + 4.)
        .collect::<Vec<_>>();
    assert_eq!(output.to_vec1::<f32>()?, expected);
    Ok(())
}
