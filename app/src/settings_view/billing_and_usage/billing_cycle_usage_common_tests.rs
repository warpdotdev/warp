use super::{
    BarSegment, TeamScopedUsage, aggregate_segments, filter_entries_by_attributed_team,
    filter_legacy_buckets, has_non_viewer_data, legend_cost_types, scope_members_to_team,
};
use crate::auth::UserUid;
use crate::workspaces::team::{MembershipRole, Team, TeamMember};
use crate::workspaces::workspace::{
    AiCreditsUsageAndCostSubjectType, AiCreditsUsageAndCostType, AiCreditsUsageBucket,
    AiCreditsUsageSource, BillingCycleUsageEntry, WorkspaceMember, WorkspaceMemberUsageInfo,
};

const VIEWER_UID: &str = "viewer-uid";
const OTHER_UID: &str = "other-uid";
const CURRENT_TEAM_UID: &str = "team-a-uid";
const OTHER_TEAM_UID: &str = "team-b-uid";

fn entry(
    subject_type: AiCreditsUsageAndCostSubjectType,
    subject_uid: Option<&str>,
    cost_type: AiCreditsUsageAndCostType,
    usage_bucket: AiCreditsUsageBucket,
    usage_source: AiCreditsUsageSource,
    credits_used: i32,
    cost_cents: i32,
) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        subject_type,
        subject_uid: subject_uid.map(|s| s.to_string()),
        subject_display_name: None,
        cost_type,
        usage_bucket,
        usage_source,
        credits_used,
        cost_cents,
        attributed_team_uid: None,
    }
}

/// Boilerplate User entry attributed to `attributed_team_uid`.
fn entry_for_team(
    subject_uid: &str,
    attributed_team_uid: Option<&str>,
    credits_used: i32,
) -> BillingCycleUsageEntry {
    BillingCycleUsageEntry {
        attributed_team_uid: attributed_team_uid.map(|s| s.to_string()),
        ..entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(subject_uid),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageSource::Local,
            credits_used,
            0,
        )
    }
}

fn workspace_member(uid: &str) -> WorkspaceMember {
    WorkspaceMember {
        uid: UserUid::new(uid),
        email: format!("{uid}@warp.dev"),
        role: MembershipRole::User,
        usage_info: WorkspaceMemberUsageInfo {
            is_unlimited: false,
            request_limit: 100,
            requests_used_since_last_refresh: 0,
            is_request_limit_prorated: false,
        },
    }
}

fn team_member(uid: &str) -> TeamMember {
    TeamMember {
        uid: UserUid::new(uid),
        email: format!("{uid}@warp.dev"),
        role: MembershipRole::User,
    }
}

/// A team whose `uid` renders as `uid_seed` padded to a `ServerId`, so tests
/// can attribute entries to it via [`team_uid_str`].
fn team(uid_seed: i64, member_uids: &[&str]) -> Team {
    Team {
        uid: uid_seed.into(),
        name: format!("team-{uid_seed}"),
        color: None,
        invite_code: None,
        members: member_uids.iter().copied().map(team_member).collect(),
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
    }
}

fn team_uid_str(team: &Team) -> String {
    team.uid.uid()
}

/// Boilerplate viewer-owned User row for predicate tests.
fn viewer_user_entry() -> BillingCycleUsageEntry {
    entry(
        AiCreditsUsageAndCostSubjectType::User,
        Some(VIEWER_UID),
        AiCreditsUsageAndCostType::BaseLimit,
        AiCreditsUsageBucket::Ai,
        AiCreditsUsageSource::Local,
        10,
        0,
    )
}

#[test]
fn has_non_viewer_data_returns_false_when_entries_empty() {
    assert!(!has_non_viewer_data(&[], Some(VIEWER_UID)));
}

#[test]
fn has_non_viewer_data_returns_false_when_only_viewer_user_rows() {
    let entries = vec![viewer_user_entry(), viewer_user_entry()];
    assert!(!has_non_viewer_data(&entries, Some(VIEWER_UID)));
}

#[test]
fn has_non_viewer_data_returns_true_for_team_aggregate_row() {
    // TeamAggregate visibility represents "everyone else's usage" as a single
    // Team-typed row, even when the workspace currently has only one member
    // (e.g. a teammate left mid-cycle after incurring AI costs).
    let entries = vec![
        viewer_user_entry(),
        entry(
            AiCreditsUsageAndCostSubjectType::Team,
            None,
            AiCreditsUsageAndCostType::Aggregate,
            AiCreditsUsageBucket::Aggregate,
            AiCreditsUsageSource::Aggregate,
            500,
            300,
        ),
    ];
    assert!(has_non_viewer_data(&entries, Some(VIEWER_UID)));
}

