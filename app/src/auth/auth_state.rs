use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use anyhow::anyhow;
use chrono::{DateTime, Duration, Utc};
use parking_lot::RwLock;
use uuid::Uuid;
use warp_core::channel::{Channel, ChannelState};
use warp_graphql::object_permissions::OwnerType;
use warpui::{AppContext, Entity, SingletonEntity};

use crate::{
    cloud_object::{GenericStringObjectFormat, JsonObjectType, ObjectType},
    report_error,
};

use super::{
    anonymous_id::get_or_create_anonymous_id,
    auth_manager::user_persistence::PersistedUser,
    credentials::Credentials,
    user::{AnonymousUserType, FirebaseAuthTokens, PersonalObjectLimits, PrincipalType, User},
    UserUid, API_KEY_PREFIX,
};

#[cfg(any(not(target_family = "wasm"), test))]
use super::user::UserMetadata;

const ANONYMOUS_USER_NOTIFICATION_BLOCK_TIMER: Duration = Duration::days(7);

/// Describes what persistence action to take based on the current auth state.
pub(super) enum PersistAction {
    /// The user has Firebase credentials and should be persisted to secure storage.
    Persist(Box<PersistedUser>),
    /// The user has been logged out and should be removed from secure storage.
    Remove,
    /// No persistence action is needed (e.g. API key or test credentials).
    DoNothing,
}

/// AuthState holds information about the currently-logged in user.
/// If you need to access AuthState, you can use the AuthStateProvider singleton model.
pub struct AuthState {
    /// The currently logged-in User. None if the user isn't logged in currently.
    user: RwLock<Option<User>>,

    /// An anonymous UUID. Can be used to consistently identify an anonymous user who is not logged in.
    anonymous_id: Uuid,

    /// State that indicates whether the current user's refresh token has been
    /// invalidated, meaning a reauth is required.
    needs_reauth: AtomicBool,

    /// The current authentication credentials.
    credentials: RwLock<Option<Credentials>>,
}

impl AuthState {
    fn new(ctx: &AppContext) -> Self {
        Self {
            user: RwLock::new(None),
            anonymous_id: get_or_create_anonymous_id(ctx),
            needs_reauth: AtomicBool::new(false),
            credentials: RwLock::new(None),
        }
    }

    #[cfg(any(test, feature = "integration_tests"))]
    pub fn new_for_test() -> Self {
        Self {
            user: RwLock::new(Some(User::test())),
            anonymous_id: Uuid::new_v4(),
            needs_reauth: AtomicBool::new(false),
            credentials: RwLock::new(Some(Credentials::Test)),
        }
    }
    #[cfg(test)]
    pub fn new_logged_out_for_test() -> Self {
        Self {
            user: RwLock::new(None),
            anonymous_id: Uuid::new_v4(),
            needs_reauth: AtomicBool::new(false),
            credentials: RwLock::new(None),
        }
    }

    #[cfg(test)]
    pub fn new_anonymous_for_test() -> Self {
        use super::user::AnonymousUserType;
        Self {
            user: RwLock::new(Some(User {
                anonymous_user_type: Some(AnonymousUserType::NativeClientAnonymousUserFeatureGated),
                ..User::test()
            })),
            anonymous_id: Uuid::new_v4(),
            needs_reauth: AtomicBool::new(false),
            credentials: RwLock::new(Some(Credentials::Test)),
        }
    }

