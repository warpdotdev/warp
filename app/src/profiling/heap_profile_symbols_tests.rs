use super::*;

/// Builds a minimal gzipped pprof profile containing one sample per entry in
/// `stacks`. Each stack is a leaf-first list of function names; a fresh
/// `Location`/`Function` pair is created for every frame (no de-duplication,
/// which keeps the test setup simple and is representative of the worst
/// case for this logic). Every sample carries a single value of `1`.
fn build_profile(stacks: &[&[&str]]) -> Vec<u8> {
    let mut string_table = vec![String::new()];
    let mut functions = Vec::new();
    let mut locations = Vec::new();
    let mut samples = Vec::new();
    let mut next_id = 1u64;

    for stack in stacks {
        let mut location_id = Vec::new();
        for &name in stack.iter() {
            let name_index = string_table.len() as i64;
            string_table.push(name.to_string());

            let id = next_id;
            next_id += 1;
            functions.push(Function {
                id,
                name: name_index,
                system_name: 0,
                filename: 0,
                start_line: 0,
            });
            locations.push(Location {
                id,
                mapping_id: 0,
                address: 0,
                line: vec![Line {
                    function_id: id,
                    line: 0,
                    column: 0,
                }],
                is_folded: false,
            });
            location_id.push(id);
        }
        samples.push(Sample {
            location_id,
            value: vec![1],
            label: Vec::new(),
        });
    }

    let profile = Profile {
        sample_type: vec![ValueType { r#type: 0, unit: 0 }],
        sample: samples,
        mapping: Vec::new(),
        location: locations,
        function: functions,
        string_table,
        drop_frames: 0,
        keep_frames: 0,
        time_nanos: 0,
        duration_nanos: 0,
        period_type: None,
        period: 0,
        comment: Vec::new(),
        default_sample_type: 0,
        doc_url: 0,
    };

    let mut encoded = Vec::new();
    profile.encode(&mut encoded).unwrap();
    gzip(&encoded).unwrap()
}

/// Decodes a gzipped pprof profile and returns, for each sample, the
/// leaf-first list of resolved function names -- the same shape the test
/// inputs are constructed from -- for easy comparison.
fn decode_stacks(gzipped_profile: &[u8]) -> Vec<Vec<String>> {
    let raw = gunzip(gzipped_profile).unwrap();
    let profile = Profile::decode(raw.as_slice()).unwrap();

    profile
        .sample
        .iter()
        .map(|sample| {
            sample
                .location_id
                .iter()
                .map(|location_id| {
                    let location = profile
                        .location
                        .iter()
                        .find(|location| location.id == *location_id)
                        .unwrap();
                    let function_id = location.line[0].function_id;
                    let function = profile
                        .function
                        .iter()
                        .find(|function| function.id == function_id)
                        .unwrap();
                    profile.string_table[function.name as usize].clone()
                })
                .collect()
        })
        .collect()
}

fn decode_values(gzipped_profile: &[u8]) -> Vec<Vec<i64>> {
    let raw = gunzip(gzipped_profile).unwrap();
    let profile = Profile::decode(raw.as_slice()).unwrap();
    profile.sample.iter().map(|s| s.value.clone()).collect()
}

#[test]
fn strips_prologue_to_first_application_frame() {
    let stack: &[&str] = &[
        "_rjem_je_prof_backtrace",
        "_rjem_je_prof_tctx_create",
        "prof_alloc_prep",
        "imalloc_body",
        "imalloc",
        "_rjem_je_malloc_default",
        "app::do_work",
        "app::main",
    ];
    let profile = build_profile(&[stack]);

    let stripped = strip_allocator_prologue(&profile).unwrap();

    assert_eq!(
        decode_stacks(&stripped),
        vec![vec!["app::do_work".to_string(), "app::main".to_string()]]
    );
    // Sample values must be preserved exactly.
    assert_eq!(decode_values(&stripped), decode_values(&profile));
}

#[test]
fn strips_varying_length_prologue_due_to_inlining() {
    // Same logical prologue, but `prof_alloc_prep` is a distinct frame in
    // the first sample and inlined away (absent) in the second -- as
    // observed across the two Sentry events on this ticket. Both should
    // still end up with the same application leaf.
    let long_stack: &[&str] = &[
        "_rjem_je_prof_backtrace",
        "_rjem_je_prof_tctx_create",
        "prof_alloc_prep",
        "imalloc_body",
        "app::do_work",
    ];
    let short_stack: &[&str] = &[
        "_rjem_je_prof_backtrace",
        "_rjem_je_prof_tctx_create",
        "imalloc_body",
        "app::do_work",
    ];
    let profile = build_profile(&[long_stack, short_stack]);

    let stripped = strip_allocator_prologue(&profile).unwrap();

    let stacks = decode_stacks(&stripped);
    assert_eq!(stacks[0], vec!["app::do_work".to_string()]);
    assert_eq!(stacks[1], vec!["app::do_work".to_string()]);
}

#[test]
fn preserves_allocator_symbol_deeper_in_the_stack() {
    // An allocator-looking frame that shows up *after* application code
    // (e.g. the app itself calling into an allocator-adjacent helper deeper
    // in the stack) must never be stripped, since stripping only applies to
    // the leading run from the leaf.
    let stack: &[&str] = &[
        "_rjem_je_prof_backtrace",
        "_rjem_je_prof_tctx_create",
        "imalloc_body",
        "app::allocate_buffer",
        "prof_helper_used_by_app",
        "app::main",
    ];
    let profile = build_profile(&[stack]);

    let stripped = strip_allocator_prologue(&profile).unwrap();

    assert_eq!(
        decode_stacks(&stripped),
        vec![vec![
            "app::allocate_buffer".to_string(),
            "prof_helper_used_by_app".to_string(),
            "app::main".to_string(),
        ]]
    );
}

#[test]
fn leaves_all_allocator_stack_untouched() {
    let stack: &[&str] = &[
        "_rjem_je_prof_backtrace",
        "_rjem_je_prof_tctx_create",
        "prof_alloc_prep",
        "imalloc_body",
        "imalloc",
        "_rjem_je_malloc_default",
    ];
    let profile = build_profile(&[stack]);

    let stripped = strip_allocator_prologue(&profile).unwrap();

    // The sample must never be emptied, even though every frame matches.
    assert_eq!(
        decode_stacks(&stripped),
        vec![stack.iter().map(|s| s.to_string()).collect::<Vec<_>>()]
    );
}

#[test]
fn is_allocator_symbol_matches_documented_patterns() {
    for name in [
        "_rjem_je_prof_backtrace",
        "_rjem_je_malloc_default",
        "_rjem_realloc",
        "imalloc",
        "imalloc_body",
        "imalloc_no_sample",
        "prof_alloc_prep",
        "prof_tctx_create",
        "prof_gctx_create",
        "malloc_default",
        "calloc_default",
        "realloc_default",
        "posix_memalign_default",
    ] {
        assert!(is_allocator_symbol(name), "expected {name} to match");
    }

    for name in ["app::do_work", "main", "malloc", "std::vec::Vec::push"] {
        assert!(!is_allocator_symbol(name), "expected {name} to not match");
    }
}

// The tests above build their input profiles with the very `Profile`/
// `Sample`/etc. mirror this module defines, so a bug in that mirror (a
// dropped or mis-tagged field, say) could shape the fixture and the code
// under test identically and never show up as a test failure. The
// `RawMessage` writer and `parse_fields` reader below are a second,
// independent implementation of the protobuf wire format -- built only from
// the varint/tag/length-delimited rules themselves, with no reference to
// this module's types -- used solely to build a fixture and check the
// output byte-for-byte without going anywhere near the mirror being tested.

/// A minimal protobuf message writer, independent of `prost` and of this
/// module's `Profile`/`Sample`/etc. types.
struct RawMessage(Vec<u8>);

impl RawMessage {
    fn new() -> Self {
        Self(Vec::new())
    }

    fn write_varint(&mut self, mut value: u64) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                self.0.push(byte);
                return;
            }
            self.0.push(byte | 0x80);
        }
    }

    fn write_tag(&mut self, field_number: u32, wire_type: u8) {
        self.write_varint((u64::from(field_number) << 3) | u64::from(wire_type));
    }

    fn varint_field(&mut self, field_number: u32, value: i64) {
        self.write_tag(field_number, 0);
        self.write_varint(value as u64);
    }

    fn bool_field(&mut self, field_number: u32, value: bool) {
        self.varint_field(field_number, i64::from(value));
    }

    fn bytes_field(&mut self, field_number: u32, bytes: &[u8]) {
        self.write_tag(field_number, 2);
        self.write_varint(bytes.len() as u64);
        self.0.extend_from_slice(bytes);
    }

    fn string_field(&mut self, field_number: u32, value: &str) {
        self.bytes_field(field_number, value.as_bytes());
    }

    fn message_field(&mut self, field_number: u32, message: &RawMessage) {
        self.bytes_field(field_number, &message.0);
    }

    fn packed_varint_field(&mut self, field_number: u32, values: &[i64]) {
        let mut packed = RawMessage::new();
        for &v in values {
            packed.write_varint(v as u64);
        }
        self.bytes_field(field_number, &packed.0);
    }

    fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq)]
