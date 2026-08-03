use super::*;

#[test]
fn test_is_unsupported_llvmpipe_adapter() {
    let supported_adapter_info = wgpu::AdapterInfo {
        name: "llvmpipe (LLVM 17.0.6, 256 bits)".to_owned(),
        // not used
        vendor: 0,
        // not used
        device: 0,
        device_type: wgpu::DeviceType::Cpu,
        driver: "llvmpipe".to_owned(),
        driver_info: "Mesa 24.0.2-arch1.2 (LLVM 17.0.6)".to_owned(),
        backend: wgpu::Backend::Vulkan,
        device_pci_bus_id: "01:00.0".to_owned(),
        subgroup_min_size: wgpu::MINIMUM_SUBGROUP_MIN_SIZE,
        subgroup_max_size: wgpu::MAXIMUM_SUBGROUP_MAX_SIZE,
        transient_saves_memory: Some(false),
        limit_bucket: None,
    };
    assert!(!is_older_lavapipe_adapter(&supported_adapter_info));

    let unsupported_adapter_info = wgpu::AdapterInfo {
        name: "llvmpipe (LLVM 17.0.6, 256 bits)".to_owned(),
        // not used
        vendor: 0,
        // not used
        device: 0,
        device_type: wgpu::DeviceType::Cpu,
        driver: "llvmpipe".to_owned(),
        driver_info: "Mesa 23.2.1-1ubuntu3.1~22.04.2 (LLVM 15.0.7)".to_owned(),
        backend: wgpu::Backend::Vulkan,
        device_pci_bus_id: "01:00.0".to_owned(),
        subgroup_min_size: wgpu::MINIMUM_SUBGROUP_MIN_SIZE,
        subgroup_max_size: wgpu::MAXIMUM_SUBGROUP_MAX_SIZE,
        transient_saves_memory: Some(false),
        limit_bucket: None,
    };

    assert!(is_older_lavapipe_adapter(&unsupported_adapter_info));
}

/// Builds an [`wgpu::AdapterInfo`] with the fields our adapter selection logic looks at.
fn adapter_info(
    name: &str,
    driver: &str,
    driver_info: &str,
    backend: wgpu::Backend,
    device_type: wgpu::DeviceType,
) -> wgpu::AdapterInfo {
    wgpu::AdapterInfo {
        name: name.to_owned(),
        vendor: 0,
        device: 0,
        device_type,
        driver: driver.to_owned(),
        driver_info: driver_info.to_owned(),
        backend,
        device_pci_bus_id: "01:00.0".to_owned(),
        subgroup_min_size: wgpu::MINIMUM_SUBGROUP_MIN_SIZE,
        subgroup_max_size: wgpu::MAXIMUM_SUBGROUP_MAX_SIZE,
        transient_saves_memory: Some(false),
        limit_bucket: None,
    }
}

/// The adapter reported in https://github.com/warpdotdev/warp/issues/14577: every frame fails with
/// a validation error when rendering through Vulkan on Mesa 21.2.6.
fn intel_xe_tgl_gt2_vulkan_adapter_info(mesa_version: &str) -> wgpu::AdapterInfo {
    adapter_info(
        "Intel(R) Xe Graphics (TGL GT2)",
        "Intel open-source Mesa driver",
        &format!("Mesa {mesa_version}"),
        wgpu::Backend::Vulkan,
        wgpu::DeviceType::IntegratedGpu,
    )
}

/// The healthy GL adapter enumerated alongside the Vulkan one in the same report.
fn intel_xe_tgl_gt2_gl_adapter_info(mesa_version: &str) -> wgpu::AdapterInfo {
    adapter_info(
        "Mesa Intel(R) Xe Graphics (TGL GT2)",
        "",
        &format!("4.6 (Core Profile) Mesa {mesa_version}"),
        wgpu::Backend::Gl,
        wgpu::DeviceType::IntegratedGpu,
    )
}

/// Ranks adapter infos the same way the final (and dominant) sorting step in [`sort_adapters`]
/// does, so we can assert on selection order without a real GPU.
fn rank_by_support(adapter_infos: Vec<wgpu::AdapterInfo>) -> Vec<wgpu::AdapterInfo> {
    let windowing_system = Some(windowing::System::X11 {
        is_x_wayland: false,
    });
    adapter_infos
        .into_iter()
        .sorted_by_key(|info| adapter_support(info, windowing_system, false))
        .collect_vec()
}

