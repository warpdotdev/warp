pub(super) mod user_persistence;

use std::result::Result as StdResult;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use settings::Setting as _;
use uuid::Uuid;
use warp_core::channel::ChannelState;
use warp_core::features::FeatureFlag;
use warp_graphql::mutations::create_anonymous_user::{
    AnonymousUserType, CreateAnonymousUserResult,
};
use warpui::{clipboard::ClipboardContent, Entity, ModelContext, SingletonEntity, UpdateModel};

use super::auth_state::{AuthState, PersistAction};
use super::auth_view_modal::{AuthRedirectPayload, AuthViewVariant};
use super::credentials::{Credentials, FirebaseToken, LoginToken};
use super::user::User;
use super::AuthStateProvider;
use super::UserUid;
use crate::ai::llms::LLMPreferences;
use crate::ai::persisted_workspace::PersistedWorkspace;
use crate::ai::AIRequestUsageModel;
use crate::autoupdate::AutoupdateState;
use crate::persistence::ModelEvent;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::server_api::auth::FetchUserResult;
use crate::server::server_api::ServerApiProvider;
use crate::server::{
    graphql::get_user_facing_error_message,
    server_api::{
        auth::{
            AnonymousUserCreationError, AuthClient, MintCustomTokenError, UserAuthenticationError,
        },
        ServerApi,
    },
    telemetry::AnonymousUserSignupEntrypoint,
};
use crate::settings::cloud_preferences_syncer::CloudPreferencesSyncer;
use crate::settings::initializer::SettingsInitializer;
use crate::settings::PrivacySettings;
use crate::terminal::general_settings::GeneralSettings;
use crate::terminal::shared_session::manager::Manager as SharedSessionManager;
#[cfg(target_family = "wasm")]
use crate::uri::browser_url_handler::{parse_current_url, update_browser_url};
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::{
    persistence, report_error, report_if_error, send_telemetry_from_ctx,
    send_telemetry_sync_from_ctx, GlobalResourceHandlesProvider, TelemetryEvent,
};
#[cfg(target_family = "wasm")]
use url::Url;
use user_persistence::PersistedUser;

#[derive(Debug)]
pub enum AuthManagerEvent {
    /// Successfully authenticated a user with no errors.
    AuthComplete,
    /// Failed to authenticate a user, due to a particular `UserAuthenticationError`.
    AuthFailed(UserAuthenticationError),
    /// Failed to create an anonymous user.
    CreateAnonymousUserFailed,
    /// The user chose to skip login entirely (no Firebase user created).
    SkippedLogin,
    /// The user now needs to reauthenticate. If the user needs to reauth, an `AuthFailed`
    /// event might be triggered instead, but there are some code paths where we don't
    /// refresh the entire user, only their token, which is when this event might be emitted.
    NeedsReauth,
    /// The user is anonymous and has attempted to access a login-gated feature or link.
    AttemptedLoginGatedFeature {
        auth_view_variant: AuthViewVariant,
    },
    // The current user is anonymous and the client has received a browser intent to sign in with a different Warp account.
    // Holds an auth payload from the received browser intent.
    LoginOverrideDetected(AuthRedirectPayload),
    /// Failed to mint a new custom token for an anonymous user.
    MintCustomTokenFailed(MintCustomTokenError),
    /// Received a device authorization code as part of the device auth flow.
    ReceivedDeviceAuthorizationCode {
        #[cfg_attr(target_family = "wasm", allow(unused))]
        verification_url: String,
        #[cfg_attr(target_family = "wasm", allow(unused))]
        verification_url_complete: Option<String>,
        #[cfg_attr(target_family = "wasm", allow(unused))]
        user_code: String,
    },
}

pub type LoginGatedFeature = &'static str;

type URLConstructorCallback = Box<dyn FnOnce(Option<&str>) -> String>;

/// AuthManager is a singleton model which manages the currently logged-in user's state.
/// If you need to access the state, use `AuthStateProvider`.
pub struct AuthManager {
    auth_state: Arc<AuthState>,
    server_api: Arc<ServerApi>,
    auth_client: Arc<dyn AuthClient>,
    /// A generated state token that the web app must provide back to the client.
    pending_auth_state: Option<String>,
}

