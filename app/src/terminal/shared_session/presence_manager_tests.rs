use std::collections::{HashMap, HashSet};
use std::iter;

use itertools::Itertools;
use session_sharing_protocol::common::{
    ParticipantId, ParticipantInfo, ParticipantList, ProfileData, Role, Selection, Sharer, Viewer,
};
use warp_core::command::ExitCode;
use warpui::App;

use crate::auth::UserUid;
use crate::terminal::model::ansi::{CommandFinishedValue, Handler};
use crate::terminal::model::blocks::BlockList;
use crate::terminal::model::test_utils::TestBlockListBuilder;
use crate::terminal::shared_session::presence_manager::{PresenceManager, PRESET_COLORS};

fn viewer_with_uid(uid: &str, is_present: bool) -> Viewer {
    Viewer {
        info: ParticipantInfo {
            profile_data: ProfileData {
                firebase_uid: uid.to_owned(),
                ..Default::default()
            },
            ..Default::default()
        },
        role: Role::Reader,
        is_present,
    }
}

#[test]
fn single_distinct_present_viewer_uid_filters_absent_duplicates() {
    let viewers = [
        viewer_with_uid("same", true),
        viewer_with_uid("same", true),
        viewer_with_uid("other", false),
    ];

    assert_eq!(
        PresenceManager::single_distinct_present_viewer_uid_from_viewers(viewers.iter()),
        Some("same")
    );
}

#[test]
fn single_distinct_present_viewer_uid_returns_none_for_zero_or_multiple_uids() {
    assert_eq!(
        PresenceManager::single_distinct_present_viewer_uid_from_viewers([].iter()),
        None
    );

    let viewers = [viewer_with_uid("one", true), viewer_with_uid("two", true)];
    assert_eq!(
        PresenceManager::single_distinct_present_viewer_uid_from_viewers(viewers.iter()),
        None
    );
}

#[test]
fn test_choosing_preset_colors() {
    App::test((), |mut app| async move {
        // Initialize with a sharer.
        let firebase_uid = UserUid::new("mock_firebase_uid");
        let presence_manager =
            app.add_model(|_| PresenceManager::new_for_sharer(ParticipantId::new(), firebase_uid));

        let sharer_id = ParticipantId::new();
        let sharer = Sharer {
            info: ParticipantInfo {
                id: sharer_id.clone(),
                profile_data: ProfileData {
                    ..Default::default()
                },
                ..Default::default()
            },
        };
        let mut viewers = Vec::new();
        let sharer_clone = sharer.clone();
        let viewers_clone = viewers.clone();

        presence_manager
            .update(&mut app, |presence_manager, ctx| {
                presence_manager.update_participants(
                    ParticipantList {
                        sharer: sharer_clone,
                        viewers: viewers_clone,
                        present_viewers: Default::default(),
                        absent_viewers: Default::default(),
                        guests: Default::default(),
                        pending_guests: Default::default(),
                    },
                    ctx,
                );
                let spawned_future = presence_manager
                    .load_participants_imgs_future_handle
                    .as_ref()
                    .expect("should have future handle");
                ctx.await_spawned_future(spawned_future.future_id())
            })
            .await;

        // We ourselves are the sharer, so no color is saved
        presence_manager.read(&app, |presence_manager: &PresenceManager, _ctx| {
            let sharer = presence_manager.get_sharer();
            assert!(sharer.is_none());

            let viewers = presence_manager.get_present_viewers().collect_vec();
            assert_eq!(viewers.len(), 0);
        });

        // Add new viewers one-by-one. Each new viewer should take the next preset color, while existing viewers keep their colors.
        let viewer_ids = iter::repeat_with(ParticipantId::new).take(PRESET_COLORS.len());
        let mut id_to_expected_color = HashMap::new();

        for (i, id) in viewer_ids.enumerate() {
            // Add a new viewer.
            viewers.push(Viewer {
                info: ParticipantInfo {
                    id: id.clone(),
                    ..Default::default()
                },
                role: Role::Reader,
                is_present: true,
            });
            let sharer_clone = sharer.clone();
            let viewers_clone = viewers.clone();
            presence_manager
                .update(&mut app, |presence_manager, ctx| {
                    presence_manager.update_participants(
                        ParticipantList {
                            sharer: sharer_clone,
                            viewers: viewers_clone,
                            present_viewers: Default::default(),
                            absent_viewers: Default::default(),
                            guests: Default::default(),
                            pending_guests: Default::default(),
                        },
                        ctx,
                    );
                    let spawned_future = presence_manager
                        .load_participants_imgs_future_handle
                        .as_ref()
                        .expect("should have future handle");
                    ctx.await_spawned_future(spawned_future.future_id())
                })
                .await;

            // Expect the new viewer to take the next preset color, while continuing to expect old viewers to keep their colors.
            id_to_expected_color.insert(id, PRESET_COLORS[i]);
            presence_manager.read(&app, |presence_manager, _ctx| {
                let viewers = presence_manager.get_present_viewers().collect_vec();
                assert_eq!(viewers.len(), i + 1);
                for viewer in presence_manager.get_present_viewers() {
                    let expected_color = *id_to_expected_color
                        .get(&viewer.info.id)
                        .expect("should have expected viewer ids only");
                    assert_eq!(viewer.color, expected_color);
                    assert!(matches!(viewer.role, Some(Role::Reader)));
                }
            });
        }

        // Set the first viewer as no longer present, and add a new participant.
        viewers.get_mut(0).unwrap().is_present = false;
        assert!(!viewers.first().unwrap().is_present);
        let old_participant_id = viewers.first().unwrap().info.id.clone();
        let new_id = ParticipantId::new();
        viewers.push(Viewer {
            info: ParticipantInfo {
                id: new_id.clone(),
                ..Default::default()
            },
            role: Role::Reader,
            is_present: true,
        });
        presence_manager
            .update(&mut app, |presence_manager, ctx| {
                presence_manager.update_participants(
                    ParticipantList {
                        sharer,
                        viewers,
                        present_viewers: Default::default(),
                        absent_viewers: Default::default(),
                        guests: Default::default(),
                        pending_guests: Default::default(),
                    },
                    ctx,
                );
                let spawned_future = presence_manager
                    .load_participants_imgs_future_handle
                    .as_ref()
                    .expect("should have future handle");
                ctx.await_spawned_future(spawned_future.future_id())
            })
            .await;

        // The color previously taken by the first viewer should be reused for the new participant, while other participants keep their existing colors.
        let old_participant_color = id_to_expected_color
            .remove(&old_participant_id)
            .expect("old participant exists");
        id_to_expected_color.insert(new_id, old_participant_color);
        presence_manager.read(&app, |presence_manager, _ctx| {
            let viewers = presence_manager.get_present_viewers().collect_vec();
            assert_eq!(viewers.len(), PRESET_COLORS.len());
            for viewer in viewers {
                assert_eq!(
                    viewer.color,
                    *id_to_expected_color
                        .get(&viewer.info.id)
                        .expect("should have expected viewer ids only")
                );
                assert!(matches!(viewer.role, Some(Role::Reader)));
            }
        });
    });
}

