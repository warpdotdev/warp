//! Strips jemalloc's own allocator/profiler sampling-prologue frames from the
//! leaf end of each sample in a *symbolized* pprof heap profile.
//!
//! Every jemalloc heap-profile sample is captured from inside jemalloc's own
//! sampling machinery (`prof_backtrace` walks the stack starting at the
//! current frame, skipping zero frames). As a result the leaf of every
//! sample is a run of jemalloc/profiler frames (`_rjem_je_prof_backtrace`,
//! `_rjem_je_prof_tctx_create`, `prof_alloc_prep`, `imalloc_body`, `imalloc`,
//! `_rjem_je_malloc_default`, and the calloc/realloc/posix_memalign
//! equivalents) rather than the application code that actually requested the
//! memory. This module removes that leading run so the leaf of each sample
//! becomes the first application frame, without altering sample values,
//! totals, or any other part of the profile.
//!
//! This only has an effect on an already-symbolized profile, i.e. one whose
//! `Function` entries carry real names. It is a no-op (frames are left
//! unmatched, and thus untouched) on a raw, unsymbolized profile.

use std::collections::HashSet;

use anyhow::Context as _;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use prost::Message as _;

/// Symbol prefixes that identify jemalloc's own allocator/profiler
/// bookkeeping, as opposed to application code. These are the raw (mangled)
/// C symbol names jemalloc exports, not Rust/C++ demangled names.
const ALLOCATOR_SYMBOL_PREFIXES: &[&str] = &[
    // The Warp jemallocator fork's namespace prefixes.
    "_rjem_je_",
    "_rjem_",
    // Sampling entry points, e.g. `imalloc`, `imalloc_body`, `imalloc_no_sample`.
    "imalloc",
    // Profiling bookkeeping, e.g. `prof_alloc_prep`, `prof_backtrace`,
    // `prof_tctx_create`, `prof_gctx_create`.
    "prof_",
];

/// Symbol suffixes for the public entry points' `*_default` implementations,
/// which may appear without one of the prefixes above depending on symbol
/// visibility/namespacing.
const ALLOCATOR_SYMBOL_SUFFIXES: &[&str] = &[
    "malloc_default",
    "calloc_default",
    "realloc_default",
    "posix_memalign_default",
];

fn is_allocator_symbol(name: &str) -> bool {
    ALLOCATOR_SYMBOL_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
        || ALLOCATOR_SYMBOL_SUFFIXES
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

/// Strips the leading run of allocator/profiler frames from every sample in
/// a gzipped pprof profile, returning the re-encoded, re-gzipped profile.
///
/// Only a *leading* run (from the leaf inward) is stripped: an allocator
/// frame that appears after the first application frame is left in place. A
/// sample made up entirely of allocator frames is left untouched rather than
/// emptied. Sample values and all other profile fields are preserved
/// exactly; only `Sample.location_id` lists are trimmed.
pub fn strip_allocator_prologue(gzipped_profile: &[u8]) -> anyhow::Result<Vec<u8>> {
    let raw = gunzip(gzipped_profile).context("Failed to gunzip pprof profile")?;
    let mut profile = Profile::decode(raw.as_slice()).context("Failed to decode pprof profile")?;

    let allocator_location_ids = allocator_location_ids(&profile);
    for sample in &mut profile.sample {
        strip_leading_allocator_locations(&allocator_location_ids, &mut sample.location_id);
    }

    let mut encoded = Vec::new();
    profile
        .encode(&mut encoded)
        .context("Failed to encode pprof profile")?;
    gzip(&encoded).context("Failed to gzip pprof profile")
}

/// Returns the set of location ids whose resolved function name(s) are
/// entirely allocator/profiler symbols. A location with inlined frames
/// (multiple `Line` entries) only counts as an allocator location if every
/// inlined frame is allocator code; a location with no resolvable function
/// name is conservatively treated as application code.
fn allocator_location_ids(profile: &Profile) -> HashSet<u64> {
    let function_symbol = |function_id: u64| -> Option<&str> {
        let function = profile.function.iter().find(|f| f.id == function_id)?;
        let name_index = if function.name != 0 {
            function.name
        } else {
            function.system_name
        };
        usize::try_from(name_index)
            .ok()
            .and_then(|idx| profile.string_table.get(idx))
            .map(String::as_str)
    };

    profile
        .location
        .iter()
        .filter(|location| {
            !location.line.is_empty()
                && location
                    .line
                    .iter()
                    .all(|line| function_symbol(line.function_id).is_some_and(is_allocator_symbol))
        })
        .map(|location| location.id)
        .collect()
}

/// Removes the leading run of `location_ids` (leaf-first, per the pprof
/// format) that are allocator locations. Leaves the list untouched if every
/// location in it is an allocator location, so a sample is never emptied.
fn strip_leading_allocator_locations(
    allocator_location_ids: &HashSet<u64>,
    location_ids: &mut Vec<u64>,
) {
    let strip_count = location_ids
        .iter()
        .take_while(|id| allocator_location_ids.contains(id))
        .count();
    if strip_count < location_ids.len() {
        location_ids.drain(0..strip_count);
    }
}

fn gunzip(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    use std::io::Read as _;

    let mut decoder = GzDecoder::new(bytes);
    let mut decoded = Vec::new();
    decoder.read_to_end(&mut decoded)?;
    Ok(decoded)
}

fn gzip(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    use std::io::Write as _;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?)
}

