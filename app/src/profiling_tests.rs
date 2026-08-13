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
