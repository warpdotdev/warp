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
/// is symbolized offline against the matching debug-info file (by build-id).  On macOS the mapping
/// table is validated before it is used -- see [`ensure_mappings_are_symbolicatable`] -- so that a
/// broken mapping collection fails loudly instead of producing another dead Sentry attachment.
///
/// This is the same dump that [`handle_get_heap`] serves over HTTP, but invoked directly so callers
/// don't need to reach the local HTTP server.  Unlike that endpoint, this path validates the
/// mapping table before returning it -- see [`dump_pprof_for_current_platform`].
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

    // This profile is about to be attached to a Sentry event, so validate it first.
    dump_pprof_for_current_platform(&mut prof_ctl, true)
}

/// Builds a raw (unsymbolized), gzipped pprof from the current jemalloc heap dump, using this
/// platform's mapping table when one is available.
///
/// On macOS this supplies the mapping table via [`dump_macos_pprof`], since
/// [`jemalloc_pprof::JemallocProfCtl::dump_pprof`] builds its mapping table from
/// `mappings::MAPPINGS`, which is hard-coded to `None` on every non-Linux target because it walks
/// `dl_iterate_phdr`/ELF program headers.  A pprof with no mappings carries no image paths, no
/// build-ids, and raw run-time addresses, so nothing can symbolize it and the attachment is useless
/// for triage.  Everywhere else, `dump_pprof` already does the right thing (Linux) or is the best we
/// can do (other platforms), unchanged.
///
/// `validate_for_sentry_attachment` gates [`ensure_mappings_are_symbolicatable`] (macOS only; a
/// no-op elsewhere). Pass `true` only for a profile that is about to be attached to a Sentry event
/// -- [`dump_jemalloc_pprof_bytes`] does. [`handle_get_heap`] passes `false`: a developer hitting
/// that endpoint because mapping collection looks broken is better served by the raw, possibly
/// mapping-less profile than by an opaque HTTP 500 from the very guard meant to protect Sentry from
/// that same broken profile.
#[cfg(all(feature = "jemalloc_pprof", target_os = "macos"))]
fn dump_pprof_for_current_platform(
    prof_ctl: &mut jemalloc_pprof::JemallocProfCtl,
    validate_for_sentry_attachment: bool,
) -> anyhow::Result<Vec<u8>> {
    let mappings = macos_mappings::collect();
    if validate_for_sentry_attachment {
        ensure_mappings_are_symbolicatable(&mappings)?;
    }
    dump_macos_pprof(prof_ctl, &mappings)
}

#[cfg(all(feature = "jemalloc_pprof", not(target_os = "macos")))]
fn dump_pprof_for_current_platform(
    prof_ctl: &mut jemalloc_pprof::JemallocProfCtl,
    _validate_for_sentry_attachment: bool,
) -> anyhow::Result<Vec<u8>> {
    prof_ctl.dump_pprof()
}

/// Builds a raw (unsymbolized), gzipped pprof from the current jemalloc heap dump using the given
/// macOS mapping table, exactly as [`jemalloc_pprof::JemallocProfCtl::dump_pprof`] would if
/// `mappings::MAPPINGS` were populated on this platform.
///
/// Shared by [`dump_jemalloc_pprof_bytes`] and [`handle_get_heap`] so both produce the same,
/// actually-symbolicatable profile on macOS instead of the HTTP endpoint falling back to a
/// mapping-less one.
#[cfg(all(feature = "jemalloc_pprof", target_os = "macos"))]
fn dump_macos_pprof(
    prof_ctl: &mut jemalloc_pprof::JemallocProfCtl,
    mappings: &[pprof_util::Mapping],
) -> anyhow::Result<Vec<u8>> {
    use std::io::BufReader;

    let dump = prof_ctl.dump()?;
    let profile = pprof_util::parse_jeheap(BufReader::new(dump), Some(mappings))?;
    Ok(profile.to_pprof(("inuse_space", "bytes"), ("space", "bytes"), None))
}

