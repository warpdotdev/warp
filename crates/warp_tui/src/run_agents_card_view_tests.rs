use std::cell::RefCell;
use std::rc::Rc;

use ai::agent::orchestration_config::{
    OrchestrationConfig, OrchestrationConfigStatus, OrchestrationExecutionMode,
};
use warp::tui_export::{
    register_orchestration_test_singletons, AIActionStatus, AIAgentAction, AIAgentActionId,
    AIAgentActionType, AuthSecretSelection, BlocklistAIActionModel, OptionRow, OptionSnapshot,
    OptionSourceStatus, RunAgentsAgentRunConfig, RunAgentsExecutionMode, RunAgentsRequest, TaskId,
};
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, ModelHandle};
use warpui_core::elements::tui::{Modifier, TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::{TuiFrame, TuiPresenter};
use warpui_core::{App, TypedActionView as _, ViewHandle, WindowInvalidation};

use super::{
    build_request, ConfigPage, TuiRunAgentsCardAction, TuiRunAgentsCardView,
    TuiRunAgentsCardViewEvent,
};
use crate::option_selector::TuiOptionSelectorAction;
use crate::test_fixtures::{
    add_active_test_conversation, add_test_action_model_with_surface, TestHostView,
};
use crate::tui_builder::TuiUiBuilder;

/// Builds a request with the given harness and execution mode.
fn request(harness: &str, execution_mode: RunAgentsExecutionMode) -> RunAgentsRequest {
    RunAgentsRequest {
        summary: "Parallelize the task.".to_string(),
        base_prompt: "base".to_string(),
        skills: Vec::new(),
        model_id: "auto".to_string(),
        harness_type: harness.to_string(),
        execution_mode,
        agent_run_configs: vec![RunAgentsAgentRunConfig {
            name: "researcher".to_string(),
            prompt: "research".to_string(),
            title: "Researcher".to_string(),
        }],
        plan_id: "plan-1".to_string(),
        harness_auth_secret_name: None,
    }
}

#[test]
fn only_the_model_page_is_searchable() {
    assert!(ConfigPage::Model.is_searchable());
    for page in [
        ConfigPage::Location,
        ConfigPage::Harness,
        ConfigPage::ApiKey,
        ConfigPage::Host,
        ConfigPage::Environment,
    ] {
        assert!(!page.is_searchable(), "{page:?}");
    }
}

/// A Cloud execution mode with the given env/host.
fn remote(environment_id: &str, worker_host: &str) -> RunAgentsExecutionMode {
    RunAgentsExecutionMode::Remote {
        environment_id: environment_id.to_string(),
        worker_host: worker_host.to_string(),
        computer_use_enabled: true,
    }
}

#[test]
fn local_collapses_the_page_sequence_to_two_pages() {
    let state = TuiRunAgentsCardView::config_state_from_request(
        &request("oz", RunAgentsExecutionMode::Local),
        None,
    );
    assert_eq!(
        TuiRunAgentsCardView::page_sequence(&state),
        vec![ConfigPage::Location, ConfigPage::Model],
    );
}

#[test]
fn cloud_oz_uses_five_pages_without_the_api_key_page() {
    let state = TuiRunAgentsCardView::config_state_from_request(
        &request("oz", remote("env-1", "warp")),
        None,
    );
    assert_eq!(
        TuiRunAgentsCardView::page_sequence(&state),
        vec![
            ConfigPage::Location,
            ConfigPage::Harness,
            ConfigPage::Host,
            ConfigPage::Environment,
            ConfigPage::Model,
        ],
    );
}

#[test]
fn cloud_managed_credential_harness_inserts_the_api_key_page() {
    let state = TuiRunAgentsCardView::config_state_from_request(
        &request("claude", remote("env-1", "warp")),
        None,
    );
    assert_eq!(
        TuiRunAgentsCardView::page_sequence(&state),
        vec![
            ConfigPage::Location,
            ConfigPage::Harness,
            ConfigPage::ApiKey,
            ConfigPage::Host,
            ConfigPage::Environment,
            ConfigPage::Model,
        ],
    );
}

#[test]
fn edit_state_carries_the_request_auth_secret() {
    let mut with_secret = request("claude", remote("env-1", "warp"));
    with_secret.harness_auth_secret_name = Some("work-key".to_string());
    let state = TuiRunAgentsCardView::config_state_from_request(&with_secret, None);
    assert_eq!(
        state.auth_secret_selection,
        AuthSecretSelection::Named("work-key".to_string()),
    );
    // Absence means "no choice yet", not Inherit.
    let state =
        TuiRunAgentsCardView::config_state_from_request(&request("claude", remote("", "")), None);
    assert_eq!(state.auth_secret_selection, AuthSecretSelection::Unset);
}

#[test]
fn edit_state_resolves_empty_fields_from_an_approved_config() {
    let mut inherit_all = request("", RunAgentsExecutionMode::Local);
    inherit_all.model_id = String::new();
    let config = OrchestrationConfig {
        model_id: "auto".to_string(),
        harness_type: "claude".to_string(),
        execution_mode: OrchestrationExecutionMode::Remote {
            environment_id: "env-2".to_string(),
            worker_host: "warp".to_string(),
        },
    };
    let state = TuiRunAgentsCardView::config_state_from_request(
        &inherit_all,
        Some(&(config.clone(), OrchestrationConfigStatus::Approved)),
    );
    assert_eq!(state.harness_type, "claude");
    assert_eq!(state.model_id, "auto");
    assert!(state.execution_mode.is_remote());

    // A disapproved config does not resolve inherited fields.
    let state = TuiRunAgentsCardView::config_state_from_request(
        &inherit_all,
        Some(&(config, OrchestrationConfigStatus::Disapproved)),
    );
    assert_eq!(state.harness_type, "");
    assert!(!state.execution_mode.is_remote());
}

#[test]
fn build_request_carries_card_fields_and_edited_run_wide_state() {
    let original = request("oz", remote("env-1", "warp"));
    let mut state = TuiRunAgentsCardView::config_state_from_request(&original, None);
    state.model_id = "gpt-5".to_string();
    state.harness_type = "codex".to_string();
    state.set_environment_id("env-9".to_string());
    state.set_worker_host("self-hosted".to_string());
    state.auth_secret_selection = AuthSecretSelection::Named("codex-key".to_string());

    let built = build_request(&original, &state);
    // Card fields pass through unchanged.
    assert_eq!(built.summary, original.summary);
    assert_eq!(built.base_prompt, original.base_prompt);
    assert_eq!(built.agent_run_configs, original.agent_run_configs);
    assert_eq!(built.plan_id, original.plan_id);
    // Run-wide fields come from the edited state; the per-call
    // computer-use flag is preserved through the round trip.
    assert_eq!(built.model_id, "gpt-5");
    assert_eq!(built.harness_type, "codex");
    assert_eq!(
        built.execution_mode,
        RunAgentsExecutionMode::Remote {
            environment_id: "env-9".to_string(),
            worker_host: "self-hosted".to_string(),
            computer_use_enabled: true,
        },
    );
    assert_eq!(built.harness_auth_secret_name.as_deref(), Some("codex-key"));
}

#[test]
fn build_request_omits_the_auth_secret_when_the_picker_is_not_applicable() {
    // A stale Named(_) selection must not leak into a Local dispatch.
    let original = request("claude", RunAgentsExecutionMode::Local);
    let mut state = TuiRunAgentsCardView::config_state_from_request(&original, None);
    state.auth_secret_selection = AuthSecretSelection::Named("stale".to_string());
    assert_eq!(
        build_request(&original, &state).harness_auth_secret_name,
        None
    );
}

// ── Blocked-card fixtures ────────────────────────────────────

type CapturedCardEvents = Rc<RefCell<Vec<TuiRunAgentsCardViewEvent>>>;

struct BlockedCard {
    card: ViewHandle<TuiRunAgentsCardView>,
    action_model: ModelHandle<BlocklistAIActionModel>,
    action_id: AIAgentActionId,
    events: CapturedCardEvents,
}

/// Queues `request` as a Blocked `RunAgents` action against the real action
/// model and constructs an interactive card for it.
fn blocked_card(app: &mut App, request: &RunAgentsRequest) -> BlockedCard {
    register_orchestration_test_singletons(app);
    let (action_model, _, terminal_surface_id) = add_test_action_model_with_surface(app);
    let conversation_id = add_active_test_conversation(app, terminal_surface_id);
    let action = AIAgentAction {
        id: AIAgentActionId::from("run-agents-1".to_string()),
        task_id: TaskId::new("task-1".to_string()),
        action: AIAgentActionType::RunAgents(request.clone()),
        requires_result: true,
    };
    let action_id = action.id.clone();
    action_model.update(app, |model, ctx| {
        model.queue_pending_action_for_test(conversation_id, action.clone(), ctx);
    });

    let card_action_model = action_model.clone();
    let request = request.clone();
    let card = app.update(|ctx| {
        let (window_id, _) = ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| TestHostView,
        );
        let run_agents_executor = card_action_model.as_ref(ctx).run_agents_executor(ctx);
        ctx.add_typed_action_tui_view(window_id, move |ctx| {
            TuiRunAgentsCardView::new(
                action,
                &request,
                None,
                card_action_model,
                run_agents_executor,
                Some("auto".to_string()),
                false,
                ctx,
            )
        })
    });
    let events: CapturedCardEvents = Rc::new(RefCell::new(Vec::new()));
    let events_for_subscription = events.clone();
    app.update(|ctx| {
        ctx.subscribe_to_view(&card, move |_, event, _| {
            events_for_subscription.borrow_mut().push(event.clone());
        });
    });
    BlockedCard {
        card,
        action_model,
        action_id,
        events,
    }
}