impl AuthManager {
    /// Creates a new instance of the AuthManager. The auth state must already be initialized through
    /// [`AuthStateProvider`].
    pub fn new(
        server_api: Arc<ServerApi>,
        auth_client: Arc<dyn AuthClient>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();

        Self {
            auth_state,
            server_api,
            auth_client,
            pending_auth_state: None,
        }
    }

    #[cfg(test)]
    pub fn new_for_test(ctx: &mut ModelContext<Self>) -> Self {
        use crate::server::server_api::ServerApiProvider;

        let server_api = ServerApiProvider::as_ref(ctx).get();
        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();

        Self {
            auth_state,
            server_api: server_api.clone(),
            auth_client: server_api,
            pending_auth_state: None,
        }
    }

    /// Fetches and ultimately sets the user's auth state from an auth payload.
    /// Typically, this function is triggered when a user clicks the intent link from their browser
    /// back to Warp after login (or pastes the URL in the app).
    pub fn initialize_user_from_auth_payload(
        &mut self,
        auth_payload: AuthRedirectPayload,
        enforce_state_validation: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let AuthRedirectPayload {
            refresh_token,
            user_uid,
            deleted_anonymous_user,
            state,
        } = auth_payload.clone();

        if let Some(received_state) = &state {
            if !self.consume_auth_state(received_state) {
                if self.should_silently_ignore_stale_redirect(&user_uid) {
                    log::info!(
                        "Dropping auth redirect with stale state for already-logged-in user"
                    );
                    return;
                }
                ctx.emit(AuthManagerEvent::AuthFailed(
                    UserAuthenticationError::InvalidStateParameter,
                ));
                return;
            }
        } else if enforce_state_validation {
            if self.should_silently_ignore_stale_redirect(&user_uid) {
                log::info!("Dropping auth redirect without state for already-logged-in user");
                return;
            }
            ctx.emit(AuthManagerEvent::AuthFailed(
                UserAuthenticationError::MissingStateParameter,
            ));
            return;
        }

        let auth_client = self.auth_client.clone();

        if self.auth_state.is_user_anonymous().unwrap_or_default() {
            let incoming_user_matches_current_user = match user_uid {
                None => false,
                Some(incoming_user_uid) => self
                    .auth_state
                    .user_id()
                    .map(|current_user_uid| current_user_uid == incoming_user_uid)
                    .unwrap_or_default(),
            };
            if !incoming_user_matches_current_user && !deleted_anonymous_user.unwrap_or_default() {
                ctx.emit(AuthManagerEvent::LoginOverrideDetected(auth_payload));
                return;
            }
            send_telemetry_from_ctx!(TelemetryEvent::AnonymousUserLinkedFromBrowser, ctx);
        }

        let _ = ctx.spawn(
            async move {
                auth_client
                    .fetch_user(
                        LoginToken::Firebase(FirebaseToken::Refresh(refresh_token)),
                        false, /* for_refresh */
                    )
                    .await
            },
            Self::on_user_fetched,
        );
    }

    pub fn resume_interrupted_auth_payload(
        &mut self,
        auth_payload: AuthRedirectPayload,
        ctx: &mut ModelContext<Self>,
    ) {
        let AuthRedirectPayload {
            refresh_token,
            user_uid: _,
            deleted_anonymous_user: _,
            state: _,
        } = auth_payload;

        let auth_client = self.auth_client.clone();

        let _ = ctx.spawn(
            async move {
                auth_client
                    .fetch_user(
                        LoginToken::Firebase(FirebaseToken::Refresh(refresh_token)),
                        false, /* for_refresh */
                    )
                    .await
            },
            Self::on_user_fetched,
        );
    }

    #[cfg(target_family = "wasm")]
    pub fn initialize_user_from_session_cookie(&self, ctx: &mut ModelContext<Self>) {
        let auth_client = self.auth_client.clone();
        let _ = ctx.spawn(
            async move {
                auth_client
                    .fetch_user(LoginToken::SessionCookie, false)
                    .await
            },
            Self::on_user_fetched,
        );
    }

    /// Refreshes the user's auth state using their existing credentials.
    pub fn refresh_user(&self, ctx: &mut ModelContext<Self>) {
        let Some(credentials) = self.auth_state.credentials() else {
            log::warn!("Attempted to refresh user without credentials");
            return;
        };

        let Some(token) = credentials.login_token() else {
            log::info!("Attempted to refresh a user with no login token, skipping");
            return;
        };

        let auth_client = self.auth_client.clone();
        let _ = ctx.spawn(
            async move { auth_client.fetch_user(token, true).await },
            Self::on_user_fetched,
        );
    }

