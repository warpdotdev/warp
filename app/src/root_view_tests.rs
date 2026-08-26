use ai::LLMId;
use onboarding::{
    AgentOnboardingView, OfferVariant, OnboardingAuthState, OnboardingIntention, SelectedSettings,
    UICustomizationSettings,
};
use session_sharing_protocol::common::SessionId;
use warp_core::features::FeatureFlag;
use warp_core::user_preferences::GetUserPreferences as _;
use warpui::elements::Empty;
use warpui::platform::WindowStyle;
use warpui::{
    App, AppContext, Element, Entity, EntityId, SingletonEntity, TypedActionView, View, ViewHandle,
};

#[cfg(not(target_family = "wasm"))]
use super::should_show_pre_login_onboarding_for;
use super::{
    AccountFirstCompletion, AuthOnboardingState, AuthOnboardingTarget,
    HAS_COMPLETED_ONBOARDING_KEY, NewWorkspaceSource, RootView, WorkspaceArgs,
    has_completed_local_onboarding, offer_variant_for_account_class,
    refresh_pending_onboarding_choices, requires_post_onboarding_login,
};
use crate::GlobalResourceHandles;
use crate::ai::agent::api::ServerConversationToken;
use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::login_slide::{LoginSlideSource, LoginSlideView};
use crate::experiments::{BlockOnboarding, Experiment};
use crate::server::server_api::ServerApiProvider;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::themes::onboarding_theme_picker_themes;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::FtueAccountClass;

fn initialize_app(app: &mut App) {
    app.update(crate::settings::init_and_register_user_preferences);
    app.add_singleton_model(|_ctx| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AuthManager::new_for_test);
}

#[test]
fn account_first_class_uses_paid_status_then_fresh_request_limit() {
    assert_eq!(
        RootView::account_first_class(true, Some(0)),
        FtueAccountClass::Paid
    );
    assert_eq!(
        RootView::account_first_class(true, Some(300)),
        FtueAccountClass::Paid
    );
    assert_eq!(
        RootView::account_first_class(true, None),
        FtueAccountClass::Paid
    );
    assert_eq!(
        RootView::account_first_class(false, Some(300)),
        FtueAccountClass::FreeIcp
    );
    assert_eq!(
        RootView::account_first_class(false, Some(0)),
        FtueAccountClass::FreeStandard
    );
    assert_eq!(
        RootView::account_first_class(false, None),
        FtueAccountClass::FreeStandard
    );
}

fn set_local_onboarding_completed(app: &mut App, completed: bool) {
    app.update(|ctx| {
        ctx.private_user_preferences()
            .write_value(
                HAS_COMPLETED_ONBOARDING_KEY,
                serde_json::to_string(&completed).unwrap(),
            )
            .unwrap();
    });
}

#[test]
fn account_first_requires_login_even_without_ai_or_drive_settings() {
    let _account_first = FeatureFlag::AccountFirstOnboarding.override_enabled(true);

    assert!(requires_post_onboarding_login(false, false, false));
    assert!(!requires_post_onboarding_login(true, false, false));
}

#[test]
fn fallback_flow_only_requires_login_for_account_backed_settings() {
    let _account_first = FeatureFlag::AccountFirstOnboarding.override_enabled(false);

    assert!(!requires_post_onboarding_login(false, false, false));
    assert!(requires_post_onboarding_login(false, true, false));
    assert!(requires_post_onboarding_login(false, false, true));
}

#[test]
fn account_first_classes_route_to_paid_or_the_expected_offer() {
    assert_eq!(
        offer_variant_for_account_class(FtueAccountClass::Paid),
        None
    );
    assert_eq!(
        offer_variant_for_account_class(FtueAccountClass::FreeIcp),
        Some(OfferVariant::HeadStart)
    );
    assert_eq!(
        offer_variant_for_account_class(FtueAccountClass::FreeStandard),
        Some(OfferVariant::ChooseHowToStart)
    );
}