/// Renders the card through the real presenter so child views resolve.
fn render_card_frame(
    app: &mut App,
    card: &ViewHandle<TuiRunAgentsCardView>,
    width: u16,
) -> TuiFrame {
    let mut presenter = TuiPresenter::new();
    app.update(|ctx| {
        let window_id = card.window_id(ctx);
        // Mirror the runtime's draw: `invalidate` renders the card and its
        // selector into the presenter cache, then `present` resolves the
        // embedded selector via `TuiChildView`.
        let mut invalidation = WindowInvalidation::default();
        invalidation.updated.insert(card.id());
        invalidation.updated.insert(card.as_ref(ctx).selector.id());
        presenter.invalidate(&invalidation, ctx, window_id);
        presenter.present(ctx, card, TuiRect::new(0, 0, width, 60))
    })
}

/// Returns the card's trimmed rendered lines at `width`.
fn render_card_lines(
    app: &mut App,
    card: &ViewHandle<TuiRunAgentsCardView>,
    width: u16,
) -> Vec<String> {
    render_card_frame(app, card, width)
        .buffer
        .to_lines()
        .into_iter()
        .map(|line| line.trim_end().to_owned())
        .collect()
}

/// Dispatches a card action directly to the view.
fn act(app: &mut App, card: &ViewHandle<TuiRunAgentsCardView>, action: TuiRunAgentsCardAction) {
    card.update(app, |card, ctx| card.handle_action(&action, ctx));
}

