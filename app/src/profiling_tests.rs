use std::io::Read as _;

use super::*;

/// Real jemalloc heap profiles captured from a standalone binary linking the
/// same `jemalloc_pprof` version this workspace resolves (0.8.2), so these
/// exercise the actual wire format `dump_pprof()` produces rather than a
/// hand-rolled re-encoding of it.
///
/// Captured from a normal `x86_64-unknown-linux-gnu` build, whose linker
/// embeds a GNU build-id by default.
const FIXTURE_WITH_BUILD_ID: &[u8] = include_bytes!("profiling_fixture_with_build_id.pb.gz");
/// Captured from an `x86_64-unknown-linux-musl` build using this repo's
/// vendored musl-cross-make toolchain (`script/linux/configure_musl_toolchain`)
/// without an explicit `--build-id` flag -- the same toolchain and
/// configuration that produced the daemon's pre-fix degenerate mapping table.
const FIXTURE_WITHOUT_BUILD_ID: &[u8] = include_bytes!("profiling_fixture_without_build_id.pb.gz");
/// The first 4 bytes of [`FIXTURE_WITH_BUILD_ID`]'s ungzipped protobuf: a tag
/// and length prefix declaring 4 bytes of payload for the profile's first
/// field, with only 2 of those bytes actually present.
const FIXTURE_TRUNCATED: &[u8] = include_bytes!("profiling_fixture_truncated.pb");

fn ungzip(gzipped: &[u8]) -> Vec<u8> {
    let mut profile = Vec::new();
    flate2::read::GzDecoder::new(gzipped)
        .read_to_end(&mut profile)
        .expect("fixture is valid gzip");
    profile
}

#[test]
fn detects_mapping_with_a_real_build_id() {
    let profile = ungzip(FIXTURE_WITH_BUILD_ID);
    let (total, missing_build_id) = count_pprof_mappings_missing_build_id(&profile).unwrap();
    assert!(total > 0);
    assert_eq!(missing_build_id, 0);
}

#[test]
fn detects_mappings_with_no_build_id() {
    let profile = ungzip(FIXTURE_WITHOUT_BUILD_ID);
    let (total, missing_build_id) = count_pprof_mappings_missing_build_id(&profile).unwrap();
    assert!(total > 0);
    assert_eq!(missing_build_id, total);
}

#[test]
fn truncated_profile_is_an_error() {
    assert!(count_pprof_mappings_missing_build_id(FIXTURE_TRUNCATED).is_err());
}

#[test]
fn end_to_end_through_gzip_detects_missing_build_id() {
    let (total, missing_build_id) =
        ungzip_and_count_pprof_mappings_missing_build_id(FIXTURE_WITHOUT_BUILD_ID).unwrap();
    assert!(total > 0);
    assert_eq!(missing_build_id, total);
}

#[test]
fn end_to_end_through_gzip_reports_zero_when_build_id_present() {
    let (total, missing_build_id) =
        ungzip_and_count_pprof_mappings_missing_build_id(FIXTURE_WITH_BUILD_ID).unwrap();
    assert!(total > 0);
    assert_eq!(missing_build_id, 0);
}

#[test]
fn not_gzip_is_an_error() {
    // Missing the gzip magic bytes entirely, so decompression itself must
    // fail before any protobuf parsing runs.
    assert!(ungzip_and_count_pprof_mappings_missing_build_id(&[0, 1, 2, 3]).is_err());
}