#[test]
fn account_first_completion_metadata_matches_terminal_outcomes() {
    let cases = [
        (
            AccountFirstCompletion::AccountSkipped,
            "account_skipped",
            None,
            false,
        ),
        (
            AccountFirstCompletion::PaidTeam,
            "paid_team",
            Some(FtueAccountClass::Paid),
            true,
        ),
        (
            AccountFirstCompletion::FreeIcpSetupLater,
            "free_icp_setup_later",
            Some(FtueAccountClass::FreeIcp),
            true,
        ),
        (
            AccountFirstCompletion::FreeStandardSetupLater,
            "free_standard_setup_later",
            Some(FtueAccountClass::FreeStandard),
            true,
        ),
        (
            AccountFirstCompletion::FreeStandardCreditsPurchased,
            "free_standard_credits_purchased",
            // Buying one-time credits does not put the user on a plan, so they
            // stay free-standard.
            Some(FtueAccountClass::FreeStandard),
            true,
        ),
        (
            AccountFirstCompletion::UpgradeCompleted,
            "upgrade_completed",
            Some(FtueAccountClass::Paid),
            true,
        ),
    ];

    for (completion, completion_type, account_class, starts_agent_tutorial) in cases {
        assert_eq!(completion.completion_type(), completion_type);
        assert_eq!(completion.account_class(), account_class);
        assert_eq!(completion.starts_agent_tutorial(), starts_agent_tutorial);
    }
}

#[test]
fn refreshing_pending_onboarding_choices_replaces_stale_settings() {
    let settings = |use_vertical_tabs| SelectedSettings::Terminal {
        ui_customization: Some(UICustomizationSettings {
            use_vertical_tabs,
            show_conversation_history: false,
            show_project_explorer: true,
            show_global_search: false,
            show_warp_drive: false,
            show_code_review_button: true,
        }),
        cli_agent_toolbar_enabled: true,
        show_agent_notifications: false,
    };

    let mut pending_settings = Some(settings(false));
    let mut pending_tutorial = None;
    let latest_settings = settings(true);

    refresh_pending_onboarding_choices(
        &latest_settings,
        &mut pending_settings,
        &mut pending_tutorial,
    );

    let Some(SelectedSettings::Terminal {
        ui_customization: Some(ui),
        ..
    }) = pending_settings
    else {
        panic!("latest terminal settings should replace the pending snapshot");
    };
    assert!(ui.use_vertical_tabs);
    assert!(pending_tutorial.is_some());
}

/// Regression test for the bug fixed by introducing
/// `RootView::sync_local_onboarding_to_server`: when a user completed onboarding
/// pre-login and later authenticated via a non-login-slide entrypoint (i.e. while
/// already in `Terminal` state), the server-side `is_onboarded` flag was never
/// flipped. The helper runs unconditionally on `AuthComplete` and must flip the
/// flag when all preconditions hold.
#[test]
fn test_sync_flips_server_is_onboarded_when_local_onboarding_completed() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Seed the "has_completed_local_onboarding" preference and make the user
        // appear not yet onboarded on the server. The default test user is
        // non-anonymous, so the guards in the helper won't short-circuit.
        set_local_onboarding_completed(&mut app, true);
        app.update(|ctx| {
            AuthStateProvider::as_ref(ctx).get().set_is_onboarded(false);
            assert!(has_completed_local_onboarding(ctx));
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(false)
            );
        });

        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
            RootView::sync_local_onboarding_to_server(&auth_state, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(true),
                "sync should have invoked AuthManager::set_user_onboarded"
            );
        });
    });
}

/// If the user hasn't completed local onboarding, the helper must leave the
/// server-side flag untouched — onboarding hasn't actually happened yet.
#[test]
fn test_sync_noop_when_local_onboarding_not_completed() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Do not set HAS_COMPLETED_ONBOARDING_KEY; it defaults to false.
        app.update(|ctx| {
            AuthStateProvider::as_ref(ctx).get().set_is_onboarded(false);
        });

        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
            RootView::sync_local_onboarding_to_server(&auth_state, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(false),
                "sync should not have changed is_onboarded when local onboarding is incomplete"
            );
        });
    });
}

/// The server-side flag should also be left untouched when it is already set,
/// even if local onboarding is complete — avoids redundant server calls.
#[test]
fn test_sync_noop_when_already_onboarded_on_server() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        set_local_onboarding_completed(&mut app, true);
        app.update(|ctx| {
            // User::test() defaults to is_onboarded = true; assert that and
            // leave it in place.
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(true)
            );
        });

        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
            RootView::sync_local_onboarding_to_server(&auth_state, ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                AuthStateProvider::as_ref(ctx).get().is_onboarded(),
                Some(true)
            );
        });
    });
}

struct SsoLinkTestHarnessView {
    login_slide_view: ViewHandle<LoginSlideView>,
    onboarding_view: ViewHandle<AgentOnboardingView>,
}