/// The action's current status in the real action model.
fn action_status(app: &App, fixture: &BlockedCard) -> Option<AIActionStatus> {
    app.read(|app| {
        fixture
            .action_model
            .as_ref(app)
            .get_action_status(&fixture.action_id)
    })
}

#[test]
fn acceptance_card_renders_required_content_across_widths() {
    App::test((), |mut app| async move {
        let mut two_agents = request("oz", remote("", "warp"));
        two_agents.agent_run_configs.push(RunAgentsAgentRunConfig {
            name: "reviewer".to_string(),
            prompt: "review".to_string(),
            title: "Reviewer".to_string(),
        });
        let fixture = blocked_card(&mut app, &two_agents);
        for width in [40u16, 80, 132] {
            let lines = render_card_lines(&mut app, &fixture.card, width);
            let all = lines.join("\n");
            // question, agent names, and run-wide values.
            assert!(all.contains("Can I start"), "width {width}: {all}");
            assert!(all.contains("Agents (2):"), "width {width}");
            assert!(all.contains("researcher"), "width {width}");
            assert!(all.contains("reviewer"), "width {width}");
            assert!(all.contains("Location:"), "width {width}");
            assert!(all.contains("Cloud"), "width {width}");
            assert!(all.contains("Harness:"), "width {width}");
            assert!(all.contains("Model:"), "width {width}");
            assert!(all.contains("Host:"), "width {width}");
            assert!(all.contains("Environment:"), "width {width}");
            // The footer hints replace the input footer and wrap (rather
            // than clip) at narrow widths; compare whitespace-normalized
            // so a wrapped key name still matches.
            let flat = all.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(flat.contains("Enter to accept"), "width {width}: {all}");
            assert!(flat.contains("Ctrl + E to edit"), "width {width}: {all}");
            assert!(flat.contains("Ctrl + C to reject"), "width {width}: {all}");
        }
    });
}

