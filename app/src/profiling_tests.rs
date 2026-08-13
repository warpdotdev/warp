use std::path::Path;

use pprof_util::{BuildId, Mapping};

use super::{macos_mappings, mappings_are_symbolicatable};

// --- Mach-O load-command parsing ---------------------------------------------------------------

const LC_SEGMENT_64: u32 = 0x19;
const LC_UUID: u32 = 0x1b;
const VM_PROT_EXECUTE: i32 = 0x4;

fn push_load_command(buf: &mut Vec<u8>, cmd: u32, body: &[u8]) {
    let cmdsize = u32::try_from(8 + body.len()).expect("test command bodies are tiny");
    buf.extend_from_slice(&cmd.to_ne_bytes());
    buf.extend_from_slice(&cmdsize.to_ne_bytes());
    buf.extend_from_slice(body);
}

fn segment_64_body(vmaddr: u64, vmsize: u64, fileoff: u64, initprot: i32) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&[0u8; 16]); // segname
    body.extend_from_slice(&vmaddr.to_ne_bytes());
    body.extend_from_slice(&vmsize.to_ne_bytes());
    body.extend_from_slice(&fileoff.to_ne_bytes());
    body.extend_from_slice(&0u64.to_ne_bytes()); // filesize
    body.extend_from_slice(&initprot.to_ne_bytes()); // maxprot
    body.extend_from_slice(&initprot.to_ne_bytes()); // initprot
    body.extend_from_slice(&0u32.to_ne_bytes()); // nsects
    body.extend_from_slice(&0u32.to_ne_bytes()); // flags
    body
}

#[test]
fn parse_load_commands_extracts_executable_segment_and_build_id() {
    let mut buf = Vec::new();
    let uuid: Vec<u8> = (1..=16).collect();
    push_load_command(&mut buf, LC_UUID, &uuid);
    push_load_command(
        &mut buf,
        LC_SEGMENT_64,
        &segment_64_body(0x1000, 0x2000, 0x0, VM_PROT_EXECUTE),
    );

    let (build_id, segments) = macos_mappings::parse_load_commands(&buf, 2);

    assert_eq!(build_id, Some(BuildId(uuid)));
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].vmaddr, 0x1000);
    assert_eq!(segments[0].vmsize, 0x2000);
    assert_eq!(segments[0].fileoff, 0x0);
}

#[test]
fn parse_load_commands_skips_non_executable_segments() {
    let mut buf = Vec::new();
    push_load_command(
        &mut buf,
        LC_SEGMENT_64,
        &segment_64_body(0x1000, 0x2000, 0, 0 /* not executable */),
    );

    let (build_id, segments) = macos_mappings::parse_load_commands(&buf, 1);

    assert_eq!(build_id, None);
    assert!(segments.is_empty());
}

#[test]
fn parse_load_commands_skips_zero_sized_segments() {
    let mut buf = Vec::new();
    push_load_command(
        &mut buf,
        LC_SEGMENT_64,
        &segment_64_body(0x1000, 0 /* vmsize */, 0, VM_PROT_EXECUTE),
    );

    let (_, segments) = macos_mappings::parse_load_commands(&buf, 1);

    assert!(segments.is_empty());
}

#[test]
fn parse_load_commands_stops_at_a_command_that_overruns_the_buffer() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&LC_SEGMENT_64.to_ne_bytes());
    buf.extend_from_slice(&1000u32.to_ne_bytes()); // cmdsize far exceeds buf.len()

    let (build_id, segments) = macos_mappings::parse_load_commands(&buf, 1);

    assert_eq!(build_id, None);
    assert!(segments.is_empty());
}

#[test]
fn parse_load_commands_stops_at_an_undersized_cmdsize_instead_of_looping_forever() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&LC_SEGMENT_64.to_ne_bytes());
    buf.extend_from_slice(&0u32.to_ne_bytes()); // smaller than a load_command header itself

    // `ncmds` is huge; if the parser didn't bail on the bogus size it would spin (or panic on an
    // out-of-bounds slice) long before this test would ever complete.
    let (build_id, segments) = macos_mappings::parse_load_commands(&buf, u32::MAX);

    assert_eq!(build_id, None);
    assert!(segments.is_empty());
}

#[test]
fn parse_load_commands_ignores_a_uuid_command_too_small_to_carry_a_uuid() {
    let mut buf = Vec::new();
    // Claims to be `LC_UUID` but its declared size can't actually hold a 16-byte UUID.
    push_load_command(&mut buf, LC_UUID, &[0u8; 4]);

    let (build_id, _) = macos_mappings::parse_load_commands(&buf, 1);

    assert_eq!(build_id, None);
}

