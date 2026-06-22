use super::Priority;

#[test]
fn test_priority_normalization() {
    let too_small = Priority::new(-201);
    assert_eq!(Priority::min(), too_small);

    let too_large = Priority::new(201);
    assert_eq!(Priority::max(), too_large);

    let fourty_two = Priority::new(42);
    assert_eq!(42, fourty_two.0);
}

#[test]
fn test_priority_comparison() {
    let super_important = Priority::new(200);
    let important = Priority::new(40);
    let not_important = Priority::new(-80);

    assert!(super_important == super_important);
    assert!(super_important > important);
    assert!(super_important > not_important);

    assert!(important == important);
    assert!(important > not_important);

    assert!(not_important == not_important);
}
