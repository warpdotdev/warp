use itertools::Itertools as _;

use super::*;

type TestAttributeMap = AttributeMap<usize>;

#[test]
fn test_iterate_over_empty_map() {
    // Values:
    // * [0, ): 0
    let map = TestAttributeMap::new(0);

    assert_eq!(
        map.iter_from(ByteOffset::zero()).next(),
        Some(usize::default())
    );
    assert_eq!(
        map.iter_from(ByteOffset::from(25625)).next(),
        Some(usize::default())
    );
}

#[test]
fn test_iterate_across_attribute_change() {
    // Values:
    // * [0, 2): 0
    // * [2, ): 1
    let mut map = TestAttributeMap::new(0);
    map.push_attribute_change(ByteOffset::from(2).., 1);

    let iter = map.iter_from(ByteOffset::zero());

    assert_eq!(iter.take(4).collect_vec(), vec![0, 0, 1, 1]);
}
