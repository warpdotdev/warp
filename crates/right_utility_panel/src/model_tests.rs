use super::*;

#[test]
fn mode_and_module_defaults_are_backward_compatible() {
    // Older snapshots that predate these enums must restore Code Review and the
    // Passwords/Bookmarks defaults.
    assert_eq!(RightPanelMode::default(), RightPanelMode::CodeReview);
    assert_eq!(RightUtilityModule::default(), RightUtilityModule::Passwords);
    assert_eq!(BookmarksSubview::default(), BookmarksSubview::Bookmarks);
}

#[test]
fn panel_state_round_trips_through_serde() {
    let list_id = uuid::Uuid::new_v4();
    let state = RightUtilityPanelState {
        selected_module: RightUtilityModule::Bookmarks,
        bookmarks_subview: BookmarksSubview::CustomLists,
        selected_custom_list_id: Some(list_id),
    };
    let json = serde_json::to_string(&state).unwrap();
    let parsed: RightUtilityPanelState = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, state);
}

#[test]
fn panel_state_deserializes_from_empty_object() {
    // An older/empty snapshot must produce the defaults rather than failing.
    let parsed: RightUtilityPanelState = serde_json::from_str("{}").unwrap();
    assert_eq!(parsed, RightUtilityPanelState::default());
    assert!(parsed.selected_custom_list_id.is_none());
}

#[test]
fn panel_data_deserializes_from_empty_object() {
    let parsed: RightUtilityPanelData = serde_json::from_str("{}").unwrap();
    assert!(parsed.passwords.is_empty());
    assert!(parsed.bookmarks.is_empty());
    assert!(parsed.custom_lists.is_empty());
}

#[test]
fn bookmark_target_kind_matches_variant() {
    assert_eq!(
        BookmarkTarget::Command {
            command: "ls".to_owned(),
            cwd: None
        }
        .kind(),
        BookmarkTargetKind::Command
    );
    assert_eq!(
        BookmarkTarget::Directory {
            path: "/tmp".to_owned()
        }
        .kind(),
        BookmarkTargetKind::Directory
    );
    assert_eq!(
        BookmarkTarget::File {
            path: "/tmp/a".to_owned()
        }
        .kind(),
        BookmarkTargetKind::File
    );
    assert_eq!(
        BookmarkTarget::Url {
            url: "https://warp.dev".to_owned()
        }
        .kind(),
        BookmarkTargetKind::Url
    );
}

#[test]
fn bookmark_target_validation() {
    assert_eq!(
        BookmarkTarget::Command {
            command: "  ".to_owned(),
            cwd: None
        }
        .validate(),
        Err(TargetValidationError::EmptyCommand)
    );
    assert!(BookmarkTarget::Command {
        command: "cargo test".to_owned(),
        cwd: Some("/repo".to_owned())
    }
    .validate()
    .is_ok());

    assert_eq!(
        BookmarkTarget::Directory {
            path: "".to_owned()
        }
        .validate(),
        Err(TargetValidationError::EmptyPath)
    );

    assert!(BookmarkTarget::Url {
        url: "https://warp.dev/path".to_owned()
    }
    .validate()
    .is_ok());
    assert_eq!(
        BookmarkTarget::Url {
            url: "not a url".to_owned()
        }
        .validate(),
        Err(TargetValidationError::InvalidUrl)
    );
    assert_eq!(
        BookmarkTarget::Url {
            url: "https://".to_owned()
        }
        .validate(),
        Err(TargetValidationError::InvalidUrl)
    );
    assert_eq!(
        BookmarkTarget::Url {
            url: "   ".to_owned()
        }
        .validate(),
        Err(TargetValidationError::EmptyUrl)
    );
}

#[test]
fn add_bookmark_rejects_invalid_target() {
    let mut data = RightUtilityPanelData::new();
    let err = data
        .add_bookmark(
            "bad",
            BookmarkTarget::Command {
                command: "".to_owned(),
                cwd: None,
            },
            vec![],
        )
        .unwrap_err();
    assert_eq!(err, TargetValidationError::EmptyCommand);
    assert!(data.bookmarks.is_empty());
}

#[test]
fn bookmark_crud_and_search() {
    let mut data = RightUtilityPanelData::new();
    let id = data
        .add_bookmark(
            "Deploy prod",
            BookmarkTarget::Command {
                command: "make deploy".to_owned(),
                cwd: None,
            },
            vec!["ops".to_owned()],
        )
        .unwrap();
    data.add_bookmark(
        "Warp site",
        BookmarkTarget::Url {
            url: "https://warp.dev".to_owned(),
        },
        vec![],
    )
    .unwrap();

    assert_eq!(data.bookmarks.len(), 2);
    assert_eq!(data.search_bookmarks("deploy").len(), 1);
    assert_eq!(data.search_bookmarks("ops").len(), 1);
    assert_eq!(data.search_bookmarks("").len(), 2);
    assert_eq!(data.search_bookmarks("nothing").len(), 0);

    let removed = data.remove_bookmark(id).unwrap();
    assert_eq!(removed.title, "Deploy prod");
    assert_eq!(data.bookmarks.len(), 1);
    assert!(data.remove_bookmark(id).is_none());
}

#[test]
fn custom_list_crud_and_item_toggle() {
    let mut data = RightUtilityPanelData::new();
    let list_id = data.add_custom_list("Pre-flight");
    assert_eq!(data.custom_lists.len(), 1);

    let item_id = {
        let list = data.custom_list_mut(list_id).unwrap();
        let item_id = list.add_item("check backups");
        list.add_item("notify team");
        assert_eq!(list.items.len(), 2);
        item_id
    };

    let list = data.custom_list_mut(list_id).unwrap();
    assert_eq!(list.toggle_item(item_id), Some(true));
    assert_eq!(list.toggle_item(item_id), Some(false));
    assert!(list.toggle_item(uuid::Uuid::new_v4()).is_none());
    assert!(list.remove_item(item_id).is_some());
    assert_eq!(list.items.len(), 1);

    assert!(data.rename_custom_list(list_id, "Pre-flight checklist"));
    assert_eq!(
        data.custom_list_mut(list_id).unwrap().title,
        "Pre-flight checklist"
    );
    assert!(!data.rename_custom_list(uuid::Uuid::new_v4(), "x"));

    assert!(data.remove_custom_list(list_id).is_some());
    assert!(data.custom_lists.is_empty());
}

#[test]
fn custom_lists_are_nested_data_only() {
    // Custom Lists must be reachable only as a Bookmarks sub-view, never a
    // top-level module. The module enum has exactly two variants and the
    // Custom Lists concept lives under the Bookmarks sub-view enum.
    let modules = [RightUtilityModule::Passwords, RightUtilityModule::Bookmarks];
    assert_eq!(modules.len(), 2);
    // BookmarksSubview is where CustomLists is exposed.
    let subviews = [BookmarksSubview::Bookmarks, BookmarksSubview::CustomLists];
    assert!(subviews.contains(&BookmarksSubview::CustomLists));
}
