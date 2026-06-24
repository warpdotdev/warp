use super::{number_to_alphabet, number_to_roman, ListNumbering};
use crate::elements::OrderedListLabel;

#[test]
fn test_entirely_automatic() {
    let mut numbering = ListNumbering::new();
    // Start at level 0.
    assert_eq!(
        numbering.advance(0, None),
        OrderedListLabel {
            label_index: 1,
            display_label: "1".to_owned()
        }
    );
    assert_eq!(
        numbering.advance(0, None),
        OrderedListLabel {
            label_index: 2,
            display_label: "2".to_owned()
        }
    );
    // Indent, which should start over again at 1.
    assert_eq!(
        numbering.advance(1, None),
        OrderedListLabel {
            label_index: 1,
            display_label: "a".to_owned()
        }
    );
    assert_eq!(
        numbering.advance(1, None),
        OrderedListLabel {
            label_index: 2,
            display_label: "b".to_owned()
        }
    );
    // Un-indent, which should resume at 3.
    assert_eq!(
        numbering.advance(0, None),
        OrderedListLabel {
            label_index: 3,
            display_label: "3".to_owned()
        }
    );
    // Re-indent, which should restart at 1.
    assert_eq!(
        numbering.advance(1, None),
        OrderedListLabel {
            label_index: 1,
            display_label: "a".to_owned()
        }
    );
}

#[test]
fn test_indent_jump() {
    // Ensure that we can skip indent levels.
    let mut numbering = ListNumbering::new();
    assert_eq!(
        numbering.advance(0, None),
        OrderedListLabel {
            label_index: 1,
            display_label: "1".to_owned()
        }
    );
    // Skip multiple levels of indentation.
    assert_eq!(
        numbering.advance(4, None),
        OrderedListLabel {
            label_index: 1,
            display_label: "a".to_owned()
        }
    );
    assert_eq!(
        numbering.advance(4, None),
        OrderedListLabel {
            label_index: 2,
            display_label: "b".to_owned()
        }
    );
    // Skip multiple levels un-indenting.
    assert_eq!(
        numbering.advance(2, None),
        OrderedListLabel {
            label_index: 1,
            display_label: "i".to_owned()
        }
    );
    assert_eq!(
        numbering.advance(1, None),
        OrderedListLabel {
            label_index: 1,
            display_label: "a".to_owned()
        }
    );
    assert_eq!(
        numbering.advance(0, None),
        OrderedListLabel {
            label_index: 2,
            display_label: "2".to_owned()
        }
    );
}

#[test]
fn test_assigned_numbers() {
    let mut numbering = ListNumbering::new();
    assert_eq!(
        numbering.advance(0, Some(4)),
        OrderedListLabel {
            label_index: 4,
            display_label: "4".to_owned()
        }
    );
    assert_eq!(
        numbering.advance(0, None),
        OrderedListLabel {
            label_index: 5,
            display_label: "5".to_owned()
        }
    );
    assert_eq!(
        numbering.advance(0, None),
        OrderedListLabel {
            label_index: 6,
            display_label: "6".to_owned()
        }
    );
    // Assigned numbers not at the start should be ignored.
    assert_eq!(
        numbering.advance(0, Some(1)),
        OrderedListLabel {
            label_index: 7,
            display_label: "7".to_owned()
        }
    );
    // Assigned numbers at a new indent level are respected.
    assert_eq!(
        numbering.advance(1, Some(3)),
        OrderedListLabel {
            label_index: 3,
            display_label: "c".to_owned()
        }
    );
    assert_eq!(
        numbering.advance(1, None),
        OrderedListLabel {
            label_index: 4,
            display_label: "d".to_owned()
        }
    );
    // The custom start number shouldn't be lost when un-indenting.
    assert_eq!(
        numbering.advance(0, Some(2)),
        OrderedListLabel {
            label_index: 8,
            display_label: "8".to_owned()
        }
    );
}

#[test]
fn test_reset() {
    let mut numbering = ListNumbering::new();
    assert_eq!(
        numbering.advance(0, None),
        OrderedListLabel {
            label_index: 1,
            display_label: "1".to_owned()
        }
    );
    assert_eq!(
        numbering.advance(1, None),
        OrderedListLabel {
            label_index: 1,
            display_label: "a".to_owned()
        }
    );
    numbering.reset();
    // After a reset, all levels should be 1.
    assert_eq!(
        numbering.advance(1, None),
        OrderedListLabel {
            label_index: 1,
            display_label: "a".to_owned()
        }
    );
    assert_eq!(
        numbering.advance(0, None),
        OrderedListLabel {
            label_index: 1,
            display_label: "1".to_owned()
        }
    );

    // This assigned number is ignored, since it's not at the start of a list.
    assert_eq!(
        numbering.advance(0, Some(5)),
        OrderedListLabel {
            label_index: 2,
            display_label: "2".to_owned()
        }
    );

    numbering.reset();
    // This assigned number is kept because of the reset.
    assert_eq!(
        numbering.advance(0, Some(5)),
        OrderedListLabel {
            label_index: 5,
            display_label: "5".to_owned()
        }
    );
}

#[test]
fn test_number_to_roman() {
    assert_eq!(number_to_roman(0), "i");
    assert_eq!(number_to_roman(5), "vi");
    assert_eq!(number_to_roman(29), "xxx");
    assert_eq!(number_to_roman(30), "i");
    assert_eq!(number_to_roman(61), "ii");
}

#[test]
fn test_number_to_alphabet() {
    assert_eq!(number_to_alphabet(0), "a");
    assert_eq!(number_to_alphabet(25), "z");
    assert_eq!(number_to_alphabet(26), "aa");
    assert_eq!(number_to_alphabet(27), "bb");
    assert_eq!(number_to_alphabet(77), "zzz");
    assert_eq!(number_to_alphabet(78), "a");
}
