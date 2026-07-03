use crate::settings::CodeSettings;
use settings::Setting as _;
use warpui::{Entity, ModelContext, SingletonEntity};

/// A model for managing local one-time flows that should be shown only once.
///
/// The model holds the canonical state of whether a flow is currently being shown and
/// automatically triggers it when appropriate conditions are met.
pub struct OneTimeModalModel;

impl OneTimeModalModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let mut model = Self;
        model.check_and_trigger_all_modals(ctx);
        model
    }

    fn check_and_trigger_all_modals(&mut self, ctx: &mut ModelContext<Self>) {
        // Never show one-time modals when running without local workspace UI.
        if false {
            return;
        }

        // Existing users should never see the code toolbelt new feature popup.
        CodeSettings::handle(ctx).update(ctx, |settings, ctx| {
            if let Err(e) = settings
                .dismissed_code_toolbelt_new_feature_popup
                .set_value(true, ctx)
            {
                log::warn!("Failed to mark code toolbelt new feature popup as dismissed: {e}");
            }
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OneTimeModalEvent {
    VisibilityChanged { is_open: bool },
}

impl Entity for OneTimeModalModel {
    type Event = OneTimeModalEvent;
}

impl SingletonEntity for OneTimeModalModel {}
