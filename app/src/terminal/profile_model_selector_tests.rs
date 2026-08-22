use std::sync::Arc;

use warpui::platform::WindowStyle;
use warpui::{App, EntityId, TypedActionView, ViewHandle};

use super::*;
use crate::ai::llms::{AvailableLLMs, DisableReason, LLMInfo, LLMProvider, ModelsByFeature};
use crate::menu::MenuItem;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};
use crate::workspaces::team::{Team, TeamVisibility};
use crate::workspaces::workspace::{
    ByoFirstPartyKey, ManagedByokByoePolicy, TeamByoSettings, Workspace,
};

fn team(uid: i64, name: &str) -> Team {
    Team {
        uid: uid.into(),
        name: name.to_string(),
        color: None,
        invite_link: None,
        members: vec![],
        pending_email_invites: vec![],
        invite_link_domain_restrictions: vec![],
        billing_metadata: Default::default(),
        stripe_customer_id: None,
        settings: Default::default(),
        is_eligible_for_discovery: false,
        has_billing_history: false,
        visibility: TeamVisibility::Open,
    }
}

fn workspace(teams: Vec<Team>) -> Workspace {
    Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_string(),
        stripe_customer_id: None,
        teams,
        billing_metadata: Default::default(),
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: None,
        has_billing_history: false,
        settings: Default::default(),
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members: vec![],
        total_requests_used_since_last_refresh: 0,
    }
}

fn model_item_disabled(
    selector: &ProfileModelSelector,
    model_id: &LLMId,
    app: &AppContext,
) -> bool {
    selector.model_dropdown.read(app, |menu, _| {
        menu.items()
            .iter()
            .find_map(|item| match (item, item.item_on_select_action()) {
                (
                    MenuItem::Item(fields),
                    Some(ProfileModelSelectorAction::SelectModel(item_id)),
                ) if item_id == model_id => Some(fields.is_disabled()),
                _ => None,
            })
            .expect("model should be present in the selector")
    })
}

fn setup_selector(
    app: &mut App,
) -> (
    ViewHandle<ProfileModelSelector>,
    EntityId,
    Workspace,
    Team,
    LLMId,
) {
    initialize_app_for_terminal_view(app);

    let mut team_a = team(111, "team-a");
    team_a.settings.team_byo = Some(TeamByoSettings {
        first_party_enabled: true,
        endpoints_enabled: false,
        allow_user_keys: false,
        allow_user_endpoints: false,
        first_party_keys: vec![ByoFirstPartyKey {
            provider: LLMProvider::Anthropic,
            credential_uid: "cred-a".to_string(),
        }],
        endpoints: vec![],
    });
    let team_b = team(222, "team-b");
    let mut workspace = workspace(vec![team_a.clone(), team_b.clone()]);
    workspace.billing_metadata.tier.managed_byok_byoe_policy =
        Some(ManagedByokByoePolicy { enabled: true });
    let workspace_uid = workspace.uid;

    UserWorkspaces::handle(app).update(app, |workspaces, ctx| {
        workspaces.update_workspaces(vec![workspace.clone()], ctx);
        workspaces.set_current_workspace_uid(workspace_uid, ctx);
    });

    let model_id = LLMId::from("claude-opus");
    let mut model = LLMInfo::new_for_test(model_id.as_str());
    model.display_name = "Opus".to_string();
    model.base_model_name = "Opus".to_string();
    model.provider = LLMProvider::Anthropic;
    model.disable_reason = Some(DisableReason::RequiresUpgrade);
    LLMPreferences::handle(app).update(app, |preferences, ctx| {
        preferences.update_feature_model_choices(
            Ok(ModelsByFeature {
                agent_mode: AvailableLLMs::new(
                    LLMId::from("auto"),
                    vec![LLMInfo::new_for_test("auto"), model],
                    None,
                )
                .expect("model choices should not be empty"),
                ..Default::default()
            }),
            ctx,
        );
    });

    let terminal = add_window_with_terminal(app, None);
    let terminal_view_id = terminal.id();
    let (input_model, terminal_model) = terminal.read(app, |terminal, _| {
        (terminal.ai_input_model().clone(), terminal.model.clone())
    });
    let (window_id, selector) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        ProfileModelSelector::new(
            Arc::new(MenuPositioning::AboveInputBox),
            terminal_view_id,
            input_model,
            None,
            terminal_model,
            None,
            ctx,
        )
    });
    UserWorkspaces::handle(app).update(app, |workspaces, ctx| {
        workspaces.set_team_for_window(window_id, team_a.uid, ctx);
    });

    (selector, terminal_view_id, workspace, team_b, model_id)
}

fn reassign_window_to_team_b(app: &mut App, mut workspace: Workspace, team_b: Team) {
    workspace.teams = vec![team_b];
    UserWorkspaces::handle(app).update(app, |workspaces, ctx| {
        workspaces.update_workspaces(vec![workspace], ctx);
    });
}

#[test]
fn model_menu_revalidates_presentation_after_team_change() {
    App::test((), |mut app| async move {
        let (selector, _, workspace, team_b, model_id) = setup_selector(&mut app);

        assert!(!selector.read(&app, |selector, app| {
            model_item_disabled(selector, &model_id, app)
        }));

        reassign_window_to_team_b(&mut app, workspace, team_b);

        assert!(selector.read(&app, |selector, app| {
            model_item_disabled(selector, &model_id, app)
        }));
    });
}

#[test]
fn model_selector_rejects_stale_selection_after_team_change() {
    App::test((), |mut app| async move {
        let (selector, terminal_view_id, workspace, team_b, model_id) = setup_selector(&mut app);
        let action = ProfileModelSelectorAction::SelectModel(model_id.clone());

        assert!(!selector.read(&app, |selector, app| {
            model_item_disabled(selector, &model_id, app)
        }));

        reassign_window_to_team_b(&mut app, workspace, team_b);
        selector.update(&mut app, |selector, ctx| {
            selector.handle_action(&action, ctx);
        });

        app.read(|ctx| {
            assert!(
                LLMPreferences::as_ref(ctx)
                    .get_base_llm_override(terminal_view_id)
                    .is_none()
            );
        });
    });
}
