use std::collections::HashSet;
use std::iter;

use itertools::Itertools;
use session_sharing_protocol::common::{
    ParticipantId, ParticipantInfo, ParticipantList, ProfileData, Role, Selection, Sharer, Viewer,
};
use warp_core::command::ExitCode;
use warpui::App;

use super::PRESET_COLORS;
// PRESET_COLORS is a private const in the parent module; accessible here because
// this file is declared as `mod tests` inside presence_manager.rs.
use super::{PresenceManager, assign_colors_for_participants, color_for_participant_id_index};
use crate::auth::UserUid;
use crate::terminal::model::ansi::{
    CommandFinishedValue, CompletionMetadata, Handler, PrecmdValue, PromptMetadata,
};
use crate::terminal::model::blocks::BlockList;
use crate::terminal::model::test_utils::TestBlockListBuilder;

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
fn test_viewer_colors_come_from_preset_palette() {
    // Colors are now derived deterministically from each participant's ID via
    // `color_for_participant_id`, so viewers no longer receive sequential
    // preset colors. This test verifies that every viewer's color still comes
    // from the `PRESET_COLORS` palette and that the color is stable across
    // repeated `update_participants` calls.
    App::test((), |mut app| async move {
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

        // We ourselves are the sharer, so we are not in the viewers list.
        presence_manager.read(&app, |presence_manager: &PresenceManager, _ctx| {
            assert!(presence_manager.get_sharer().is_none());
            assert_eq!(presence_manager.get_present_viewers().count(), 0);
        });

        // Add viewers one-by-one. Each viewer should receive a collision-free color from
        // PRESET_COLORS via `assign_colors_for_participants`, and all present viewers at
        // any point in time must have distinct colors.
        let viewer_ids: Vec<_> = iter::repeat_with(ParticipantId::new).take(3).collect();

        for (i, id) in viewer_ids.iter().enumerate() {
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

            presence_manager.read(&app, |presence_manager, _ctx| {
                assert_eq!(presence_manager.get_present_viewers().count(), i + 1);
                let colors: Vec<_> = presence_manager
                    .get_present_viewers()
                    .map(|v| v.color)
                    .collect();
                for &color in &colors {
                    assert!(
                        PRESET_COLORS.contains(&color),
                        "viewer color must be a preset color"
                    );
                }
                // All viewers must have distinct colors (collision-free guarantee).
                let unique: HashSet<_> = colors.iter().copied().collect();
                assert_eq!(
                    colors.len(),
                    unique.len(),
                    "all viewers must have distinct colors"
                );
                for viewer in presence_manager.get_present_viewers() {
                    assert!(matches!(viewer.role, Some(Role::Reader)));
                }
            });
        }

        // Mark one viewer absent and add a new one. The new viewer still gets
        // a deterministic color from PRESET_COLORS.
        viewers.first_mut().unwrap().is_present = false;
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

        presence_manager.read(&app, |presence_manager, _ctx| {
            let colors: Vec<_> = presence_manager
                .get_present_viewers()
                .map(|v| v.color)
                .collect();
            for &color in &colors {
                assert!(
                    PRESET_COLORS.contains(&color),
                    "viewer color must be a preset color after departure/join"
                );
            }
            // Distinctness must hold after departure/join too.
            let unique: HashSet<_> = colors.iter().copied().collect();
            assert_eq!(
                colors.len(),
                unique.len(),
                "all viewers must have distinct colors after departure/join"
            );
            for viewer in presence_manager.get_present_viewers() {
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
            let sharer = presence_manager.get_sharer().expect("should have sharer");

            // The viewers returned by presence manager should not include ourselves.
            let viewers = presence_manager.get_present_viewers().collect_vec();
            assert_eq!(viewers.len(), 3);
            for viewer in &viewers {
                assert_ne!(viewer.info.id, self_id);
                assert!(
                    PRESET_COLORS.contains(&viewer.color),
                    "viewer color must be a preset color"
                );
            }

            // Sharer color must also come from the preset palette.
            assert!(
                PRESET_COLORS.contains(&sharer.color),
                "sharer color must be a preset color"
            );

            // All 4 participants (sharer + 3 viewers, not self) must have distinct colors.
            let mut all_colors: Vec<_> = viewers.iter().map(|v| v.color).collect();
            all_colors.push(sharer.color);
            let unique: HashSet<_> = all_colors.iter().copied().collect();
            assert_eq!(
                all_colors.len(),
                unique.len(),
                "sharer and all viewers must have distinct colors"
            );
        });
    });
}

#[test]
fn test_get_participant_for_attribution_resolves_self() {
    // Regression test for REMOTE-2363: the browser's shared-session view showed the
    // same avatar/color for every message because `get_participant` excluded the local
    // user (self), so viewer-initiated exchanges always fell back to the theme accent.
    // `get_participant_for_attribution` must resolve the viewer's own identity so that
    // each participant's messages render with distinct, per-author colors.
    App::test((), |mut app| async move {
        let self_id = ParticipantId::new();
        let self_display_name = "Browser Viewer".to_string();
        let self_firebase_uid = UserUid::new("self_uid");

        let sharer_id = ParticipantId::new();
        let sharer_display_name = "Terminal Sharer".to_string();
        let sharer = Sharer {
            info: ParticipantInfo {
                id: sharer_id.clone(),
                profile_data: ProfileData {
                    display_name: sharer_display_name.clone(),
                    ..Default::default()
                },
                ..Default::default()
            },
        };

        // The viewer list includes self as the first entry (as the server would send it)
        // followed by one other viewer.
        let other_viewer_id = ParticipantId::new();
        let viewers = vec![
            Viewer {
                info: ParticipantInfo {
                    id: self_id.clone(),
                    profile_data: ProfileData {
                        display_name: self_display_name.clone(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                role: Role::Reader,
                is_present: true,
            },
            Viewer {
                info: ParticipantInfo {
                    id: other_viewer_id.clone(),
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
                participant_list,
                ctx,
            )
        });

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
            // Self must NOT appear in present_viewers (live-presence exclusion is preserved).
            assert!(presence_manager.get_participant(&self_id).is_none());

            // `get_participant_for_attribution` must resolve the sharer.
            let sharer_attr = presence_manager
                .get_participant_for_attribution(&sharer_id)
                .expect("sharer must be resolvable for attribution");
            assert_eq!(sharer_attr.display_name, sharer_display_name);
            let sharer_color = sharer_attr.color;

            // `get_participant_for_attribution` must resolve self.
            let self_attr = presence_manager
                .get_participant_for_attribution(&self_id)
                .expect("self must be resolvable for attribution");
            assert_eq!(self_attr.display_name, self_display_name);
            let self_color = self_attr.color;

            // `get_participant_for_attribution` must resolve the other viewer.
            let other_attr = presence_manager
                .get_participant_for_attribution(&other_viewer_id)
                .expect("other viewer must be resolvable for attribution");
            let other_color = other_attr.color;

            // All colors must come from the preset palette.
            assert!(
                PRESET_COLORS.contains(&self_color),
                "self color must be a preset color"
            );
            assert!(
                PRESET_COLORS.contains(&sharer_color),
                "sharer color must be a preset color"
            );
            assert!(
                PRESET_COLORS.contains(&other_color),
                "other viewer color must be a preset color"
            );

            // All three participants must have DISTINCT colors (AC1/AC2: collision-free
            // guarantee from `assign_colors_for_participants`).
            assert_ne!(
                self_color, sharer_color,
                "self and sharer must have different colors"
            );
            assert_ne!(
                self_color, other_color,
                "self and other viewer must have different colors"
            );
            assert_ne!(
                sharer_color, other_color,
                "sharer and other viewer must have different colors"
            );

            // An unknown participant ID must not resolve.
            assert!(
                presence_manager
                    .get_participant_for_attribution(&ParticipantId::new())
                    .is_none(),
                "unknown participant must not resolve for attribution"
            );
        });
    });
}

#[test]
fn test_get_participant_for_attribution_resolves_sharer_self() {
    // On the sharer's terminal client, `sharer` is None and `present_viewers` excludes self.
    // `get_participant_for_attribution` must still resolve the sharer's own identity after
    // `update_participants` populates `own_profile_info` from the incoming sharer info.
    App::test((), |mut app| async move {
        let sharer_id = ParticipantId::new();
        let sharer_display_name = "Terminal Sharer".to_string();
        let firebase_uid = UserUid::new("sharer_uid");

        // Create a sharer-mode presence manager.
        let presence_manager =
            app.add_model(|_| PresenceManager::new_for_sharer(sharer_id.clone(), firebase_uid));

        // Before update_participants, own_profile_info is None, so attribution returns None.
        presence_manager.read(&app, |pm, _ctx| {
            assert!(
                pm.get_participant_for_attribution(&sharer_id).is_none(),
                "before update_participants, sharer self must not resolve"
            );
        });

        // Simulate the server sending a participant list that includes the sharer's own info.
        let viewer_id = ParticipantId::new();
        let participant_list = ParticipantList {
            sharer: Sharer {
                info: ParticipantInfo {
                    id: sharer_id.clone(),
                    profile_data: ProfileData {
                        display_name: sharer_display_name.clone(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
            viewers: vec![Viewer {
                info: ParticipantInfo {
                    id: viewer_id.clone(),
                    ..Default::default()
                },
                role: Role::Reader,
                is_present: true,
            }],
            present_viewers: Default::default(),
            absent_viewers: Default::default(),
            guests: Default::default(),
            pending_guests: Default::default(),
        };

        presence_manager
            .update(&mut app, |pm, ctx| {
                pm.update_participants(participant_list, ctx);
                let spawned_future = pm
                    .load_participants_imgs_future_handle
                    .as_ref()
                    .expect("should have future handle");
                ctx.await_spawned_future(spawned_future.future_id())
            })
            .await;

        presence_manager.read(&app, |pm, _ctx| {
            // The sharer is still not in the live-presence maps.
            assert!(
                pm.get_sharer().is_none(),
                "sharer must not see themselves in get_sharer()"
            );
            assert!(
                pm.get_participant(&sharer_id).is_none(),
                "sharer must not see themselves in get_participant()"
            );

            // But attribution must resolve the sharer's own identity.
            let attr = pm
                .get_participant_for_attribution(&sharer_id)
                .expect("sharer self must be resolvable for attribution after update_participants");
            assert_eq!(attr.display_name, sharer_display_name);

            // Color must come from the preset palette and be the collision-free
            // assignment for this participant set (sharer + one viewer).
            assert!(
                PRESET_COLORS.contains(&attr.color),
                "sharer color must be a preset color"
            );
            let expected = assign_colors_for_participants(&[sharer_id.clone(), viewer_id.clone()]);
            assert_eq!(
                attr.color, expected[&sharer_id],
                "sharer color must match assign_colors_for_participants"
            );
        });
    });
}

#[test]
fn test_color_parity_across_managers() {
    // Cross-client parity: two independently-constructed `PresenceManager` instances
    // (one for the browser viewer, one for the terminal sharer) must assign the same
    // color to the same participant ID.  This property is what acceptance criterion 3
    // (AC3) of REMOTE-2363 requires.
    App::test((), |mut app| async move {
        let sharer_id = ParticipantId::new();
        let viewer_id = ParticipantId::new();

        let sharer_firebase_uid = UserUid::new("sharer_uid");
        let viewer_firebase_uid = UserUid::new("viewer_uid");

        // Build the shared participant list that both clients would receive.
        let make_participant_list = || ParticipantList {
            sharer: Sharer {
                info: ParticipantInfo {
                    id: sharer_id.clone(),
                    ..Default::default()
                },
            },
            viewers: vec![Viewer {
                info: ParticipantInfo {
                    id: viewer_id.clone(),
                    ..Default::default()
                },
                role: Role::Reader,
                is_present: true,
            }],
            present_viewers: Default::default(),
            absent_viewers: Default::default(),
            guests: Default::default(),
            pending_guests: Default::default(),
        };

        // Terminal (sharer) manager.
        let sharer_manager = app
            .add_model(|_| PresenceManager::new_for_sharer(sharer_id.clone(), sharer_firebase_uid));
        sharer_manager
            .update(&mut app, |pm, ctx| {
                pm.update_participants(make_participant_list(), ctx);
                let spawned_future = pm
                    .load_participants_imgs_future_handle
                    .as_ref()
                    .expect("should have future handle");
                ctx.await_spawned_future(spawned_future.future_id())
            })
            .await;

        // Browser (viewer) manager.
        let viewer_manager = app.add_model(|ctx| {
            PresenceManager::new_for_viewer(
                viewer_id.clone(),
                viewer_firebase_uid,
                make_participant_list(),
                ctx,
            )
        });
        viewer_manager
            .update(&mut app, |pm, ctx| {
                let spawned_future = pm
                    .load_participants_imgs_future_handle
                    .as_ref()
                    .expect("should have future handle");
                ctx.await_spawned_future(spawned_future.future_id())
            })
            .await;

        // Both clients must agree on the color for the sharer.
        let sharer_color_on_terminal = sharer_manager.read(&app, |pm, _| {
            pm.get_participant_for_attribution(&sharer_id)
                .expect("sharer manager must resolve sharer self")
                .color
        });
        let sharer_color_on_browser = viewer_manager.read(&app, |pm, _| {
            pm.get_participant_for_attribution(&sharer_id)
                .expect("viewer manager must resolve sharer")
                .color
        });
        assert_eq!(
            sharer_color_on_terminal, sharer_color_on_browser,
            "sharer must have the same color on terminal and browser (AC3)"
        );

        // Both clients must agree on the color for the viewer.
        let viewer_color_on_terminal = sharer_manager.read(&app, |pm, _| {
            pm.get_participant(&viewer_id)
                .expect("sharer manager must see viewer in present_viewers")
                .color
        });
        let viewer_color_on_browser = viewer_manager.read(&app, |pm, _| {
            pm.get_participant_for_attribution(&viewer_id)
                .expect("viewer manager must resolve self")
                .color
        });
        assert_eq!(
            viewer_color_on_terminal, viewer_color_on_browser,
            "viewer must have the same color on terminal and browser (AC3)"
        );

        // AC1/AC2: the two participants must have distinct colors.
        assert_ne!(
            sharer_color_on_terminal, viewer_color_on_terminal,
            "sharer and viewer must have different colors"
        );
    });
}

#[test]
fn test_color_index_uses_u64_arithmetic_not_usize() {
    // `color_for_participant_id_index` must compute modulo in `u64`, not `usize`.
    //
    // On wasm32 (the browser target) `usize` is 32 bits wide.  A naive
    // `(hash as usize) % n` would silently discard the upper 32 bits of the
    // 64-bit hash value *before* the modulo, yielding a different palette index
    // for the ~57 % of participant IDs whose upper 32 bits are non-zero.
    //
    // This test pins the expected index against an explicit `u64` reference
    // computation. On a native 64-bit host `u64 == usize` so both formulas agree
    // and the test is trivially green. On wasm32 CI a function that uses
    // `(hash as usize) % n` would return a different result, failing the
    // assertion for every ID where the two formulas diverge.
    for _ in 0..20 {
        let id = ParticipantId::new();
        let s = id.to_string();

        // Reference: u64 modulo before narrowing to usize.
        let hash_u64: u64 = s.bytes().fold(0u64, |acc, b| {
            acc.wrapping_mul(31).wrapping_add(u64::from(b))
        });
        let expected_index = (hash_u64 % PRESET_COLORS.len() as u64) as usize;

        // Hypothetical buggy path (what `(hash as usize) % n` would give on wasm32):
        let buggy_u32_index = (hash_u64 as u32 as usize) % PRESET_COLORS.len();
        // (On a 64-bit host `buggy_u32_index` can equal `expected_index`, but the
        // assertion below catches the wasm32 regression regardless.)
        let _ = buggy_u32_index; // suppress unused warning on 64-bit

        assert_eq!(
            color_for_participant_id_index(&id),
            expected_index,
            "color_for_participant_id_index must use u64 modulo to avoid wasm32 truncation \
             (id={s}, hash={hash_u64:#018x})"
        );
    }
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
