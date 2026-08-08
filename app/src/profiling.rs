//! Logic related to application profiling (e.g.: CPU and heap profiling).
//!
//! Profiling functionality is gated by Cargo feature flags:
//! * `pprof_cpu_profiling` enables use of pprof to produce CPU profiles
//! * `dhat_heap_profiling` enables use of dhat to produce heap profiles
//! * `jemalloc_auto_heap_profiling` enables the jemalloc allocator and
//!   automatic heap profile generation every 500MB of memory allocated.
//!
//! If run from a release bundle, profiles will be written to
//! [`warp_core::paths::state_dir()`].  Otherwise, profiles will be written
//! to the current working directory.

use cfg_if::cfg_if;

// When using jemalloc heap profiling, this static variable enables and
// configures the profiling behavior.
cfg_if! {
    if #[cfg(feature = "jemalloc_auto_heap_profiling")] {
        #[cfg_attr(target_vendor = "apple", unsafe(export_name = "_rjem_malloc_conf"))]
        #[cfg_attr(not(target_vendor = "apple"), unsafe(export_name = "malloc_conf"))]
        pub static MALLOC_CONF: &[u8] =
            b"prof:true,prof_active:true,lg_prof_interval:29,lg_prof_sample:21,prof_prefix:/tmp/jeprof\0";
    } else if #[cfg(feature = "jemalloc_pprof")] {
        #[cfg_attr(target_vendor = "apple", unsafe(export_name = "_rjem_malloc_conf"))]
        #[cfg_attr(not(target_vendor = "apple"), unsafe(export_name = "malloc_conf"))]
        pub static MALLOC_CONF: &[u8] =
            b"prof:true,prof_active:true,lg_prof_sample:21\0";
    }
}

/// When the dhat_heap_profiling feature is enabled, a global profiler object
/// that tracks allocations until the profiler is dropped.
#[cfg(feature = "dhat_heap_profiling")]
static HEAP_PROFILER: parking_lot::Mutex<Option<dhat::Profiler>> = parking_lot::Mutex::new(None);

#[cfg(feature = "pprof_cpu_profiling")]
static CPU_PROFILER: parking_lot::Mutex<Option<pprof::ProfilerGuard>> =
    parking_lot::Mutex::new(None);

/// Initializes the profiling subsystem.
pub fn init() {
    #[cfg(feature = "dhat_heap_profiling")]
    let _ = HEAP_PROFILER.lock().insert(
        dhat::Profiler::builder()
            .file_name(heap_profile_path())
            .build(),
    );

    #[cfg(feature = "pprof_cpu_profiling")]
    let _ = CPU_PROFILER
        .lock()
        .insert(pprof::ProfilerGuard::new(1000).unwrap());
}

/// Dumps dhat heap profiling information.
///
/// Note that this is implemented by uninitializing the profiler, and as such
/// can only be done once per run of the application.
#[cfg(feature = "dhat_heap_profiling")]
pub fn dump_dhat_heap_profile() {
    let _ = HEAP_PROFILER.lock().take();
}

/// Writes a heap profile to disk and returns the generated path.
pub async fn dump_heap_profile_to_disk() -> anyhow::Result<std::path::PathBuf> {
    cfg_if::cfg_if! {
        if #[cfg(feature = "dhat_heap_profiling")] {
            let path = heap_profile_path();
            dump_dhat_heap_profile();
            Ok(path)
        } else if #[cfg(feature = "heap_usage_tracking")] {
            use anyhow::Context as _;

            let path = heap_profile_path();
            let profile_data = dump_jemalloc_heap_profile_inner().await?;
            async_fs::write(&path, profile_data).await
                .with_context(|| format!("Failed to write heap profile to {}", path.display()))?;
            Ok(path)
        } else {
            anyhow::bail!("heap profiling is not enabled in this build");
        }
    }
}