#[test]
fn has_non_viewer_data_returns_true_for_other_user_row() {
    // PerUserTotals / FullBreakdown emit per-user rows, so a departed teammate
    // shows up as a User entry with a non-viewer UID.
    let entries = vec![entry(
        AiCreditsUsageAndCostSubjectType::User,
        Some(OTHER_UID),
        AiCreditsUsageAndCostType::BaseLimit,
        AiCreditsUsageBucket::Ai,
        AiCreditsUsageSource::Local,
        50,
        0,
    )];
    assert!(has_non_viewer_data(&entries, Some(VIEWER_UID)));
}

#[test]
fn has_non_viewer_data_returns_true_for_service_account_row() {
    let entries = vec![entry(
        AiCreditsUsageAndCostSubjectType::ServiceAccount,
        Some("sa-uid"),
        AiCreditsUsageAndCostType::BaseLimit,
        AiCreditsUsageBucket::Ai,
        AiCreditsUsageSource::Cloud,
        25,
        0,
    )];
    assert!(has_non_viewer_data(&entries, Some(VIEWER_UID)));
}

#[test]
fn has_non_viewer_data_treats_missing_subject_uid_as_non_viewer() {
    // Defensive: a User row with no UID is conservatively treated as a non-
    // viewer subject so we never accidentally drop team scaffolding.
    let entries = vec![entry(
        AiCreditsUsageAndCostSubjectType::User,
        None,
        AiCreditsUsageAndCostType::BaseLimit,
        AiCreditsUsageBucket::Ai,
        AiCreditsUsageSource::Local,
        1,
        0,
    )];
    assert!(has_non_viewer_data(&entries, Some(VIEWER_UID)));
}

#[test]
fn has_non_viewer_data_treats_missing_viewer_uid_as_non_viewer() {
    // Signed-out / unidentified viewer: any subject we can't prove belongs
    // to them counts as non-viewer data.
    let entries = vec![viewer_user_entry()];
    assert!(has_non_viewer_data(&entries, None));
}

#[test]
fn filter_legacy_buckets_drops_voice_and_suggested_code_diffs_in_input_order() {
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageSource::Local,
            10,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Voice,
            AiCreditsUsageSource::Local,
            3,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Compute,
            AiCreditsUsageSource::Local,
            5,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::SuggestedCodeDiffs,
            AiCreditsUsageSource::Local,
            7,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::Aggregate,
            AiCreditsUsageBucket::Aggregate,
            AiCreditsUsageSource::Aggregate,
            100,
            50,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Platform,
            AiCreditsUsageSource::Cloud,
            2,
            0,
        ),
    ];

    let filtered = filter_legacy_buckets(&entries);

    let kept_buckets: Vec<_> = filtered.iter().map(|e| e.usage_bucket.clone()).collect();
    assert_eq!(
        kept_buckets,
        vec![
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageBucket::Compute,
            AiCreditsUsageBucket::Aggregate,
            AiCreditsUsageBucket::Platform,
        ],
        "expected Voice + SuggestedCodeDiffs dropped while preserving the rest in input order"
    );
}

#[test]
fn aggregate_segments_merges_dupes_drops_zeros_and_sorts() {
    let entries = [
        // Same (BonusGrant, Compute) appears twice across different sources;
        // should merge into one segment.
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BonusGrant,
            AiCreditsUsageBucket::Compute,
            AiCreditsUsageSource::Local,
            10,
            5,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BonusGrant,
            AiCreditsUsageBucket::Compute,
            AiCreditsUsageSource::Cloud,
            7,
            3,
        ),
        // BaseLimit/Ai — should sort before any BonusGrant entry.
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageSource::Local,
            20,
            0,
        ),
        // Zero-credit entry: must be dropped before totals are computed (so
        // the stray cost_cents don't leak into the row total).
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::Payg,
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageSource::Local,
            0,
            42,
        ),
    ];

    let (segments, total_credits, total_cost_cents) = aggregate_segments(entries.iter());

    let key = |s: &BarSegment| (s.cost_type.clone(), s.usage_bucket.clone());
    let keys: Vec<_> = segments.iter().map(key).collect();
    assert_eq!(
        keys,
        vec![
            (
                AiCreditsUsageAndCostType::BaseLimit,
                AiCreditsUsageBucket::Ai
            ),
            (
                AiCreditsUsageAndCostType::BonusGrant,
                AiCreditsUsageBucket::Compute
            ),
        ],
        "expected BaseLimit/Ai before BonusGrant/Compute, Payg zero-credit dropped"
    );

    let bonus = &segments[1];
    assert_eq!(bonus.credits, 17, "10 + 7 merged credits");
    assert_eq!(bonus.cost_cents, 8, "5 + 3 merged cost cents");

    // Totals are summed *after* the zero-credit segment is dropped, so the
    // stray 42 cents on the Payg/Ai entry must not appear here.
    assert_eq!(total_credits, 20 + 17);
    assert_eq!(total_cost_cents, 8);
}