    /// Creates and initializes auth state. Checks, in order:
    /// 1. Test user (test/integration/skip_login builds)
    /// 2. Provided API key
    /// 3. WARP_USER_SECRET environment variable
    /// 4. Persisted user from secure storage
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn initialize(ctx: &AppContext, api_key: Option<String>) -> Self {
        let state = Self::new(ctx);

        if Self::should_use_test_user() {
            state.set_user(Some(User::test()));
            #[cfg(any(test, feature = "integration_tests", feature = "skip_login"))]
            state.set_credentials(Some(Credentials::Test));
            return state;
        }

        if let Some(api_key_value) = api_key {
            log::info!("Authenticating via API key");
            let formatted = if api_key_value.starts_with(API_KEY_PREFIX) {
                api_key_value
            } else {
                format!("{API_KEY_PREFIX}{api_key_value}")
            };
            state.set_credentials(Some(Credentials::ApiKey {
                key: formatted,
                owner_type: None,
            }));
            return state;
        }

        // Try WARP_USER_SECRET environment variable.
        if let Some(persisted) = option_env!("WARP_USER_SECRET")
            .and_then(|s| serde_json::from_str::<PersistedUser>(s).ok())
        {
            state.apply_persisted_user(persisted);
            return state;
        }

        // Try reading from secure storage.
        match PersistedUser::from_secure_storage(ctx) {
            Ok(persisted) => {
                if persisted.auth_tokens.refresh_token.is_empty() {
                    log::warn!(
                        "Found persisted user with empty refresh token; clearing secure storage entry"
                    );
                    let _ = PersistedUser::remove_from_secure_storage(ctx).map_err(|err| {
                        log::warn!("Unable to clear invalid user from secure storage: {err:?}");
                    });
                } else {
                    state.apply_persisted_user(persisted);
                }
            }
            Err(err) => {
                log::info!("Unable to read user from secure storage: {err:?}");
            }
        }

        state
    }

    fn should_use_test_user() -> bool {
        cfg!(any(test, feature = "skip_login")) || ChannelState::channel() == Channel::Integration
    }

    /// Determines the appropriate persistence action based on the current auth state.
    pub(super) fn persist_action(&self) -> PersistAction {
        let user = self.user.read().clone();
        let credentials = self.credentials.read().clone();

        match (user, credentials) {
            (Some(user), Some(Credentials::Firebase(firebase_tokens))) => {
                let anonymous_user_type = user.anonymous_user_type();
                let linked_at = user.linked_at();
                let personal_object_limits = user.personal_object_limits();

                #[allow(deprecated)]
                let persisted = PersistedUser {
                    auth_tokens: firebase_tokens,
                    refresh_token: String::new(),
                    local_id: user.local_id,
                    metadata: user.metadata,
                    is_onboarded: user.is_onboarded,
                    needs_sso_link: user.needs_sso_link,
                    anonymous_user_type,
                    linked_at,
                    personal_object_limits,
                    is_on_work_domain: user.is_on_work_domain,
                };
                PersistAction::Persist(Box::new(persisted))
            }
            // Remove persisted auth state if it is unset in-memory.
            (None, None) => PersistAction::Remove,
            // Do not persist if using API keys, session cookies, or test credentials.
            (Some(_), Some(Credentials::ApiKey { .. })) => PersistAction::DoNothing,
            (Some(_), Some(Credentials::Bearer(_))) => PersistAction::DoNothing,
            (Some(_), Some(Credentials::SessionCookie)) => PersistAction::DoNothing,
            #[cfg(any(test, feature = "integration_tests", feature = "skip_login"))]
            (Some(_), Some(Credentials::Test)) => PersistAction::DoNothing,
            // Credentials without a user, or user without credentials - transient states
            // during initialization or refresh; no persistence action needed.
            (None, Some(_)) | (Some(_), None) => PersistAction::DoNothing,
        }
    }

    /// Applies a deserialized PersistedUser, splitting it into User and Credentials.
    fn apply_persisted_user(&self, persisted: PersistedUser) {
        let user = User {
            is_onboarded: persisted.is_onboarded,
            local_id: persisted.local_id,
            metadata: persisted.metadata,
            needs_sso_link: persisted.needs_sso_link,
            anonymous_user_type: persisted.anonymous_user_type,
            is_on_work_domain: persisted.is_on_work_domain,
            linked_at: persisted.linked_at,
            personal_object_limits: persisted.personal_object_limits,
            principal_type: PrincipalType::default(),
            global_skills: Vec::new(),
        };
        *self.user.write() = Some(user);

        if persisted.auth_tokens.refresh_token.is_empty() {
            log::warn!("Skipping credentials update due to empty refresh token");
            return;
        }
        *self.credentials.write() = Some(Credentials::Firebase(persisted.auth_tokens));
    }

    /// Sets the user. This should only be called by the AuthManager, to ensure
    /// side-effects are handled properly (e.g. notifying other models, persisting
    /// the user to secure storage, etc.).
    pub(super) fn set_user(&self, user: Option<User>) {
        *self.user.write() = user;
    }

    /// Returns the current credentials.
    pub fn credentials(&self) -> Option<Credentials> {
        self.credentials.read().clone()
    }

