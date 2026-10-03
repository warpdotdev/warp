use std::io;

use super::NextInstance;

#[test]
fn take_or_create_returns_the_existing_value_without_calling_create() {
    let next = NextInstance::new(1);

    let value = next
        .take_or_create(|| panic!("create should not be called when a value is present"))
        .unwrap();

    assert_eq!(value, 1);
}

#[test]
fn restore_makes_the_value_available_to_the_next_take_or_create() {
    let next = NextInstance::new(1);
    let _ = next.take_or_create(|| unreachable!()).unwrap();

    next.restore(Some(2));

    let value = next.take_or_create(|| unreachable!()).unwrap();
    assert_eq!(value, 2);
}

/// Regression test: a previous accept attempt (see `crate::native`'s Windows listener) can fail
/// after taking the current instance and before a replacement is created, leaving the slot
/// empty via `restore(None)`. The next call must recover by creating a new instance instead of
/// panicking on an assumption that the slot is always populated.
#[test]
fn take_or_create_recovers_after_a_previous_call_left_the_slot_empty() {
    let next = NextInstance::new(1);
    let _ = next.take_or_create(|| unreachable!()).unwrap();

    // Simulates the failure path: creating a replacement failed, so the slot is left empty.
    next.restore(None);

    let value = next.take_or_create(|| Ok(2)).unwrap();
    assert_eq!(value, 2);
}

#[test]
fn take_or_create_propagates_the_create_error_and_leaves_the_slot_recoverable() {
    let next: NextInstance<i32> = NextInstance::new(1);
    let _ = next.take_or_create(|| unreachable!()).unwrap();
    next.restore(None);

    let err = next
        .take_or_create(|| Err(io::Error::other("boom")))
        .unwrap_err();
    assert_eq!(err.to_string(), "boom");

    // A later call still recovers; the earlier failure didn't leave the slot in a bad state.
    let value = next.take_or_create(|| Ok(2)).unwrap();
    assert_eq!(value, 2);
}
