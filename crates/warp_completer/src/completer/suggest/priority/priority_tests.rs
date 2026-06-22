use super::{Priority, MAX_PRIORITY, MIN_PRIORITY};

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
    let super_important = Priority::new(100);
    let important = Priority::new(20);
    let not_important = Priority::new(-60);
    let default = Priority::default();

    assert!(super_important == super_important);
    assert!(super_important > important);
    assert!(super_important > default);
    assert!(super_important > not_important);

    assert!(important < super_important);
    assert!(important == important);
    assert!(important > default);
    assert!(important > not_important);

    assert!(not_important < super_important);
    assert!(not_important < important);
    assert!(not_important < default);
    assert!(not_important == not_important);

    assert!(default == default);
}

/// Test that we can correctly convert from the new  Priority to the original as defined in
/// `warp_command_signatures`.
#[test]
fn test_new_to_old_priority() {
    use warp_command_signatures::{Importance, Order, Priority as OldPriority};
    assert_eq!(
        OldPriority::from(Priority::new(MIN_PRIORITY)),
        OldPriority::Global(Importance::Less(Order(1))),
    );

    assert_eq!(
        OldPriority::from(Priority::new(-51)),
        OldPriority::Global(Importance::Less(Order(50))),
    );

    assert_eq!(
        OldPriority::from(Priority::new(-1)),
        OldPriority::Global(Importance::Less(Order(100)))
    );

    assert_eq!(
        OldPriority::from(Priority::default()),
        OldPriority::default()
    );

    assert_eq!(
        OldPriority::from(Priority::new(1)),
        OldPriority::Global(Importance::More(Order(1))),
    );

    assert_eq!(
        OldPriority::from(Priority::new(50)),
        OldPriority::Global(Importance::More(Order(50))),
    );

    assert_eq!(
        OldPriority::from(Priority::new(MAX_PRIORITY)),
        OldPriority::Global(Importance::More(Order(100))),
    );
}

/// Test that we can correctly convert from the old Priority as definined in
/// `warp_command_signatures` to the new Priority.
#[test]
fn test_old_to_new_priority() {
    use warp_command_signatures::{Importance, Order, Priority as OldPriority};

    assert_eq!(
        Priority::from(OldPriority::Global(Importance::Less(Order(1)))),
        Priority::new(MIN_PRIORITY)
    );

    assert_eq!(
        Priority::from(OldPriority::Global(Importance::Less(Order(50)))),
        Priority::new(-51)
    );

    assert_eq!(
        Priority::from(OldPriority::Global(Importance::Less(Order(100)))),
        Priority::new(-1)
    );

    assert_eq!(Priority::from(OldPriority::Default), Priority::default());

    assert_eq!(
        Priority::from(OldPriority::Global(Importance::More(Order(1)))),
        Priority::new(1)
    );

    assert_eq!(
        Priority::from(OldPriority::Global(Importance::More(Order(50)))),
        Priority::new(50)
    );

    assert_eq!(
        Priority::from(OldPriority::Global(Importance::More(Order(100)))),
        Priority::new(MAX_PRIORITY)
    );
}
