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
        transient_saves_memory: false,
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
        transient_saves_memory: false,
    };

    assert!(is_older_lavapipe_adapter(&unsupported_adapter_info));
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
        transient_saves_memory: false,
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
        transient_saves_memory: false,
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
        transient_saves_memory: false,
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
        transient_saves_memory: false,
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
        transient_saves_memory: false,
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
        transient_saves_memory: false,
    }));
}