enum RawField {
    Varint(u64),
    LengthDelimited(Vec<u8>),
}

fn read_varint(bytes: &mut &[u8]) -> u64 {
    let mut result = 0u64;
    let mut shift = 0;
    loop {
        let byte = bytes[0];
        *bytes = &bytes[1..];
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return result;
        }
        shift += 7;
    }
}

/// Parses a message into its top-level `(field_number, value)` pairs, in
/// wire order, with no knowledge of the pprof schema.
fn parse_fields(mut bytes: &[u8]) -> Vec<(u32, RawField)> {
    let mut fields = Vec::new();
    while !bytes.is_empty() {
        let key = read_varint(&mut bytes);
        let field_number = (key >> 3) as u32;
        let wire_type = (key & 0x7) as u8;
        let value = match wire_type {
            0 => RawField::Varint(read_varint(&mut bytes)),
            2 => {
                let len = read_varint(&mut bytes) as usize;
                let (payload, rest) = bytes.split_at(len);
                bytes = rest;
                RawField::LengthDelimited(payload.to_vec())
            }
            other => panic!("unexpected wire type {other} in test fixture"),
        };
        fields.push((field_number, value));
    }
    fields
}

fn select(fields: &[(u32, RawField)], field_number: u32) -> Vec<&RawField> {
    fields
        .iter()
        .filter(|(n, _)| *n == field_number)
        .map(|(_, v)| v)
        .collect()
}

