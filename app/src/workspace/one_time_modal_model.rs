use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warp_core::send_telemetry_from_ctx;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity, WindowId};

use super::hoa_onboarding;
use super::view::free_ai_removal_modal::FreeAiRemovalModalTelemetryEvent;
use crate::ai::blocklist::agent_view::toolbar_item::AgentToolbarItemKind;
use crate::auth::auth_manager::AuthManagerEvent;
use crate::auth::{AuthManager, AuthStateProvider};
use crate::channel::{Channel, ChannelState};
use crate::server::experiments::{
    is_free_ai_removal_experiment_enabled, ServerExperiments, ServerExperimentsEvent,
};
use crate::settings::cloud_preferences_syncer::{
    CloudPreferencesSyncer, CloudPreferencesSyncerEvent,
};
use crate::settings::{AISettings, CodeSettings};
use crate::terminal::general_settings::GeneralSettings;
use crate::terminal::session_settings::{AgentToolbarChipSelection, SessionSettings};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::CustomerType;

/// A generic model for managing one-time modals that should be shown to users only once.
///
/// Initially implemented for the ADE launch modal, but designed to be extensible to support
/// other types of one-time modals in the future. The model holds the canonical state of whether
/// a modal is currently being shown and automatically triggers the modal when appropriate
/// conditions are met (e.g., user becomes onboarded).
pub struct OneTimeModalModel {
    is_build_plan_migration_modal_open: bool,
    /// Whether the Oz launch modal is currently being shown.
    is_oz_launch_modal_open: bool,
    /// Whether the OpenWarp launch modal is currently being shown.
    is_openwarp_launch_modal_open: bool,
    is_orchestration_launch_modal_open: bool,
    /// Whether the free-AI-removal notice modal is currently being shown.
    is_free_ai_removal_modal_open: bool,
    /// Whether the HOA onboarding flow is currently being shown.
    is_hoa_onboarding_open: bool,
    /// Whether the initial one-time modal checks have run. The seen markers are
    /// cloud-synced settings, so event-driven re-checks must wait for the initial
    /// cloud preferences load to avoid acting on stale values.
    has_completed_initial_modal_checks: bool,
    /// Whether `UserWorkspaces` has emitted `TeamsChanged`, meaning workspace billing
    /// data reflects more than the local cache and "no workspace" can be trusted to
    /// mean a solo (Free) user rather than not-yet-loaded data.
    has_fetched_workspaces: bool,
    /// The window ID where the currently open one-time modal should be displayed.
    /// This is captured when a modal is first opened and ensures the modal stays on that window.
    target_window_id: Option<WindowId>,
}