/// Fails with a reported error if `mappings` is not usable for offline symbolication, instead of
/// letting a broken mapping table silently produce another unsymbolicatable Sentry attachment --
/// the exact recurring failure (APP-4817/APP-4818) this module exists to fix.  We treat this as
/// `report_error!`-worthy rather than a plain log: it means our own dyld/Mach-O walk in
/// [`macos_mappings`] regressed, which is exactly the kind of "programming against a
/// not-fully-understood external system" case where the failure may be our bug.
#[cfg(all(feature = "jemalloc_pprof", target_os = "macos"))]
fn ensure_mappings_are_symbolicatable(mappings: &[pprof_util::Mapping]) -> anyhow::Result<()> {
    let current_exe = std::env::current_exe().unwrap_or_default();
    if mappings_are_symbolicatable(mappings, &current_exe) {
        return Ok(());
    }

    warp_errors::report_error!(
        "macOS heap profile mapping table cannot be symbolicated offline",
        extra: {
            "current_exe" => %current_exe.display(),
            "mapping_count" => %mappings.len(),
        },
        warp_errors::ReportErrorLogMode::OncePerRun
    );
    anyhow::bail!("heap profile mapping table cannot be symbolicated offline")
}

/// Returns whether `mappings` are usable for offline symbolication: non-empty, with a mapping for
/// `current_exe` itself that carries a build-id.  Without both, Sentry (or a human with the
/// matching dSYM) has nothing to symbolicate the profile against.
///
/// Falls back to comparing canonicalized paths when a direct comparison fails: real-macOS testing
/// showed `std::env::current_exe()` and dyld's reported image path can differ when the executable
/// is reached through a symlink (e.g. a build cache mounted at another path), even though they
/// name the same file.
#[cfg(all(feature = "jemalloc_pprof", any(target_os = "macos", test)))]
fn mappings_are_symbolicatable(
    mappings: &[pprof_util::Mapping],
    current_exe: &std::path::Path,
) -> bool {
    if mappings.is_empty() {
        return false;
    }

    if mappings
        .iter()
        .any(|mapping| mapping.pathname == current_exe && mapping.build_id.is_some())
    {
        return true;
    }

    let Ok(canonical_current_exe) = std::fs::canonicalize(current_exe) else {
        return false;
    };
    mappings.iter().any(|mapping| {
        mapping.build_id.is_some()
            && std::fs::canonicalize(&mapping.pathname).is_ok_and(|p| p == canonical_current_exe)
    })
}

/// Collects the pprof mapping table for the current process on macOS.
///
/// This is the Mach-O equivalent of what the `mappings` crate does for ELF on Linux: it walks every
/// image loaded into the process via dyld and records, for each executable segment, where that
/// segment lives at run time, where it lives in the file, and the image's build-id.  Sentry uses
/// the build-id -- the Mach-O `LC_UUID`, which is exactly the debug-id the release process uploads
/// dSYMs under -- to symbolize the profile offline.
///
/// NOTE: `pprof_util::BuildId`'s `Display` renders continuous lowercase hex with no separators
/// (see `BuildId::fmt`), e.g. `e621e1f8c36c495a93fc0c247a3e6e5f`.  That is NOT the canonical
/// hyphenated Mach-O UUID format that `dwarfdump --uuid`, `lldb`, and Sentry's own `DebugId` use
/// (`E621E1F8-C36C-495A-93FC-0C247A3E6E5F`, plus a `-0` age suffix for `DebugId` specifically).
/// Both encode the same 16 bytes, so this is a cosmetic mismatch, not a symbolication blocker --
/// but it does mean a human cross-referencing this profile's build-id against a dSYM's UUID must
/// strip the dashes (and any trailing age) first.  We can't fix the formatting here: `BuildId` is
/// an upstream `pprof_util` type we don't control the `Display` impl of.
#[cfg(feature = "jemalloc_pprof")]
mod macos_mappings {
    #[cfg(any(target_os = "macos", test))]
    use pprof_util::BuildId;
    #[cfg(any(target_os = "macos", test))]
    use pprof_util::Mapping;

