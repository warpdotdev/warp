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
/// On Linux the profile is produced in-process via the `jemalloc_pprof` crate
/// as a raw (unsymbolized) pprof -- sample addresses + mappings + GNU build-id
/// -- and is symbolized offline against the debug-info file uploaded to Sentry
/// by the release process (matched by build-id).  On other platforms it spawns
/// the bundled `pprof` binary to fetch and symbolicate the heap profile from
/// the local HTTP server.  Either way, the resulting profile is attached to a
/// Sentry event.
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

                    // On macOS the heap profile arrives unsymbolicated: the
                    // shipped release binary is stripped and the in-process
                    // jemalloc profiler cannot populate Mach-O mappings (there
                    // is no `/proc/self/maps`), so the pprof contains only raw
                    // instruction addresses.  Record the main image's
                    // debug-id, runtime load address, and `__TEXT` vmaddr as
                    // context so the profile can be symbolicated offline
                    // against the debug-info file uploaded to Sentry (matched
                    // by `debug_id`).  See APP-4796.
                    #[cfg(target_os = "macos")]
                    if let Some(image) = macos_main_image_symbolication_info() {
                        let mut image_map: std::collections::BTreeMap<
                            String,
                            sentry::protocol::Value,
                        > = std::collections::BTreeMap::new();
                        image_map.insert("code_file".to_string(), image.code_file.into());
                        image_map.insert("debug_id".to_string(), image.debug_id.into());
                        image_map.insert(
                            "image_base".to_string(),
                            format!("0x{:x}", image.image_base).into(),
                        );
                        image_map.insert(
                            "text_vmaddr".to_string(),
                            format!("0x{:x}", image.text_vmaddr).into(),
                        );
                        scope.set_context(
                            "heap_profile_image",
                            sentry::protocol::Context::Other(image_map),
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

/// Symbolication metadata for the running main executable, used to make an
/// otherwise-unsymbolicated macOS heap profile symbolicatable offline.
#[cfg(all(target_os = "macos", feature = "heap_usage_tracking"))]
struct MachoImageInfo {
    /// On-disk path of the main executable.
    code_file: String,
    /// Mach-O `LC_UUID` formatted as a Sentry debug-id (e.g.
    /// `11e1342b-e904-3852-a433-14274590ecac`).
    debug_id: String,
    /// Runtime load address of the `__TEXT` segment (`vmaddr` + ASLR slide).
    image_base: u64,
    /// `__TEXT` segment `vmaddr` as recorded in the Mach-O/dSYM (no slide).
    text_vmaddr: u64,
}

/// Collects [`MachoImageInfo`] for the running main executable so that an
/// unsymbolicated macOS heap profile can be symbolicated offline against the
/// debug-info file uploaded to Sentry (matched by `debug_id`).
///
/// macOS release binaries are stripped and the in-process jemalloc profiler
/// cannot populate Mach-O mappings (there is no `/proc/self/maps`), so the
/// uploaded pprof contains only raw instruction addresses.  Recording the main
/// image's `debug_id`, runtime load address (`image_base`) and `__TEXT`
/// `vmaddr` lets offline tooling translate those addresses to symbols.
#[cfg(all(target_os = "macos", feature = "heap_usage_tracking"))]
fn macos_main_image_symbolication_info() -> Option<MachoImageInfo> {
    // SAFETY: Image index 0 is always the main executable.  Its Mach-O header
    // and load commands are mapped and readable for the lifetime of the
    // process, and `sizeofcmds` bounds the readable load-command region.
    unsafe {
        let header = libc::_dyld_get_image_header(0);
        if header.is_null() {
            return None;
        }
        let header_ptr = header as *const u8;

        // Read `sizeofcmds` (offset 20 in `mach_header_64`) to bound the slice
        // covering the header and its load commands.
        let header_only = std::slice::from_raw_parts(header_ptr, 32);
        let sizeofcmds = u32::from_le_bytes(header_only.get(20..24)?.try_into().ok()?) as usize;
        let total = 32usize.checked_add(sizeofcmds)?;
        let buf = std::slice::from_raw_parts(header_ptr, total);

        let (debug_id, text_vmaddr) = parse_macho_image_uuid_and_text_vmaddr(buf)?;

        let slide = libc::_dyld_get_image_vmaddr_slide(0);
        let image_base = text_vmaddr.wrapping_add(slide as u64);

        let name_ptr = libc::_dyld_get_image_name(0);
        let code_file = if name_ptr.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(name_ptr)
                .to_string_lossy()
                .into_owned()
        };

        Some(MachoImageInfo {
            code_file,
            debug_id,
            image_base,
            text_vmaddr,
        })
    }
}

/// Parses a 64-bit little-endian Mach-O header (including its load commands)
/// and returns the image's `LC_UUID` (formatted as a Sentry debug-id, e.g.
/// `11e1342b-e904-3852-a433-14274590ecac`) together with the `__TEXT`
/// segment's `vmaddr`.
///
/// `buf` must cover the Mach-O header and all of its load commands.  Returns
/// `None` if the buffer is not a recognized 64-bit little-endian Mach-O image,
/// is truncated, or is missing an `LC_UUID`/`__TEXT` segment.
#[cfg(any(all(target_os = "macos", feature = "heap_usage_tracking"), test))]
fn parse_macho_image_uuid_and_text_vmaddr(buf: &[u8]) -> Option<(String, u64)> {
    // MH_MAGIC_64 stored natively in a little-endian image.
    const MH_MAGIC_64_LE: [u8; 4] = [0xcf, 0xfa, 0xed, 0xfe];
    const LC_SEGMENT_64: u32 = 0x19;
    const LC_UUID: u32 = 0x1b;
    const HEADER_LEN: usize = 32;

    if buf.len() < HEADER_LEN || buf[0..4] != MH_MAGIC_64_LE {
        return None;
    }

    let read_u32 = |off: usize| -> Option<u32> {
        buf.get(off..off + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    let read_u64 = |off: usize| -> Option<u64> {
        buf.get(off..off + 8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
    };

    let ncmds = read_u32(16)? as usize;

    let mut uuid: Option<[u8; 16]> = None;
    let mut text_vmaddr: Option<u64> = None;

    let mut offset = HEADER_LEN;
    for _ in 0..ncmds {
        let cmd = read_u32(offset)?;
        let cmdsize = read_u32(offset + 4)? as usize;
        // A load command must contain at least its `cmd`/`cmdsize` words and
        // must not run past the end of the buffer.
        if cmdsize < 8 || offset.checked_add(cmdsize)? > buf.len() {
            return None;
        }

        match cmd {
            LC_UUID => {
                uuid = Some(buf.get(offset + 8..offset + 24)?.try_into().unwrap());
            }
            LC_SEGMENT_64 => {
                let segname = buf.get(offset + 8..offset + 24)?;
                let name_len = segname
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(segname.len());
                if &segname[..name_len] == b"__TEXT" {
                    // `vmaddr` follows the 16-byte segment name.
                    text_vmaddr = read_u64(offset + 24);
                }
            }
            _ => {}
        }

        offset += cmdsize;
    }

    let uuid = uuid?;
    let debug_id = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        uuid[0], uuid[1], uuid[2], uuid[3], uuid[4], uuid[5], uuid[6], uuid[7], uuid[8], uuid[9],
        uuid[10], uuid[11], uuid[12], uuid[13], uuid[14], uuid[15],
    );
    Some((debug_id, text_vmaddr?))
}

#[cfg(feature = "heap_usage_tracking")]
async fn dump_jemalloc_heap_profile_inner() -> anyhow::Result<Vec<u8>> {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "linux")] {
            // `jemalloc_pprof` only supports Linux. We build it WITHOUT the
            // `symbolize` feature, so `dump_pprof()` returns a raw, gzipped
            // pprof (sample addresses + mappings + GNU build-id) that is
            // symbolized offline against the debug-info file by build-id.  Dump
            // it directly in-process -- no external `pprof`/Go binary, HTTP
            // round-trip, or port dependency required (the latter matter for
            // the headless remote server daemon, which has no bundled helpers
            // next to it).
            dump_jemalloc_pprof_bytes().await
        } else {
            use anyhow::Context as _;

            // Create a temporary file for the profile output.
            let temp_dir = tempfile::tempdir().context("Failed to create temporary directory")?;
            let profile_path = temp_dir.path().join("heap-profile.pb");

            // Run pprof to fetch and symbolicate the heap profile.
            let pprof_path = pprof_binary_path()?;
            let output = command::r#async::Command::new(pprof_path)
                .args(["--proto", "--symbolize=local", "-output"])
                .arg(&profile_path)
                .arg("http://127.0.0.1:9277/debug/pprof/heap")
                .output()
                .await
                .context("Failed to execute pprof")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("pprof failed: {stderr}");
            }

            // Read the profile data from the temporary file.
            let profile_data =
                std::fs::read(&profile_path).context("Failed to read heap profile from disk")?;

            Ok(profile_data)
        }
    }
}

