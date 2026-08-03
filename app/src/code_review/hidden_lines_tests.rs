use super::*;

#[test]
fn test_empty_diffs() {
    let hidden_lines = calculate_hidden_lines(&[], 20, &[]);

    let expected_hidden = [LineCount::from(0)..LineCount::from(20)]
        .into_iter()
        .collect::<RangeSet<LineCount>>();

    assert_eq!(hidden_lines, expected_hidden);
}

#[test]
fn test_single_diff_middle_of_file() {
    // File with 20 lines, change at lines 10-12 (1-indexed)
    let diffs = vec![DiffDelta {
        replacement_line_range: 10..13, // 1-indexed, replacing lines 10, 11, 12
        insertion: "new line 1\nnew line 2\nnew line 3".to_string(), // 3 lines
    }];

    let hidden_lines = calculate_hidden_lines(&diffs, 20, &[]);

    let mut expected_hidden = RangeSet::new();
    expected_hidden.insert(0.into()..5.into()); // lines 1-5 (1-indexed)
    expected_hidden.insert(16.into()..20.into()); // lines 17-20 (1-indexed)

    assert_eq!(hidden_lines, expected_hidden);
}

#[test]
fn test_insertion_at_beginning() {
    // Insert at the very beginning of file
    let diffs = vec![DiffDelta {
        replacement_line_range: 1..1,                    // Insert at beginning
        insertion: "new line 1\nnew line 2".to_string(), // 2 lines
    }];

    let hidden_lines = calculate_hidden_lines(&diffs, 20, &[]);

    let mut expected_hidden = RangeSet::new();
    expected_hidden.insert(4.into()..20.into());

    assert_eq!(hidden_lines, expected_hidden);
}

#[test]
fn test_multiple_diffs_overlapping_context() {
    // Two changes close enough that their context overlaps
    let diffs = vec![
        DiffDelta {
            replacement_line_range: 5..6,      // 1-indexed, replace line 5
            insertion: "change 1".to_string(), // 1 line
        },
        DiffDelta {
            replacement_line_range: 8..9,      // 1-indexed, replace line 8
            insertion: "change 2".to_string(), // 1 line
        },
    ];

    let hidden_lines = calculate_hidden_lines(&diffs, 20, &[]);
    let mut expected_hidden = RangeSet::new();
    expected_hidden.insert(12.into()..20.into());

    assert_eq!(hidden_lines, expected_hidden);
}

#[test]
fn test_multiple_diffs_separate_context() {
    // Two changes far apart
    let diffs = vec![
        DiffDelta {
            replacement_line_range: 3..4,      // 1-indexed, replace line 3
            insertion: "change 1".to_string(), // 1 line
        },
        DiffDelta {
            replacement_line_range: 15..16,    // 1-indexed, replace line 15
            insertion: "change 2".to_string(), // 1 line
        },
    ];

    let hidden_lines = calculate_hidden_lines(&diffs, 20, &[]);

    let mut expected_hidden = RangeSet::new();
    expected_hidden.insert(7.into()..10.into());
    expected_hidden.insert(19.into()..20.into());

    assert_eq!(hidden_lines, expected_hidden);
}

#[test]
fn test_comments_only_no_diffs() {
    // File with 20 lines, comments at lines 5 and 15 (0-indexed)
    let comment_lines = vec![RenderLineCount::from(5), RenderLineCount::from(15)];

    let hidden_lines = calculate_hidden_lines(&[], 20, &comment_lines);

    let mut expected_hidden = RangeSet::new();
    // Comment at line 5 shows lines 1-9 (0-indexed), so hide lines 0 and 10-14
    expected_hidden.insert(0.into()..1.into());
    expected_hidden.insert(10.into()..11.into());
    // Comment at line 15 shows lines 11-19 (0-indexed), so hide line 20
    // Note: lines 10-19 are visible due to comment at line 15

    assert_eq!(hidden_lines, expected_hidden);
}

#[test]
fn test_comment_overlapping_with_diff_context() {
    // File with 20 lines, diff at lines 8-9 (1-indexed) and comment at line 10 (0-indexed)
    let diffs = vec![DiffDelta {
        replacement_line_range: 8..10, // 1-indexed, replacing lines 8, 9
        insertion: "new line 1\nnew line 2".to_string(),
    }];
    let comment_lines = vec![RenderLineCount::from(10)];

    let hidden_lines = calculate_hidden_lines(&diffs, 20, &comment_lines);

    // Diff at lines 8-9 (1-indexed) = lines 7-8 (0-indexed) with ±4 context = lines 3-12
    // Comment at line 10 (0-indexed) with ±4 context = lines 6-14
    // Combined visible range: lines 3-14
    let mut expected_hidden = RangeSet::new();
    expected_hidden.insert(0.into()..3.into());
    expected_hidden.insert(15.into()..20.into());

    assert_eq!(hidden_lines, expected_hidden);
}

#[test]
fn test_comment_separate_from_diffs() {
    // File with 30 lines, diff at lines 5-6 (1-indexed) and comment at line 20 (0-indexed)
    let diffs = vec![DiffDelta {
        replacement_line_range: 5..7, // 1-indexed, replacing lines 5, 6
        insertion: "changed line".to_string(),
    }];
    let comment_lines = vec![RenderLineCount::from(20)];

    let hidden_lines = calculate_hidden_lines(&diffs, 30, &comment_lines);

    // Diff at lines 5-6 (1-indexed) = lines 4-5 (0-indexed) with ±4 context = lines 0-9
    // Comment at line 20 (0-indexed) with ±4 context = lines 16-24
    let mut expected_hidden = RangeSet::new();
    expected_hidden.insert(10.into()..16.into());
    expected_hidden.insert(25.into()..30.into());

    assert_eq!(hidden_lines, expected_hidden);
}

#[test]
fn test_comment_outside_file_bounds() {
    // File with 10 lines, comment at line 15 (0-indexed) - outside bounds
    let comment_lines = vec![RenderLineCount::from(15)];

    let hidden_lines = calculate_hidden_lines(&[], 10, &comment_lines);

    // Comment is outside file bounds, so everything should be hidden
    let mut expected_hidden = RangeSet::new();
    expected_hidden.insert(0.into()..10.into());

    assert_eq!(hidden_lines, expected_hidden);
}

#[test]
fn test_comment_at_file_beginning() {
    // File with 20 lines, comment at line 1 (0-indexed)
    let comment_lines = vec![RenderLineCount::from(1)];

    let hidden_lines = calculate_hidden_lines(&[], 20, &comment_lines);

    // Comment at line 1 with ±4 context = lines 0-5 (saturating_sub handles negative)
    let mut expected_hidden = RangeSet::new();
    expected_hidden.insert(6.into()..20.into());

    assert_eq!(hidden_lines, expected_hidden);
}

#[test]
fn test_comment_at_file_end() {
    // File with 20 lines, comment at line 18 (0-indexed)
    let comment_lines = vec![RenderLineCount::from(18)];

    let hidden_lines = calculate_hidden_lines(&[], 20, &comment_lines);

    // Comment at line 18 with ±4 context = lines 14-19 (clamped to file bounds)
    let mut expected_hidden = RangeSet::new();
    expected_hidden.insert(0.into()..14.into());

    assert_eq!(hidden_lines, expected_hidden);
}
