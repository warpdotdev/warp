use chrono::{Duration, Utc};

use super::*;

#[test]
fn usage_history_notifications_create_details_views() {
    use warpui::App;
    use warpui::platform::WindowStyle;

    use crate::test_util::terminal::initialize_app_for_terminal_view;

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let (_, page) = app.add_window(WindowStyle::NotStealFocus, BillingAndUsagePageV2View::new);
        let mut tools = serde_json::json!({
            "runCommandsExecuted": 0,
            "applyFileDiffStats": {"count": 0, "linesAdded": 0, "linesRemoved": 0, "filesChanged": 0}
        });
        for name in [
            "runCommandStats",
            "readFilesStats",
            "searchCodebaseStats",
            "grepStats",
            "fileGlobStats",
            "callMcpToolStats",
            "readMcpResourceStats",
            "suggestPlanStats",
            "suggestCreatePlanStats",
            "writeToLongRunningShellCommandStats",
            "readShellCommandOutputStats",
            "useComputerStats",
        ] {
            tools[name] = serde_json::json!({"count": 0});
        }
        let entry = serde_json::from_value(serde_json::json!({
            "conversationId": "history-entry", "title": "History entry", "lastUpdated": "2026-10-14T19:30:00Z",
            "usageMetadata": {
                "contextWindowUsage": 0, "contextWindowSegments": [], "creditsSpent": 0,
                "platformCreditsSpent": 0, "summarized": false, "tokenUsage": [],
                "totalBilledCostInCents": null, "totalPlatformCostInCents": null,
                "chargedUsageByCategory": null,
                "warpTokenUsage": [{"modelId": "gpt-5", "totalTokens": 100, "tokenUsageByCategory": []}],
                "byokTokenUsage": [], "toolUsageMetadata": tools
            }
        })).unwrap();
        page.update(&mut app, |page, ctx| {
            page.usage_history.model.update(ctx, |model, ctx| {
                model.set_entries_for_test(vec![entry], ctx)
            });
        });
        page.read(&app, |page, ctx| {
            let details = page
                .usage_history
                .details_views
                .get("history-entry")
                .expect("history notification must create its details view");
            let content = details
                .as_ref(ctx)
                .render(ctx)
                .debug_text_content()
                .unwrap();
            assert!(
                content.contains("INFERENCE USAGE") && content.contains("100 tokens"),
                "{content:?}"
            );
        });
    });
}

fn workspace_uid(id: i64) -> WorkspaceUid {
    WorkspaceUid::from(ServerId::from(id))
}

fn grant(scope: BonusGrantScope, grant_type: BonusGrantType, remaining: i32) -> BonusGrant {
    BonusGrant {
        created_at: Utc::now(),
        cost_cents: 0,
        expiration: None,
        grant_type,
        reason: "test".to_string(),
        user_facing_message: None,
        request_credits_granted: remaining,
        request_credits_remaining: remaining,
        usage_cents_granted: None,
        usage_cents_remaining: None,
        scope,
    }
}

fn grant_in_dollars(
    scope: BonusGrantScope,
    remaining: i32,
    usage_cents_remaining: f64,
) -> BonusGrant {
    BonusGrant {
        usage_cents_granted: Some(usage_cents_remaining),
        usage_cents_remaining: Some(usage_cents_remaining),
        ..grant(scope, BonusGrantType::Any, remaining)
    }
}

#[test]
fn classifies_grants_into_personal_team_and_workspace_buckets() {
    let current = workspace_uid(1);
    let grants = vec![
        grant(BonusGrantScope::User, BonusGrantType::Any, 10),
        grant(BonusGrantScope::Team(current), BonusGrantType::Any, 20),
        grant(BonusGrantScope::Workspace(current), BonusGrantType::Any, 30),
    ];

    let classified = ClassifiedGrants::new(&grants, Some(current));

    assert_eq!(classified.personal.total_balance(), 10);
    assert_eq!(classified.team.total_balance(), 20);
    assert_eq!(classified.workspace.total_balance(), 30);
    assert!(classified.has_any());
}

#[test]
fn excludes_grants_scoped_to_a_different_workspace() {
    let current = workspace_uid(1);
    let other = workspace_uid(2);
    let grants = vec![
        grant(BonusGrantScope::Team(other), BonusGrantType::Any, 20),
        grant(BonusGrantScope::Workspace(other), BonusGrantType::Any, 30),
    ];

    let classified = ClassifiedGrants::new(&grants, Some(current));

    assert!(classified.team.is_empty());
    assert!(classified.workspace.is_empty());
    assert!(!classified.has_any());
}

#[test]
fn hides_buckets_with_no_grants() {
    let current = workspace_uid(1);
    let grants = vec![grant(BonusGrantScope::User, BonusGrantType::Any, 10)];

    let classified = ClassifiedGrants::new(&grants, Some(current));

    assert!(!classified.personal.is_empty());
    assert!(classified.team.is_empty());
    assert!(classified.workspace.is_empty());
}