#[test]
fn legend_cost_types_excludes_zero_credit_bucket() {
    // Regression: a base-limit row with no usage must not surface "Base" in
    // the legend while only Pay-as-you-go credits were actually spent.
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageSource::Local,
            0,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::Payg,
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageSource::Local,
            50,
            120,
        ),
    ];

    assert_eq!(
        legend_cost_types(&entries),
        vec![AiCreditsUsageAndCostType::Payg],
        "zero-credit BaseLimit row must be dropped from the legend"
    );
}

#[test]
fn legend_cost_types_includes_used_buckets_in_display_order() {
    // Buckets with real usage appear in the canonical legend order regardless
    // of input order (Payg listed before BaseLimit here).
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::Payg,
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageSource::Local,
            5,
            10,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Ai,
            AiCreditsUsageSource::Local,
            8,
            0,
        ),
    ];

    assert_eq!(
        legend_cost_types(&entries),
        vec![
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageAndCostType::Payg,
        ],
        "used buckets should render in canonical order, not input order"
    );
}

#[test]
fn filter_entries_by_attributed_team_keeps_only_the_team_in_view() {
    // The regression this guards: a team-A admin was shown team B's usage
    // because the workspace-wide history was rendered unfiltered.
    let entries = vec![
        entry_for_team(OTHER_UID, Some(CURRENT_TEAM_UID), 10),
        entry_for_team("third-uid", Some(OTHER_TEAM_UID), 999),
    ];

    let filtered = filter_entries_by_attributed_team(&entries, CURRENT_TEAM_UID, Some(VIEWER_UID));

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].subject_uid.as_deref(), Some(OTHER_UID));
    assert_eq!(filtered[0].credits_used, 10);
}

#[test]
fn filter_entries_by_attributed_team_drops_other_peoples_unattributed_entries() {
    // Usage the server left unattributed belongs to no team in particular, so
    // it must not be billed to the team in view.
    let entries = vec![entry_for_team(OTHER_UID, None, 20)];

    assert!(
        filter_entries_by_attributed_team(&entries, CURRENT_TEAM_UID, Some(VIEWER_UID)).is_empty()
    );
}

#[test]
fn filter_entries_by_attributed_team_always_keeps_the_viewers_own_usage() {
    // A workspace admin can be pointed at a team they don't belong to, and
    // their own usage is attributed elsewhere. Dropping it would show them
    // "Your usage: 0" on their own page. Mirrors the server's `isOwn`
    // carve-out in `filterEntriesToTeamScope`; showing you your own data is
    // not a leak.
    let entries = vec![
        entry_for_team(VIEWER_UID, Some(OTHER_TEAM_UID), 20),
        entry_for_team(VIEWER_UID, None, 5),
        entry_for_team(OTHER_UID, Some(OTHER_TEAM_UID), 999),
    ];

    let filtered = filter_entries_by_attributed_team(&entries, CURRENT_TEAM_UID, Some(VIEWER_UID));

    let credits: Vec<_> = filtered.iter().map(|e| e.credits_used).collect();
    assert_eq!(
        credits,
        vec![20, 5],
        "the viewer's own rows survive whatever they're attributed to, and \
         nobody else's out-of-scope usage comes with them"
    );
}

#[test]
fn filter_entries_by_attributed_team_own_exception_needs_a_real_viewer_uid() {
    // Signed-out / unidentified viewer: nothing can be proven to be "own", so
    // the exception must not widen into "keep everything". An empty uid is
    // treated the same way, matching the server's `callerUID != ""` guard.
    let entries = vec![
        entry_for_team(VIEWER_UID, Some(OTHER_TEAM_UID), 20),
        entry_for_team(OTHER_UID, Some(CURRENT_TEAM_UID), 10),
    ];

    for viewer_uid in [None, Some("")] {
        let filtered = filter_entries_by_attributed_team(&entries, CURRENT_TEAM_UID, viewer_uid);
        assert_eq!(
            filtered.iter().map(|e| e.credits_used).collect::<Vec<_>>(),
            vec![10],
            "only in-scope usage should survive for viewer_uid {viewer_uid:?}"
        );
    }
}

