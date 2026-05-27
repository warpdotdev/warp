use std::sync::Arc;

use serde::{Deserialize, Serialize};
use warp_core::user_preferences::GetUserPreferences as _;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::auth::AuthStateProvider;
use crate::safe_info;
use crate::server::server_api::referral::{ReferralInfo, ReferralsClient};

// Note: The name of this key is from before this model was created. For consistency, it should
// remain the same value
const SENT_REFERRAL_THEME_KEY: &str = "ReferralThemeActive";
const RECEIVED_REFERRAL_THEME_KEY: &str = "ReceivedReferralTheme";

pub enum ReferralThemeEvent {
    SentReferralThemeActivated,
    ReceivedReferralThemeActivated,
}

/// Model to track the status of referral theme(s)
///
/// Note: An invariant of this type, relied upon by the rest of the code, is that themes will only
/// ever become available, they can not be revoked.
pub struct ReferralThemeStatus {
    sent_referral_theme: ReferralThemeFetchStatus,
    received_referral_theme: ReferralThemeFetchStatus,
}

impl Entity for ReferralThemeStatus {
    type Event = ReferralThemeEvent;
}

impl ReferralThemeStatus {
    /// Creates a new ReferralThemeStatus model
    ///
    /// The initial values for the theme availability will be loaded from user default storage
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let sent_referral_theme = parse_sent_referral_fetch_status(
            ctx.private_user_preferences()
                .read_value(SENT_REFERRAL_THEME_KEY)
                .unwrap_or_default(),
        );

        let received_referral_theme = ctx
            .private_user_preferences()
            .read_value(RECEIVED_REFERRAL_THEME_KEY)
            .unwrap_or_default()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(ReferralThemeFetchStatus::NotFetched);

        Self {
            sent_referral_theme,
            received_referral_theme,
        }
    }

    /// Is the "Sent Referral" theme available (i.e. has the user sent at least one referral)?
    pub fn sent_referral_theme_active(&self) -> bool {
        self.sent_referral_theme.is_active()
    }

    /// Is the "Received Referral" theme available (i.e. was the user referred by another)?
    pub fn received_referral_theme_active(&self) -> bool {
        self.received_referral_theme.is_active()
    }

    /// Fetch the referral statuses, sending events if the values change
    pub fn query_referral_status(
        &self,
        referrals_client: Arc<dyn ReferralsClient>,
        ctx: &mut ModelContext<Self>,
    ) {
        if !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            return;
        }

        let sent_referrals_client = referrals_client.clone();
        let _ = ctx.spawn(
            async move { sent_referrals_client.get_referral_info().await },
            Self::handle_referral_status_response,
        );
    }

    /// Handle the response from the server indicating the number of referrals the user has sent
    fn handle_referral_status_response(
        &mut self,
        response: anyhow::Result<ReferralInfo>,
        ctx: &mut ModelContext<Self>,
    ) {
        match response {
            Ok(info) => {
                // If the "you referred someone" theme isn't active, see if the user has since
                // referred someone. If so, activate the theme and emit an event with the change.
                if !self.sent_referral_theme.is_active() && info.number_claimed > 0 {
                    // The user has referred at least one other user and doesn't yet have the
                    // referral theme. Update the user defaults and this model to reflect that
                    self.sent_referral_theme = ReferralThemeFetchStatus::Active;
                    let _ = ctx
                        .private_user_preferences()
                        .write_value(SENT_REFERRAL_THEME_KEY, "true".to_owned());
                    ctx.emit(ReferralThemeEvent::SentReferralThemeActivated);
                }

                // We only need to check if the user was referred once (they can never be referred after the
                // fact). So if we're in this unfetched state, look at the response to find out if we should
                // activate the "you were referred by someone" theme.
                if matches!(
                    self.received_referral_theme,
                    ReferralThemeFetchStatus::NotFetched
                ) {
                    if info.is_referred {
                        // The user _was_ referred. Store that value and notify the listeners that the
                        // theme is now active
                        self.received_referral_theme = ReferralThemeFetchStatus::Active;
                        ctx.emit(ReferralThemeEvent::ReceivedReferralThemeActivated);
                    } else {
                        // The user was _not_ referred. Store that value into user defaults but no need to
                        // notify since no theme became active
                        self.received_referral_theme = ReferralThemeFetchStatus::Inactive;
                    }
                    // Store any new value in user defaults
                    let _ = ctx.private_user_preferences().write_value(
                        RECEIVED_REFERRAL_THEME_KEY,
                        self.received_referral_theme.to_json(),
                    );
                }
            }
            Err(e) => {
                safe_info!(
                    safe: ("Unable to retrieve user referral info"),
                    full: ("Unable to retrieve user referral info: {}", e)
                );
            }
        }
    }
}

/// Type used for tracking the fetch status of different referral themes
///
/// For the received referral theme, we only need to check until we get a successful response.
/// Since the user can only sign up once and they were either referred or not, the response should
/// be definitive.
///
/// For the sent referral theme, we still need to keep checking even if a previous response
/// indicated that it wasn't available, since the user could have sent a referral in the interim.
#[derive(Serialize, Deserialize, Clone, Copy)]
enum ReferralThemeFetchStatus {
    NotFetched,
    Inactive,
    Active,
}

impl ReferralThemeFetchStatus {
    fn is_active(self) -> bool {
        matches!(self, ReferralThemeFetchStatus::Active)
    }

    fn to_json(self) -> String {
        serde_json::to_string(&self).expect("FetchStatus should serialize properly")
    }
}

/// Parse the sent referral status into a ReferralThemeFetchStatus
///
/// Note: For historical reasons, the status is stored as a boolean literal (`true` or `false`),
/// so we need to map that onto the fetch status.
fn parse_sent_referral_fetch_status(stored_value: Option<String>) -> ReferralThemeFetchStatus {
    stored_value
        .and_then(|s| s.parse::<bool>().ok())
        .map(|active| {
            if active {
                ReferralThemeFetchStatus::Active
            } else {
                ReferralThemeFetchStatus::Inactive
            }
        })
        .unwrap_or(ReferralThemeFetchStatus::NotFetched)
}
