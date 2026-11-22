use notify_rust::Hint;

use super::{NOTIFICATION_SOUND_NAME, build_notification};
use crate::notification::UserNotification;

const DESKTOP_ENTRY: &str = "dev.warp.Warp";

fn user_notification(play_sound: bool) -> UserNotification {
    UserNotification::new_with_sound("Title".to_owned(), "Body".to_owned(), None, play_sound)
}

#[test]
fn requests_sound_when_play_sound_is_enabled() {
    let notification = build_notification(&user_notification(true), Some(DESKTOP_ENTRY));

    assert!(
        notification
            .hints
            .contains(&Hint::SoundName(NOTIFICATION_SOUND_NAME.to_owned()))
    );
    assert!(!notification.hints.contains(&Hint::SuppressSound(true)));
}

#[test]
fn suppresses_sound_when_play_sound_is_disabled() {
    let notification = build_notification(&user_notification(false), Some(DESKTOP_ENTRY));

    assert!(notification.hints.contains(&Hint::SuppressSound(true)));
    assert!(
        !notification
            .hints
            .iter()
            .any(|hint| matches!(hint, Hint::SoundName(_)))
    );
}

#[test]
fn sets_desktop_entry_when_known() {
    let notification = build_notification(&user_notification(true), Some(DESKTOP_ENTRY));

    assert!(
        notification
            .hints
            .contains(&Hint::DesktopEntry(DESKTOP_ENTRY.to_owned()))
    );
}

#[test]
fn omits_desktop_entry_when_unknown() {
    let notification = build_notification(&user_notification(true), None);

    assert!(
        !notification
            .hints
            .iter()
            .any(|hint| matches!(hint, Hint::DesktopEntry(_)))
    );
}

#[test]
fn keeps_title_and_body() {
    let notification = build_notification(&user_notification(true), None);

    assert_eq!(notification.summary, "Title");
    assert_eq!(notification.body, "Body");
}