#[test]
fn test_dont_include_self_in_viewers() {
    App::test((), |mut app| async move {
        let self_id = ParticipantId::new();
        let self_firebase_uid = UserUid::new("mock_firebase_uid");

        let sharer = Sharer {
            ..Default::default()
        };
        let viewers = vec![
            Viewer {
                info: ParticipantInfo {
                    id: self_id.clone(),
                    ..Default::default()
                },
                role: Role::Reader,
                is_present: true,
            },
            Viewer {
                info: ParticipantInfo {
                    ..Default::default()
                },
                role: Role::Reader,
                is_present: true,
            },
            Viewer {
                info: ParticipantInfo {
                    ..Default::default()
                },
                role: Role::Reader,
                is_present: true,
            },
            Viewer {
                info: ParticipantInfo {
                    ..Default::default()
                },
                role: Role::Reader,
                is_present: true,
            },
        ];
        let participant_list = ParticipantList {
            sharer,
            viewers,
            present_viewers: Default::default(),
            absent_viewers: Default::default(),
            guests: Default::default(),
            pending_guests: Default::default(),
        };

        let presence_manager = app.add_model(|ctx| {
            PresenceManager::new_for_viewer(
                self_id.clone(),
                self_firebase_uid,
                participant_list.clone(),
                ctx,
            )
        });

        // Ensure participants are loaded before continuing.
        presence_manager
            .update(&mut app, |presence_manager, ctx| {
                let spawned_future = presence_manager
                    .load_participants_imgs_future_handle
                    .as_ref()
                    .expect("should have future handle");
                ctx.await_spawned_future(spawned_future.future_id())
            })
            .await;

        presence_manager.read(&app, |presence_manager, _ctx| {
            let mut participant_colors = HashSet::new();
            let sharer = presence_manager.get_sharer().expect("should have sharer");
            participant_colors.insert(sharer.color);

            // The viewers returned by presence manager should not include ourselves.
            let viewers = presence_manager.get_present_viewers().collect_vec();
            assert_eq!(viewers.len(), 3);
            for viewer in viewers {
                assert_ne!(viewer.info.id, self_id);
                participant_colors.insert(viewer.color);
            }

            // The sharer and 3 other viewers should all use colors from the preset colors.
            let preset_colors = HashSet::from_iter(PRESET_COLORS[..4].iter().copied());
            assert!(participant_colors.eq(&preset_colors));
        });
    });
}