    /// Sets the credentials. Should only be called within the auth module.
    pub(super) fn set_credentials(&self, credentials: Option<Credentials>) {
        *self.credentials.write() = credentials;
    }
    /// Applies auth data received by the remote server daemon handshake.
    ///
    /// Empty values are authoritative: an empty token clears bearer credentials, and an empty user
    /// ID clears the daemon user identity.
    #[cfg(any(not(target_family = "wasm"), test))]
    pub(crate) fn apply_remote_server_auth_context(
        &self,
        auth_token: String,
        user_id: String,
        user_email: String,
    ) {
        self.set_remote_server_bearer_token(auth_token);
        self.set_remote_server_user(user_id, user_email);
    }

    #[cfg(any(not(target_family = "wasm"), test))]
    pub(crate) fn set_remote_server_bearer_token(&self, auth_token: String) {
        if auth_token.is_empty() {
            self.set_credentials(None);
            return;
        }
        self.set_credentials(Some(Credentials::Bearer(auth_token)));
    }

    #[cfg(any(not(target_family = "wasm"), test))]
    fn set_remote_server_user(&self, user_id: String, user_email: String) {
        let mut user = self.user.write();
        if user_id.is_empty() {
            *user = None;
            return;
        }

        match user.as_mut() {
            Some(user) => {
                user.local_id = UserUid::new(&user_id);
                user.metadata.email = user_email;
            }
            None => {
                *user = Some(User {
                    local_id: UserUid::new(&user_id),
                    metadata: UserMetadata {
                        email: user_email,
                        display_name: None,
                        photo_url: None,
                    },
                    is_onboarded: false,
                    needs_sso_link: false,
                    anonymous_user_type: None,
                    is_on_work_domain: false,
                    linked_at: None,
                    personal_object_limits: None,
                    principal_type: PrincipalType::default(),
                    global_skills: Vec::new(),
                });
            }
        }
    }

    /// Updates the Firebase auth tokens within the current credentials.
    /// Reports an error if the current credentials are not Firebase.
    pub(crate) fn update_firebase_tokens(&self, new_auth_tokens: FirebaseAuthTokens) {
        let mut write_lock = self.credentials.write();
        if let Some(Credentials::Firebase(tokens)) = write_lock.as_mut() {
            *tokens = new_auth_tokens;
        } else {
            report_error!(anyhow!(
                "Tried to update Firebase tokens without Firebase credentials"
            ));
        }
    }

    /// Determines whether the user should be considered as logged in.
    pub fn is_logged_in(&self) -> bool {
        self.credentials.read().is_some()
    }

    /// Returns whether the user should be treated as not having a full account.
    /// True if the user is anonymous OR if there is no user at all (fully logged out).
    ///
    /// Note: uses `unwrap_or(true)` intentionally (not `unwrap_or_default()`) so that
    /// during the transient state where credentials exist but user data hasn't loaded
    /// yet, the user is conservatively treated as lacking a full account.
    pub fn is_anonymous_or_logged_out(&self) -> bool {
        !self.is_logged_in() || self.is_user_anonymous().unwrap_or(true)
    }

    /// Returns the cached access token, if any exists. This method *will not* check if the JWT is
    /// still valid! Usually, you want to use [`ServerApi::get_or_refresh_access_token`] instead!
    pub fn get_access_token_ignoring_validity(&self) -> Option<String> {
        let credentials = self.credentials.read();
        credentials.as_ref()?.bearer_token().bearer_token()
    }

    /// Returns the user's display name.
    pub fn username_for_display(&self) -> Option<String> {
        Some(self.user.read().as_ref()?.username_for_display().to_owned())
    }

    /// Returns the user's display name, does NOT fall back to email.
    pub fn display_name(&self) -> Option<String> {
        self.user
            .read()
            .as_ref()
            .and_then(|user| user.display_name().to_owned())
    }

    /// Returns the user's email. Note the non-obvious semantics of this function:
    /// If the user is logged in and not anonymous, the email will always be populated.
    /// If the user is logged in and anonymous, their email will be an empty string.
    /// If the user is not logged in, their email will be `None`.
    pub fn user_email(&self) -> Option<String> {
        self.user
            .read()
            .as_ref()
            .map(|user| user.metadata.email.clone())
    }

