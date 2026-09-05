use std::collections::{HashMap, HashSet};
use std::iter;

use itertools::Itertools;
use session_sharing_protocol::common::{
    ParticipantId, ParticipantInfo, ParticipantList, ProfileData, Role, Selection, Sharer, Viewer,
};
use warp_core::command::ExitCode;
use warpui::App;

use crate::auth::UserUid;
use crate::terminal::model::ansi::{
    CommandFinishedValue, CompletionMetadata, Handler, PrecmdValue, PromptMetadata,
};
use crate::terminal::model::blocks::BlockList;
use crate::terminal::model::test_utils::TestBlockListBuilder;
use crate::terminal::shared_session::presence_manager::{PRESET_COLORS, PresenceManager};

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

        // With session-scoped color retention, the absent viewer's color stays reserved
        // for the session lifetime and is not reused by the new joiner (REMOTE-2361).
        // The new joiner gets a fresh color (random here since all presets are taken).
        // All still-present viewers keep their original colors.
        let absent_viewer_color = id_to_expected_color
            .remove(&old_participant_id)
            .expect("old participant exists");
        presence_manager.read(&app, |presence_manager, _ctx| {
            let viewers = presence_manager.get_present_viewers().collect_vec();
            assert_eq!(viewers.len(), PRESET_COLORS.len());
            for viewer in viewers {
                if viewer.info.id == new_id {
                    // The new joiner must not receive the absent viewer's session color.
                    assert_ne!(
                        viewer.color, absent_viewer_color,
                        "new joiner must not receive the absent viewer's session color"
                    );
                } else {
                    // All other viewers keep their original preset colors.
                    assert_eq!(
                        viewer.color,
                        *id_to_expected_color
                            .get(&viewer.info.id)
                            .expect("should have expected viewer ids only")
                    );
                }
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

/// Regression test for REMOTE-2361 (color-reuse collision):
/// A new joiner after a disconnect must not be assigned the absent viewer's color.
/// If the absent viewer's color were freed and reused, two distinct authors would
/// render with indistinguishable avatar colors, which is the symptom REMOTE-2361 reports.
#[test]
fn test_new_joiner_does_not_get_absent_viewer_color() {
    App::test((), |mut app| async move {
        let firebase_uid = UserUid::new("mock_firebase_uid");
        let presence_manager =
            app.add_model(|_| PresenceManager::new_for_sharer(ParticipantId::new(), firebase_uid));

        let sharer_id = ParticipantId::new();
        let viewer_a_id = ParticipantId::new();

        let make_participant_list = |viewers: Vec<Viewer>| ParticipantList {
            sharer: Sharer {
                info: ParticipantInfo {
                    id: sharer_id.clone(),
                    ..Default::default()
                },
            },
            viewers,
            present_viewers: Default::default(),
            absent_viewers: Default::default(),
            guests: Default::default(),
            pending_guests: Default::default(),
        };

        // Step 1: add viewer A as a present participant; they get PRESET_COLORS[0].
        let future_id = presence_manager.update(&mut app, |pm, ctx| {
            pm.update_participants(
                make_participant_list(vec![Viewer {
                    info: ParticipantInfo {
                        id: viewer_a_id.clone(),
                        ..Default::default()
                    },
                    role: Role::Reader,
                    is_present: true,
                }]),
                ctx,
            );
            pm.load_participants_imgs_future_handle
                .as_ref()
                .expect("should have future handle")
                .future_id()
        });
        presence_manager
            .update(&mut app, |_, ctx| ctx.await_spawned_future(future_id))
            .await;

        let viewer_a_color = presence_manager.read(&app, |pm, _| {
            pm.get_participant(&viewer_a_id)
                .expect("viewer A should be present")
                .color
        });
        // Viewer A should have received the first preset color.
        assert_eq!(viewer_a_color, PRESET_COLORS[0]);

        // Step 2: viewer A disconnects.
        let future_id = presence_manager.update(&mut app, |pm, ctx| {
            pm.update_participants(
                make_participant_list(vec![Viewer {
                    info: ParticipantInfo {
                        id: viewer_a_id.clone(),
                        ..Default::default()
                    },
                    role: Role::Reader,
                    is_present: false,
                }]),
                ctx,
            );
            pm.load_participants_imgs_future_handle
                .as_ref()
                .expect("should have future handle")
                .future_id()
        });
        presence_manager
            .update(&mut app, |_, ctx| ctx.await_spawned_future(future_id))
            .await;

        // Step 3: a new viewer B joins after A left.
        let viewer_b_id = ParticipantId::new();
        let future_id = presence_manager.update(&mut app, |pm, ctx| {
            pm.update_participants(
                make_participant_list(vec![
                    Viewer {
                        info: ParticipantInfo {
                            id: viewer_a_id.clone(),
                            ..Default::default()
                        },
                        role: Role::Reader,
                        is_present: false,
                    },
                    Viewer {
                        info: ParticipantInfo {
                            id: viewer_b_id.clone(),
                            ..Default::default()
                        },
                        role: Role::Reader,
                        is_present: true,
                    },
                ]),
                ctx,
            );
            pm.load_participants_imgs_future_handle
                .as_ref()
                .expect("should have future handle")
                .future_id()
        });
        presence_manager
            .update(&mut app, |_, ctx| ctx.await_spawned_future(future_id))
            .await;

        presence_manager.read(&app, |pm, _| {
            let viewer_b_color = pm
                .get_participant(&viewer_b_id)
                .expect("viewer B should be present")
                .color;
            // B must not receive A's reserved color — that would make them visually
            // indistinguishable, which is the REMOTE-2361 defect.
            assert_ne!(
                viewer_b_color, viewer_a_color,
                "new joiner must not be assigned the absent viewer's session color"
            );
            // B should get the second preset color (the first is still reserved for A).
            assert_eq!(viewer_b_color, PRESET_COLORS[1]);
        });
    });
}

/// Regression test for REMOTE-2361 (rejoin path):
/// A viewer who rejoins after a disconnect must keep their original color.
/// Without this fix, the rejoin path allocates a fresh color and leaves a stale
/// absent_viewers entry behind, causing historical AI blocks to flip color mid-session.
#[test]
fn test_rejoining_viewer_keeps_original_color() {
    App::test((), |mut app| async move {
        let firebase_uid = UserUid::new("mock_firebase_uid");
        let presence_manager =
            app.add_model(|_| PresenceManager::new_for_sharer(ParticipantId::new(), firebase_uid));

        let sharer_id = ParticipantId::new();
        let viewer_id = ParticipantId::new();

        let make_participant_list = |viewers: Vec<Viewer>| ParticipantList {
            sharer: Sharer {
                info: ParticipantInfo {
                    id: sharer_id.clone(),
                    ..Default::default()
                },
            },
            viewers,
            present_viewers: Default::default(),
            absent_viewers: Default::default(),
            guests: Default::default(),
            pending_guests: Default::default(),
        };

        // Step 1: viewer joins initially.
        let future_id = presence_manager.update(&mut app, |pm, ctx| {
            pm.update_participants(
                make_participant_list(vec![Viewer {
                    info: ParticipantInfo {
                        id: viewer_id.clone(),
                        ..Default::default()
                    },
                    role: Role::Reader,
                    is_present: true,
                }]),
                ctx,
            );
            pm.load_participants_imgs_future_handle
                .as_ref()
                .expect("should have future handle")
                .future_id()
        });
        presence_manager
            .update(&mut app, |_, ctx| ctx.await_spawned_future(future_id))
            .await;

        let original_color = presence_manager.read(&app, |pm, _| {
            pm.get_participant(&viewer_id)
                .expect("viewer should be present")
                .color
        });

        // Step 2: viewer disconnects.
        let future_id = presence_manager.update(&mut app, |pm, ctx| {
            pm.update_participants(
                make_participant_list(vec![Viewer {
                    info: ParticipantInfo {
                        id: viewer_id.clone(),
                        ..Default::default()
                    },
                    role: Role::Reader,
                    is_present: false,
                }]),
                ctx,
            );
            pm.load_participants_imgs_future_handle
                .as_ref()
                .expect("should have future handle")
                .future_id()
        });
        presence_manager
            .update(&mut app, |_, ctx| ctx.await_spawned_future(future_id))
            .await;

        // Historical avatar lookup must still work after disconnect.
        presence_manager.read(&app, |pm, _| {
            let (_info, color) = pm
                .get_participant_info_for_avatar(&viewer_id)
                .expect("should resolve absent viewer for avatar");
            assert_eq!(
                color, original_color,
                "absent viewer color must be retained"
            );
        });

        // Step 3: viewer rejoins.
        let future_id = presence_manager.update(&mut app, |pm, ctx| {
            pm.update_participants(
                make_participant_list(vec![Viewer {
                    info: ParticipantInfo {
                        id: viewer_id.clone(),
                        ..Default::default()
                    },
                    role: Role::Reader,
                    is_present: true,
                }]),
                ctx,
            );
            pm.load_participants_imgs_future_handle
                .as_ref()
                .expect("should have future handle")
                .future_id()
        });
        presence_manager
            .update(&mut app, |_, ctx| ctx.await_spawned_future(future_id))
            .await;

        presence_manager.read(&app, |pm, _| {
            // Rejoining viewer is back in present_viewers with their original color.
            let rejoined = pm
                .get_participant(&viewer_id)
                .expect("rejoined viewer should be present");
            assert_eq!(
                rejoined.color, original_color,
                "rejoining viewer must keep their original session color"
            );

            // get_participant_info_for_avatar must also return the stable color.
            let (_info, color) = pm
                .get_participant_info_for_avatar(&viewer_id)
                .expect("should resolve rejoined viewer for avatar");
            assert_eq!(
                color, original_color,
                "get_participant_info_for_avatar must return the stable session color after rejoin"
            );
        });
    });
}

/// Regression test for REMOTE-2361:
/// After a viewer disconnects from a shared cloud agent session, AI-block avatars
/// authored by that viewer should still resolve to the viewer's original color and
/// identity, rather than falling back to the local user.
#[test]
fn test_absent_viewer_color_retained_for_avatar_lookup() {
    App::test((), |mut app| async move {
        let firebase_uid = UserUid::new("mock_firebase_uid");
        let presence_manager =
            app.add_model(|_| PresenceManager::new_for_sharer(ParticipantId::new(), firebase_uid));

        let sharer_id = ParticipantId::new();
        let sharer = Sharer {
            info: ParticipantInfo {
                id: sharer_id.clone(),
                ..Default::default()
            },
        };
        let viewer_id = ParticipantId::new();

        // Step 1: add the viewer as a present participant.
        let present_viewers = vec![Viewer {
            info: ParticipantInfo {
                id: viewer_id.clone(),
                ..Default::default()
            },
            role: Role::Reader,
            is_present: true,
        }];
        presence_manager
            .update(&mut app, |presence_manager, ctx| {
                presence_manager.update_participants(
                    ParticipantList {
                        sharer: sharer.clone(),
                        viewers: present_viewers,
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

        // Record the color assigned while the viewer is present.
        let viewer_color = presence_manager.read(&app, |pm, _ctx| {
            pm.get_participant(&viewer_id)
                .expect("viewer should be present")
                .color
        });

        // Step 2: mark the viewer as absent (disconnect).
        let absent_viewers = vec![Viewer {
            info: ParticipantInfo {
                id: viewer_id.clone(),
                ..Default::default()
            },
            role: Role::Reader,
            is_present: false,
        }];
        presence_manager
            .update(&mut app, |presence_manager, ctx| {
                presence_manager.update_participants(
                    ParticipantList {
                        sharer: sharer.clone(),
                        viewers: absent_viewers,
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

        presence_manager.read(&app, |pm, _ctx| {
            // get_participant must NOT return absent viewers (present-only API preserved).
            assert!(
                pm.get_participant(&viewer_id).is_none(),
                "get_participant should not return absent viewers"
            );

            // get_participant_info_for_avatar MUST resolve the absent viewer and return
            // the original assigned color, not fall back to the local user.
            let (_info, color) = pm
                .get_participant_info_for_avatar(&viewer_id)
                .expect("get_participant_info_for_avatar should resolve absent viewers");
            assert_eq!(
                color, viewer_color,
                "absent viewer color must be retained for historical AI-block avatar rendering"
            );
        });
    });
}

fn block_list_for_test(max_block_index: usize) -> BlockList {
    let mut block_list = TestBlockListBuilder::new().build();

    // Block 0 already exists as part of creating the blocklist
    for i in 1..max_block_index {
        let completion_metadata = CompletionMetadata {
            exit_code: ExitCode::from(0),
            next_block_id: i.to_string().into(),
        };
        block_list.command_finished(CommandFinishedValue {
            completion_metadata: completion_metadata.clone(),
            session_id: None,
        });
        block_list.precmd_with_completion_metadata(PrecmdValue {
            completion_metadata,
            prompt_metadata: PromptMetadata::default(),
        });
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
