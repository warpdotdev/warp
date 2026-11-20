use candle_core::cutile;
use candle_core::{CpuStorage, CudaStorage, CustomOp2, Layout, Result, Shape, Tensor};
use cutile::cuda_async::device_operation::DeviceOp;
use cutile::tile_kernel::TileKernel;

const BLOCK_SIZE: usize = 32;

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

struct Add {
    compile_only: bool,
}

impl CustomOp2 for Add {
    fn name(&self) -> &'static str {
        "cutile-add"
    }

    fn cpu_fwd(
        &self,
        _lhs: &CpuStorage,
        _lhs_layout: &Layout,
        _rhs: &CpuStorage,
        _rhs_layout: &Layout,
    ) -> Result<(CpuStorage, Shape)> {
        candle_core::bail!("cutile-add requires a CUDA tensor")
    }

    fn cuda_fwd(
        &self,
        lhs: &CudaStorage,
        lhs_layout: &Layout,
        rhs: &CudaStorage,
        rhs_layout: &Layout,
    ) -> Result<(CudaStorage, Shape)> {
        if !lhs_layout.is_contiguous() || !rhs_layout.is_contiguous() {
            candle_core::bail!("cutile-add requires contiguous tensors")
        }
        if lhs_layout.dims() != rhs_layout.dims() {
            candle_core::bail!("cutile-add requires equal shapes")
        }
        let len = lhs_layout.shape().elem_count();
        if !len.is_multiple_of(BLOCK_SIZE) {
            candle_core::bail!("cutile-add length must be divisible by {BLOCK_SIZE}")
        }

        let device = &lhs.device;
        let context = cutile::CutileContext::new(device)?;
        let lhs = context.read_storage::<f32>(lhs, lhs_layout)?;
        let rhs = context.read_storage::<f32>(rhs, rhs_layout)?;
        let mut output = unsafe { device.alloc::<f32>(len)? };
        let out = context.write(&mut output, 0)?;
        let launcher = unsafe {
            kernels::add(
                out.device_pointer(),
                lhs.device_pointer(),
                rhs.device_pointer(),
                len as i32,
            )
        }
        .grid(((len / BLOCK_SIZE) as u32, 1, 1));

        if self.compile_only {
            // Precompile this specialization without executing the kernel.
            cutile::kernel("add warmup", || launcher.compile_on(context.stream()))?;
        } else {
            cutile::kernel("add launch", || unsafe {
                launcher.async_on(context.stream())
            })?;
        }
        drop((out, lhs, rhs));
        Ok((
            CudaStorage::wrap_cuda_slice(output, device.clone()),
            lhs_layout.shape().clone(),
        ))
    }
}

fn main() -> Result<()> {
    // Persist compiled cubins so later process starts can skip the tileiras compilation stage.
    cutile::jit_cache::enable_default()?;

    let device = candle_core::Device::new_cuda(0)?;
    let lhs = Tensor::arange(0f32, BLOCK_SIZE as f32, &device)?;
    let rhs = Tensor::ones(BLOCK_SIZE, candle_core::DType::F32, &device)?;

    // Precompile the specialization without executing the add kernel.
    let _ = lhs.apply_op2_no_bwd(&rhs, &Add { compile_only: true })?;
    let output = lhs.apply_op2_no_bwd(
        &rhs,
        &Add {
            compile_only: false,
        },
    )?;
    println!("{:?}", output.to_vec1::<f32>()?);
    Ok(())
}