    /// Returns whether the user considered onboarded to Warp.
    pub fn is_onboarded(&self) -> Option<bool> {
        self.user.read().as_ref().map(|user| user.is_onboarded)
    }

    /// Returns the user's email domain (anything after the @ sign of their email).
    pub fn user_email_domain(&self) -> Option<String> {
        self.user.read().as_ref().map(|user| {
            user.metadata
                .email
                .clone()
                .split('@')
                .nth(1)
                .unwrap_or("")
                .to_string()
        })
    }

    /// Returns whether or not the user is anonymous.
    /// Anonymous users are real Warp users, but have no providers linked in Firebase.
    /// Returns `None` if there is no user data.
    pub fn is_user_anonymous(&self) -> Option<bool> {
        self.user
            .read()
            .as_ref()
            .map(|user| user.is_user_anonymous())
    }

    /// Returns whether or not the user is a "web client anonymous user", aka their account
    /// originated from viewing Warp on web.
    pub fn is_user_web_anonymous_user(&self) -> Option<bool> {
        self.user.read().as_ref().map(|user| {
            user.anonymous_user_type() == Some(AnonymousUserType::WebClientAnonymousUser)
                && user.linked_at().is_none()
        })
    }

    /// Returns whether or not the user is a feature gated anonymous user.
    pub fn is_anonymous_user_feature_gated(&self) -> Option<bool> {
        self.user.read().as_ref().map(|user| {
            if !self.is_user_anonymous().unwrap_or_default() {
                return false;
            }

            matches!(
                user.anonymous_user_type(),
                Some(AnonymousUserType::NativeClientAnonymousUserFeatureGated)
            )
        })
    }