impl OneTimeModalModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        // Subscribe to UserWorkspaces to detect when sunsetted_to_build_ts changes
        ctx.subscribe_to_model(
            &crate::workspaces::user_workspaces::UserWorkspaces::handle(ctx),
            |me, event, ctx| {
                use crate::workspaces::user_workspaces::UserWorkspacesEvent;
                match event {
                    UserWorkspacesEvent::SunsettedToBuildDataUpdated => {
                        // When sunsetted_to_build_ts is updated, check if we should show the modal
                        me.check_and_trigger_build_plan_migration_modal(ctx);
                    }
                    UserWorkspacesEvent::TeamsChanged => {
                        me.has_fetched_workspaces = true;
                        me.maybe_recheck_free_ai_removal_modal(ctx);
                    }
                    _ => {}
                }
            },
        );

        // The free-AI-removal notice depends on the server experiment assignment, which
        // can arrive after the initial modal checks have already run.
        ctx.subscribe_to_model(&ServerExperiments::handle(ctx), |me, event, ctx| {
            let ServerExperimentsEvent::ExperimentsUpdated = event;
            me.maybe_recheck_free_ai_removal_modal(ctx);
        });

        // Subscribe to auth manager events to automatically trigger modal when user becomes onboarded
        ctx.subscribe_to_model(&AuthManager::handle(ctx), |_, event, ctx| {
            let AuthManagerEvent::AuthComplete = event else {
                return;
            };

            let auth_state = crate::auth::AuthStateProvider::as_ref(ctx).get().clone();
            let is_existing_user = auth_state.is_onboarded().unwrap_or_default();
            if is_existing_user {
                // Settings modals settings are synced to the cloud, not respecting the user's sync setting, so they
                // must all await initial load to be triggered, else we risk reading a stale triggered value.
                ctx.subscribe_to_model(
                    &CloudPreferencesSyncer::handle(ctx),
                    move |me, event, ctx| {
                        if let CloudPreferencesSyncerEvent::InitialLoadCompleted = event {
                            ctx.unsubscribe_from_model(&CloudPreferencesSyncer::handle(ctx));
                            me.has_completed_initial_modal_checks = true;
                            me.check_and_trigger_all_modals(ctx);
                            maybe_ensure_handoff_chip_in_toolbar(ctx);
                        }
                    },
                );
            } else {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    if let Err(e) = settings
                        .did_check_to_trigger_oz_launch_modal
                        .set_value(true, ctx)
                    {
                        log::warn!("Failed to mark Oz launch modal as dismissed: {e}");
                    }
                    if let Err(e) = settings
                        .did_check_to_trigger_orchestration_launch_modal
                        .set_value(true, ctx)
                    {
                        log::warn!("Failed to mark orchestration launch modal as dismissed: {e}");
                    }
                });
                // Accounts created after the FREE_AI_REMOVAL rollout go through the new
                // onboarding and are treated as already-noticed (no modal, no email).
                mark_free_ai_removal_notice_seen(ctx);
                GeneralSettings::handle(ctx).update(ctx, |settings, ctx| {
                    if let Err(e) = settings
                        .did_check_to_trigger_openwarp_launch_modal
                        .set_value(true, ctx)
                    {
                        log::warn!("Failed to mark OpenWarp launch modal as dismissed: {e}");
                    }
                });
            }
        });

        Self {
            is_build_plan_migration_modal_open: false,
            is_oz_launch_modal_open: false,
            is_openwarp_launch_modal_open: false,
            is_orchestration_launch_modal_open: false,
            is_free_ai_removal_modal_open: false,
            is_hoa_onboarding_open: false,
            has_completed_initial_modal_checks: false,
            has_fetched_workspaces: false,
            target_window_id: None,
        }
    }

    /// Returns whether the Oz launch modal is currently open.
    pub fn is_oz_launch_modal_open(&self) -> bool {
        self.is_oz_launch_modal_open && self.target_window_id.is_some()
    }

    /// Returns the window ID where the currently open one-time modal should be displayed.
    pub fn target_window_id(&self) -> Option<WindowId> {
        self.target_window_id
    }

    pub fn mark_oz_launch_modal_dismissed(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_oz_launch_modal_open(false, ctx);
    }

    /// Returns whether the OpenWarp launch modal is currently open.
    pub fn is_openwarp_launch_modal_open(&self) -> bool {
        self.is_openwarp_launch_modal_open && self.target_window_id.is_some()
    }

    pub fn mark_openwarp_launch_modal_dismissed(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_openwarp_launch_modal_open(false, ctx);
    }

    pub fn is_orchestration_launch_modal_open(&self) -> bool {
        self.is_orchestration_launch_modal_open && self.target_window_id.is_some()
    }

    pub fn mark_orchestration_launch_modal_dismissed(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_orchestration_launch_modal_open(false, ctx);
    }

    /// Returns whether the HOA onboarding flow is currently open.
    pub fn is_hoa_onboarding_open(&self) -> bool {
        self.is_hoa_onboarding_open && self.target_window_id.is_some()
    }

    pub fn mark_hoa_onboarding_dismissed(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_hoa_onboarding_open(false, ctx);
    }

    /// Returns true if any one-time modal is currently open.
    pub fn is_any_modal_open(&self) -> bool {
        (self.is_oz_launch_modal_open
            || self.is_openwarp_launch_modal_open
            || self.is_orchestration_launch_modal_open
            || self.is_build_plan_migration_modal_open
            || self.is_free_ai_removal_modal_open
            || self.is_hoa_onboarding_open)
            && self.target_window_id.is_some()
    }

    #[cfg(debug_assertions)]
    pub fn force_open_oz_launch_modal(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_oz_launch_modal_open(true, ctx);
    }

    #[cfg(debug_assertions)]
    pub fn force_open_openwarp_launch_modal(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_openwarp_launch_modal_open(true, ctx);
    }

    #[cfg(debug_assertions)]
    pub fn force_open_orchestration_launch_modal(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_orchestration_launch_modal_open(true, ctx);
    }

    pub fn update_target_window_id(&mut self, window_id: WindowId, ctx: &mut ModelContext<Self>) {
        let was_any_modal_visible = self.is_any_modal_open();
        self.target_window_id = Some(window_id);
        if was_any_modal_visible != self.is_any_modal_open() {
            ctx.emit(OneTimeModalEvent::VisibilityChanged {
                is_open: self.is_any_modal_open(),
            });
        }
    }

    fn set_oz_launch_modal_open(&mut self, is_open: bool, ctx: &mut ModelContext<Self>) -> bool {
        if self.is_oz_launch_modal_open != is_open {
            self.is_oz_launch_modal_open = is_open;
            ctx.emit(OneTimeModalEvent::VisibilityChanged { is_open });
            return true;
        }
        false
    }

    fn set_openwarp_launch_modal_open(
        &mut self,
        is_open: bool,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if self.is_openwarp_launch_modal_open != is_open {
            self.is_openwarp_launch_modal_open = is_open;
            ctx.emit(OneTimeModalEvent::VisibilityChanged { is_open });
            return true;
        }
        false
    }

    fn set_orchestration_launch_modal_open(
        &mut self,
        is_open: bool,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if self.is_orchestration_launch_modal_open != is_open {
            self.is_orchestration_launch_modal_open = is_open;
            ctx.emit(OneTimeModalEvent::VisibilityChanged { is_open });
            return true;
        }
        false
    }

    fn check_and_trigger_all_modals(&mut self, ctx: &mut ModelContext<Self>) {
        // Never show one-time modals on WASM.
        if cfg!(target_family = "wasm") {
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

        // The OpenWarp launch modal takes priority over the Oz launch modal
        // when both are enabled.
        if self.check_and_trigger_openwarp_launch_modal(ctx) {
            return;
        }

        if self.check_and_trigger_oz_launch_modal(ctx) {
            return;
        }

        if self.check_and_trigger_orchestration_launch_modal(ctx) {
            return;
        }

        if self.check_and_trigger_free_ai_removal_modal(ctx) {
            return;
        }

        if self.check_and_trigger_hoa_onboarding(ctx) {
            return;
        }

        self.check_and_trigger_build_plan_migration_modal(ctx);
    }

    /// Returns whether the free-AI-removal notice modal is currently open.
    pub fn is_free_ai_removal_modal_open(&self) -> bool {
        self.is_free_ai_removal_modal_open && self.target_window_id.is_some()
    }

    pub fn mark_free_ai_removal_modal_dismissed(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_free_ai_removal_modal_open(false, ctx);
    }

    #[cfg(debug_assertions)]
    pub fn force_open_free_ai_removal_modal(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_free_ai_removal_modal_open(true, ctx);
    }

    fn set_free_ai_removal_modal_open(
        &mut self,
        is_open: bool,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if self.is_free_ai_removal_modal_open != is_open {
            self.is_free_ai_removal_modal_open = is_open;
            ctx.emit(OneTimeModalEvent::VisibilityChanged { is_open });
            return true;
        }
        false
    }

    /// Re-evaluates the free-AI-removal notice outside the initial startup check, e.g.
    /// when the experiments payload or workspace billing data arrives after startup.
    fn maybe_recheck_free_ai_removal_modal(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.has_completed_initial_modal_checks || self.is_any_modal_open() {
            return;
        }
        self.check_and_trigger_free_ai_removal_modal(ctx);
    }

    fn check_and_trigger_free_ai_removal_modal(&mut self, ctx: &mut ModelContext<Self>) -> bool {
        // Gated on the server-side FREE_AI_REMOVAL experiment arm rather than a client
        // feature flag, so the notice lands together with enforcement for each enrolled
        // user. Not-enrolled users are left untouched (no seen marker is written).
        if !is_free_ai_removal_experiment_enabled(ctx) {
            return false;
        }

        if *AISettings::as_ref(ctx).did_check_to_trigger_free_ai_removal_modal {
            return false;
        }

        // Anonymous users have no BYOK or upgrade path; leave them unmarked so the
        // decision is made after they sign in.
        if AuthStateProvider::as_ref(ctx)
            .get()
            .is_anonymous_or_logged_out()
        {
            return false;
        }

        let customer_type = UserWorkspaces::as_ref(ctx)
            .current_workspace()
            .map(|workspace| workspace.billing_metadata.customer_type);
        let is_warp_ai_enabled = *AISettings::as_ref(ctx).is_any_ai_enabled;

        let decision = free_ai_removal_modal_decision(
            customer_type,
            is_warp_ai_enabled,
            self.has_fetched_workspaces,
        );
        if decision == FreeAiRemovalModalDecision::Defer {
            return false;
        }

        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            if let Err(e) = settings
                .did_check_to_trigger_free_ai_removal_modal
                .set_value(true, ctx)
            {
                log::warn!("Failed to mark free AI removal modal as seen: {e}");
            }
        });

        if decision == FreeAiRemovalModalDecision::MarkSeenSilently {
            return false;
        }

        let should_show = !matches!(ChannelState::channel(), Channel::Integration);
        if should_show {
            send_telemetry_from_ctx!(FreeAiRemovalModalTelemetryEvent::Shown, ctx);
        }
        self.set_free_ai_removal_modal_open(should_show, ctx);
        should_show
    }

    fn set_hoa_onboarding_open(&mut self, is_open: bool, ctx: &mut ModelContext<Self>) -> bool {
        if self.is_hoa_onboarding_open != is_open {
            self.is_hoa_onboarding_open = is_open;
            ctx.emit(OneTimeModalEvent::VisibilityChanged { is_open });
            return true;
        }
        false
    }

    fn check_and_trigger_hoa_onboarding(&mut self, ctx: &mut ModelContext<Self>) -> bool {
        if !FeatureFlag::HOAOnboardingFlow.is_enabled() {
            return false;
        }

        if hoa_onboarding::has_completed_hoa_onboarding(ctx) {
            return false;
        }

        // All required dependent feature flags must be enabled.
        if !FeatureFlag::VerticalTabs.is_enabled()
            || !FeatureFlag::HOANotifications.is_enabled()
            || !FeatureFlag::TabConfigs.is_enabled()
        {
            return false;
        }

        self.set_hoa_onboarding_open(true, ctx)
    }

    fn check_and_trigger_oz_launch_modal(&mut self, ctx: &mut ModelContext<Self>) -> bool {
        // Only show if the feature flag is enabled.
        if !FeatureFlag::OzLaunchModal.is_enabled() {
            return false;
        }

        let ai_settings = AISettings::as_ref(ctx);
        let oz_modal_shown = *ai_settings.did_check_to_trigger_oz_launch_modal;

        // If Oz modal has already been shown, don't show anything.
        if oz_modal_shown {
            return false;
        }

        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            if let Err(e) = settings
                .did_check_to_trigger_oz_launch_modal
                .set_value(true, ctx)
            {
                log::warn!("Failed to mark Oz launch modal as dismissed: {e}");
            }
        });

        let should_show_oz_modal = !matches!(ChannelState::channel(), Channel::Integration);
        self.set_oz_launch_modal_open(should_show_oz_modal, ctx);
        should_show_oz_modal
    }

    fn check_and_trigger_openwarp_launch_modal(&mut self, ctx: &mut ModelContext<Self>) -> bool {
        // Only show if the feature flag is enabled.
        if !FeatureFlag::OpenWarpLaunchModal.is_enabled() {
            return false;
        }

        let general_settings = GeneralSettings::as_ref(ctx);
        let openwarp_modal_shown = *general_settings
            .did_check_to_trigger_openwarp_launch_modal
            .value();

        if openwarp_modal_shown {
            return false;
        }

        GeneralSettings::handle(ctx).update(ctx, |settings, ctx| {
            if let Err(e) = settings
                .did_check_to_trigger_openwarp_launch_modal
                .set_value(true, ctx)
            {
                log::warn!("Failed to mark OpenWarp launch modal as dismissed: {e}");
            }
        });

        let should_show_openwarp_modal = !matches!(ChannelState::channel(), Channel::Integration);
        self.set_openwarp_launch_modal_open(should_show_openwarp_modal, ctx);
        should_show_openwarp_modal
    }

    fn check_and_trigger_orchestration_launch_modal(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if !FeatureFlag::OrchestrationLaunchModal.is_enabled() {
            return false;
        }

        let ai_settings = AISettings::as_ref(ctx);
        if *ai_settings.did_check_to_trigger_orchestration_launch_modal {
            return false;
        }

        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            if let Err(e) = settings
                .did_check_to_trigger_orchestration_launch_modal
                .set_value(true, ctx)
            {
                log::warn!("Failed to mark orchestration launch modal as dismissed: {e}");
            }
        });

        let should_show = !matches!(ChannelState::channel(), Channel::Integration);
        self.set_orchestration_launch_modal_open(should_show, ctx);
        should_show
    }

    pub fn is_build_plan_migration_modal_open(&self) -> bool {
        self.is_build_plan_migration_modal_open && self.target_window_id.is_some()
    }

    pub fn mark_build_plan_migration_modal_dismissed(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_build_plan_migration_modal_open(false, ctx);
    }

    #[cfg(debug_assertions)]
    pub fn force_open_build_plan_migration_modal(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_build_plan_migration_modal_open(true, ctx);
    }

    fn set_build_plan_migration_modal_open(
        &mut self,
        is_open: bool,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if self.is_build_plan_migration_modal_open != is_open {
            self.is_build_plan_migration_modal_open = is_open;
            ctx.emit(OneTimeModalEvent::VisibilityChanged { is_open });
            return true;
        }
        false
    }

    fn check_and_trigger_build_plan_migration_modal(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        use crate::workspaces::user_workspaces::UserWorkspaces;

        // Check if already dismissed
        let general_settings = GeneralSettings::as_ref(ctx);
        if *general_settings
            .build_plan_migration_modal_dismissed
            .value()
        {
            return false;
        }

        // Check if user is authenticated
        let auth_state = crate::auth::AuthStateProvider::as_ref(ctx).get();

        if auth_state.is_anonymous_or_logged_out() {
            return false;
        }

        // Check if current workspace has sunsetted_to_build_ts set
        let user_workspaces = UserWorkspaces::as_ref(ctx);
        let Some(current_team) = user_workspaces.current_team() else {
            return false;
        };

        // Check if user is admin of the team
        let Some(user_email) = auth_state.user_email() else {
            return false;
        };

        if !current_team.has_admin_permissions(&user_email) {
            return false;
        }

        // Check if service agreement has sunsetted_to_build_ts set
        let has_sunsetted_to_build = current_team
            .billing_metadata
            .service_agreements
            .first()
            .is_some_and(|sa| sa.sunsetted_to_build_ts.is_some());

        if !has_sunsetted_to_build {
            return false;
        }

        // All conditions met, show the modal
        self.set_build_plan_migration_modal_open(true, ctx)
    }
}

