use std::result::Result as StdResult;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::anyhow;
use warp_core::channel::IapConfig;
use warp_graphql::mutations::create_anonymous_user::{
    AnonymousUserType, CreateAnonymousUserResult,
};
use warp_server_auth::credentials::{FirebaseToken, LoginToken};
use warp_server_client::iap::PathResolver;

use super::*;
use crate::server::server_api::auth::{
    AuthClient, FetchUserResult, MintCustomTokenError, SyncedUserSettings, UserAuthenticationError,
};

#[test]
fn app_api_key_requires_validation() {
    let app = LaunchMode::App {
        args: Default::default(),
        api_key: Some("app-api-key".to_owned()),
    };

    assert!(matches!(
        app.auth_initialization(),
        AuthInitialization::PendingApiKey(api_key) if api_key == "app-api-key"
    ));
}

#[test]
fn tui_api_key_requires_validation() {
    let tui = LaunchMode::Tui {
        entrypoint: TuiEntryPoint::Interactive {
            mount: Box::new(|_| {}),
            api_key: Some("tui-api-key".to_owned()),
        },
    };

    assert!(matches!(
        tui.auth_initialization(),
        AuthInitialization::PendingApiKey(api_key) if api_key == "tui-api-key"
    ));
}

#[test]
fn command_line_api_key_requires_validation() {
    let command_line = LaunchMode::CommandLine {
        command: CliCommand::Whoami,
        global_options: GlobalOptions {
            api_key: Some("cli-api-key".to_owned()),
            ..Default::default()
        },
        debug: false,
        is_sandboxed: false,
        computer_use_override: None,
    };

    assert!(matches!(
        command_line.auth_initialization(),
        AuthInitialization::PendingApiKey(api_key) if api_key == "cli-api-key"
    ));
}

#[test]
fn startup_without_api_key_loads_persisted_auth() {
    let app = LaunchMode::App {
        args: Default::default(),
        api_key: None,
    };

    assert!(matches!(
        app.auth_initialization(),
        AuthInitialization::Persisted
    ));
}

#[test]
fn tui_uses_distinct_secure_storage_service_name() {
    let launch_mode = LaunchMode::Tui {
        entrypoint: TuiEntryPoint::Interactive {
            mount: Box::new(|_| {}),
            api_key: None,
        },
    };
    assert!(matches!(
        &launch_mode,
        LaunchMode::Tui {
            entrypoint: TuiEntryPoint::Interactive { .. }
        }
    ));

    assert_eq!(
        launch_mode.secure_storage_service_name("dev.warp.Warp-Dev"),
        "dev.warp.Warp-Dev.tui"
    );
}

#[test]
fn app_keeps_default_secure_storage_service_name() {
    let launch_mode = LaunchMode::App {
        args: Default::default(),
        api_key: None,
    };

    assert_eq!(
        launch_mode.secure_storage_service_name("dev.warp.Warp-Dev"),
        "dev.warp.Warp-Dev"
    );
}

#[test]
fn startup_auth_is_non_blocking_for_gui_and_tui() {
    // The GUI and TUI front-ends both skip the startup IAP wait: blocking here is
    // what deadlocks a cloud sandbox once its bootstrap JWT expires (see
    // warpdotdev/warp#15342). Every other launch mode keeps the blocking
    // behavior so this scope can't widen without a deliberate decision.
    let non_blocking_modes = [
        LaunchMode::App {
            args: Default::default(),
            api_key: None,
        },
        LaunchMode::Tui {
            entrypoint: TuiEntryPoint::Interactive {
                mount: Box::new(|_| {}),
                api_key: None,
            },
        },
    ];
    for mode in non_blocking_modes {
        assert!(
            startup_auth_is_non_blocking(&mode),
            "{} must not block startup auth on IAP",
            mode.as_str_for_tracing()
        );
    }

    let blocking_modes = [
        LaunchMode::CommandLine {
            command: CliCommand::Whoami,
            global_options: GlobalOptions::default(),
            debug: false,
            is_sandboxed: false,
            computer_use_override: None,
        },
        LaunchMode::Test {
            driver: Box::new(None),
            is_integration_test: false,
        },
        LaunchMode::RemoteServerProxy,
        LaunchMode::RemoteServerDaemon {
            identity_key: "test".to_owned(),
        },
    ];
    for mode in blocking_modes {
        assert!(
            !startup_auth_is_non_blocking(&mode),
            "{} must block startup auth on IAP",
            mode.as_str_for_tracing()
        );
    }
}