    /// `MH_MAGIC_64` from `<mach-o/loader.h>`.  Only used to interpret a live Mach-O header, so
    /// (unlike the load-command constants below) it has no reason to exist outside macOS.
    #[cfg(target_os = "macos")]
    const MH_MAGIC_64: u32 = 0xfeed_facf;
    /// `LC_SEGMENT_64` from `<mach-o/loader.h>`.
    #[cfg(any(target_os = "macos", test))]
    const LC_SEGMENT_64: u32 = 0x19;
    /// `LC_UUID` from `<mach-o/loader.h>`.
    #[cfg(any(target_os = "macos", test))]
    const LC_UUID: u32 = 0x1b;
    /// `VM_PROT_EXECUTE` from `<mach/vm_prot.h>`.
    #[cfg(any(target_os = "macos", test))]
    const VM_PROT_EXECUTE: i32 = 0x4;

    /// Size, in bytes, of `struct mach_header_64` from `<mach-o/loader.h>` (8 `u32`/`i32` fields).
    /// Only used to locate the load commands that follow a live Mach-O header, so macOS-only.
    #[cfg(target_os = "macos")]
    const MACH_HEADER_64_LEN: usize = 32;
    /// Byte offset of the `ncmds` field within `struct mach_header_64`.  macOS-only; see
    /// `MACH_HEADER_64_LEN`.
    #[cfg(target_os = "macos")]
    const NCMDS_OFFSET: usize = 16;
    /// Byte offset of the `sizeofcmds` field within `struct mach_header_64`.  macOS-only; see
    /// `MACH_HEADER_64_LEN`.
    #[cfg(target_os = "macos")]
    const SIZEOFCMDS_OFFSET: usize = 20;
    /// Size, in bytes, of `struct load_command`: `cmd` and `cmdsize`, each a `u32`.
    #[cfg(any(target_os = "macos", test))]
    const LOAD_COMMAND_LEN: usize = 8;
    /// Size, in bytes, of `struct segment_command_64`.
    #[cfg(any(target_os = "macos", test))]
    const SEGMENT_COMMAND_64_LEN: usize = 72;
    /// Size, in bytes, of `struct uuid_command`: the `load_command` header plus a 16-byte UUID.
    #[cfg(any(target_os = "macos", test))]
    const UUID_COMMAND_LEN: usize = LOAD_COMMAND_LEN + 16;

    // dyld's image add/remove notification API, from `<mach-o/dyld.h>`.  We use these instead of
    // the simpler `_dyld_image_count`/`_dyld_get_image_header`/`_dyld_get_image_name`-by-index API
    // (what this module used to do) because Apple documents that by-index API as unsafe to use
    // concurrently with image loading/unloading: another thread's `dlopen`/`dlclose` between two
    // index-based calls can hand back a mismatched header/name/slide tuple, or a header pointer for
    // an image that has already been unmapped -- a crash, in a process already at 10-20GB, at
    // exactly the moment we're trying to diagnose it.
    //
    // Registering these callbacks instead gives us a coherent, incrementally-maintained snapshot in
    // `LOADED_IMAGES`: `_dyld_register_func_for_add_image` synchronously invokes `on_image_added`
    // once per already-loaded image before returning -- a full, race-free initial snapshot -- then
    // keeps invoking it for every image loaded afterward. `_dyld_register_func_for_remove_image` is
    // documented to run `on_image_removed` to completion *before* dyld unmaps the image being
    // unloaded, so as long as removal and every read both go through `LOADED_IMAGES`'s lock, a
    // concurrent unload can only ever block behind that lock, never race it: `on_image_removed`
    // cannot finish (and dyld cannot proceed to unmap) while `collect()` still holds the lock and is
    // reading a header, and `collect()` cannot observe a header after `on_image_removed` has removed
    // it. This is the standard, Apple-recommended replacement for the by-index API for exactly this
    // reason (the by-index accessors' own thread-safety caveats point at it).
    //
    // Residual risk: neither `_dyld_register_func_for_*` callback can be unregistered, so we
    // register them (via `Once`) at most once per process rather than once per `collect()` call.
    // The registration+lock design also means a `collect()` call briefly blocks any concurrent
    // `dlclose` (bounded by how long we hold the lock, which is just a linear walk of already-loaded
    // images -- no I/O), and would deadlock if `collect()` were ever invoked reentrantly from within
    // a dyld image-add/remove callback on the same thread; neither this module nor its current
    // callers do that.
    #[cfg(target_os = "macos")]
    unsafe extern "C" {
        fn _dyld_register_func_for_add_image(callback: DyldImageCallback);
        fn _dyld_register_func_for_remove_image(callback: DyldImageCallback);
    }