/// One-time migration: if the user has a custom agent toolbar layout that
/// predates the handoff-to-cloud chip, append the chip so they get the
/// new feature without losing their customization.
///
/// Users on `Default` already see the chip via `AgentToolbarItemKind::default_right()`.
fn maybe_ensure_handoff_chip_in_toolbar(ctx: &mut ModelContext<OneTimeModalModel>) {
    if !FeatureFlag::OzHandoff.is_enabled()
        || !FeatureFlag::HandoffLocalCloud.is_enabled()
        || !cfg!(all(feature = "local_fs", not(target_family = "wasm")))
    {
        return;
    }

    let session_settings = SessionSettings::as_ref(ctx);
    if *session_settings.did_add_handoff_chip_to_toolbar {
        return;
    }

    // Mark as done so future app starts skip this path.
    SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
        if let Err(e) = settings
            .did_add_handoff_chip_to_toolbar
            .set_value(true, ctx)
        {
            log::warn!("Failed to mark handoff chip toolbar migration as done: {e}");
        }
    });

    // `Default` already includes the chip — nothing to do.
    let selection = SessionSettings::as_ref(ctx)
        .agent_footer_chip_selection
        .clone();
    let AgentToolbarChipSelection::Custom { mut left, right } = selection else {
        return;
    };

    let handoff = AgentToolbarItemKind::HandoffToCloud;
    if left.contains(&handoff) || right.contains(&handoff) {
        return;
    }

    left.push(handoff);
    SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
        if let Err(e) = settings
            .agent_footer_chip_selection
            .set_value(AgentToolbarChipSelection::Custom { left, right }, ctx)
        {
            log::warn!("Failed to add handoff chip to toolbar: {e}");
        }
    });
}