#[test]
fn retry_gate_fires_when_iap_ready_arrives_before_attempt_settles() {
    // Reproduces the reported race: IAP becomes ready while the optimistic
    // attempt is still in flight. The retry must wait for the attempt to
    // settle rather than firing immediately.
    let mut gate = StartupAuthRetryGate::default();
    assert!(!gate.on_iap_token_ready());
    assert!(gate.on_first_attempt_settled(false));
}

#[test]
fn retry_gate_fires_when_attempt_settles_before_iap_ready() {
    let mut gate = StartupAuthRetryGate::default();
    assert!(!gate.on_first_attempt_settled(false));
    assert!(gate.on_iap_token_ready());
}

#[test]
fn retry_gate_does_not_fire_when_first_attempt_already_authenticated() {
    for iap_ready_first in [true, false] {
        let mut gate = StartupAuthRetryGate::default();
        if iap_ready_first {
            assert!(!gate.on_iap_token_ready());
            assert!(!gate.on_first_attempt_settled(true));
        } else {
            assert!(!gate.on_first_attempt_settled(true));
            assert!(!gate.on_iap_token_ready());
        }
    }
}

#[test]
fn retry_gate_fires_at_most_once() {
    let mut gate = StartupAuthRetryGate::default();
    assert!(!gate.on_first_attempt_settled(false));
    assert!(gate.on_iap_token_ready());
    // A later proactive refresh can report `StateChanged` again; the retry
    // must not fire a second time.
    assert!(!gate.on_iap_token_ready());
}

#[test]
fn retry_gate_never_fires_if_iap_never_becomes_ready() {
    // Mirrors what happens once `IapManager` exhausts its retries: the gate
    // just sits idle rather than retrying or panicking.
    let mut gate = StartupAuthRetryGate::default();
    assert!(!gate.on_first_attempt_settled(false));
    assert!(!gate.retried);
}

/// A minimal `AuthClient` test double whose `fetch_user` can be held
/// genuinely pending under this test's explicit control. `MockAuthClient`
/// can't do this: its `.returning()` closures must produce the method's
/// result value directly rather than a future, so a mocked call always
/// resolves on its first poll - which would make the wiring test below
/// unable to distinguish "IAP ready while the first attempt is still in
/// flight" from "IAP ready after it already settled" (the latter is the
/// case the old, ungated implementation also handled correctly). Every
/// method this test doesn't exercise panics if called, the same failure
/// mode `MockAuthClient` gives for an unconfigured expectation.
struct PendingFirstCallAuthClient {
    call_count: Arc<AtomicUsize>,
    first_call_released: async_channel::Receiver<()>,
}

#[async_trait::async_trait]
impl AuthClient for PendingFirstCallAuthClient {
    async fn create_anonymous_user(
        &self,
        _referral_code: Option<String>,
        _anonymous_user_type: AnonymousUserType,
    ) -> anyhow::Result<CreateAnonymousUserResult> {
        unimplemented!("not exercised by this test")
    }

    async fn get_or_refresh_access_token(
        &self,
    ) -> anyhow::Result<warp_server_auth::credentials::AuthToken> {
        unimplemented!("not exercised by this test")
    }

    async fn fetch_user(
        &self,
        _token: LoginToken,
        _for_refresh: bool,
    ) -> StdResult<FetchUserResult, UserAuthenticationError> {
        let call_number = self.call_count.fetch_add(1, Ordering::SeqCst);
        if call_number == 0 {
            // Genuinely suspend here - not merely delay - until the test
            // explicitly releases it, so the IAP-ready `StateChanged` below
            // is guaranteed to arrive while this first attempt is in flight.
            let _ = self.first_call_released.recv().await;
        }
        Err(UserAuthenticationError::Unexpected(anyhow!(
            "blocked by IAP challenge"
        )))
    }

    async fn fetch_new_custom_token(
        &self,
    ) -> anyhow::Result<warp_graphql::mutations::mint_custom_token::MintCustomTokenResult> {
        unimplemented!("not exercised by this test")
    }