/// Dumps a jemalloc heap profile and sends it to Sentry.
///
/// The profile is produced in-process from the `jemalloc_pprof` profiler as a raw (unsymbolized)
/// pprof -- sample addresses plus mappings carrying each loaded image's path and build-id -- and is
/// symbolized offline against the debug-info file uploaded to Sentry by the release process
/// (matched by build-id).  The resulting profile is attached to a Sentry event.
#[cfg(feature = "heap_usage_tracking")]
pub async fn dump_jemalloc_heap_profile(memory_breakdown: serde_json::Value) {
    use sentry::protocol::{Attachment, AttachmentType};

    let result = dump_jemalloc_heap_profile_inner().await;
    match result {
        Ok(profile_data) => {
            let attachment = Attachment {
                buffer: profile_data,
                filename: "heap-profile.pb".to_string(),
                ty: Some(AttachmentType::Attachment),
                ..Default::default()
            };
            sentry::with_scope(
                |scope| {
                    scope.add_attachment(attachment);

                    // Attach the memory breakdown as structured context so it
                    // is visible directly in the Sentry event.
                    if let serde_json::Value::Object(map) = memory_breakdown {
                        let context_map: std::collections::BTreeMap<
                            String,
                            sentry::protocol::Value,
                        > = map.into_iter().collect();
                        scope.set_context(
                            "memory_breakdown",
                            sentry::protocol::Context::Other(context_map),
                        );
                    }
                },
                || {
                    sentry::capture_message(
                        "Excessive memory usage detected",
                        sentry::Level::Warning,
                    )
                },
            );
            log::info!("Sent heap profile to Sentry");
        }
        Err(err) => {
            log::warn!("Failed to dump heap profile: {err:#}");
        }
    }
}

#[cfg(feature = "heap_usage_tracking")]
async fn dump_jemalloc_heap_profile_inner() -> anyhow::Result<Vec<u8>> {
    cfg_if::cfg_if! {
        if #[cfg(any(target_os = "linux", target_os = "macos"))] {
            // We build `jemalloc_pprof` WITHOUT the `symbolize` feature, so we produce a raw,
            // gzipped pprof (sample addresses + mappings + build-id) that is symbolized offline
            // against the debug-info file by build-id.  Dump it directly in-process -- no external
            // `pprof`/Go binary, HTTP round-trip, or port dependency required (the latter matter
            // for the headless remote server daemon, which has no bundled helpers next to it).
            dump_jemalloc_pprof_bytes().await
        } else {
            // `heap_usage_tracking` is only ever enabled for the Linux and macOS bundles, both of
            // which take the branch above.
            anyhow::bail!("heap profiling is not supported on this platform");
        }
    }
}

/// Produces a raw (unsymbolized), gzipped pprof heap profile directly from the in-process jemalloc
/// profiler.  The profile carries sample addresses, mappings, and each loaded image's build-id, and
/// is symbolized offline against the matching debug-info file (by build-id).
///
/// This is the same dump that [`handle_get_heap`] serves over HTTP, but invoked directly so callers
/// don't need to reach the local HTTP server.
#[cfg(all(
    feature = "jemalloc_pprof",
    any(target_os = "linux", target_os = "macos")
))]
async fn dump_jemalloc_pprof_bytes() -> anyhow::Result<Vec<u8>> {
    let Some(prof_ctl) = jemalloc_pprof::PROF_CTL.as_ref() else {
        anyhow::bail!("heap profiler not initialized");
    };
    let mut prof_ctl = prof_ctl.lock().await;
    if !prof_ctl.activated() {
        anyhow::bail!("heap profiling not activated");
    }

    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            // `ProfCtl::dump_pprof` builds the pprof mapping table from `mappings::MAPPINGS`, which
            // is hard-coded to `None` on every non-Linux target because it walks
            // `dl_iterate_phdr`/ELF program headers.  A pprof with no mappings carries no image
            // paths, no build-ids, and raw run-time addresses, so nothing can symbolize it and the
            // attachment is useless for triage.  Supply the mapping table ourselves from dyld and
            // then run the same conversion `dump_pprof` would.
            use std::io::BufReader;

            let dump = prof_ctl.dump()?;
            let mappings = macos_mappings::collect();
            let profile = pprof_util::parse_jeheap(BufReader::new(dump), Some(&mappings))?;
            Ok(profile.to_pprof(("inuse_space", "bytes"), ("space", "bytes"), None))
        } else {
            prof_ctl.dump_pprof()
        }
    }
}