#[test]
fn test_is_unsupported_intel_uhd_adapter() {
    assert!(is_older_vulkan_intel_uhd_adapter(&wgpu::AdapterInfo {
        name: String::from("Intel(R) HD Graphics 620 (KBL GT2)"),
        vendor: 0,
        device: 0,
        device_type: wgpu::DeviceType::IntegratedGpu,
        driver: String::from("Intel open-source Mesa driver"),
        driver_info: String::from("Mesa 21.2.6"),
        backend: wgpu::Backend::Vulkan,
        device_pci_bus_id: "01:00.0".to_owned(),
        subgroup_min_size: wgpu::MINIMUM_SUBGROUP_MIN_SIZE,
        subgroup_max_size: wgpu::MAXIMUM_SUBGROUP_MAX_SIZE,
        transient_saves_memory: Some(false),
        limit_bucket: None,
    }));
    assert!(!is_older_vulkan_intel_uhd_adapter(&wgpu::AdapterInfo {
        name: String::from("Intel(R) HD Graphics 620 (KBL GT2)"),
        vendor: 0,
        device: 0,
        device_type: wgpu::DeviceType::IntegratedGpu,
        driver: String::from("Intel open-source Mesa driver"),
        // Version is recent enough
        driver_info: String::from("Mesa 23.2.6"),
        backend: wgpu::Backend::Vulkan,
        device_pci_bus_id: "01:00.0".to_owned(),
        subgroup_min_size: wgpu::MINIMUM_SUBGROUP_MIN_SIZE,
        subgroup_max_size: wgpu::MAXIMUM_SUBGROUP_MAX_SIZE,
        transient_saves_memory: Some(false),
        limit_bucket: None,
    }));
    assert!(!is_older_vulkan_intel_uhd_adapter(&wgpu::AdapterInfo {
        name: String::from("Intel(R) HD Graphics 620 (KBL GT2)"),
        vendor: 0,
        device: 0,
        device_type: wgpu::DeviceType::IntegratedGpu,
        driver: String::from("Intel open-source Mesa driver"),
        // Info string is messed up
        driver_info: String::from("Mssa 21.2.6"),
        backend: wgpu::Backend::Vulkan,
        device_pci_bus_id: "01:00.0".to_owned(),
        subgroup_min_size: wgpu::MINIMUM_SUBGROUP_MIN_SIZE,
        subgroup_max_size: wgpu::MAXIMUM_SUBGROUP_MAX_SIZE,
        transient_saves_memory: Some(false),
        limit_bucket: None,
    }));
    assert!(is_older_vulkan_intel_uhd_adapter(&wgpu::AdapterInfo {
        name: String::from("Intel(R) HD Graphics 620 (KBL GT2)"),
        vendor: 0,
        device: 0,
        device_type: wgpu::DeviceType::IntegratedGpu,
        driver: String::from("Intel open-source Mesa driver"),
        // Additional info should be ignored
        driver_info: String::from("Mesa 21.2.6 foo bar"),
        backend: wgpu::Backend::Vulkan,
        device_pci_bus_id: "01:00.0".to_owned(),
        subgroup_min_size: wgpu::MINIMUM_SUBGROUP_MIN_SIZE,
        subgroup_max_size: wgpu::MAXIMUM_SUBGROUP_MAX_SIZE,
        transient_saves_memory: Some(false),
        limit_bucket: None,
    }));
    assert!(!is_older_vulkan_intel_uhd_adapter(&wgpu::AdapterInfo {
        name: String::from("Intel(R) HD Graphics 620 (KBL GT2)"),
        vendor: 0,
        device: 0,
        device_type: wgpu::DeviceType::IntegratedGpu,
        driver: String::from("Intel open-source Mesa driver"),
        // No version number
        driver_info: String::from("Mesa"),
        backend: wgpu::Backend::Vulkan,
        device_pci_bus_id: "01:00.0".to_owned(),
        subgroup_min_size: wgpu::MINIMUM_SUBGROUP_MIN_SIZE,
        subgroup_max_size: wgpu::MAXIMUM_SUBGROUP_MAX_SIZE,
        transient_saves_memory: Some(false),
        limit_bucket: None,
    }));
    assert!(is_older_vulkan_intel_uhd_adapter(&wgpu::AdapterInfo {
        name: String::from("Intel(R) HD Graphics 620 (KBL GT2)"),
        vendor: 0,
        device: 0,
        device_type: wgpu::DeviceType::IntegratedGpu,
        driver: String::from("Intel open-source Mesa driver"),
        // Nonsense version string
        driver_info: String::from("Mesa wtfis&this"),
        backend: wgpu::Backend::Vulkan,
        device_pci_bus_id: "01:00.0".to_owned(),
        subgroup_min_size: wgpu::MINIMUM_SUBGROUP_MIN_SIZE,
        subgroup_max_size: wgpu::MAXIMUM_SUBGROUP_MAX_SIZE,
        transient_saves_memory: Some(false),
        limit_bucket: None,
    }));
}