    #[cfg(target_os = "macos")]
    type DyldImageCallback = extern "C" fn(*const u8, isize);

    /// A loaded Mach-O image, as reported by dyld's add-image callback.
    #[cfg(target_os = "macos")]
    struct LoadedImage {
        header: *const u8,
        slide: isize,
        pathname: std::path::PathBuf,
    }

    // SAFETY: `header` is only ever dereferenced by `collect()`/`collect_image`, and only while
    // holding `LOADED_IMAGES`'s lock; `on_image_removed` removes an image's entry under that same
    // lock, and dyld guarantees it does so before unmapping the image (see the module-level safety
    // comment above), so a `LoadedImage` can be safely handed to whichever thread calls `collect()`.
    #[cfg(target_os = "macos")]
    unsafe impl Send for LoadedImage {}

    #[cfg(target_os = "macos")]
    static LOADED_IMAGES: parking_lot::Mutex<Vec<LoadedImage>> =
        parking_lot::Mutex::new(Vec::new());
    #[cfg(target_os = "macos")]
    static REGISTER_DYLD_CALLBACKS: std::sync::Once = std::sync::Once::new();

    #[cfg(target_os = "macos")]
    extern "C" fn on_image_added(header: *const u8, slide: isize) {
        let Some(pathname) = image_pathname(header) else {
            return;
        };
        LOADED_IMAGES.lock().push(LoadedImage {
            header,
            slide,
            pathname,
        });
    }

    #[cfg(target_os = "macos")]
    extern "C" fn on_image_removed(header: *const u8, _slide: isize) {
        LOADED_IMAGES.lock().retain(|image| image.header != header);
    }

    /// Resolves the on-disk path of the image whose Mach-O header starts at `header`, via `dladdr`.
    ///
    /// Unlike the by-index dyld accessors this module used to use, `dladdr` only requires that
    /// `header` be a currently-valid address inside a mapped image -- which it is here, since it is
    /// the very start of the image's mapping and we are called from dyld's own add-image callback
    /// for that image.
    #[cfg(target_os = "macos")]
    fn image_pathname(header: *const u8) -> Option<std::path::PathBuf> {
        use std::ffi::{CStr, OsStr};
        use std::os::unix::ffi::OsStrExt as _;

        // SAFETY: `Dl_info` is a plain-old-data struct with no invalid bit patterns; `dladdr` fills
        // it in (or returns 0 without touching it) before we read any field.
        let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
        // SAFETY: `dladdr` requires only that `header` be a valid address within this process; the
        // caller guarantees that of the header dyld just reported as newly (or already) loaded.
        if unsafe { libc::dladdr(header.cast(), &mut info) } == 0 || info.dli_fname.is_null() {
            return None;
        }
        // SAFETY: `dladdr` documents `dli_fname` as a null-terminated string owned by dyld, valid
        // for the life of the process.
        let name = unsafe { CStr::from_ptr(info.dli_fname) };
        Some(std::path::PathBuf::from(OsStr::from_bytes(name.to_bytes())))
    }

