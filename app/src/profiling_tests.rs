use super::parse_macho_image_uuid_and_text_vmaddr;

/// Builds a minimal 64-bit little-endian Mach-O header containing a single
/// `__TEXT` `LC_SEGMENT_64` (with the given `vmaddr`) followed by an `LC_UUID`.
fn build_macho(uuid: [u8; 16], text_vmaddr: u64) -> Vec<u8> {
    // An `LC_SEGMENT_64` load command with zero sections is 72 bytes.
    let mut seg = Vec::new();
    seg.extend_from_slice(&0x19u32.to_le_bytes()); // cmd = LC_SEGMENT_64
    seg.extend_from_slice(&72u32.to_le_bytes()); // cmdsize
    let mut segname = [0u8; 16];
    segname[..6].copy_from_slice(b"__TEXT");
    seg.extend_from_slice(&segname);
    seg.extend_from_slice(&text_vmaddr.to_le_bytes()); // vmaddr
    seg.extend_from_slice(&0u64.to_le_bytes()); // vmsize
    seg.extend_from_slice(&0u64.to_le_bytes()); // fileoff
    seg.extend_from_slice(&0u64.to_le_bytes()); // filesize
    seg.extend_from_slice(&0i32.to_le_bytes()); // maxprot
    seg.extend_from_slice(&0i32.to_le_bytes()); // initprot
    seg.extend_from_slice(&0u32.to_le_bytes()); // nsects
    seg.extend_from_slice(&0u32.to_le_bytes()); // flags
    assert_eq!(seg.len(), 72);

    let mut uuid_cmd = Vec::new();
    uuid_cmd.extend_from_slice(&0x1bu32.to_le_bytes()); // cmd = LC_UUID
    uuid_cmd.extend_from_slice(&24u32.to_le_bytes()); // cmdsize
    uuid_cmd.extend_from_slice(&uuid);
    assert_eq!(uuid_cmd.len(), 24);

    let sizeofcmds = (seg.len() + uuid_cmd.len()) as u32;

    let mut buf = Vec::new();
    buf.extend_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]); // MH_MAGIC_64 (LE)
    buf.extend_from_slice(&0x0100_000cu32.to_le_bytes()); // cputype (arm64)
    buf.extend_from_slice(&0u32.to_le_bytes()); // cpusubtype
    buf.extend_from_slice(&2u32.to_le_bytes()); // filetype = MH_EXECUTE
    buf.extend_from_slice(&2u32.to_le_bytes()); // ncmds
    buf.extend_from_slice(&sizeofcmds.to_le_bytes()); // sizeofcmds
    buf.extend_from_slice(&0u32.to_le_bytes()); // flags
    buf.extend_from_slice(&0u32.to_le_bytes()); // reserved
    buf.extend_from_slice(&seg);
    buf.extend_from_slice(&uuid_cmd);
    buf
}

#[test]
fn parses_uuid_and_text_vmaddr() {
    let uuid = [
        0x11, 0xe1, 0x34, 0x2b, 0xe9, 0x04, 0x38, 0x52, 0xa4, 0x33, 0x14, 0x27, 0x45, 0x90, 0xec,
        0xac,
    ];
    let buf = build_macho(uuid, 0x1_0000_0000);
    let (debug_id, text_vmaddr) = parse_macho_image_uuid_and_text_vmaddr(&buf).unwrap();
    assert_eq!(debug_id, "11e1342b-e904-3852-a433-14274590ecac");
    assert_eq!(text_vmaddr, 0x1_0000_0000);
}

#[test]
fn rejects_non_macho_buffer() {
    assert!(parse_macho_image_uuid_and_text_vmaddr(b"not a mach-o binary at all!!").is_none());
}

#[test]
fn rejects_truncated_load_commands() {
    let buf = build_macho([0u8; 16], 0x1_0000_0000);
    // Chop off part of the trailing LC_UUID command so it runs past the end.
    let truncated = &buf[..buf.len() - 4];
    assert!(parse_macho_image_uuid_and_text_vmaddr(truncated).is_none());
}