#[test]
fn acceptance_card_matches_the_design_layout_and_styles() {
    App::test((), |mut app| async move {
        let mut seven_agents = request("oz", remote("", "warp"));
        seven_agents.agent_run_configs = [
            "infrastructure-bot",
            "ui-implementer",
            "dependency-bot",
            "verification-bot",
            "design-bot",
            "event-pipeline-monitor",
            "performance-regression-guard",
        ]
        .into_iter()
        .map(|name| RunAgentsAgentRunConfig {
            name: name.to_string(),
            prompt: "work".to_string(),
            title: name.to_string(),
        })
        .collect();
        let fixture = blocked_card(&mut app, &seven_agents);

        let lines = render_card_lines(&mut app, &fixture.card, 80);
        // Header row, blank body padding row, then the inset agent list.
        assert!(lines[0].starts_with(" ■ Can I start additional agents for this task?"));
        assert!(lines[1].trim().is_empty());
        assert!(lines[2].starts_with("   Agents (7):"));
        // The glyph is hash-assigned; assert the inset and the first name.
        assert!(lines[3].starts_with("   "), "{}", lines[3]);
        assert!(lines[3].contains("infrastructure-bot"), "{}", lines[3]);
        // The identity line wraps with muted bullet separators, and the
        // metadata renders as one inline row after a blank separator.
        assert!(lines[3].contains(" • "), "{}", lines[3]);
        let metadata = lines
            .iter()
            .find(|line| line.contains("Location: "))
            .expect("inline metadata row");
        assert!(metadata.contains("Location: Cloud"));
        assert!(metadata.contains(" • "));
        assert!(metadata.contains("Harness: "));

        let frame = render_card_frame(&mut app, &fixture.card, 80);
        let builder_styles = app.read(|app| {
            let builder = TuiUiBuilder::from_app(app);
            (
                builder.orchestration_header_background(),
                builder.orchestration_surface_background(),
            )
        });
        let (header_bg, surface_bg) = builder_styles;
        // Distinct header tint over the body tint; footer stays untinted.
        assert_ne!(header_bg, surface_bg);
        assert_eq!(frame.buffer[(0, 0)].bg, header_bg);
        assert_eq!(frame.buffer[(0, 1)].bg, surface_bg);
        let footer_row = render_card_lines(&mut app, &fixture.card, 80)
            .iter()
            .position(|line| line.contains("Enter to accept"))
            .expect("acceptance footer row") as u16;
        assert_ne!(frame.buffer[(0, footer_row)].bg, header_bg);
        assert_ne!(frame.buffer[(0, footer_row)].bg, surface_bg);
        // The row above the footer is an untinted margin row.
        assert_ne!(frame.buffer[(0, footer_row - 1)].bg, header_bg);
        assert_ne!(frame.buffer[(0, footer_row - 1)].bg, surface_bg);
        // The agent glyph and name share the identity color, with the
        // name bolded; identity colors are set (not default foreground).
        let glyph_cell = &frame.buffer[(3, 3)];
        let name_cell = &frame.buffer[(5, 3)];
        assert_eq!(glyph_cell.fg, name_cell.fg);
        assert!(name_cell.modifier.contains(Modifier::BOLD));
        assert!(!glyph_cell.modifier.contains(Modifier::BOLD));
    });
}

#[test]
fn agent_identities_stay_stable_across_rerenders_and_edits() {
    App::test((), |mut app| async move {
        let base = request("oz", RunAgentsExecutionMode::Local);
        let fixture = blocked_card(&mut app, &base);
        fn agent_line(app: &mut App, fixture: &BlockedCard) -> String {
            render_card_lines(app, &fixture.card, 80)
                .into_iter()
                .find(|line| line.contains("researcher"))
                .expect("agent row")
                .split("•")
                .find(|entry| entry.contains("researcher"))
                .expect("researcher entry")
                .trim()
                .to_string()
        }
        let before = agent_line(&mut app, &fixture);
        // Stable across plain re-renders…
        assert_eq!(before, agent_line(&mut app, &fixture));
        // …and across a streamed edit that appends an agent.
        let mut extended = base.clone();
        extended.agent_run_configs.push(RunAgentsAgentRunConfig {
            name: "reviewer".to_string(),
            prompt: "review".to_string(),
            title: "Reviewer".to_string(),
        });
        fixture.card.update(&mut app, |card, ctx| {
            card.update_request(&extended, ctx);
        });
        assert_eq!(before, agent_line(&mut app, &fixture));
    });
}