fn block_list_for_test(max_block_index: usize) -> BlockList {
    let mut block_list = TestBlockListBuilder::new().build();

    // Block 0 already exists as part of creating the blocklist
    for i in 1..max_block_index {
        block_list.command_finished(CommandFinishedValue {
            exit_code: ExitCode::from(0),
            next_block_id: i.to_string().into(),
            session_id: None,
        });
        block_list.precmd(Default::default());
    }
    block_list
}

#[test]
fn test_selected_block_index_for_avatar() {
    App::test((), |mut app| async move {
        // Initialize with a sharer who has blocks selected.
        let mut sharer = Sharer {
            info: ParticipantInfo {
                id: ParticipantId::new(),
                profile_data: ProfileData {
                    ..Default::default()
                },
                selection: Selection::Blocks {
                    block_ids: vec![
                        "1".to_string().into(),
                        "4".to_string().into(),
                        "2".to_string().into(),
                        "10".to_string().into(),
                        "9".to_string().into(),
                    ],
                },
            },
        };
        let viewers = Vec::new();
        let participant_list = ParticipantList {
            sharer: sharer.clone(),
            viewers: viewers.clone(),
            present_viewers: Default::default(),
            absent_viewers: Default::default(),
            guests: Default::default(),
            pending_guests: Default::default(),
        };

        let firebase_uid = UserUid::new("mock_firebase_uid");
        let presence_manager = app.add_model(|ctx| {
            PresenceManager::new_for_viewer(
                ParticipantId::new(),
                firebase_uid,
                participant_list.clone(),
                ctx,
            )
        });

        // Ensure participants are loaded before continuing.
        presence_manager
            .update(&mut app, |presence_manager, ctx| {
                let spawned_future = presence_manager
                    .load_participants_imgs_future_handle
                    .as_ref()
                    .expect("should have future handle");
                ctx.await_spawned_future(spawned_future.future_id())
            })
            .await;

        let block_list = block_list_for_test(15);
        // Check the selected block index for sharer avatar
        presence_manager.read(&app, |presence_manager, _ctx| {
            let sharer = presence_manager.get_sharer().expect("should have sharer");
            let index = sharer
                .get_selected_block_index_for_avatar(&block_list)
                .expect("sharer should have selected block index for avatar");
            // 9 is the top of the last continuous block selection
            assert_eq!(index, 9.into())
        });

        // Now try with just one block selected.
        sharer.info.selection = Selection::Blocks {
            block_ids: vec!["7".to_string().into()],
        };
        presence_manager
            .update(&mut app, |presence_manager, ctx| {
                presence_manager.update_participants(
                    ParticipantList {
                        sharer,
                        viewers,
                        present_viewers: Default::default(),
                        absent_viewers: Default::default(),
                        guests: Default::default(),
                        pending_guests: Default::default(),
                    },
                    ctx,
                );
                let spawned_future = presence_manager
                    .load_participants_imgs_future_handle
                    .as_ref()
                    .expect("should have future handle");
                ctx.await_spawned_future(spawned_future.future_id())
            })
            .await;
        presence_manager.read(&app, |presence_manager, _ctx| {
            let sharer = presence_manager.get_sharer().expect("should have sharer");
            let index = sharer
                .get_selected_block_index_for_avatar(&block_list)
                .expect("sharer should have selected block index for avatar");
            assert_eq!(index, 7.into())
        });
    });
}