impl Entity for SsoLinkTestHarnessView {
    type Event = ();
}

impl View for SsoLinkTestHarnessView {
    fn ui_name() -> &'static str {
        "SsoLinkTestHarnessView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for SsoLinkTestHarnessView {
    type Action = ();
}

/// Regression test: completing browser auth with `needs_sso_link = true` while
/// a pre-terminal onboarding state was showing (`Onboarding`, `LoginSlide`, or
/// `PostAuthOnboarding`) used to silently no-op in `show_needs_sso_link_view`,
/// leaving the UI stuck on the login slide ("Sign in on your browser to
/// continue") instead of showing the SSO blocker. Each of those states must
/// convert to `NeedsSsoLink` and preserve its target.
#[test]
fn test_show_needs_sso_link_view_blocks_pre_terminal_onboarding_states() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_ctx| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| KeybindingChangedNotifier::new());
        app.add_singleton_model(UserWorkspaces::default_mock);

        let (_, harness) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let login_slide_view = ctx.add_typed_action_view(|ctx| {
                LoginSlideView::new(
                    true,
                    false,
                    "Dark",
                    false,
                    OnboardingIntention::AgentDrivenDevelopment,
                    LoginSlideSource::OnboardingFlow,
                    ctx,
                )
            });
            let onboarding_view = ctx.add_typed_action_view(|ctx| {
                AgentOnboardingView::new(
                    onboarding_theme_picker_themes(),
                    false,
                    Vec::new(),
                    LLMId::from("auto"),
                    false,
                    OnboardingAuthState::LoggedOut,
                    ctx,
                )
            });
            SsoLinkTestHarnessView {
                login_slide_view,
                onboarding_view,
            }
        });

        let (login_slide_view, onboarding_view) = app.read(|ctx| {
            let harness = harness.as_ref(ctx);
            (
                harness.login_slide_view.clone(),
                harness.onboarding_view.clone(),
            )
        });

        fn workspace_target(app: &mut App) -> (AuthOnboardingTarget, EntityId) {
            let global_resource_handles = GlobalResourceHandles::mock(app);
            let marker = global_resource_handles.tips_completed.id();
            let target = AuthOnboardingTarget::Workspace(Box::new(WorkspaceArgs {
                global_resource_handles,
                server_time: None,
                workspace_setting: NewWorkspaceSource::Empty {
                    previous_active_window: None,
                    shell: None,
                },
            }));
            (target, marker)
        }

        fn assert_becomes_needs_sso_link(
            mut state: AuthOnboardingState,
            marker: EntityId,
            case: &str,
        ) {
            state.show_needs_sso_link_view();
            match state {
                AuthOnboardingState::NeedsSsoLink(AuthOnboardingTarget::Workspace(args)) => {
                    assert_eq!(
                        args.global_resource_handles.tips_completed.id(),
                        marker,
                        "{case}: the pre-login target must be preserved"
                    );
                }
                _ => panic!("{case}: expected transition to NeedsSsoLink"),
            }
        }

        let (target, marker) = workspace_target(&mut app);
        assert_becomes_needs_sso_link(
            AuthOnboardingState::LoginSlide {
                login_slide_view: login_slide_view.clone(),
                onboarding_view: onboarding_view.clone(),
                target,
            },
            marker,
            "LoginSlide",
        );

        let (target, marker) = workspace_target(&mut app);
        assert_becomes_needs_sso_link(
            AuthOnboardingState::Onboarding {
                onboarding_view: onboarding_view.clone(),
                target,
            },
            marker,
            "Onboarding",
        );

        let (target, marker) = workspace_target(&mut app);
        assert_becomes_needs_sso_link(
            AuthOnboardingState::PostAuthOnboarding {
                onboarding_view,
                target,
                account_class: FtueAccountClass::FreeStandard,
                upgrade_started: false,
            },
            marker,
            "PostAuthOnboarding",
        );
    });
}