#[test]
fn accept_dispatches_through_the_action_model_exactly_once() {
    App::test((), |mut app| async move {
        let fixture = blocked_card(&mut app, &request("oz", RunAgentsExecutionMode::Local));
        assert!(matches!(
            action_status(&app, &fixture),
            Some(AIActionStatus::Blocked)
        ));
        assert!(app.read(|app| fixture.card.as_ref(app).wants_focus(app)));

        act(&mut app, &fixture.card, TuiRunAgentsCardAction::Accept);
        // The action left the Blocked queue through `execute_run_agents`
        // and the card stopped blocking the input.
        assert!(!matches!(
            action_status(&app, &fixture),
            Some(AIActionStatus::Blocked) | None
        ));
        assert!(app.read(|app| !fixture.card.as_ref(app).wants_focus(app)));

        // A second decision is a no-op: no reject can follow.
        act(&mut app, &fixture.card, TuiRunAgentsCardAction::Reject);
        assert!(!fixture
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, TuiRunAgentsCardViewEvent::RejectRequested)));
    });
}

#[test]
fn reject_emits_the_cancellation_event_exactly_once() {
    App::test((), |mut app| async move {
        let fixture = blocked_card(&mut app, &request("oz", RunAgentsExecutionMode::Local));
        act(&mut app, &fixture.card, TuiRunAgentsCardAction::Reject);
        act(&mut app, &fixture.card, TuiRunAgentsCardAction::Reject);
        let rejects = fixture
            .events
            .borrow()
            .iter()
            .filter(|event| matches!(event, TuiRunAgentsCardViewEvent::RejectRequested))
            .count();
        // Exactly one decision; the card stops blocking.
        assert_eq!(rejects, 1);
        assert!(app.read(|app| !fixture.card.as_ref(app).wants_focus(app)));
    });
}

#[test]
fn invalid_configurations_cannot_launch_and_surface_a_reason() {
    App::test((), |mut app| async move {
        // OpenCode + Cloud is a hard block.
        let fixture = blocked_card(&mut app, &request("opencode", remote("env-1", "warp")));
        act(&mut app, &fixture.card, TuiRunAgentsCardAction::Accept);
        // The card stays active and blocked, showing the reason.
        assert!(matches!(
            action_status(&app, &fixture),
            Some(AIActionStatus::Blocked)
        ));
        assert!(app.read(|app| fixture.card.as_ref(app).wants_focus(app)));
        let all = render_card_lines(&mut app, &fixture.card, 80).join("\n");
        assert!(all.contains("OpenCode is not supported on Cloud yet"));
    });
}

#[test]
fn configure_walks_pages_and_esc_returns_to_acceptance() {
    App::test((), |mut app| async move {
        let fixture = blocked_card(&mut app, &request("oz", remote("env-1", "warp")));
        act(&mut app, &fixture.card, TuiRunAgentsCardAction::Configure);
        let lines = render_card_lines(&mut app, &fixture.card, 80);
        let all = lines.join("\n");
        assert!(lines[0].contains("■ Can I start additional agents for this task?"));
        assert!(lines[1].trim().is_empty());
        assert!(lines[2].starts_with("   Edit agent configuration"));
        assert!(lines[2].contains("← 1 of 5 →"));
        assert!(lines[3].trim().is_empty());
        assert!(lines[4].starts_with("   Where should the agent run?"));
        assert!(lines[5].starts_with("   (1) Cloud"));
        assert!(!all.contains("Search:"));
        assert!(all.contains("Enter to accept"));
        assert!(all.contains("Tab or ← → to navigate"));
        assert!(all.contains("Esc to go back"));

        let frame = render_card_frame(&mut app, &fixture.card, 80);
        let (header_bg, surface_bg) = app.read(|app| {
            let builder = TuiUiBuilder::from_app(app);
            (
                builder.orchestration_header_background(),
                builder.orchestration_surface_background(),
            )
        });
        assert_eq!(frame.buffer[(0, 0)].bg, header_bg);
        assert_eq!(frame.buffer[(0, 2)].bg, surface_bg);
        let footer_row = lines
            .iter()
            .position(|line| line.contains("Enter to accept"))
            .expect("configuration footer row");
        assert_ne!(frame.buffer[(0, footer_row as u16)].bg, surface_bg);

        // Esc returns to the acceptance card without deciding.
        act(&mut app, &fixture.card, TuiRunAgentsCardAction::Back);
        let all = render_card_lines(&mut app, &fixture.card, 80).join("\n");
        assert!(all.contains("Enter to accept"));
        assert!(matches!(
            action_status(&app, &fixture),
            Some(AIActionStatus::Blocked)
        ));
    });
}

