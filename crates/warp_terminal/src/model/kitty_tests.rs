use super::*;

/// Parses a full kitty graphics escape body (control data, `;`, base64 payload) into an action,
/// the same way the APC handler does for a single-chunk message.
fn action_for(control_data: &str, payload: &str) -> Result<KittyAction, KittyError> {
    let chunk = parse_kitty_chunk(format!("{control_data};{payload}").into_bytes());
    let message = KittyMessage::try_from(PendingKittyMessage {
        control_data: chunk.control_data,
        payload: vec![chunk.payload],
    })
    .expect("payload should decode");

    KittyAction::try_from(message)
}

#[test]
fn store_and_display_with_unicode_placeholder_is_accepted_as_virtual() {
    let action = action_for("a=T,U=1,f=32,s=1,v=1,i=7,c=2,r=3", "AAAAAA==")
        .expect("U=1 on a=T should be accepted");

    let KittyAction::StoreAndDisplay(action) = action else {
        panic!("expected StoreAndDisplay action");
    };
    assert!(action.placement_data.virtual_placement);
    assert_eq!(action.image_id, 7);
    assert_eq!(action.placement_data.cols, Some(2));
    assert_eq!(action.placement_data.rows, Some(3));
}

#[test]
fn store_and_display_without_unicode_placeholder_is_not_virtual() {
    let action = action_for("a=T,f=32,s=1,v=1,i=7", "AAAAAA==").expect("a=T should be accepted");

    let KittyAction::StoreAndDisplay(action) = action else {
        panic!("expected StoreAndDisplay action");
    };
    assert!(!action.placement_data.virtual_placement);
}

#[test]
fn display_stored_image_with_unicode_placeholder_is_accepted_as_virtual() {
    let action = action_for("a=p,U=1,i=42,c=16,r=4", "").expect("U=1 on a=p should be accepted");

    let KittyAction::DisplayStoredImage(action) = action else {
        panic!("expected DisplayStoredImage action");
    };
    assert!(action.placement_data.virtual_placement);
    assert_eq!(action.image_id, 42);
    assert_eq!(action.placement_data.cols, Some(16));
    assert_eq!(action.placement_data.rows, Some(4));
}

#[test]
fn display_stored_image_without_unicode_placeholder_is_not_virtual() {
    let action = action_for("a=p,i=42", "").expect("a=p should be accepted");

    let KittyAction::DisplayStoredImage(action) = action else {
        panic!("expected DisplayStoredImage action");
    };
    assert!(!action.placement_data.virtual_placement);
}