/// Regression test: a logged-out user cold-starting a window on a shared-session or cloud-
/// conversation deep link used to be sent through pre-login onboarding whenever it hadn't
/// been completed locally, deferring the link behind the full FTUE instead of honoring it.
/// Deep links must skip that gate regardless of the onboarding flag or local completion
/// state; other window sources are unaffected.
#[cfg(not(target_family = "wasm"))]
#[test]
fn should_show_pre_login_onboarding_for_skips_content_deep_links() {
    let _agent_onboarding = FeatureFlag::AgentOnboarding.override_enabled(true);

    App::test((), |mut app| async move {
        app.update(crate::settings::init_and_register_user_preferences);
        set_local_onboarding_completed(&mut app, false);

        app.update(|ctx| {
            assert!(should_show_pre_login_onboarding_for(
                &NewWorkspaceSource::Empty {
                    previous_active_window: None,
                    shell: None,
                },
                ctx,
            ));
            assert!(!should_show_pre_login_onboarding_for(
                &NewWorkspaceSource::SharedSessionAsViewer {
                    session_id: SessionId::new(),
                },
                ctx,
            ));
            assert!(!should_show_pre_login_onboarding_for(
                &NewWorkspaceSource::FromCloudConversationId {
                    conversation_id: ServerConversationToken::new("token".to_string()),
                },
                ctx,
            ));
        });
    });
}

/// Regression test: constructs a real `RootView` (the actual cold-start entry point,
/// rather than calling `should_show_pre_login_onboarding_for` directly) for a logged-out
/// user opening a shared-session deep link, and asserts it does not land in
/// `AuthOnboardingState::Onboarding`.
#[test]
fn root_view_new_skips_onboarding_for_shared_session_cold_start() {
    let _agent_onboarding = FeatureFlag::AgentOnboarding.override_enabled(true);

    App::test((), |mut app| async move {
        crate::workspace::view::tests::initialize_app(&mut app);
        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get();
            auth_state.set_credentials(None);
            auth_state.set_user(None);
        });

        let global_resource_handles = GlobalResourceHandles::mock(&mut app);
        let session_id = SessionId::new();
        let (_, root_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            RootView::new(
                global_resource_handles,
                NewWorkspaceSource::SharedSessionAsViewer { session_id },
                ctx,
            )
        });

        app.read(|ctx| {
            assert!(
                !matches!(
                    root_view.as_ref(ctx).auth_onboarding_state,
                    AuthOnboardingState::Onboarding { .. }
                ),
                "a cold-started shared-session window must not enter onboarding"
            );
        });
    });
}

/// Returns the pending/live target carried by any pre-`Terminal` state that wraps one,
/// so entry-point tests can assert on it without re-matching every variant.
fn pending_target(state: &AuthOnboardingState) -> Option<&AuthOnboardingTarget> {
    match state {
        AuthOnboardingState::Onboarding { target, .. }
        | AuthOnboardingState::LoginSlide { target, .. }
        | AuthOnboardingState::PostAuthOnboarding { target, .. } => Some(target),
        AuthOnboardingState::NeedsSsoLink(target) => Some(target),
        _ => None,
    }
}

fn assert_pending_workspace_retargeted(
    target: &AuthOnboardingTarget,
    session_id: SessionId,
    case: &str,
) {
    let AuthOnboardingTarget::Workspace(args) = target else {
        panic!("{case}: expected a pending workspace target, found an existing terminal");
    };
    assert!(
        matches!(
            args.workspace_setting,
            NewWorkspaceSource::SharedSessionAsViewer { session_id: id } if id == session_id
        ),
        "{case}: should retarget to the requested session"
    );
}

fn pending_workspace_args(app: &mut App) -> Box<WorkspaceArgs> {
    Box::new(WorkspaceArgs {
        global_resource_handles: GlobalResourceHandles::mock(app),
        server_time: None,
        workspace_setting: NewWorkspaceSource::Empty {
            previous_active_window: None,
            shell: None,
        },
    })
}