/// Produces a raw (unsymbolized), gzipped pprof heap profile directly from the
/// in-process jemalloc profiler. The profile carries sample addresses,
/// mappings, and the GNU build-id, and is symbolized offline against the
/// matching debug-info file (by build-id).
///
/// This is the same dump that [`handle_get_heap`] serves over HTTP, but
/// invoked directly so callers don't need to reach the local HTTP server.
/// Requires the `jemalloc_pprof` feature, which is Linux-only.
#[cfg(all(feature = "jemalloc_pprof", target_os = "linux"))]
async fn dump_jemalloc_pprof_bytes() -> anyhow::Result<Vec<u8>> {
    let Some(prof_ctl) = jemalloc_pprof::PROF_CTL.as_ref() else {
        anyhow::bail!("heap profiler not initialized");
    };
    let mut prof_ctl = prof_ctl.lock().await;
    if !prof_ctl.activated() {
        anyhow::bail!("heap profiling not activated");
    }
    prof_ctl.dump_pprof()
}

#[cfg(all(feature = "heap_usage_tracking", not(target_os = "linux")))]
fn pprof_binary_path() -> anyhow::Result<std::path::PathBuf> {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            use anyhow::Context as _;

            let app_bundle_dir = std::path::PathBuf::from(warp_core::macos::get_bundle_path().context("Failed to get app bundle path")?);
            Ok(app_bundle_dir.join("Contents/Helpers/pprof"))
        }
        else {
            Err(anyhow::anyhow!("pprof binary path not supported on this platform"))
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

#[cfg(test)]
#[path = "profiling_tests.rs"]
mod tests;
