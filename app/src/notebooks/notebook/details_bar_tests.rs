use warpui::{App, SingletonEntity};

use super::editor_display_name;
use crate::auth::UserUid;
use crate::workspaces::user_profiles::{UserProfileWithUID, UserProfiles};

fn initialize_app(app: &mut App) {
    app.update(crate::settings::init_and_register_user_preferences);
    app.add_singleton_model(|_| UserProfiles::new(vec![]));
}

#[test]
fn test_editor_display_name() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        UserProfiles::handle(&app).update(&mut app, |profiles, _ctx| {
            profiles.insert_profiles(&vec![
                UserProfileWithUID {
                    firebase_uid: UserUid::new("abc123"),
                    display_name: Some("The Editor".to_string()),
                    email: "editor@warp.dev".to_string(),
                    photo_url: "http://example.com/profile.jpg".to_string(),
                },
                UserProfileWithUID {
                    firebase_uid: UserUid::new("def456"),
                    display_name: None,
                    email: "anon@warp.dev".to_string(),
                    photo_url: "http://example.com/profile.jpg".to_string(),
                },
            ])
        });

        app.read(|ctx| {
            // If there's no known editor, default to "Other user";
            assert_eq!(&editor_display_name(None, ctx), "Other user");

            // If the editor doesn't have a profile, default to their email.
            assert_eq!(
                &editor_display_name(Some("unknown@warp.dev"), ctx),
                "unknown@warp.dev"
            );

            // If the profile is missing a display name, default to the email.
            assert_eq!(
                &editor_display_name(Some("anon@warp.dev"), ctx),
                "anon@warp.dev"
            );

            // If there's a display name available, use that.
            assert_eq!(
                &editor_display_name(Some("editor@warp.dev"), ctx),
                "The Editor"
            );
        });
    })
}