/// Regression test: `RootView::join_shared_session_in_existing_window` used to silently
/// drop a shared-session deep link unless the window had already reached `Terminal`
/// ("Auth not complete before trying to join shared session"), and even after retargeting
/// was added, a pre-terminal state that wrapped an *existing* workspace (e.g. onboarding
/// shown over a normal session) still fell through that same drop, since there was no
/// pending `workspace_setting` left to retarget. Drives the real public action for every
/// pre-`Terminal` state, including that nested-`Terminal` case, which must join the
/// session immediately instead of being dropped.
#[test]
fn join_shared_session_in_existing_window_covers_pre_terminal_and_nested_terminal_states() {
    App::test((), |mut app| async move {
        crate::workspace::view::tests::initialize_app(&mut app);

        let global_resource_handles = GlobalResourceHandles::mock(&mut app);
        let (_, root_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            RootView::new(
                global_resource_handles,
                NewWorkspaceSource::Empty {
                    previous_active_window: None,
                    shell: None,
                },
                ctx,
            )
        });
        let session_id = SessionId::new();

        // Auth: the only variant that owns the pending workspace directly.
        let args = pending_workspace_args(&mut app);
        root_view.update(&mut app, |root_view, _| {
            root_view.auth_onboarding_state = AuthOnboardingState::Auth(args);
        });
        assert!(
            root_view.update(&mut app, |root_view, ctx| root_view
                .join_shared_session_in_existing_window(&session_id, ctx)),
            "Auth: expected the link to be handled"
        );
        app.read(|ctx| {
            let AuthOnboardingState::Auth(args) = &root_view.as_ref(ctx).auth_onboarding_state
            else {
                panic!("Auth: expected to remain in Auth");
            };
            assert!(matches!(
                args.workspace_setting,
                NewWorkspaceSource::SharedSessionAsViewer { session_id: id } if id == session_id
            ));
        });

        // Onboarding/LoginSlide/PostAuthOnboarding/NeedsSsoLink wrapping a pending workspace:
        // each retargets that pending workspace.
        let login_slide_view = root_view.update(&mut app, |_, ctx| {
            ctx.add_typed_action_view(|ctx| {
                LoginSlideView::new(
                    true,
                    false,
                    "Dark",
                    false,
                    OnboardingIntention::AgentDrivenDevelopment,
                    LoginSlideSource::OnboardingFlow,
                    ctx,
                )
            })
        });
        type BuildState = Box<dyn FnOnce(AuthOnboardingTarget) -> AuthOnboardingState>;
        let cases: Vec<(&str, BuildState)> = vec![
            (
                "Onboarding (pending)",
                Box::new({
                    let onboarding_view = root_view.update(&mut app, |_, ctx| {
                        RootView::create_agent_onboarding_view(ctx)
                    });
                    move |target| AuthOnboardingState::Onboarding {
                        onboarding_view,
                        target,
                    }
                }),
            ),
            (
                "LoginSlide (pending)",
                Box::new({
                    let onboarding_view = root_view.update(&mut app, |_, ctx| {
                        RootView::create_agent_onboarding_view(ctx)
                    });
                    move |target| AuthOnboardingState::LoginSlide {
                        login_slide_view,
                        onboarding_view,
                        target,
                    }
                }),
            ),
            (
                "PostAuthOnboarding (pending)",
                Box::new({
                    let onboarding_view = root_view.update(&mut app, |_, ctx| {
                        RootView::create_agent_onboarding_view(ctx)
                    });
                    move |target| AuthOnboardingState::PostAuthOnboarding {
                        onboarding_view,
                        target,
                        account_class: FtueAccountClass::FreeStandard,
                        upgrade_started: false,
                    }
                }),
            ),
            (
                "NeedsSsoLink (pending)",
                Box::new(AuthOnboardingState::NeedsSsoLink),
            ),
        ];
        for (case, build_state) in cases {
            let args = pending_workspace_args(&mut app);
            let target = AuthOnboardingTarget::Workspace(args);
            root_view.update(&mut app, |root_view, _| {
                root_view.auth_onboarding_state = build_state(target);
            });
            assert!(
                root_view.update(&mut app, |root_view, ctx| root_view
                    .join_shared_session_in_existing_window(&session_id, ctx)),
                "{case}: expected the link to be handled"
            );
            app.read(|ctx| {
                let state = &root_view.as_ref(ctx).auth_onboarding_state;
                let target = pending_target(state)
                    .unwrap_or_else(|| panic!("{case}: expected to remain pre-terminal"));
                assert_pending_workspace_retargeted(target, session_id, case);
            });
        }

        // Onboarding wrapping an *existing* (nested) Terminal workspace: this is the case
        // finding #1 fixed. It must join the session in that workspace directly, since
        // there is no pending `workspace_setting` to retarget.
        let nested_workspace = crate::workspace::view::tests::mock_workspace(&mut app);
        let tab_count_before = nested_workspace.read(&app, |workspace, _| workspace.tab_count());
        root_view.update(&mut app, |root_view, ctx| {
            let onboarding_view = RootView::create_agent_onboarding_view(ctx);
            root_view.auth_onboarding_state = AuthOnboardingState::Onboarding {
                onboarding_view,
                target: AuthOnboardingTarget::Terminal(nested_workspace.clone()),
            };
        });
        assert!(
            root_view.update(&mut app, |root_view, ctx| root_view
                .join_shared_session_in_existing_window(&session_id, ctx)),
            "Onboarding (nested Terminal): expected the link to be handled"
        );
        nested_workspace.read(&app, |workspace, _| {
            assert_eq!(
                workspace.tab_count(),
                tab_count_before + 1,
                "Onboarding (nested Terminal): the session should be joined in the existing workspace"
            );
        });

        // Terminal: joins directly, exercised through the same public entry point.
        let terminal_workspace = crate::workspace::view::tests::mock_workspace(&mut app);
        let tab_count_before = terminal_workspace.read(&app, |workspace, _| workspace.tab_count());
        root_view.update(&mut app, |root_view, _| {
            root_view.auth_onboarding_state =
                AuthOnboardingState::Terminal(terminal_workspace.clone());
        });
        assert!(
            root_view.update(&mut app, |root_view, ctx| root_view
                .join_shared_session_in_existing_window(&session_id, ctx)),
            "Terminal: expected the link to be handled"
        );
        terminal_workspace.read(&app, |workspace, _| {
            assert_eq!(workspace.tab_count(), tab_count_before + 1);
        });
    });
}