#[test]
fn bucket_balance_is_dollars_only_when_shown_in_dollars_with_every_grant_priced() {
    let current = workspace_uid(1);
    let dollars = ClassifiedGrants::new(
        &[
            grant_in_dollars(BonusGrantScope::Team(current), 10, 18.0),
            grant_in_dollars(BonusGrantScope::Team(current), 20, 36.0),
        ],
        Some(current),
    );
    assert_eq!(dollars.team.total_usage_cents_balance(), Some(54.0));
    assert_eq!(
        dollars.team.balance(UsageDisplayUnit::Dollars),
        BalanceAmount::Cents(54.0)
    );
    assert_eq!(dollars.team.total_balance(), 30);
    // Cents the server sends are never shown when displaying in credits.
    assert_eq!(
        dollars.team.balance(UsageDisplayUnit::Credits),
        BalanceAmount::Credits(30)
    );

    let mixed = ClassifiedGrants::new(
        &[
            grant_in_dollars(BonusGrantScope::Team(current), 10, 18.0),
            grant(BonusGrantScope::Team(current), BonusGrantType::Any, 20),
        ],
        Some(current),
    );
    assert_eq!(mixed.team.total_usage_cents_balance(), None);
    assert_eq!(
        mixed.team.balance(UsageDisplayUnit::Dollars),
        BalanceAmount::Credits(30)
    );
}

#[test]
fn balance_amount_formats_in_its_unit() {
    assert_eq!(BalanceAmount::Credits(1_500).format(), "1,500");
    assert_eq!(
        BalanceAmount::Credits(1_500).pool_label("Team"),
        "Team credits"
    );
    assert_eq!(BalanceAmount::Cents(1729.8).format(), "$17.30");
    assert_eq!(
        BalanceAmount::Cents(1729.8).pool_label("Team"),
        "Team usage"
    );
}

#[test]
fn base_allowance_balance_uses_dollars_when_shown_in_dollars() {
    assert_eq!(
        base_allowance_balance(
            UsageDisplayUnit::Dollars,
            1_000,
            39,
            false,
            Some(1800.0),
            Some(75.0)
        ),
        (
            BalanceAmount::Cents(1725.0),
            Some(BalanceAmount::Cents(1800.0))
        )
    );
    // Overspend never shows a negative balance.
    assert_eq!(
        base_allowance_balance(
            UsageDisplayUnit::Dollars,
            1_000,
            1_000,
            false,
            Some(1800.0),
            Some(1850.0)
        ),
        (
            BalanceAmount::Cents(0.0),
            Some(BalanceAmount::Cents(1800.0))
        )
    );
}

#[test]
fn base_allowance_balance_falls_back_to_credits() {
    assert_eq!(
        base_allowance_balance(UsageDisplayUnit::Dollars, 1_000, 39, false, None, None),
        (
            BalanceAmount::Credits(961),
            Some(BalanceAmount::Credits(1_000))
        )
    );
    // A missing used figure means the dollar balance is unknown.
    assert_eq!(
        base_allowance_balance(
            UsageDisplayUnit::Dollars,
            1_000,
            39,
            false,
            Some(1800.0),
            None
        ),
        (
            BalanceAmount::Credits(961),
            Some(BalanceAmount::Credits(1_000))
        )
    );
    // Cents the server sends are never shown when displaying in credits.
    assert_eq!(
        base_allowance_balance(
            UsageDisplayUnit::Credits,
            1_000,
            39,
            false,
            Some(1800.0),
            Some(75.0)
        ),
        (
            BalanceAmount::Credits(961),
            Some(BalanceAmount::Credits(1_000))
        )
    );
    // Unlimited subjects keep today's display.
    assert_eq!(
        base_allowance_balance(UsageDisplayUnit::Dollars, 999_999, 39, true, None, None),
        (BalanceAmount::Credits(999_960), None)
    );
    assert_eq!(
        base_allowance_balance(
            UsageDisplayUnit::Dollars,
            999_999,
            39,
            true,
            Some(1800.0),
            Some(70.2)
        ),
        (BalanceAmount::Credits(999_960), None)
    );
}

#[test]
fn excludes_ambient_expired_and_depleted_grants() {
    let current = workspace_uid(1);
    let mut expired = grant(BonusGrantScope::Team(current), BonusGrantType::Any, 5);
    expired.expiration = Some(Utc::now() - Duration::days(1));

    let grants = vec![
        // Ambient-only credits are surfaced separately, not as balance cards.
        grant(
            BonusGrantScope::Workspace(current),
            BonusGrantType::AmbientOnly,
            100,
        ),
        // A grant with no credits left should not render a card.
        grant(BonusGrantScope::Team(current), BonusGrantType::Any, 0),
        expired,
    ];

    let classified = ClassifiedGrants::new(&grants, Some(current));

    assert!(!classified.has_any());
}
