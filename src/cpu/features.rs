use std::sync::OnceLock;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CpuFeatures {
    pub(crate) avx: bool,
    pub(crate) avx2: bool,
    pub(crate) fma: bool,
    pub(crate) f16c: bool,
    pub(crate) avx512f: bool,
    pub(crate) avx512bw: bool,
    pub(crate) avx512vl: bool,
    pub(crate) avx512bf16: bool,
    pub(crate) avx512vnni: bool,
    pub(crate) neon: bool,
    pub(crate) dotprod: bool,
    pub(crate) i8mm: bool,
    pub(crate) fp16: bool,
    pub(crate) bf16: bool,
}

pub(crate) fn get() -> CpuFeatures {
    static FEATURES: OnceLock<CpuFeatures> = OnceLock::new();
    *FEATURES.get_or_init(detect)
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn detect() -> CpuFeatures {
    CpuFeatures {
        avx: std::is_x86_feature_detected!("avx"),
        avx2: std::is_x86_feature_detected!("avx2"),
        fma: std::is_x86_feature_detected!("fma"),
        f16c: std::is_x86_feature_detected!("f16c"),
        avx512f: std::is_x86_feature_detected!("avx512f"),
        avx512bw: std::is_x86_feature_detected!("avx512bw"),
        avx512vl: std::is_x86_feature_detected!("avx512vl"),
        avx512bf16: std::is_x86_feature_detected!("avx512bf16"),
        avx512vnni: std::is_x86_feature_detected!("avx512vnni"),
        ..CpuFeatures::default()
    }
}

#[cfg(target_arch = "aarch64")]
fn detect() -> CpuFeatures {
    CpuFeatures {
        neon: true,
        dotprod: std::arch::is_aarch64_feature_detected!("dotprod"),
        i8mm: std::arch::is_aarch64_feature_detected!("i8mm"),
        fp16: std::arch::is_aarch64_feature_detected!("fp16"),
        bf16: std::arch::is_aarch64_feature_detected!("bf16"),
        ..CpuFeatures::default()
    }
}

#[cfg(all(target_arch = "arm", not(target_arch = "aarch64")))]
fn detect() -> CpuFeatures {
    CpuFeatures {
        neon: cfg!(target_feature = "neon"),
        fp16: cfg!(target_feature = "fp16"),
        ..CpuFeatures::default()
    }
}

#[cfg(not(any(
    target_arch = "x86",
    target_arch = "x86_64",
    target_arch = "arm",
    target_arch = "aarch64"
)))]
fn detect() -> CpuFeatures {
    CpuFeatures::default()
}