fn as_bytes(field: &RawField) -> &[u8] {
    match field {
        RawField::LengthDelimited(bytes) => bytes,
        RawField::Varint(_) => panic!("expected a length-delimited field"),
    }
}

fn packed_varints(field: &RawField) -> Vec<u64> {
    let mut bytes = as_bytes(field);
    let mut values = Vec::new();
    while !bytes.is_empty() {
        values.push(read_varint(&mut bytes));
    }
    values
}

fn value_type(r#type: i64, unit: i64) -> RawMessage {
    let mut m = RawMessage::new();
    m.varint_field(1, r#type);
    m.varint_field(2, unit);
    m
}

/// `str` (2) and `num_unit` (4) are left at their proto3 default (0) and so
/// correctly go unwritten, matching how `prost` omits default-valued scalar
/// fields when it re-encodes a decoded message.
fn label(key: i64, num: i64) -> RawMessage {
    let mut m = RawMessage::new();
    m.varint_field(1, key);
    m.varint_field(3, num);
    m
}

/// `column` (3) is left at 0 and so correctly goes unwritten.
fn line(function_id: i64, line_no: i64) -> RawMessage {
    let mut m = RawMessage::new();
    m.varint_field(1, function_id);
    m.varint_field(2, line_no);
    m
}

/// `is_folded` (5) is left at false and so correctly goes unwritten.
fn location(id: i64, mapping_id: i64, address: i64, line_msg: &RawMessage) -> RawMessage {
    let mut m = RawMessage::new();
    m.varint_field(1, id);
    m.varint_field(2, mapping_id);
    m.varint_field(3, address);
    m.message_field(4, line_msg);
    m
}

/// `filename` (4) is left at 0; `start_line` (5) is only written when
/// non-default, matching `prost`'s canonical encoding.
fn function(id: i64, name_index: i64, start_line: i64) -> RawMessage {
    let mut m = RawMessage::new();
    m.varint_field(1, id);
    m.varint_field(2, name_index);
    m.varint_field(3, name_index); // system_name
    if start_line != 0 {
        m.varint_field(5, start_line);
    }
    m
}

/// `file_offset` (4) is left at 0 and `has_inline_frames` (10) at false, so
/// both correctly go unwritten.
fn mapping(
    id: i64,
    memory_start: i64,
    memory_limit: i64,
    filename: i64,
    build_id: i64,
) -> RawMessage {
    let mut m = RawMessage::new();
    m.varint_field(1, id);
    m.varint_field(2, memory_start);
    m.varint_field(3, memory_limit);
    m.varint_field(5, filename);
    m.varint_field(6, build_id);
    m.bool_field(7, true);
    m.bool_field(8, true);
    m.bool_field(9, true);
    m
}

/// Builds a realistic, fully-populated pprof profile byte-for-byte by hand,
/// independent of the `Profile`/`Sample`/etc. mirror under test: a `Mapping`
/// with a build id, two `sample_type` entries, non-default top-level scalar
/// metadata (`drop_frames`/`keep_frames`/`time_nanos`/`duration_nanos`/
/// `period`/`period_type`/`comment`/`default_sample_type`/`doc_url`), and one
/// sample carrying a numeric label -- everything the filtering-focused tests
/// above leave unexercised. The sample's stack is the same
/// allocator-prologue-then-application shape used elsewhere in this file,
/// with the real `prof_sys.c`/`prof.c` line numbers from the ticket.
fn build_independent_fixture() -> Vec<u8> {
    const STRINGS: [&str; 16] = [
        "",                                                             // 0
        "allocations",                                                  // 1
        "count",                                                        // 2
        "space",                                                        // 3
        "bytes",                                                        // 4
        "_rjem_je_prof_backtrace",                                      // 5
        "_rjem_je_prof_tctx_create",                                    // 6
        "prof_alloc_prep",                                              // 7
        "imalloc_body",                                                 // 8
        "app::do_work",                                                 // 9
        "app::main",                                                    // 10
        "/bin/warp",                                                    // 11
        "deadbeefcafef00d1234567890abcde",                              // 12 (build id)
        "https://example.com/warp-heap-profile-fixture",                // 13 (doc url)
        "thread_id",                                                    // 14 (label key)
        "independent fixture for heap_profile_symbols round-trip test", // 15 (comment)
    ];

    let mut profile = RawMessage::new();

    // 1: sample_type
    profile.message_field(1, &value_type(1, 2)); // (allocations, count)
    profile.message_field(1, &value_type(3, 4)); // (space, bytes)

    // 2: sample -- leaf-first prologue (locations 1-4) then application
    // frames (locations 5-6), matching the shape used in the tests above.
    let mut sample = RawMessage::new();
    sample.packed_varint_field(1, &[1, 2, 3, 4, 5, 6]);
    sample.packed_varint_field(2, &[1, 2_097_152]);
    sample.message_field(3, &label(14, 7)); // thread_id = 7
    profile.message_field(2, &sample);

    // 3: mapping
    profile.message_field(3, &mapping(1, 0x1_0000_0000, 0x1_0010_0000, 11, 12));

    // 4: location
    let lines = [
        line(1, 530), // _rjem_je_prof_backtrace @ prof_sys.c:530
        line(2, 232), // _rjem_je_prof_tctx_create @ prof.c:232
        line(3, 171), // prof_alloc_prep @ prof_inlines.h:171
        line(4, 1),   // imalloc_body
        line(5, 10),  // app::do_work
        line(6, 1),   // app::main
    ];
    for (index, line_msg) in lines.iter().enumerate() {
        let location_id = (index + 1) as i64;
        profile.message_field(
            4,
            &location(location_id, 1, 0x1_0000_1000 + location_id, line_msg),
        );
    }

    // 5: function
    let functions = [
        (1, 5, 0),
        (2, 6, 0),
        (3, 7, 0),
        (4, 8, 0),
        (5, 9, 42),
        (6, 10, 7),
    ];
    for (id, name_index, start_line) in functions {
        profile.message_field(5, &function(id, name_index, start_line));
    }

    // 6: string_table
    for s in STRINGS {
        profile.string_field(6, s);
    }

    // 7-10: scalar metadata.
    profile.varint_field(7, 42); // drop_frames
    profile.varint_field(8, 43); // keep_frames
    profile.varint_field(9, 1_700_000_000_000_000_000); // time_nanos
    profile.varint_field(10, 60_000_000_000); // duration_nanos

    // 11: period_type
    profile.message_field(11, &value_type(3, 4)); // (space, bytes)

    // 12-15: remaining scalar metadata.
    profile.varint_field(12, 2_097_152); // period
    profile.packed_varint_field(13, &[15]); // comment
    profile.varint_field(14, 3); // default_sample_type
    profile.varint_field(15, 13); // doc_url

    profile.into_bytes()
}

#[test]
fn round_trip_preserves_every_field_except_stripped_location_ids() {
    let fixture = build_independent_fixture();
    let gzipped = gzip(&fixture).unwrap();

    let stripped_gzipped = strip_allocator_prologue(&gzipped).unwrap();
    assert_eq!(
        &stripped_gzipped[0..2],
        &[0x1f, 0x8b],
        "output must still be gzip-compressed"
    );

    let stripped = gunzip(&stripped_gzipped).unwrap();

    let input_fields = parse_fields(&fixture);
    let output_fields = parse_fields(&stripped);

    // Every top-level field except `sample` (2) must be byte-for-byte
    // identical: the filter must never touch mappings, locations,
    // functions, the string table, or any scalar profile metadata.
    for field_number in [1u32, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15] {
        assert_eq!(
            select(&input_fields, field_number),
            select(&output_fields, field_number),
            "field {field_number} must be preserved exactly"
        );
    }

    let input_samples = select(&input_fields, 2);
    let output_samples = select(&output_fields, 2);
    assert_eq!(input_samples.len(), 1);
    assert_eq!(output_samples.len(), 1);

    let input_sample_fields = parse_fields(as_bytes(input_samples[0]));
    let output_sample_fields = parse_fields(as_bytes(output_samples[0]));

    // Sample values and labels are untouched.
    assert_eq!(
        select(&input_sample_fields, 2),
        select(&output_sample_fields, 2),
        "sample values must be preserved exactly"
    );
    assert_eq!(
        select(&input_sample_fields, 3),
        select(&output_sample_fields, 3),
        "sample labels must be preserved exactly"
    );

    // Only the leading allocator run of location ids is stripped.
    let input_location_ids = packed_varints(select(&input_sample_fields, 1)[0]);
    let output_location_ids = packed_varints(select(&output_sample_fields, 1)[0]);
    assert_eq!(input_location_ids, vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(output_location_ids, vec![5, 6]);
}