/// Collects the pprof mapping table for the current process on macOS.
///
/// This is the Mach-O equivalent of what the `mappings` crate does for ELF on Linux: it walks every
/// image loaded into the process via dyld and records, for each executable segment, where that
/// segment lives at run time, where it lives in the file, and the image's build-id.  Sentry uses
/// the build-id -- the Mach-O `LC_UUID`, which is exactly the debug-id the release process uploads
/// dSYMs under -- to symbolize the profile offline.
#[cfg(all(feature = "jemalloc_pprof", target_os = "macos"))]
mod macos_mappings {
    use std::ffi::{CStr, OsStr, c_char};
    use std::os::unix::ffi::OsStrExt as _;
    use std::path::{Path, PathBuf};

    use pprof_util::{BuildId, Mapping};

    /// `MH_MAGIC_64` from `<mach-o/loader.h>`.
    const MH_MAGIC_64: u32 = 0xfeed_facf;
    /// `LC_SEGMENT_64` from `<mach-o/loader.h>`.
    const LC_SEGMENT_64: u32 = 0x19;
    /// `LC_UUID` from `<mach-o/loader.h>`.
    const LC_UUID: u32 = 0x1b;
    /// `VM_PROT_EXECUTE` from `<mach/vm_prot.h>`.
    const VM_PROT_EXECUTE: i32 = 0x4;

    // dyld's image introspection API, from `<mach-o/dyld.h>`.  Declared here rather than taken from
    // `libc`, whose bindings for these are deprecated in favour of a `mach2` dependency we would
    // not otherwise need.
    unsafe extern "C" {
        fn _dyld_image_count() -> u32;
        fn _dyld_get_image_header(image_index: u32) -> *const MachHeader64;
        fn _dyld_get_image_name(image_index: u32) -> *const c_char;
        fn _dyld_get_image_vmaddr_slide(image_index: u32) -> isize;
    }

    /// `struct mach_header_64` from `<mach-o/loader.h>`.
    ///
    /// The unread fields are kept so the declaration mirrors the C layout.
    #[repr(C)]
    #[derive(Clone, Copy)]
    #[allow(dead_code)]
    struct MachHeader64 {
        magic: u32,
        cputype: i32,
        cpusubtype: i32,
        filetype: u32,
        ncmds: u32,
        sizeofcmds: u32,
        flags: u32,
        reserved: u32,
    }