#[test]
fn scrolling_a_long_option_list_requests_a_card_remeasure() {
    App::test((), |mut app| async move {
        let fixture = blocked_card(&mut app, &request("oz", remote("env-1", "warp")));
        act(&mut app, &fixture.card, TuiRunAgentsCardAction::Configure);
        let selector = app.read(|app| fixture.card.as_ref(app).selector.clone());
        // Give the page more rows than the viewport so moving the selection
        // eventually scrolls and toggles an overflow marker.
        selector.update(&mut app, |selector, ctx| {
            let rows = (0..8)
                .map(|index| OptionRow {
                    id: format!("row-{index}"),
                    label: format!("row-{index}"),
                    harness: None,
                    badge: None,
                    disabled_reason: None,
                })
                .collect();
            selector.refresh_snapshot(
                OptionSnapshot {
                    rows,
                    selected_id: Some("row-0".to_string()),
                    status: OptionSourceStatus::Ready,
                    footer: None,
                },
                ctx,
            );
        });
        fixture.events.borrow_mut().clear();

        // Scrolling past the viewport reveals the `↑` marker: the card asks
        // its ancestors to re-measure so the taller card is not clipped.
        for _ in 0..6 {
            selector.update(&mut app, |selector, ctx| {
                selector.handle_action(&TuiOptionSelectorAction::MoveDown, ctx);
            });
        }
        assert!(fixture
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, TuiRunAgentsCardViewEvent::LayoutInvalidated)));
    });
}

#[test]
fn switching_to_local_mid_flow_collapses_the_sequence() {
    App::test((), |mut app| async move {
        let fixture = blocked_card(&mut app, &request("oz", remote("env-1", "warp")));
        act(&mut app, &fixture.card, TuiRunAgentsCardAction::Configure);
        // Select "Local" (second row) and confirm it: the sequence
        // collapses to Location, Model and advances to Model.
        let selector = app.read(|app| fixture.card.as_ref(app).selector.clone());
        selector.update(&mut app, |selector, ctx| {
            selector.handle_action(&TuiOptionSelectorAction::MoveDown, ctx);
        });
        act(
            &mut app,
            &fixture.card,
            TuiRunAgentsCardAction::ConfirmSelection,
        );
        let all = render_card_lines(&mut app, &fixture.card, 80).join("\n");
        assert!(all.contains("Which model should the agent use?"), "{all}");
        assert!(all.contains("2 of 2"), "{all}");
        assert!(all.contains("Search:"), "{all}");
    });
}

#[test]
fn horizontal_navigation_commits_the_selection_before_moving() {
    App::test((), |mut app| async move {
        let fixture = blocked_card(&mut app, &request("oz", remote("env-1", "warp")));
        act(&mut app, &fixture.card, TuiRunAgentsCardAction::Configure);
        let selector = app.read(|app| fixture.card.as_ref(app).selector.clone());

        // Left commits Local and clamps on the first page.
        selector.update(&mut app, |selector, ctx| {
            selector.handle_action(&TuiOptionSelectorAction::MoveDown, ctx);
        });
        act(
            &mut app,
            &fixture.card,
            TuiRunAgentsCardAction::CommitAndPreviousPage,
        );
        let all = render_card_lines(&mut app, &fixture.card, 80).join("\n");
        assert!(all.contains("Where should the agent run?"), "{all}");
        assert!(all.contains("← 1 of 2 →"), "{all}");
        assert!(app.read(|app| {
            !fixture
                .card
                .as_ref(app)
                .orchestration_edit_state
                .orchestration_config_state
                .execution_mode
                .is_remote()
        }));

        // Right commits Cloud, expands the sequence, and moves to Harness.
        selector.update(&mut app, |selector, ctx| {
            selector.handle_action(&TuiOptionSelectorAction::MoveUp, ctx);
        });
        act(
            &mut app,
            &fixture.card,
            TuiRunAgentsCardAction::CommitAndNextPage,
        );
        let all = render_card_lines(&mut app, &fixture.card, 80).join("\n");
        assert!(all.contains("Which harness should the agent use?"), "{all}");
        assert!(all.contains("← 2 of 5 →"), "{all}");
        assert!(app.read(|app| {
            fixture
                .card
                .as_ref(app)
                .orchestration_edit_state
                .orchestration_config_state
                .execution_mode
                .is_remote()
        }));
    });
}