    /// Authenticate asynchronously using the OAuth2 device authorization flow.
    ///
    /// This is only used by the Warp CLI if running on a device that does not have the Warp app installed.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn authorize_device(&self, ctx: &mut ModelContext<Self>) {
        // Clear any stale user state so old credentials don't interfere
        // with the fresh device auth flow.
        self.auth_state.set_credentials(None);

        let auth_client = self.auth_client.clone();
        // Request a device code the user can enter in their browser.
        ctx.spawn(
            async move { auth_client.request_device_code().await },
            Self::on_device_code_received,
        );
    }

    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    fn on_device_code_received(
        &mut self,
        result: Result<oauth2::StandardDeviceAuthorizationResponse, UserAuthenticationError>,
        ctx: &mut ModelContext<Self>,
    ) {
        match result {
            Ok(details) => {
                // Emit the device authorization details so that they can be shown to the user.
                ctx.emit(AuthManagerEvent::ReceivedDeviceAuthorizationCode {
                    verification_url: details.verification_uri().to_string(),
                    verification_url_complete: details
                        .verification_uri_complete()
                        .map(|complete| complete.secret().to_string()),
                    user_code: details.user_code().secret().to_string(),
                });

                let auth_client = self.auth_client.clone();
                ctx.spawn(
                    async move {
                        // Wait for the user to approve the device authorization request.
                        let token = auth_client
                            .exchange_device_access_token(&details, Duration::from_secs(600))
                            .await?;

                        // Exchange the custom access token for Firebase auth tokens and fetch the user.
                        auth_client
                            .fetch_user(LoginToken::Firebase(token), false)
                            .await
                    },
                    Self::on_user_fetched,
                );
            }
            Err(err) => ctx.emit(AuthManagerEvent::AuthFailed(err)),
        }
    }

    /// Callback for handling a successful fetch of a user from warp-server and Firebase.
    /// This does the heavy-lifting of setting up all components of the application that depend
    /// on a user's authenticated state, and emits events to subscribers that let them know
    /// an auth event has occurred.
    fn on_user_fetched(
        &mut self,
        fetch_user_result: StdResult<FetchUserResult, UserAuthenticationError>,
        ctx: &mut ModelContext<Self>,
    ) {
        match fetch_user_result {
            Ok(fetch_user_result) => {
                let FetchUserResult {
                    user,
                    credentials,
                    server_experiments,
                    from_refresh,
                    llms,
                } = fetch_user_result;

                self.set_and_persist(Some(user.clone()), Some(credentials), ctx);

                self.set_needs_reauth(false, ctx);

                // Must be called on the main thread.
                #[cfg(feature = "crash_reporting")]
                crate::crash_reporting::set_user_id(
                    user.local_id,
                    Some(user.metadata.email.clone()),
                    ctx,
                );

                ServerApiProvider::handle(ctx).update(ctx, |provider, ctx| {
                    provider.handle_experiments_fetched(server_experiments, ctx);
                });

                SettingsInitializer::handle(ctx).update(ctx, |initializer, ctx| {
                    initializer.handle_user_fetched(self.auth_state.clone(), ctx);
                });

                // Reset the initial-load condition so that any cloud preference
                // sync waits for the *new* user's cloud objects rather than
                // resolving immediately against stale data from a prior session.
                // Only do this for non-refresh fetches (login/signup), not for
                // token refreshes where the user identity hasn't changed.
                if !from_refresh {
                    UpdateManager::handle(ctx).update(ctx, |manager, _| {
                        manager.reset_initial_load();
                    });
                }

                // Now that we have a user, start polling for team and cloud object information.
                // The polling loop's first tick fires immediately, so there is no need for a
                // separate out-of-band refresh here.
                TeamTesterStatus::handle(ctx).update(ctx, |model, ctx| {
                    model.initiate_data_pollers(false, ctx);
                });

                CloudPreferencesSyncer::handle(ctx).update(ctx, |model, ctx| {
                    model.handle_user_fetched(self.auth_state.clone(), ctx)
                });

                AIRequestUsageModel::handle(ctx).update(ctx, |usage_model, ctx| {
                    usage_model.refresh_request_usage_async(ctx);
                });

                LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
                    prefs.update_feature_model_choices(Ok(llms), ctx);
                });

                PersistedWorkspace::handle(ctx).update(ctx, |index_manager_updater, ctx| {
                    index_manager_updater.on_user_changed(ctx);
                });

                if !user.is_user_anonymous() {
                    GeneralSettings::handle(ctx).update(ctx, |settings, ctx| {
                        report_if_error!(settings
                            .did_non_anonymous_user_log_in
                            .set_value(true, ctx));
                    });
                }

                // Force refresh for shared sessions if user may have changed.
                if !from_refresh {
                    SharedSessionManager::handle(ctx).update(ctx, |manager, ctx| {
                        manager.stop_all_shared_sessions(ctx);
                        manager.rejoin_all_shared_sessions(ctx);
                    });
                }

                let global_resource_handles =
                    GlobalResourceHandlesProvider::as_ref(ctx).get().clone();

                // As part of Logout v0:
                // Reconstruct the database if it was removed.
                // Do nothing if the database was not removed.
                persistence::reconstruct(&global_resource_handles.model_event_sender);
                if let Some(model_event_sender) = &global_resource_handles.model_event_sender {
                    if let Err(e) =
                        model_event_sender.send(ModelEvent::UpsertCurrentUserInformation {
                            user_information: PersistedCurrentUserInformation {
                                email: self.auth_state.user_email().unwrap_or_default(),
                            },
                        })
                    {
                        log::error!("Error persisting user information to database: {e:?}");
                    };
                }

                // Fetch the user's privacy settings from the server if any or update the server settings.
                let privacy_settings_handle = PrivacySettings::handle(ctx);
                let privacy_settings_snapshot =
                    privacy_settings_handle.as_ref(ctx).get_snapshot(ctx);
                ctx.update_model(&privacy_settings_handle, |privacy_settings, ctx| {
                    privacy_settings.fetch_or_update_settings(ctx);
                });

                // Now that the user is logged in, do the daily version check.
                if FeatureFlag::Autoupdate.is_enabled() {
                    AutoupdateState::handle(ctx).update(ctx, |autoupdate_state, ctx| {
                        autoupdate_state.maybe_daily_check_for_update(ctx);
                    });
                }

                let server_api = self.server_api.clone();
                let user_id = self.auth_state.user_id().unwrap_or_default();
                let anonymous_id = self.auth_state.anonymous_id();
                let _ = ctx.spawn(
                    // Synchronously add the identify and login event to the telemetry event queue and
                    // then flush the queue to ensure the events get to Rudderstack. We need to do this
                    // one-off because the login event happens only once for the user and we don't want
                    // to drop the event if the user quits the app before the next flush of the queue.
                    // TODO(alokedesai): Investigate a more robust way of handling events
                    // that don't get flushed to Rudderstack outside of this event specifically.
                    async move {
                        warpui::telemetry::record_identify_user_event(
                            user_id.as_string(),
                            anonymous_id.clone(),
                            warpui::time::get_current_time(),
                        );
                        warpui::telemetry::record_event(
                            Some(user_id.as_string()),
                            anonymous_id,
                            TelemetryEvent::Login.name().into(),
                            TelemetryEvent::Login.payload(),
                            TelemetryEvent::Login.contains_ugc(),
                            warpui::time::get_current_time(),
                        );

                        // Note that this snapshot might get overwritten to disabled after the server fetch.
                        // However, it is still fine to flush to Rudderstack here as the login event is low-risk
                        // and it is better to err on the side of over-reporting than under-reporting.
                        if let Err(e) = server_api
                            .flush_telemetry_events(privacy_settings_snapshot)
                            .await
                        {
                            log::info!("Failed to flush events from Telemetry queue: {e}");
                        }
                        server_api.notify_login().await;
                    },
                    |_, _, _| {},
                );

                // Once the user is authenticated, attempt to report the sandbox that Warp is running in, if any.
                ctx.spawn(
                    async { warp_isolation_platform::detect() },
                    |_, platform, ctx| {
                        if let Some(platform) = platform {
                            send_telemetry_from_ctx!(
                                TelemetryEvent::DetectedIsolationPlatform { platform },
                                ctx
                            );
                        }
                    },
                );

                ctx.emit(AuthManagerEvent::AuthComplete);
            }
            Err(error) => {
                match error {
                    UserAuthenticationError::DeniedAccessToken(_) => {
                        self.set_needs_reauth(true, ctx);
                    }
                    UserAuthenticationError::UserAccountDisabled(_) => {}
                    UserAuthenticationError::Unexpected(_) => {}
                    UserAuthenticationError::InvalidStateParameter => {}
                    UserAuthenticationError::MissingStateParameter => {}
                }

                ctx.emit(AuthManagerEvent::AuthFailed(error));
            }
        }
    }

    /// Sets the user and credentials in auth state and persists to secure storage.
    /// Persistence depends on the credential type - currently, we only persist
    /// state if authenticated via a Firebase token.
    fn set_and_persist(
        &self,
        user: Option<User>,
        credentials: Option<Credentials>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.auth_state.set_user(user);
        self.auth_state.set_credentials(credentials);
        self.persist(ctx);
    }

    /// Persists (or removes) the current user and credentials to/from secure storage,
    /// based on the current auth state.
    fn persist(&self, ctx: &mut ModelContext<Self>) {
        match self.auth_state.persist_action() {
            PersistAction::Persist(persisted_user) => {
                if persisted_user.auth_tokens.refresh_token.is_empty() {
                    log::warn!("Skipping user persistence due to empty refresh token");
                    return;
                }
                let _ = persisted_user.write_to_secure_storage(ctx).map_err(|err| {
                    log::warn!("Unable to persist user to secure storage: {err:?}");
                });
            }
            PersistAction::Remove => {
                let _ = PersistedUser::remove_from_secure_storage(ctx).map_err(|err| {
                    log::warn!("Unable to clear user from secure storage: {err:?}");
                });
            }
            PersistAction::DoNothing => {}
        }
    }

    /// Helper function for logging out the user.
    /// NOTE: You probably want to call auth::log_out instead; this only manages the auth state,
    /// it doesn't shut down any other user-dependent parts of the app.
    /// TODO(jeff): Can we move those pieces in here?
    pub(super) fn log_out(&mut self, ctx: &mut ModelContext<Self>) {
        // Clear any dangling CSRF token from an auth flow that was started but never
        // completed before this logout, so it can't be replayed against the next session
        // in the same process.
        self.pending_auth_state = None;
        self.set_and_persist(None, None, ctx);
    }

    /// Sets whether or not this user's Firebase credentials are invalid and thus needs to reauth.
    pub fn set_needs_reauth(&self, needs_reauth: bool, ctx: &mut ModelContext<Self>) {
        let became_true = self.auth_state.set_needs_reauth(needs_reauth);

        if became_true {
            send_telemetry_from_ctx!(TelemetryEvent::NeedsReauth, ctx);
            ctx.emit(AuthManagerEvent::NeedsReauth);
        }
    }

    pub fn create_anonymous_user(
        &self,
        referral_code: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let anonymous_user_type = AnonymousUserType::NativeClientAnonymousUserFeatureGated;

        let auth_client = self.auth_client.clone();
        let _ = ctx.spawn(
            async move {
                auth_client
                    .create_anonymous_user(referral_code, anonymous_user_type)
                    .await
            },
            Self::on_create_anonymous_user,
        );
    }

    fn on_create_anonymous_user(
        &mut self,
        response: Result<CreateAnonymousUserResult>,
        ctx: &mut ModelContext<Self>,
    ) {
        let custom_token = match response {
            Ok(response_data) => match response_data {
                CreateAnonymousUserResult::CreateAnonymousUserOutput(output) => Ok(output.id_token),
                CreateAnonymousUserResult::UserFacingError(user_facing_error) => {
                    Err(AnonymousUserCreationError::UserFacingError(
                        get_user_facing_error_message(user_facing_error),
                    ))
                }
                CreateAnonymousUserResult::Unknown => Err(AnonymousUserCreationError::Unknown),
            },
            Err(_) => Err(AnonymousUserCreationError::CreationFailed),
        };

        match custom_token {
            Ok(custom_token) => {
                // Exchange the custom token for an ID token.
                let auth_client = self.auth_client.clone();
                let _ = ctx.spawn(
                    async move {
                        auth_client
                            .fetch_user(
                                LoginToken::Firebase(FirebaseToken::Custom(custom_token)),
                                false, /* for_refresh */
                            )
                            .await
                    },
                    Self::on_user_fetched,
                );
            }

            Err(err) => {
                report_error!(
                    anyhow!(err).context("Encountered an error trying to create anonymous users")
                );
                ctx.emit(AuthManagerEvent::CreateAnonymousUserFailed);
            }
        }
    }

    pub fn attempt_login_gated_feature(
        &self,
        feature: LoginGatedFeature,
        auth_view_variant: AuthViewVariant,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.auth_state.is_anonymous_or_logged_out() {
            send_telemetry_from_ctx!(
                TelemetryEvent::AnonymousUserAttemptLoginGatedFeature { feature },
                ctx
            );
            ctx.emit(AuthManagerEvent::AttemptedLoginGatedFeature { auth_view_variant });
        };
    }

    pub fn anonymous_user_hit_drive_object_limit(&self, ctx: &mut ModelContext<Self>) {
        if self.auth_state.is_anonymous_or_logged_out() {
            send_telemetry_from_ctx!(TelemetryEvent::AnonymousUserHitCloudObjectLimit, ctx);
            ctx.emit(AuthManagerEvent::AttemptedLoginGatedFeature {
                auth_view_variant: AuthViewVariant::HitDriveObjectLimitCloseable,
            });
        };
    }

    pub fn initiate_anonymous_user_linking(
        &self,
        entrypoint: AnonymousUserSignupEntrypoint,
        ctx: &mut ModelContext<Self>,
    ) {
        let auth_client = self.auth_client.clone();
        let _ = ctx.spawn(
            async move { auth_client.fetch_new_custom_token().await },
            move |me, response, ctx| {
                let custom_token = me.auth_client.on_custom_token_fetched(response);

                match custom_token {
                    Ok(custom_token) => {
                        // Send synchronously since this is an important event in the sign up funnel and we
                        // don't want to lose events if the user quits before the event queue is flushed.
                        send_telemetry_sync_from_ctx!(
                            TelemetryEvent::InitiateAnonymousUserSignup { entrypoint },
                            ctx
                        );
                        let login_options_url = me.login_options_url(&custom_token);
                        if cfg!(target_family = "wasm") {
                            #[cfg(target_family = "wasm")]
                            if let Some(current_url) = parse_current_url() {
                                update_browser_url(
                                    Url::parse(&format!(
                                        "{}?redirect_to={}",
                                        login_options_url,
                                        current_url.path()
                                    ))
                                    .ok(),
                                    true,
                                );
                            } else {
                                update_browser_url(Url::parse(&login_options_url).ok(), true);
                            }
                        } else {
                            ctx.open_url(&login_options_url);
                        }
                    }
                    Err(e) => {
                        ctx.emit(AuthManagerEvent::MintCustomTokenFailed(e));
                    }
                }
            },
        );
    }

    // Opens a page in the web app and logs the user in using a customToken if they are an anonymous user.
    // Accepts a callback that constructs the URL using the customToken to open a page and log in an anonymous user.
    pub fn open_url_maybe_with_anonymous_token(
        &self,
        ctx: &mut ModelContext<Self>,
        construct_url: URLConstructorCallback,
    ) {
        if !self.auth_state.is_user_anonymous().unwrap_or_default()
            || !self.auth_state.is_logged_in()
        {
            // Not an anonymous Firebase user, or fully logged out — open URL without token.
            let url: String = construct_url(None);
            ctx.open_url(&url);
            return;
        }

        let auth_client = self.auth_client.clone();
        let _ = ctx.spawn(
            async move { auth_client.fetch_new_custom_token().await },
            move |me, response, ctx| {
                let custom_token = me.auth_client.on_custom_token_fetched(response);
                match custom_token {
                    Ok(custom_token) => {
                        let url: String = construct_url(Some(&custom_token));
                        ctx.open_url(&url);
                    }
                    Err(e) => {
                        report_error!(anyhow!(
                        "Failed to fetch custom token for authenticating anonymous user in browser: {e:?}"
                    ))
                }
                };
            },
        );
    }

    pub fn copy_anonymous_user_linking_url_to_clipboard(&self, ctx: &mut ModelContext<Self>) {
        if !self.auth_state.is_user_anonymous().unwrap_or_default() {
            return;
        }
        let auth_client = self.auth_client.clone();
        let _ = ctx.spawn(
            async move { auth_client.fetch_new_custom_token().await },
            move |me, response, ctx| {
                let custom_token = me.auth_client.on_custom_token_fetched(response);

                match custom_token {
                    Ok(custom_token) => {
                        let login_options_url = me.login_options_url(&custom_token);
                        ctx.clipboard().write(ClipboardContent {
                            plain_text: login_options_url,
                            paths: None,
                            ..Default::default()
                        });
                    }
                    Err(e) => {
                        ctx.emit(AuthManagerEvent::MintCustomTokenFailed(e));
                    }
                };
            },
        );
    }

    /// Generates a unique state parameter for the authentication flow.
    fn generate_auth_state(&mut self) -> String {
        let state = Uuid::new_v4().to_string();
        self.pending_auth_state = Some(state.clone());
        state
    }

    pub fn sign_up_url(&mut self) -> String {
        let state = self.generate_auth_state();
        format!(
            // TODO: we should probably be able to remove the public_beta flag
            "{}/signup/remote?scheme={}&state={}&public_beta=true",
            ChannelState::server_root_url(),
            ChannelState::url_scheme(),
            state,
        )
    }

    pub fn sign_in_url(&mut self) -> String {
        let state = self.generate_auth_state();
        format!(
            "{}/login/remote?scheme={}&state={}",
            ChannelState::server_root_url(),
            ChannelState::url_scheme(),
            state,
        )
    }

    /// The upgrade confirmation page will kick the user back to the app with a refresh token
    /// if we send a `state` query param to /upgrade
    pub fn upgrade_url(&mut self) -> String {
        let state = self.generate_auth_state();
        format!(
            "{}/upgrade?scheme={}&state={}",
            ChannelState::server_root_url(),
            ChannelState::url_scheme(),
            state,
        )
    }

    pub fn login_options_url(&mut self, custom_token: &str) -> String {
        let state = self.generate_auth_state();
        format!(
            "{}/login_options/{}?state={}",
            ChannelState::server_root_url(),
            custom_token,
            state,
        )
    }

    pub fn link_sso_url(&mut self, email: &str) -> String {
        let state = self.generate_auth_state();
        format!(
            "{}/link_sso?email={}&state={}",
            ChannelState::server_root_url(),
            email,
            state,
        )
    }

    /// Validates and consumes the pending auth state token. Returns `true` if the
    /// provided state matches; in that case the pending state is cleared so the
    /// CSRF token is single-use. A subsequent call with the same value will fail.
    fn consume_auth_state(&mut self, received_state: &str) -> bool {
        if self.pending_auth_state.as_deref() == Some(received_state) {
            self.pending_auth_state = None;
            true
        } else {
            false
        }
    }

    /// Returns whether an auth redirect that failed state validation should be
    /// silently dropped rather than surfaced as an error. This covers the
    /// "user clicks the browser's 'Take me to Warp' button twice" case: once
    /// they're fully logged in, a second redirect targeting the same user is
    /// redundant and should not produce a user-visible error.
    fn should_silently_ignore_stale_redirect(&self, incoming_user_uid: &Option<UserUid>) -> bool {
        if self.auth_state.is_anonymous_or_logged_out() {
            return false;
        }
        match (self.auth_state.user_id(), incoming_user_uid) {
            (Some(current_uid), Some(incoming_uid)) => current_uid == *incoming_uid,
            _ => false,
        }
    }

    /// Sets the user as onboarded both on the server and locally.
    /// This method:
    /// 1. Updates the server by calling set_user_is_onboarded
    /// 2. Updates the local auth state and persists the user data
    pub fn set_user_onboarded(&self, ctx: &mut ModelContext<Self>) {
        // Update server
        let auth_client = self.auth_client.clone();
        let _ = ctx.spawn(
            async move { auth_client.set_user_is_onboarded().await },
            |_, _, _| {},
        );

        // Update local auth state and persist
        self.auth_state.set_is_onboarded(true);

        self.persist(ctx);
    }
}

#[derive(Clone, Debug)]
pub struct PersistedCurrentUserInformation {
    pub email: String,
}

impl Entity for AuthManager {
    type Event = AuthManagerEvent;
}

impl SingletonEntity for AuthManager {}

#[cfg(test)]
#[path = "auth_manager_tests.rs"]
mod auth_manager_test;