#[test]
fn parse_load_commands_never_reads_past_the_buffer_it_is_given() {
    // A single-byte buffer can't even hold a load_command header (8 bytes); this should stop
    // immediately rather than reading (or panicking) out of bounds.
    let buf = [0u8; 1];

    let (build_id, segments) = macos_mappings::parse_load_commands(&buf, 5);

    assert_eq!(build_id, None);
    assert!(segments.is_empty());
}

// --- Mapping arithmetic, end to end through real pprof serialization ---------------------------

/// Minimal mirrors of the `perftools.profiles` messages this test needs to inspect. `pprof_util`'s
/// own generated proto types are private to that crate, but protobuf decoding against a subset of
/// a message's fields is well-defined: unknown fields (`sample_type`, `sample`, `period_type`,
/// etc.) are simply skipped, so these tags-only mirrors decode correctly against the real wire
/// bytes `pprof_util::StackProfile::to_pprof` produces.
#[derive(Clone, PartialEq, prost::Message)]
struct TestProfile {
    #[prost(message, repeated, tag = "3")]
    mapping: Vec<TestMapping>,
    #[prost(message, repeated, tag = "4")]
    location: Vec<TestLocation>,
    #[prost(string, repeated, tag = "6")]
    string_table: Vec<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct TestMapping {
    #[prost(uint64, tag = "1")]
    id: u64,
    #[prost(int64, tag = "5")]
    filename: i64,
    #[prost(int64, tag = "6")]
    build_id: i64,
}

#[derive(Clone, PartialEq, prost::Message)]
struct TestLocation {
    #[prost(uint64, tag = "1")]
    id: u64,
    #[prost(uint64, tag = "2")]
    mapping_id: u64,
    #[prost(uint64, tag = "3")]
    address: u64,
}

/// This is the test that would have caught the original bug: it doesn't just check
/// `segment_mapping`'s arithmetic in isolation, it serializes a mapping built by that function
/// through the *real* `pprof_util::StackProfile::to_pprof` pipeline (the same one
/// `dump_macos_pprof` uses) and decodes the *real* resulting protobuf bytes, then asserts the
/// emitted `Location.address` is the static Mach-O address a symbolizer/dSYM actually expects.
/// A wrong slide sign, or a `memory_offset` that isn't literally the segment's own `vmaddr`, would
/// still produce a mapping and a profile -- just one with the wrong address, i.e. the same silent
/// `<unknown>` frames this module exists to fix. That failure mode is invisible to tests that only
/// look at `Mapping`'s fields in isolation, which is why this test goes all the way through
/// serialization instead.
#[test]
fn segment_mapping_produces_the_correct_address_through_real_pprof_serialization() {
    use std::io::Read as _;

    // An image loaded at a large, nonzero ASLR slide -- the exact scenario that silently produced
    // unresolvable addresses before this fix.
    let segment = macos_mappings::ExecutableSegment {
        vmaddr: 0x1_0000,
        vmsize: 0x2000,
        fileoff: 0,
    };
    let slide: isize = 0x5_5555_0000;
    let build_id = BuildId(vec![0xAB; 16]);
    let pathname = Path::new("/Applications/Warp.app/Contents/MacOS/stable");

    let mapping =
        macos_mappings::segment_mapping(&segment, slide, pathname, Some(build_id.clone()))
            .expect("valid vmaddr/vmsize should always produce a mapping");

    // A runtime sample address 0x20 bytes into the segment.
    let runtime_addr = mapping.memory_start + 0x20;
    assert!(
        runtime_addr < mapping.memory_end,
        "test address must fall inside the mapping"
    );

    let mut profile = pprof_util::StackProfile::default();
    profile.push_mapping(mapping);
    profile.push_stack(
        pprof_util::WeightedStack {
            addrs: vec![runtime_addr],
            weight: 1.0,
        },
        None,
    );

    let gzipped = profile.to_pprof(("inuse_space", "bytes"), ("space", "bytes"), None);
    let mut decompressed = Vec::new();
    flate2::read::GzDecoder::new(gzipped.as_slice())
        .read_to_end(&mut decompressed)
        .expect("to_pprof always produces a valid gzip stream");

    let decoded = <TestProfile as prost::Message>::decode(decompressed.as_slice())
        .expect("to_pprof always produces a valid pprof protobuf");

    assert_eq!(decoded.mapping.len(), 1, "expected exactly one mapping");
    let wire_mapping = &decoded.mapping[0];
    assert_eq!(
        decoded.string_table[usize::try_from(wire_mapping.filename).unwrap()],
        pathname.to_string_lossy()
    );
    assert_eq!(
        decoded.string_table[usize::try_from(wire_mapping.build_id).unwrap()],
        build_id.to_string()
    );

    assert_eq!(decoded.location.len(), 1, "expected exactly one location");
    let location = &decoded.location[0];
    assert_eq!(
        location.mapping_id, wire_mapping.id,
        "the location must reference the mapping we built"
    );

    // `to_pprof_proto` subtracts 1 from every sample address before rebasing it (stack addresses
    // are return addresses, one past the call instruction), so the expected file-relative address
    // is the static Mach-O address of our sample, minus 1.
    let expected_static_addr = 0x1_0000_u64 + 0x20 - 1;
    assert_eq!(
        location.address, expected_static_addr,
        "a wrong slide or memory_offset would recover the wrong address here, silently \
         reproducing the original unsymbolicatable-profile bug"
    );
}

// --- BuildId hex formatting ---------------------------------------------------------------------

/// `pprof_util::BuildId`'s `Display` renders continuous lowercase hex with no separators. That is
/// NOT the canonical hyphenated Mach-O UUID format used by `dwarfdump --uuid`, `lldb`, and Sentry's
/// own Mach-O debug-id convention (`E621E1F8-C36C-495A-93FC-0C247A3E6E5F`, plus a `-0` age suffix
/// for Sentry's `DebugId` specifically). Both encode the same 16 bytes, so cross-referencing this
/// profile's build-id against a dSYM's UUID requires a human (or tool) to strip the dashes/case
/// first. This test pins down that behavior so a future `pprof_util` upgrade that changes it is
/// caught here rather than silently changing what ends up in the pprof mapping's `build_id` field.
#[test]
fn build_id_display_is_continuous_lowercase_hex_not_the_canonical_uuid_format() {
    #[rustfmt::skip]
    let uuid_bytes: [u8; 16] = [
        0xe6, 0x21, 0xe1, 0xf8, 0xc3, 0x6c, 0x49, 0x5a,
        0x93, 0xfc, 0x0c, 0x24, 0x7a, 0x3e, 0x6e, 0x5f,
    ];

    let build_id = BuildId(uuid_bytes.to_vec());

    assert_eq!(build_id.to_string(), "e621e1f8c36c495a93fc0c247a3e6e5f");
}

// --- Mapping-table validation predicate --------------------------------------------------------

fn mapping(pathname: &str, build_id: Option<BuildId>) -> Mapping {
    Mapping {
        memory_start: 0,
        memory_end: 0,
        memory_offset: 0,
        file_offset: 0,
        pathname: pathname.into(),
        build_id,
    }
}

#[test]
fn mappings_are_symbolicatable_rejects_an_empty_mapping_table() {
    assert!(!mappings_are_symbolicatable(&[], Path::new("/bin/warp")));
}

#[test]
fn mappings_are_symbolicatable_rejects_a_table_missing_the_current_executable() {
    let mappings = [mapping(
        "/usr/lib/libSystem.B.dylib",
        Some(BuildId(vec![1])),
    )];
    assert!(!mappings_are_symbolicatable(
        &mappings,
        Path::new("/bin/warp")
    ));
}

#[test]
fn mappings_are_symbolicatable_rejects_a_current_executable_mapping_with_no_build_id() {
    let mappings = [mapping("/bin/warp", None)];
    assert!(!mappings_are_symbolicatable(
        &mappings,
        Path::new("/bin/warp")
    ));
}

#[test]
fn mappings_are_symbolicatable_accepts_a_current_executable_mapping_with_a_build_id() {
    let mappings = [
        mapping("/usr/lib/libSystem.B.dylib", None),
        mapping("/bin/warp", Some(BuildId(vec![1, 2, 3]))),
    ];
    assert!(mappings_are_symbolicatable(
        &mappings,
        Path::new("/bin/warp")
    ));
}

/// Real macOS testing found that `std::env::current_exe()` and dyld's reported image path can
/// differ when the executable is reached through a symlink (e.g. a build cache mounted at another
/// path). This exercises the canonicalized-path fallback that handles that case.
#[test]
fn mappings_are_symbolicatable_accepts_a_symlinked_current_exe_path() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let real_exe = dir.path().join("warp-real");
    std::fs::write(&real_exe, b"not a real binary, just needs to exist").expect("write file");
    let symlinked_exe = dir.path().join("warp-symlink");
    std::os::unix::fs::symlink(&real_exe, &symlinked_exe).expect("create symlink");

    // dyld reports the canonical path; `current_exe()` here is the symlink dyld loaded through.
    let mappings = [mapping(
        real_exe.to_str().expect("utf8 path"),
        Some(BuildId(vec![1, 2, 3])),
    )];

    assert!(mappings_are_symbolicatable(&mappings, &symlinked_exe));
}