    /// `struct load_command` from `<mach-o/loader.h>`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct LoadCommand {
        cmd: u32,
        cmdsize: u32,
    }

    /// `struct segment_command_64` from `<mach-o/loader.h>`.
    ///
    /// The unread fields are kept so the declaration mirrors the C layout.
    #[repr(C)]
    #[derive(Clone, Copy)]
    #[allow(dead_code)]
    struct SegmentCommand64 {
        cmd: u32,
        cmdsize: u32,
        segname: [u8; 16],
        vmaddr: u64,
        vmsize: u64,
        fileoff: u64,
        filesize: u64,
        maxprot: i32,
        initprot: i32,
        nsects: u32,
        flags: u32,
    }

    /// `struct uuid_command` from `<mach-o/loader.h>`.
    ///
    /// The unread fields are kept so the declaration mirrors the C layout.
    #[repr(C)]
    #[derive(Clone, Copy)]
    #[allow(dead_code)]
    struct UuidCommand {
        cmd: u32,
        cmdsize: u32,
        uuid: [u8; 16],
    }

    /// Returns a mapping for every executable segment of every image currently loaded into this
    /// process.
    ///
    /// Only executable segments are reported: stack addresses in a heap profile only ever land in
    /// those, and emitting every segment of every image would bloat the attachment and slow down
    /// the linear mapping lookup `pprof_util` performs per address.
    pub fn collect() -> Vec<Mapping> {
        let mut mappings = Vec::new();

        // SAFETY: `_dyld_image_count` only reads dyld's image list.  That list can grow if another
        // thread loads an image while we iterate, in which case the accessors below simply return
        // null for an index we no longer consider valid, and we skip it.
        let count = unsafe { _dyld_image_count() };

        for index in 0..count {
            // SAFETY: `index` is less than the count reported by dyld, and both accessors return
            // null rather than misbehaving for an out-of-range index.
            let (header, name) =
                unsafe { (_dyld_get_image_header(index), _dyld_get_image_name(index)) };
            if header.is_null() || name.is_null() {
                continue;
            }

            // SAFETY: dyld documents the image name as a null-terminated string that stays valid
            // for as long as the image is loaded.
            let name = unsafe { CStr::from_ptr(name) };
            let pathname = PathBuf::from(OsStr::from_bytes(name.to_bytes()));

            // SAFETY: `index` is in range, as above.
            let slide = unsafe { _dyld_get_image_vmaddr_slide(index) };

            // SAFETY: `header` points at the Mach-O header of a loaded image, which stays mapped
            // for as long as that image is loaded.
            unsafe { collect_image(header, slide, &pathname, &mut mappings) };
        }

        mappings
    }

    /// Appends the executable-segment mappings of a single loaded image.
    ///
    /// # Safety
    ///
    /// `header` must point at the Mach-O header of an image that is currently loaded into this
    /// process, and `slide` must be that image's vmaddr slide.
    unsafe fn collect_image(
        header: *const MachHeader64,
        slide: isize,
        pathname: &Path,
        mappings: &mut Vec<Mapping>,
    ) {
        // SAFETY: the caller guarantees `header` points at a mapped Mach-O header.  Every read here
        // is unaligned because dyld makes no alignment promises about the pointers it hands out.
        let MachHeader64 { magic, ncmds, .. } = unsafe { std::ptr::read_unaligned(header) };
        if magic != MH_MAGIC_64 {
            // Every image in the 64-bit-only builds we ship is Mach-O 64.
            return;
        }

        // The load commands directly follow the header.
        //
        // SAFETY: a Mach-O image is a header followed by `ncmds` load commands, all of which live
        // within the image's own mapping.
        let mut cursor = unsafe { header.add(1) }.cast::<u8>();

        let mut build_id = None;
        let mut segments = Vec::new();

        for _ in 0..ncmds {
            // SAFETY: `cursor` walks forward by each command's self-reported size, so it stays
            // inside the load command region.
            let command: LoadCommand = unsafe { std::ptr::read_unaligned(cursor.cast()) };

            let Ok(size) = usize::try_from(command.cmdsize) else {
                break;
            };
            if size < std::mem::size_of::<LoadCommand>() {
                // A bogus size would leave us walking in place forever.
                break;
            }

            match command.cmd {
                LC_SEGMENT_64 => {
                    // SAFETY: `cmd` identifies this command as a `segment_command_64`.
                    let segment: SegmentCommand64 =
                        unsafe { std::ptr::read_unaligned(cursor.cast()) };
                    if segment.initprot & VM_PROT_EXECUTE != 0 && segment.vmsize > 0 {
                        segments.push(segment);
                    }
                }
                LC_UUID => {
                    // SAFETY: `cmd` identifies this command as a `uuid_command`.
                    let uuid: UuidCommand = unsafe { std::ptr::read_unaligned(cursor.cast()) };
                    build_id = Some(BuildId(uuid.uuid.to_vec()));
                }
                _ => {}
            }

            // SAFETY: still inside the load command region, as above.
            cursor = unsafe { cursor.add(size) };
        }

        for segment in segments {
            let (Ok(vmaddr), Ok(vmsize)) = (
                usize::try_from(segment.vmaddr),
                usize::try_from(segment.vmsize),
            ) else {
                continue;
            };

            // pprof recovers the address a symbolizer needs as `addr - memory_start +
            // memory_offset`, so pairing the run-time start with the static `vmaddr` turns each
            // sampled address back into the image-relative address recorded in the dSYM.
            let memory_start = vmaddr.wrapping_add_signed(slide);

            mappings.push(Mapping {
                memory_start,
                memory_end: memory_start.saturating_add(vmsize),
                memory_offset: vmaddr,
                file_offset: segment.fileoff,
                pathname: pathname.to_path_buf(),
                build_id: build_id.clone(),
            });
        }
    }
}