#[test]
fn filter_entries_by_attributed_team_own_exception_is_limited_to_user_subjects() {
    // A service account sharing the viewer's uid is not the viewer, so it
    // gets no carve-out. Matches the server, which checks the subject type.
    let entries = vec![BillingCycleUsageEntry {
        subject_type: AiCreditsUsageAndCostSubjectType::ServiceAccount,
        ..entry_for_team(VIEWER_UID, Some(OTHER_TEAM_UID), 20)
    }];

    assert!(
        filter_entries_by_attributed_team(&entries, CURRENT_TEAM_UID, Some(VIEWER_UID)).is_empty()
    );
}

#[test]
fn team_scoped_usage_narrows_entries_and_roster_together() {
    // The composition the section relies on: entries and roster scoped as a
    // pair, with legacy buckets dropped on the way through.
    let current_team = team(1, &[VIEWER_UID, OTHER_UID]);
    let other_team_uid = team_uid_str(&team(2, &[]));
    let current_team_uid = team_uid_str(&current_team);

    let entries = vec![
        entry_for_team(OTHER_UID, Some(current_team_uid.as_str()), 10),
        entry_for_team("third-uid", Some(other_team_uid.as_str()), 999),
        BillingCycleUsageEntry {
            usage_bucket: AiCreditsUsageBucket::Voice,
            ..entry_for_team(OTHER_UID, Some(current_team_uid.as_str()), 3)
        },
    ];
    let members = vec![
        workspace_member(VIEWER_UID),
        workspace_member(OTHER_UID),
        workspace_member("third-uid"),
    ];

    let scoped = TeamScopedUsage::new(&entries, &members, Some(&current_team), Some(VIEWER_UID));

    assert_eq!(
        scoped
            .entries
            .iter()
            .map(|e| e.credits_used)
            .collect::<Vec<_>>(),
        vec![10],
        "the other team's usage and the legacy Voice bucket are both dropped"
    );
    assert_eq!(
        scoped
            .members
            .iter()
            .map(|m| m.uid.as_string())
            .collect::<Vec<_>>(),
        vec![VIEWER_UID.to_string(), OTHER_UID.to_string()],
        "the third member belongs to another team and must not get a row"
    );
}

#[test]
fn team_scoped_usage_passes_everything_through_without_a_team() {
    // Personal / no-team viewers have nothing to scope against, so the
    // history and roster survive intact and the own-usage paths keep working.
    let entries = vec![
        entry_for_team(VIEWER_UID, None, 10),
        entry_for_team(VIEWER_UID, Some(OTHER_TEAM_UID), 20),
    ];
    let members = vec![workspace_member(VIEWER_UID)];

    let scoped = TeamScopedUsage::new(&entries, &members, None, Some(VIEWER_UID));

    assert_eq!(
        scoped
            .entries
            .iter()
            .map(|e| e.credits_used)
            .collect::<Vec<_>>(),
        vec![10, 20]
    );
    assert_eq!(scoped.members, members);
}

#[test]
fn scope_members_to_team_restricts_roster_to_team_members() {
    // Without this the workspace roster seeds zero-usage rows for members of
    // every other team in the workspace.
    let members = vec![
        workspace_member(VIEWER_UID),
        workspace_member(OTHER_UID),
        workspace_member("third-uid"),
    ];
    let team_members = vec![team_member(VIEWER_UID), team_member("third-uid")];

    let scoped = scope_members_to_team(&members, Some(&team_members));

    let uids: Vec<_> = scoped.iter().map(|m| m.uid.as_string()).collect();
    assert_eq!(uids, vec![VIEWER_UID.to_string(), "third-uid".to_string()]);
}

#[test]
fn scope_members_to_team_passes_roster_through_without_a_team() {
    // Personal / no-team viewers have nothing to scope against.
    let members = vec![workspace_member(VIEWER_UID), workspace_member(OTHER_UID)];

    assert_eq!(scope_members_to_team(&members, None), members);
}

#[test]
fn legend_cost_types_excludes_legacy_only_buckets() {
    // Voice / SuggestedCodeDiffs usage is written as BaseLimit credits but is
    // dropped from the bars; the legend reads the same already-filtered
    // entries the bars do, so it must not show "Base".
    let entries = vec![
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::Voice,
            AiCreditsUsageSource::Local,
            12,
            0,
        ),
        entry(
            AiCreditsUsageAndCostSubjectType::User,
            Some(VIEWER_UID),
            AiCreditsUsageAndCostType::BaseLimit,
            AiCreditsUsageBucket::SuggestedCodeDiffs,
            AiCreditsUsageSource::Local,
            4,
            0,
        ),
    ];

    assert!(
        legend_cost_types(&filter_legacy_buckets(&entries)).is_empty(),
        "legacy-only base-limit usage must not surface any legend bucket"
    );
}
