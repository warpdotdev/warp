use settings::Setting;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::settings::{AISettings, AISettingsChangedEvent};

/// Tracks whether the BYO LLM auth banner (e.g., AWS Bedrock login) has been dismissed.
///
/// This singleton consolidates both permanent dismissal ("don't show again'") and
/// session-only dismissal (from clicking "X"). On construction, it initializes from
/// the persisted setting and subscribes to setting changes.
///
/// Use `is_dismissed()` to check if the banner should be hidden.
pub struct ByoLlmAuthBannerSessionState {
    dismissed: bool,
}

impl ByoLlmAuthBannerSessionState {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        // Initialize from the persisted permanent dismissal setting
        let dismissed = *AISettings::as_ref(ctx)
            .aws_bedrock_login_banner_dismissed
            .value();

        // Subscribe to changes in the permanent dismissal setting
        ctx.subscribe_to_model(&AISettings::handle(ctx), |state, _, event, ctx| {
            if let AISettingsChangedEvent::AwsBedrockLoginBannerDismissed { .. } = event {
                let permanently_dismissed = *AISettings::as_ref(ctx)
                    .aws_bedrock_login_banner_dismissed
                    .value();
                if permanently_dismissed && !state.dismissed {
                    state.dismissed = true;
                    ctx.notify();
                }
            }
        });

        Self { dismissed }
    }

    /// Returns whether the banner has been dismissed (either permanently or for this session).
    pub fn is_dismissed(&self) -> bool {
        self.dismissed
    }

    /// Marks the banner as dismissed for this session.
    pub fn dismiss(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.dismissed {
            self.dismissed = true;
            ctx.notify();
        }
    }
}

impl Entity for ByoLlmAuthBannerSessionState {
    type Event = ();
}

impl SingletonEntity for ByoLlmAuthBannerSessionState {}