/// The newly allowlisted `Intel(R) Xe Graphics (TGL GT2)` adapter reported in
/// https://github.com/warpdotdev/warp/issues/14577 must also be matched by name.
#[test]
fn test_is_unsupported_intel_xe_tgl_gt2_adapter() {
    assert!(is_older_vulkan_intel_uhd_adapter(
        &intel_xe_tgl_gt2_vulkan_adapter_info("21.2.6")
    ));
    assert!(!is_older_vulkan_intel_uhd_adapter(
        &intel_xe_tgl_gt2_vulkan_adapter_info("24.0.2")
    ));
    // The GL adapter for the same Intel GPU renders fine and must stay fully supported.
    assert!(!is_older_vulkan_intel_uhd_adapter(
        &intel_xe_tgl_gt2_gl_adapter_info("21.2.6")
    ));
}

#[test]
fn test_intel_xe_tgl_gt2_prefers_gl_on_older_mesa() {
    let vulkan = intel_xe_tgl_gt2_vulkan_adapter_info("21.2.6");
    let gl = intel_xe_tgl_gt2_gl_adapter_info("21.2.6");

    assert_eq!(
        adapter_support(&vulkan, None, false),
        AdapterSupport::SupportedWithIssues
    );
    assert_eq!(adapter_support(&gl, None, false), AdapterSupport::Supported);

    // The Vulkan adapter is enumerated first, but the GL adapter should win the ranking.
    let ranked = rank_by_support(vec![vulkan, gl]);
    assert_eq!(ranked[0].backend, wgpu::Backend::Gl);
    assert_eq!(ranked[1].backend, wgpu::Backend::Vulkan);
}

#[test]
fn test_intel_xe_tgl_gt2_is_used_on_newer_mesa() {
    let vulkan = intel_xe_tgl_gt2_vulkan_adapter_info("24.0.2");
    let gl = intel_xe_tgl_gt2_gl_adapter_info("24.0.2");

    assert!(!is_older_vulkan_intel_uhd_adapter(&vulkan));
    assert_eq!(
        adapter_support(&vulkan, None, false),
        AdapterSupport::Supported
    );
    // With both adapters equally supported, the stability sort is a no-op and the earlier
    // backend-priority sort (which prefers Vulkan on Linux) decides.
    let ranked = rank_by_support(vec![vulkan, gl]);
    assert_eq!(ranked[0].backend, wgpu::Backend::Vulkan);
}

/// Deprioritizing is not filtering: when no GL adapter can present, the old Intel Mesa Vulkan
/// adapter is still the best (and only) candidate, so we must not rank it as `Unsupported`.
#[test]
fn test_intel_xe_tgl_gt2_is_still_used_without_a_gl_fallback() {
    let vulkan = intel_xe_tgl_gt2_vulkan_adapter_info("21.2.6");
    let support = adapter_support(&vulkan, None, false);

    assert_eq!(support, AdapterSupport::SupportedWithIssues);
    assert!(support < AdapterSupport::Unsupported);

    let ranked = rank_by_support(vec![vulkan]);
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].backend, wgpu::Backend::Vulkan);
}