/// Marks the free-AI-removal notice as seen without showing it.
///
/// Accounts created after the FREE_AI_REMOVAL rollout are treated as already-noticed,
/// so onboarding completion calls this in addition to the new-user pre-dismissal that
/// runs at first auth.
pub fn mark_free_ai_removal_notice_seen(app: &mut AppContext) {
    AISettings::handle(app).update(app, |settings, ctx| {
        if let Err(e) = settings
            .did_check_to_trigger_free_ai_removal_modal
            .set_value(true, ctx)
        {
            log::warn!("Failed to mark free AI removal notice as seen: {e}");
        }
    });
}

/// The outcome of evaluating the free-AI-removal notice conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FreeAiRemovalModalDecision {
    /// Show the modal and write the seen marker.
    Show,
    /// Write the seen marker without showing the modal (paid plan or AI off).
    MarkSeenSilently,
    /// Not enough data to decide; re-evaluate on the next billing/experiments update.
    Defer,
}

fn free_ai_removal_modal_decision(
    customer_type: Option<CustomerType>,
    is_warp_ai_enabled: bool,
    workspaces_fetched: bool,
) -> FreeAiRemovalModalDecision {
    if !is_warp_ai_enabled {
        return FreeAiRemovalModalDecision::MarkSeenSilently;
    }
    match customer_type {
        Some(CustomerType::Free) => FreeAiRemovalModalDecision::Show,
        // A missing workspace usually means billing data hasn't loaded yet; only treat
        // it as a solo Free user once a server fetch has confirmed there is none, so a
        // paid user's modal decision never runs against absent data.
        None if workspaces_fetched => FreeAiRemovalModalDecision::Show,
        None | Some(CustomerType::Unknown) => FreeAiRemovalModalDecision::Defer,
        Some(_) => FreeAiRemovalModalDecision::MarkSeenSilently,
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

#[cfg(test)]
#[path = "one_time_modal_model_tests.rs"]
mod tests;
