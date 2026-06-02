/// Returns the full memory footprint of the current process, in bytes.
///
/// Unlike RSS (resident set size), this includes memory that has been swapped
/// out or compressed by the OS.  On macOS, this returns `phys_footprint` from
/// `task_info(TASK_VM_INFO)`, which is the same value displayed by Activity
/// Monitor.
pub fn memory_footprint_bytes() -> u64 {
    platform::memory_footprint_bytes()
}

/// Returns a platform-specific JSON object with a detailed breakdown of the
/// current process's memory usage.
///
/// Each platform populates whichever fields it can natively provide.  The
/// returned value is an opaque JSON blob suitable for attaching to Sentry
/// events and telemetry payloads.
pub fn memory_breakdown() -> serde_json::Value {
    platform::memory_breakdown()
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod platform {
    use std::mem;

    use mach2::kern_return::KERN_SUCCESS;
    use mach2::task::task_info;
    use mach2::task_info::{task_vm_info, TASK_VM_INFO};
    use mach2::traps::mach_task_self;

    /// Calls `task_info(TASK_VM_INFO)` and returns the populated struct on
    /// success, or `None` if the call fails.
    fn query_task_vm_info() -> Option<task_vm_info> {
        // SAFETY: We zero-initialise the struct and pass its exact size to the
        // kernel.  `task_info` writes into the struct up to `count` natural
        // ints and returns `KERN_SUCCESS` on success.
        unsafe {
            let mut info: task_vm_info = mem::zeroed();
            let mut count = (mem::size_of::<task_vm_info>() / mem::size_of::<i32>()) as u32;
            let kr = task_info(
                mach_task_self(),
                TASK_VM_INFO,
                &mut info as *mut _ as *mut i32,
                &mut count,
            );
            if kr == KERN_SUCCESS {
                Some(info)
            } else {
                None
            }
        }
    }

    pub fn memory_footprint_bytes() -> u64 {
        query_task_vm_info()
            .map(|info| info.phys_footprint)
            .unwrap_or(0)
    }

    pub fn memory_breakdown() -> serde_json::Value {
        let Some(info) = query_task_vm_info() else {
            return serde_json::json!({});
        };

        // Copy fields out of the packed struct into locals to avoid
        // unaligned references (task_vm_info is repr(C, packed(4))).
        let total_footprint = info.phys_footprint;
        let resident = info.resident_size;
        let compressed = info.compressed;
        let internal = info.internal;
        let device = info.device;
        let gpu_memory = info.ledger_tag_graphics_footprint;
        let gpu_memory_compressed = info.ledger_tag_graphics_footprint_compressed;
        let media_memory = info.ledger_tag_media_footprint;
        let neural_memory = info.ledger_tag_neural_footprint;
        let purgeable = info.ledger_purgeable_nonvolatile;

        serde_json::json!({
            "total_footprint": total_footprint,
            "resident": resident,
            "compressed": compressed,
            "internal": internal,
            "device": device,
            "gpu_memory": gpu_memory,
            "gpu_memory_compressed": gpu_memory_compressed,
            "media_memory": media_memory,
            "neural_memory": neural_memory,
            "purgeable": purgeable,
        })
    }
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod platform {
    /// Reads `/proc/self/status` and sums `VmRSS` + `VmSwap` to approximate
    /// the full memory footprint (resident + swapped).
    pub fn memory_footprint_bytes() -> u64 {
        read_proc_self_status().unwrap_or(0)
    }

    fn read_proc_self_status() -> Option<u64> {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let mut rss_kb: u64 = 0;
        let mut swap_kb: u64 = 0;
        for line in status.lines() {
            if let Some(value) = line.strip_prefix("VmRSS:") {
                rss_kb = parse_kb(value);
            } else if let Some(value) = line.strip_prefix("VmSwap:") {
                swap_kb = parse_kb(value);
            }
        }
        Some((rss_kb + swap_kb) * 1024)
    }

    fn parse_kb(s: &str) -> u64 {
        // Lines look like "VmRSS:    12345 kB"
        s.split_whitespace()
            .next()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    pub fn memory_breakdown() -> serde_json::Value {
        let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
            return serde_json::json!({});
        };
        let mut result = serde_json::Map::new();
        for line in status.lines() {
            let (key, value) = if let Some(v) = line.strip_prefix("VmRSS:") {
                ("vm_rss", v)
            } else if let Some(v) = line.strip_prefix("VmSwap:") {
                ("vm_swap", v)
            } else if let Some(v) = line.strip_prefix("VmSize:") {
                ("vm_size", v)
            } else {
                continue;
            };
            result.insert(
                key.to_string(),
                serde_json::Value::Number((parse_kb(value) * 1024).into()),
            );
        }
        serde_json::Value::Object(result)
    }
}

// ---------------------------------------------------------------------------
// FreeBSD
// ---------------------------------------------------------------------------

#[cfg(target_os = "freebsd")]
mod platform {
    /// FreeBSD has no `/proc/self/status` by default (linprocfs is optional and
    /// rarely mounted), so we use `getrusage(RUSAGE_SELF)`. `ru_maxrss` is
    /// reported in kilobytes and represents the maximum resident set size, not
    /// the current value, but it's the closest portable signal we have without
    /// pulling in `kvm`/`sysctl(KERN_PROC_PID)` plumbing for one telemetry
    /// number.
    pub fn memory_footprint_bytes() -> u64 {
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
            return 0;
        }
        (usage.ru_maxrss as u64).saturating_mul(1024)
    }

    pub fn memory_breakdown() -> serde_json::Value {
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
            return serde_json::json!({});
        }
        serde_json::json!({
            "ru_maxrss": (usage.ru_maxrss as u64).saturating_mul(1024),
        })
    }
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod platform {
    use std::mem;

    use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;

    #[repr(C)]
    struct ProcessMemoryCountersEx {
        base: PROCESS_MEMORY_COUNTERS,
        private_usage: usize,
    }

    /// Uses `GetProcessMemoryInfo` to read `PrivateUsage` from
    /// `PROCESS_MEMORY_COUNTERS_EX`, which accounts for private committed
    /// memory (resident + paged out).
    ///
    /// The `windows` crate doesn't expose `PROCESS_MEMORY_COUNTERS_EX`
    /// directly, but it is layout-compatible with `PROCESS_MEMORY_COUNTERS`
    /// plus one trailing `usize` field (`PrivateUsage`).  We define a minimal
    /// wrapper to read that field.
    pub fn memory_footprint_bytes() -> u64 {
        query_counters()
            .map(|c| c.private_usage as u64)
            .unwrap_or(0)
    }

    fn query_counters() -> Option<ProcessMemoryCountersEx> {
        // SAFETY: `GetCurrentProcess` returns a pseudo-handle that does not
        // need to be closed.  `K32GetProcessMemoryInfo` writes into the
        // provided struct up to `cb` bytes.
        unsafe {
            let handle = GetCurrentProcess();
            let mut counters: ProcessMemoryCountersEx = mem::zeroed();
            counters.base.cb = mem::size_of::<ProcessMemoryCountersEx>() as u32;
            if K32GetProcessMemoryInfo(handle, &mut counters.base, counters.base.cb).as_bool() {
                Some(counters)
            } else {
                None
            }
        }
    }

    pub fn memory_breakdown() -> serde_json::Value {
        let Some(counters) = query_counters() else {
            return serde_json::json!({});
        };
        serde_json::json!({
            "working_set": counters.base.WorkingSetSize,
            "private_usage": counters.private_usage,
            "peak_working_set": counters.base.PeakWorkingSetSize,
        })
    }
}