/// Regression test: on `AuthComplete`, the real dispatch in `handle_auth_manager_event` can
/// reach an already-`Terminal` state through the "skip onboarding survey if in Variant One"
/// branch (`BlockOnboarding::VariantOne`), which unconditionally calls
/// `complete_auth_and_create_workspace` again — this is the reported wasm sequence, where
/// `WebImport` has just transitioned to `Terminal` via `complete_web_import` (not exercised
/// here: that transition is `cfg(wasm)`-gated and cannot run in a native test binary, so the
/// resulting `Terminal` state is set up directly instead). That call used to wrap a
/// not-yet-onboarded user's live workspace in product onboarding regardless of what the
/// workspace was opened for. A workspace opened directly against a shared session must stay
/// in `Terminal`; a workspace with no such deep link is unaffected and still gets onboarding.
#[test]
fn auth_complete_in_variant_one_skips_onboarding_for_content_deep_link_terminal() {
    let _agent_onboarding = FeatureFlag::AgentOnboarding.override_enabled(true);
    // Force the "skip onboarding survey" experiment branch deterministically rather than
    // relying on the test user's anonymous-id hash.
    BlockOnboarding::set_override(BlockOnboarding::VariantOne);

    App::test((), |mut app| async move {
        crate::workspace::view::tests::initialize_app(&mut app);

        // Constructed while the test user's default `is_onboarded = true` holds, so each
        // workspace's own `check_and_trigger_onboarding` (which would otherwise mark the user
        // onboarded on construction) is a no-op here.
        let deep_link_workspace =
            crate::workspace::view::tests::mock_workspace_viewing_shared_session(&mut app);
        let plain_workspace = crate::workspace::view::tests::mock_workspace(&mut app);

        let global_resource_handles = GlobalResourceHandles::mock(&mut app);
        let (_, root_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            RootView::new(
                global_resource_handles,
                NewWorkspaceSource::Empty {
                    previous_active_window: None,
                    shell: None,
                },
                ctx,
            )
        });

        root_view.update(&mut app, |root_view, ctx| {
            AuthStateProvider::as_ref(ctx).get().set_is_onboarded(false);
            root_view.auth_onboarding_state =
                AuthOnboardingState::Terminal(deep_link_workspace.clone());
            root_view.handle_auth_manager_event(&AuthManagerEvent::AuthComplete, ctx);
        });
        app.read(|ctx| {
            let AuthOnboardingState::Terminal(workspace) =
                &root_view.as_ref(ctx).auth_onboarding_state
            else {
                panic!("a shared-session workspace must not be wrapped in onboarding");
            };
            assert_eq!(
                workspace.id(),
                deep_link_workspace.id(),
                "a shared-session workspace must not be replaced by onboarding"
            );
        });

        root_view.update(&mut app, |root_view, ctx| {
            AuthStateProvider::as_ref(ctx).get().set_is_onboarded(false);
            root_view.auth_onboarding_state =
                AuthOnboardingState::Terminal(plain_workspace.clone());
            root_view.handle_auth_manager_event(&AuthManagerEvent::AuthComplete, ctx);
        });
        app.read(|ctx| {
            assert!(
                matches!(
                    root_view.as_ref(ctx).auth_onboarding_state,
                    AuthOnboardingState::Onboarding { .. }
                ),
                "a workspace opened with no content deep link should still get onboarding"
            );
        });
    });
}