    fn on_custom_token_fetched(
        &self,
        _response: anyhow::Result<
            warp_graphql::mutations::mint_custom_token::MintCustomTokenResult,
        >,
    ) -> anyhow::Result<String, MintCustomTokenError> {
        unimplemented!("not exercised by this test")
    }

    async fn fetch_user_properties<'a>(
        &self,
        _auth_token: Option<&'a str>,
    ) -> anyhow::Result<warp_graphql::queries::get_user::UserOutput> {
        unimplemented!("not exercised by this test")
    }

    async fn get_user_settings(&self) -> anyhow::Result<Option<SyncedUserSettings>> {
        unimplemented!("not exercised by this test")
    }

    async fn set_is_telemetry_enabled(&self, _value: bool) -> anyhow::Result<()> {
        unimplemented!("not exercised by this test")
    }

    async fn set_is_crash_reporting_enabled(&self, _value: bool) -> anyhow::Result<()> {
        unimplemented!("not exercised by this test")
    }

    async fn set_is_cloud_conversation_storage_enabled(&self, _value: bool) -> anyhow::Result<()> {
        unimplemented!("not exercised by this test")
    }

    async fn update_user_settings(
        &self,
        _input: warp_graphql::mutations::update_user_settings::UpdateUserSettingsInput,
    ) -> anyhow::Result<()> {
        unimplemented!("not exercised by this test")
    }

    async fn set_user_is_onboarded(&self) -> anyhow::Result<bool> {
        unimplemented!("not exercised by this test")
    }

    async fn request_device_code(
        &self,
    ) -> StdResult<oauth2::StandardDeviceAuthorizationResponse, UserAuthenticationError> {
        unimplemented!("not exercised by this test")
    }

    async fn exchange_device_access_token(
        &self,
        _details: &oauth2::StandardDeviceAuthorizationResponse,
        _timeout: instant::Duration,
    ) -> StdResult<FirebaseToken, UserAuthenticationError> {
        unimplemented!("not exercised by this test")
    }

    async fn list_api_keys(
        &self,
    ) -> anyhow::Result<Vec<warp_graphql::queries::api_keys::ApiKeyProperties>> {
        unimplemented!("not exercised by this test")
    }

    async fn create_api_key(
        &self,
        _name: String,
        _team_id: Option<cynic::Id>,
        _agent_uid: Option<cynic::Id>,
        _expires_at: Option<warp_graphql::scalars::Time>,
    ) -> anyhow::Result<warp_graphql::mutations::generate_api_key::GenerateApiKeyResult> {
        unimplemented!("not exercised by this test")
    }

    async fn expire_api_key(
        &self,
        _key_uid: &warp_server_client::ids::ApiKeyUid,
    ) -> anyhow::Result<warp_graphql::mutations::expire_api_key::ExpireApiKeyResult> {
        unimplemented!("not exercised by this test")
    }

    async fn list_agent_identities(
        &self,
    ) -> anyhow::Result<Vec<warp_server_client::auth::AgentIdentity>> {
        unimplemented!("not exercised by this test")
    }
}