/// Returns the path at which heap profiles will be written.
#[cfg(any(feature = "dhat_heap_profiling", feature = "heap_usage_tracking"))]
pub fn heap_profile_path() -> std::path::PathBuf {
    cfg_if::cfg_if! {
        if #[cfg(feature = "dhat_heap_profiling")] {
            profile_output_dir().join("dhat-heap.json")
        } else {
            profile_output_dir().join("heap-profile.pb")
        }
    }
}

/// Uninitializes the profiling subsystem, writing reports to disk as-needed.
pub fn teardown() {
    #[cfg(feature = "dhat_heap_profiling")]
    let _ = HEAP_PROFILER.lock().take();

    #[cfg(feature = "pprof_cpu_profiling")]
    if let Err(err) = CPU_PROFILER
        .lock()
        .take()
        .unwrap()
        .report()
        .build()
        .map_err(Into::into)
        .and_then(write_pprof_report)
    {
        warp_errors::report_error!(err.context("Failed to write pprof data"));
    }
}

#[cfg(feature = "pprof_cpu_profiling")]
fn write_pprof_report(report: pprof::Report) -> anyhow::Result<()> {
    use pprof::protos::Message as _;

    let mut file = std::fs::File::create(profile_output_dir().join("profile.pb"))?;
    let profile = report.pprof()?;
    profile.write_to_writer(&mut file)?;
    Ok(())
}

#[cfg(any(
    feature = "dhat_heap_profiling",
    feature = "heap_usage_tracking",
    feature = "pprof_cpu_profiling"
))]
fn profile_output_dir() -> std::path::PathBuf {
    cfg_if::cfg_if! {
        if #[cfg(feature = "release_bundle")] {
            warp_core::paths::secure_state_dir().unwrap_or(warp_core::paths::state_dir())
        } else {
            std::env::current_dir().ok().unwrap_or_else(|| {
                dirs::home_dir().expect("Should not fail to compute both the current directory and the user's home directory")
            })
        }
    }
}

#[cfg(not(target_family = "wasm"))]
pub fn make_router() -> axum::Router {
    let router = axum::Router::new();

    #[cfg(feature = "jemalloc_pprof")]
    let router = router.route("/debug/pprof/heap", axum::routing::get(handle_get_heap));

    router
}

#[cfg(feature = "jemalloc_pprof")]
pub async fn handle_get_heap()
-> Result<impl axum::response::IntoResponse, (axum::http::StatusCode, String)> {
    let Some(prof_ctl) = jemalloc_pprof::PROF_CTL.as_ref() else {
        return Err((
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "heap profiler not initialized".into(),
        ));
    };
    let mut prof_ctl = prof_ctl.lock().await;

    if !prof_ctl.activated() {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            "heap profiling not activated".into(),
        ));
    }

    let pprof = prof_ctl.dump_pprof().map_err(|err| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            err.to_string(),
        )
    })?;
    Ok(pprof)
}