// Minimal hand-written mirror of the pprof wire format
// (https://github.com/google/pprof/blob/main/proto/profile.proto), covering
// every field so that decoding and re-encoding a profile is lossless. We
// hand-roll these rather than depend on a pprof crate's generated types so
// that this logic has no dependency on any particular heap/CPU profiling
// crate or its feature set.
#[derive(Clone, PartialEq, prost::Message)]
struct Profile {
    #[prost(message, repeated, tag = "1")]
    sample_type: Vec<ValueType>,
    #[prost(message, repeated, tag = "2")]
    sample: Vec<Sample>,
    #[prost(message, repeated, tag = "3")]
    mapping: Vec<Mapping>,
    #[prost(message, repeated, tag = "4")]
    location: Vec<Location>,
    #[prost(message, repeated, tag = "5")]
    function: Vec<Function>,
    #[prost(string, repeated, tag = "6")]
    string_table: Vec<String>,
    #[prost(int64, tag = "7")]
    drop_frames: i64,
    #[prost(int64, tag = "8")]
    keep_frames: i64,
    #[prost(int64, tag = "9")]
    time_nanos: i64,
    #[prost(int64, tag = "10")]
    duration_nanos: i64,
    #[prost(message, optional, tag = "11")]
    period_type: Option<ValueType>,
    #[prost(int64, tag = "12")]
    period: i64,
    #[prost(int64, repeated, tag = "13")]
    comment: Vec<i64>,
    #[prost(int64, tag = "14")]
    default_sample_type: i64,
    #[prost(int64, tag = "15")]
    doc_url: i64,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ValueType {
    #[prost(int64, tag = "1")]
    r#type: i64,
    #[prost(int64, tag = "2")]
    unit: i64,
}

#[derive(Clone, PartialEq, prost::Message)]
struct Sample {
    #[prost(uint64, repeated, tag = "1")]
    location_id: Vec<u64>,
    #[prost(int64, repeated, tag = "2")]
    value: Vec<i64>,
    #[prost(message, repeated, tag = "3")]
    label: Vec<Label>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct Label {
    #[prost(int64, tag = "1")]
    key: i64,
    #[prost(int64, tag = "2")]
    str: i64,
    #[prost(int64, tag = "3")]
    num: i64,
    #[prost(int64, tag = "4")]
    num_unit: i64,
}

#[derive(Clone, PartialEq, prost::Message)]
struct Mapping {
    #[prost(uint64, tag = "1")]
    id: u64,
    #[prost(uint64, tag = "2")]
    memory_start: u64,
    #[prost(uint64, tag = "3")]
    memory_limit: u64,
    #[prost(uint64, tag = "4")]
    file_offset: u64,
    #[prost(int64, tag = "5")]
    filename: i64,
    #[prost(int64, tag = "6")]
    build_id: i64,
    #[prost(bool, tag = "7")]
    has_functions: bool,
    #[prost(bool, tag = "8")]
    has_filenames: bool,
    #[prost(bool, tag = "9")]
    has_line_numbers: bool,
    #[prost(bool, tag = "10")]
    has_inline_frames: bool,
}

#[derive(Clone, PartialEq, prost::Message)]
struct Location {
    #[prost(uint64, tag = "1")]
    id: u64,
    #[prost(uint64, tag = "2")]
    mapping_id: u64,
    #[prost(uint64, tag = "3")]
    address: u64,
    #[prost(message, repeated, tag = "4")]
    line: Vec<Line>,
    #[prost(bool, tag = "5")]
    is_folded: bool,
}

#[derive(Clone, PartialEq, prost::Message)]
struct Line {
    #[prost(uint64, tag = "1")]
    function_id: u64,
    #[prost(int64, tag = "2")]
    line: i64,
    #[prost(int64, tag = "3")]
    column: i64,
}

#[derive(Clone, PartialEq, prost::Message)]
struct Function {
    #[prost(uint64, tag = "1")]
    id: u64,
    #[prost(int64, tag = "2")]
    name: i64,
    #[prost(int64, tag = "3")]
    system_name: i64,
    #[prost(int64, tag = "4")]
    filename: i64,
    #[prost(int64, tag = "5")]
    start_line: i64,
}

#[cfg(test)]
#[path = "heap_profile_symbols_tests.rs"]
mod tests;