    /// An executable `LC_SEGMENT_64` found while walking an image's load commands.
    #[cfg(any(target_os = "macos", test))]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct ExecutableSegment {
        pub(super) vmaddr: u64,
        pub(super) vmsize: u64,
        pub(super) fileoff: u64,
    }

    /// Returns a mapping for every executable segment of every image currently loaded into this
    /// process.
    ///
    /// Only executable segments are reported: stack addresses in a heap profile only ever land in
    /// those, and emitting every segment of every image would bloat the attachment and slow down
    /// the linear mapping lookup `pprof_util` performs per address.
    #[cfg(target_os = "macos")]
    pub fn collect() -> Vec<Mapping> {
        REGISTER_DYLD_CALLBACKS.call_once(|| {
            // SAFETY: registering dyld image callbacks carries no preconditions; dyld may invoke
            // them on any thread, at any time, including synchronously and repeatedly from within
            // this very call (once for each image already loaded), which is how `LOADED_IMAGES`
            // gets its initial, race-free contents.
            unsafe {
                _dyld_register_func_for_add_image(on_image_added);
                _dyld_register_func_for_remove_image(on_image_removed);
            }
        });

        let images = LOADED_IMAGES.lock();
        let mut mappings = Vec::new();
        for image in images.iter() {
            // SAFETY: we hold `LOADED_IMAGES`'s lock for the duration of this loop, and
            // `on_image_removed` only ever removes an entry (under that same lock) before dyld
            // unmaps it, so `image.header` is guaranteed to still point at a mapped Mach-O header.
            unsafe { collect_image(image.header, image.slide, &image.pathname, &mut mappings) };
        }
        mappings
    }

    /// Appends the executable-segment mappings of a single loaded image.
    ///
    /// # Safety
    ///
    /// `header` must point at the start of the Mach-O header of an image that is currently loaded
    /// into this process, and `slide` must be that image's vmaddr slide.
    #[cfg(target_os = "macos")]
    unsafe fn collect_image(
        header: *const u8,
        slide: isize,
        pathname: &std::path::Path,
        mappings: &mut Vec<Mapping>,
    ) {
        // Read only the three fixed-size header fields we need. We deliberately avoid casting
        // `header` to a `mach_header_64` struct pointer and reading it wholesale: we don't yet know
        // that all `MACH_HEADER_64_LEN` bytes are valid to read as one access, and we want the
        // unsafe surface limited to reads we can individually justify.
        //
        // SAFETY: the caller guarantees `header` points at a mapped Mach-O header, which is always
        // at least `MACH_HEADER_64_LEN` bytes. Every read here is unaligned because dyld makes no
        // alignment promises about the pointers it hands out.
        let magic = unsafe { header.cast::<u32>().read_unaligned() };
        if magic != MH_MAGIC_64 {
            // Every image in the 64-bit-only builds we ship is Mach-O 64.
            return;
        }
        // SAFETY: as above.
        let ncmds = unsafe { header.add(NCMDS_OFFSET).cast::<u32>().read_unaligned() };
        // SAFETY: as above.
        let sizeofcmds = unsafe { header.add(SIZEOFCMDS_OFFSET).cast::<u32>().read_unaligned() };

        // The load commands directly follow the header, and `sizeofcmds` is the exact number of
        // bytes they occupy per <mach-o/loader.h> -- unlike `ncmds`, it actually bounds how far we
        // can walk, so we hand the parser a slice no wider than that instead of trusting `ncmds`
        // alone to keep us in-bounds.
        //
        // SAFETY: a Mach-O image is a header followed by `sizeofcmds` bytes of load commands, all
        // of which live within the image's own mapping.
        let commands = unsafe {
            std::slice::from_raw_parts(header.add(MACH_HEADER_64_LEN), sizeofcmds as usize)
        };

        // Everything from here on is safe, bounds-checked slice parsing -- see
        // `parse_load_commands`.
        let (build_id, segments) = parse_load_commands(commands, ncmds);

        mappings.extend(
            segments
                .into_iter()
                .filter_map(|segment| segment_mapping(&segment, slide, pathname, build_id.clone())),
        );
    }

    /// Builds the pprof mapping for a single executable segment of an image loaded at `slide`.
    ///
    /// `memory_start` is the segment's actual runtime address (`vmaddr + slide`); `memory_offset` is
    /// left as the segment's *static* `vmaddr`. That pairing is exactly what lets `pprof_util`
    /// recover the original, unslid Mach-O address for a sampled runtime address, via
    /// `addr - memory_start + memory_offset` (see `pprof_util::StackProfile::to_pprof_proto`) --
    /// which simplifies to `addr - slide`, the address a symbolizer can look up directly in the
    /// unslid binary/dSYM. Getting this pairing right is the entire point of this module: a wrong
    /// slide sign, or `memory_offset` set to anything other than the segment's own `vmaddr`, still
    /// produces *a* mapping and *a* profile, just one that recovers the wrong address and silently
    /// reproduces the original bug. See `profiling_tests.rs` for a test that serializes a mapping
    /// built by this function through the real `pprof_util` pipeline and asserts the emitted
    /// `Location.address` is correct.
    #[cfg(any(target_os = "macos", test))]
    pub(super) fn segment_mapping(
        segment: &ExecutableSegment,
        slide: isize,
        pathname: &std::path::Path,
        build_id: Option<BuildId>,
    ) -> Option<Mapping> {
        let vmaddr = usize::try_from(segment.vmaddr).ok()?;
        let vmsize = usize::try_from(segment.vmsize).ok()?;
        let memory_start = vmaddr.wrapping_add_signed(slide);

        Some(Mapping {
            memory_start,
            memory_end: memory_start.saturating_add(vmsize),
            memory_offset: vmaddr,
            file_offset: segment.fileoff,
            pathname: pathname.to_path_buf(),
            build_id,
        })
    }

    /// Parses the Mach-O load commands following a header, given `ncmds` and the raw bytes of
    /// exactly the header's `sizeofcmds`-byte load-command region.
    ///
    /// Returns the build-id carried by `LC_UUID`, if present, and every executable `LC_SEGMENT_64`.
    /// This is pure, bounds-checked slice parsing with no unsafe code: every read is checked
    /// against `buf`'s actual length, so a corrupt or truncated header can only cut the walk short,
    /// never read out of bounds. This mirrors the level of rigor (if not the exact mechanism) of
    /// the `mappings` crate's Linux `dl_iterate_phdr` callback.
    #[cfg(any(target_os = "macos", test))]
    pub(super) fn parse_load_commands(
        buf: &[u8],
        ncmds: u32,
    ) -> (Option<BuildId>, Vec<ExecutableSegment>) {
        let mut build_id = None;
        let mut segments = Vec::new();
        let mut offset = 0usize;

        for _ in 0..ncmds {
            let Some(header) = buf.get(offset..offset + LOAD_COMMAND_LEN) else {
                break; // Not enough bytes left in `sizeofcmds` for another load command.
            };
            let cmd = u32::from_ne_bytes(header[0..4].try_into().expect("4-byte slice"));
            let cmdsize = u32::from_ne_bytes(header[4..8].try_into().expect("4-byte slice"));
            let Ok(cmdsize) = usize::try_from(cmdsize) else {
                break;
            };
            if cmdsize < LOAD_COMMAND_LEN {
                // A bogus size would leave us walking in place forever.
                break;
            }
            let Some(command) = buf.get(offset..offset + cmdsize) else {
                break; // The command claims to extend past `sizeofcmds`; the header is corrupt.
            };

            match cmd {
                LC_SEGMENT_64 if command.len() >= SEGMENT_COMMAND_64_LEN => {
                    let initprot =
                        i32::from_ne_bytes(command[60..64].try_into().expect("4-byte slice"));
                    let vmaddr =
                        u64::from_ne_bytes(command[24..32].try_into().expect("8-byte slice"));
                    let vmsize =
                        u64::from_ne_bytes(command[32..40].try_into().expect("8-byte slice"));
                    let fileoff =
                        u64::from_ne_bytes(command[40..48].try_into().expect("8-byte slice"));
                    if initprot & VM_PROT_EXECUTE != 0 && vmsize > 0 {
                        segments.push(ExecutableSegment {
                            vmaddr,
                            vmsize,
                            fileoff,
                        });
                    }
                }
                LC_UUID if command.len() >= UUID_COMMAND_LEN => {
                    build_id = Some(BuildId(command[8..24].to_vec()));
                }
                // Either an uninteresting command, or one too small for the type its `cmd` claims
                // (a corrupt header) -- skip it either way rather than misinterpreting its bytes.
                _ => {}
            }

            offset += cmdsize;
        }

        (build_id, segments)
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

    // Serve the same mapping-aware profile that the direct-dump path produces (see
    // `dump_pprof_for_current_platform`), rather than falling back to `dump_pprof`'s mapping-less
    // output on macOS. `false` here means this endpoint is never subject to
    // `ensure_mappings_are_symbolicatable`: a developer hitting this endpoint on purpose should see
    // the raw profile (or the underlying dump error), not a 500 from a guard meant to protect
    // Sentry attachments.
    let pprof = dump_pprof_for_current_platform(&mut prof_ctl, false).map_err(|err| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            err.to_string(),
        )
    })?;
    Ok(pprof)
}

#[cfg(all(test, feature = "jemalloc_pprof"))]
#[path = "profiling_tests.rs"]
mod tests;