/// Exercises the real production wiring in `authenticate_user_after_iap_access`'s
/// non-blocking branch - the actual `AuthManager` and `IapManager` subscriptions,
/// not just `StartupAuthRetryGate` in isolation. A real `AuthManager` (backed by
/// `PendingFirstCallAuthClient`) and a real `IapManager` (with its background
/// gcloud refresh left permanently pending, so it can't race with the test)
/// reproduce the exact reported race deterministically: the first `fetch_user`
/// call is held genuinely pending - not merely delayed by mock/executor timing -
/// while the IAP-ready `StateChanged` arrives, so the test controls, rather than
/// merely hopes for, the "IAP ready before the first attempt settles" ordering.
/// Only after that is observed and asserted does the test release the first call
/// to fail, then asserts exactly one retry follows.
#[test]
fn non_blocking_startup_auth_retries_exactly_once_through_real_wiring() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        // `AuthStateProvider::new_for_test` defaults to an already-logged-in
        // test user, unlike the real cold-start scenario this test reproduces
        // (no session yet, no `user_id` until the optimistic attempt actually
        // completes). Clear it so a stale `user_id` can't hide a real retry
        // regression behind an "already authenticated" short-circuit.
        app.update(|ctx| {
            let auth_state = AuthStateProvider::as_ref(ctx).get();
            auth_state.set_user(None);
            auth_state.set_credentials(None);
        });

        let fetch_user_calls = Arc::new(AtomicUsize::new(0));
        let (release_first_call, first_call_released) = async_channel::bounded::<()>(1);
        let auth_client = PendingFirstCallAuthClient {
            call_count: fetch_user_calls.clone(),
            first_call_released,
        };

        let server_api = app.read(|ctx| ServerApiProvider::as_ref(ctx).get());
        app.add_singleton_model(move |ctx| {
            AuthManager::new(server_api, Arc::new(auth_client), ctx)
        });

        let iap_config = IapConfig {
            audiences: "test-audience".into(),
            service_account_email: "test-sa@example.com".into(),
        };
        let iap_state = Arc::new(IapState::new(&iap_config));
        let iap_state_for_test = iap_state.clone();
        app.add_singleton_model(move |ctx| {
            // Never resolves, so `IapManager`'s own background gcloud-refresh
            // attempt (which would fail anyway, since gcloud isn't installed)
            // can never reach far enough to race with this test's manual
            // `set_valid_token_for_test` + `StateChanged` below.
            let path_resolver: PathResolver =
                Box::new(|_ctx: &mut AppContext| Box::pin(futures::future::pending()));
            IapManager::new(Some(iap_state), path_resolver, None, ctx)
        });

        app.update(|ctx| {
            authenticate_user_after_iap_access(
                StartupUserAuthentication::ApiKey("fake-key".to_owned()),
                true,
                ctx,
            );
        });

        // Give the executor a chance to poll the optimistic `fetch_user` up to
        // its `recv().await` suspend point, then confirm it is genuinely still
        // outstanding (not settled) before proceeding.
        warpui::r#async::Timer::after(Duration::from_millis(50)).await;
        assert_eq!(
            fetch_user_calls.load(Ordering::SeqCst),
            1,
            "the optimistic fetch_user call must still be pending at this point"
        );

        // Now fire the IAP-ready signal while that first call is provably still
        // in flight - this is the exact race the gate exists to handle.
        iap_state_for_test.set_valid_token_for_test("fake-iap-token");
        app.update(|ctx| {
            IapManager::handle(ctx).update(ctx, |_, ctx| ctx.emit(IapManagerEvent::StateChanged));
        });

        // The retry must not have fired yet: the first attempt hasn't settled.
        warpui::r#async::Timer::after(Duration::from_millis(50)).await;
        assert_eq!(
            fetch_user_calls.load(Ordering::SeqCst),
            1,
            "no retry should fire while the first attempt is still pending"
        );

        // Release the first call so it can fail and settle.
        let _ = release_first_call.try_send(());
        warpui::r#async::Timer::after(Duration::from_millis(200)).await;
        assert_eq!(
            fetch_user_calls.load(Ordering::SeqCst),
            2,
            "expected exactly one retry (2 total fetch_user calls) once the first attempt settled"
        );

        // A later `StateChanged` (e.g. a proactive refresh) must not retry again.
        app.update(|ctx| {
            IapManager::handle(ctx).update(ctx, |_, ctx| ctx.emit(IapManagerEvent::StateChanged));
        });
        warpui::r#async::Timer::after(Duration::from_millis(200)).await;
        assert_eq!(
            fetch_user_calls.load(Ordering::SeqCst),
            2,
            "no further retries should fire"
        );
    });
}

#[test]
fn launch_modes_select_expected_logging_frontend() {
    let tui = LaunchMode::Tui {
        entrypoint: TuiEntryPoint::Interactive {
            mount: Box::new(|_| {}),
            api_key: None,
        },
    };
    let app = LaunchMode::App {
        args: Default::default(),
        api_key: None,
    };
    let test = LaunchMode::Test {
        driver: Box::new(None),
        is_integration_test: false,
    };

    assert_eq!(tui.log_frontend(), LogFrontend::Tui);
    assert_eq!(app.log_frontend(), LogFrontend::Gui);
    assert_eq!(test.log_frontend(), LogFrontend::Gui);
    assert_eq!(
        LaunchMode::RemoteServerProxy.log_frontend(),
        LogFrontend::Cli
    );
    assert_eq!(
        LaunchMode::RemoteServerDaemon {
            identity_key: "test".to_owned(),
        }
        .log_frontend(),
        LogFrontend::Cli
    );
}