    /// Returns whether or not the anonymous user is past any of their Warp Drive object limits.
    pub fn is_anonymous_user_past_object_limit(
        &self,
        object_type: ObjectType,
        num_objects: usize,
    ) -> Option<bool> {
        self.user.read().as_ref().map(|user| {
            if !self.is_anonymous_user_feature_gated().unwrap_or_default() {
                return false;
            }

            if let Some(limits) = user.personal_object_limits() {
                match object_type {
                    ObjectType::Notebook => num_objects > limits.notebook_limit,
                    ObjectType::Workflow => num_objects > limits.workflow_limit,
                    ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                        JsonObjectType::EnvVarCollection,
                    )) => num_objects > limits.env_var_limit,
                    _ => false,
                }
            } else {
                false
            }
        })
    }

    /// Returns the user's photo URL from Firebase,
    /// typically acquired from linking a provider like Google/GitHub.
    pub fn user_photo_url(&self) -> Option<String> {
        self.user
            .read()
            .as_ref()
            .and_then(|user| user.metadata.photo_url.clone())
    }

    /// Returns whether or not the user needs to link their account to an SSO provider.
    /// The actual value is calculated on the server to avoid additional RPCs to Firebase.
    pub fn needs_sso_link(&self) -> Option<bool> {
        self.user.read().as_ref().map(|user| user.needs_sso_link)
    }

    /// Returns the anonymous user type.
    /// Note that a `Some()` value here does NOT mean the user is still anonymous;
    /// they might have since signed up, but we keep their anonymous user type around.
    pub fn anonymous_user_type(&self) -> Option<AnonymousUserType> {
        self.user
            .read()
            .as_ref()
            .and_then(|user| user.anonymous_user_type())
    }

    /// Returns the personal object limits the user has.
    /// Currently, only anonymous users have limits.
    pub fn personal_object_limits(&self) -> Option<PersonalObjectLimits> {
        self.user
            .read()
            .as_ref()
            .and_then(|user| user.personal_object_limits())
    }

    /// Set whether or not the user is onboarded.
    pub fn set_is_onboarded(&self, is_onboarded: bool) {
        if let Some(user) = self.user.write().as_mut() {
            user.is_onboarded = is_onboarded;
        }
    }

    /// If the user is logged in, returns their Firebase UID. Otherwise, returns None.
    pub fn user_id(&self) -> Option<UserUid> {
        self.user.read().as_ref().map(|user| user.local_id)
    }

    /// Returns the user's anonymous id.
    /// The anonymous id will be consistent across the app's lifetime. It is a random UUID.
    pub fn anonymous_id(&self) -> String {
        self.anonymous_id.to_string()
    }

    /// Returns whether a reauth is required for the current user given the state
    /// of their refresh token.
    pub fn needs_reauth(&self) -> bool {
        self.needs_reauth.load(Ordering::Relaxed)
    }

    /// Sets whether a reauth is required for the current user.
    /// Returns whether or not the reauth state was changed from false to true.
    pub(super) fn set_needs_reauth(&self, new_needs_reauth: bool) -> bool {
        let prev_needs_reauth = self.needs_reauth.swap(new_needs_reauth, Ordering::Relaxed);
        !prev_needs_reauth && new_needs_reauth
    }

    /// Returns whether or not the renotification block to encourage anonymous users to sign up
    /// has expired.
    pub fn anonymous_user_renotification_block_expired(
        &self,
        last_time_opt: Option<String>,
    ) -> bool {
        self.is_anonymous_user_feature_gated().unwrap_or_default()
            && last_time_opt
                .and_then(|last_time_string| last_time_string.parse::<DateTime<Utc>>().ok())
                .is_none_or(|last_time| {
                    Utc::now() - ANONYMOUS_USER_NOTIFICATION_BLOCK_TIMER >= last_time
                })
    }

    /// Returns whether or not the user is on a work domain.
    /// This calculation is done on the server, using a list of
    pub fn is_on_work_domain(&self) -> Option<bool> {
        self.user.read().as_ref().map(|user| user.is_on_work_domain)
    }

    /// Returns whether the current user is authenticated via API key.
    pub fn is_api_key_authenticated(&self) -> bool {
        matches!(
            self.credentials.read().as_ref(),
            Some(Credentials::ApiKey { .. })
        )
    }

    /// Returns the API key if using API key authentication.
    pub fn api_key(&self) -> Option<String> {
        let credentials = self.credentials.read();
        credentials.as_ref()?.as_api_key().map(|s| s.to_owned())
    }

    /// Returns the type of principal (user or service account).
    pub fn principal_type(&self) -> Option<PrincipalType> {
        self.user.read().as_ref().map(|user| user.principal_type)
    }

    /// Returns whether the authenticated principal is a service account.
    pub fn is_service_account(&self) -> bool {
        matches!(self.principal_type(), Some(PrincipalType::ServiceAccount))
    }

    /// Returns the cached global skill specs for the current user.
    pub fn global_skills(&self) -> Vec<String> {
        self.user
            .read()
            .as_ref()
            .map(|user| user.global_skills.clone())
            .unwrap_or_default()
    }

    /// Returns the owner type of the currently-authenticated API key.
    pub fn api_key_owner_type(&self) -> Option<OwnerType> {
        self.credentials.read().as_ref()?.api_key_owner_type()
    }
}

// Adapter for the [`warp_managed_secrets`] crate, which needs to access the current user.
impl warp_managed_secrets::ActorProvider for AuthState {
    fn actor_uid(&self) -> Option<String> {
        self.user_id().map(|uid| uid.as_string())
    }
}

/// AuthStateProvider is a singleton model which provides a reference to the global AuthState.
pub struct AuthStateProvider {
    auth_state: Arc<AuthState>,
}

impl AuthStateProvider {
    pub fn new(auth_state: Arc<AuthState>) -> Self {
        Self { auth_state }
    }

    #[cfg(test)]
    pub fn new_for_test() -> Self {
        Self {
            auth_state: Arc::new(AuthState::new_for_test()),
        }
    }

    /// Constructs a provider backed by a fully logged-out `AuthState` (no user,
    /// no credentials). Used by unit tests that need to exercise code paths
    /// gated on `AuthState::user_id()` / `UserWorkspaces::personal_drive()`
    /// returning `None`.
    #[cfg(test)]
    pub fn new_logged_out_for_test() -> Self {
        Self {
            auth_state: Arc::new(AuthState::new_logged_out_for_test()),
        }
    }

    #[cfg(test)]
    pub fn new_anonymous_for_test() -> Self {
        Self {
            auth_state: Arc::new(AuthState::new_anonymous_for_test()),
        }
    }

    pub fn get(&self) -> &Arc<AuthState> {
        &self.auth_state
    }
}

impl Entity for AuthStateProvider {
    type Event = ();
}

impl SingletonEntity for AuthStateProvider {}