fn participant_info_with_profile(
    id: &ParticipantId,
    display_name: &str,
    photo_url: Option<&str>,
) -> ParticipantInfo {
    ParticipantInfo {
        id: id.clone(),
        profile_data: ProfileData {
            firebase_uid: format!("{display_name}-uid"),
            display_name: display_name.to_owned(),
            photo_url: photo_url.map(str::to_owned),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn participant_list_with(sharer: Sharer, viewers: Vec<Viewer>) -> ParticipantList {
    ParticipantList {
        sharer,
        viewers,
        present_viewers: Default::default(),
        absent_viewers: Default::default(),
        guests: Default::default(),
        pending_guests: Default::default(),
    }
}

#[test]
fn attribution_resolves_absent_viewer_profile_without_presence_color() {
    App::test((), |mut app| async move {
        let absent_id = ParticipantId::new();
        let sharer_id = ParticipantId::new();
        let sharer = Sharer {
            info: participant_info_with_profile(&sharer_id, "Sharer", None),
        };
        let absent_viewer = Viewer {
            info: participant_info_with_profile(
                &absent_id,
                "Departed Viewer",
                Some("https://example.com/departed.png"),
            ),
            role: Role::Reader,
            is_present: false,
        };

        let presence_manager = app.add_model(|ctx| {
            PresenceManager::new_for_viewer(
                ParticipantId::new(),
                UserUid::new("self-uid"),
                participant_list_with(sharer, vec![absent_viewer]),
                ctx,
            )
        });

        presence_manager.read(&app, |presence_manager, _ctx| {
            // The live-presence lookup intentionally excludes absent viewers.
            assert!(presence_manager.get_participant(&absent_id).is_none());

            // But attribution of past activity (e.g. the agent query the
            // departed viewer initiated) should still resolve their profile.
            let attribution = presence_manager
                .get_participant_attribution(&absent_id)
                .expect("absent viewer should be resolvable for attribution");
            assert_eq!(attribution.display_name, "Departed Viewer");
            assert_eq!(
                attribution.photo_url.as_deref(),
                Some("https://example.com/departed.png")
            );
            assert_eq!(attribution.color, None);
        });
    });
}

#[test]
fn attribution_still_resolves_viewer_after_they_leave() {
    App::test((), |mut app| async move {
        let viewer_id = ParticipantId::new();
        let sharer_id = ParticipantId::new();
        let sharer = Sharer {
            info: participant_info_with_profile(&sharer_id, "Sharer", None),
        };
        let mut viewer = Viewer {
            info: participant_info_with_profile(&viewer_id, "Prompt Sender", None),
            role: Role::Reader,
            is_present: true,
        };

        let presence_manager = app.add_model(|ctx| {
            PresenceManager::new_for_viewer(
                ParticipantId::new(),
                UserUid::new("self-uid"),
                participant_list_with(sharer.clone(), vec![viewer.clone()]),
                ctx,
            )
        });

        // Ensure participants are loaded before continuing.
        presence_manager
            .update(&mut app, |presence_manager, ctx| {
                let spawned_future = presence_manager
                    .load_participants_imgs_future_handle
                    .as_ref()
                    .expect("should have future handle");
                ctx.await_spawned_future(spawned_future.future_id())
            })
            .await;

        // While present, attribution carries the live presence color.
        presence_manager.read(&app, |presence_manager, _ctx| {
            let attribution = presence_manager
                .get_participant_attribution(&viewer_id)
                .expect("present viewer should be resolvable for attribution");
            assert_eq!(attribution.display_name, "Prompt Sender");
            assert!(attribution.color.is_some());
        });

        // The viewer leaves the session.
        viewer.is_present = false;
        presence_manager.update(&mut app, |presence_manager, ctx| {
            presence_manager.update_participants(participant_list_with(sharer, vec![viewer]), ctx);
        });

        presence_manager.read(&app, |presence_manager, _ctx| {
            assert!(presence_manager.get_participant(&viewer_id).is_none());

            let attribution = presence_manager
                .get_participant_attribution(&viewer_id)
                .expect("departed viewer should still be resolvable for attribution");
            assert_eq!(attribution.display_name, "Prompt Sender");
            assert_eq!(attribution.color, None);
        });
    });
}

#[test]
fn attribution_resolves_sharer_with_presence_color() {
    App::test((), |mut app| async move {
        let sharer_id = ParticipantId::new();
        let sharer = Sharer {
            info: participant_info_with_profile(&sharer_id, "Sharer", None),
        };

        let presence_manager = app.add_model(|ctx| {
            PresenceManager::new_for_viewer(
                ParticipantId::new(),
                UserUid::new("self-uid"),
                participant_list_with(sharer, vec![]),
                ctx,
            )
        });

        presence_manager.read(&app, |presence_manager, _ctx| {
            let attribution = presence_manager
                .get_participant_attribution(&sharer_id)
                .expect("sharer should be resolvable for attribution");
            assert_eq!(attribution.display_name, "Sharer");
            assert!(attribution.color.is_some());
        });
    });
}

#[test]
fn attribution_returns_none_for_unknown_participant() {
    App::test((), |mut app| async move {
        let sharer = Sharer {
            info: participant_info_with_profile(&ParticipantId::new(), "Sharer", None),
        };

        let presence_manager = app.add_model(|ctx| {
            PresenceManager::new_for_viewer(
                ParticipantId::new(),
                UserUid::new("self-uid"),
                participant_list_with(sharer, vec![]),
                ctx,
            )
        });

        presence_manager.read(&app, |presence_manager, _ctx| {
            assert!(presence_manager
                .get_participant_attribution(&ParticipantId::new())
                .is_none());
        });
    });
}
