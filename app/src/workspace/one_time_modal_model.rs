use std::future::Future;

use ai::api_keys::ApiKeyManager;
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warp_core::send_telemetry_from_ctx;
use warp_util::sync::Condition;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity, WindowId};

use super::hoa_onboarding;
use super::view::feature_intro_modal::{FEATURE_INTROS, FeatureIntroId};
use super::view::free_ai_removal_modal::{
    FreeAiRemovalModalTelemetryEvent, FreeAiRemovalModalVariant,
};
use crate::ai::blocklist::agent_view::toolbar_item::AgentToolbarItemKind;
use crate::ai::{AIRequestUsageModel, AIRequestUsageModelEvent};
use crate::auth::auth_manager::AuthManagerEvent;
use crate::auth::{AuthManager, AuthStateProvider};
use crate::channel::{Channel, ChannelState};
use crate::root_view::has_completed_local_onboarding;
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
    /// Whether the auto-handoff sleep discoverability modal is currently being shown.
    is_auto_handoff_sleep_modal_open: bool,
    /// Set while the auto-handoff sleep modal is closed and reset while it is
    /// open, so async work (e.g. auto-resume-after-error) can wait for the
    /// modal to close. Mirrors the `Condition` pattern used by
    /// `NetworkStatus::pending_reconnect`.
    auto_handoff_sleep_modal_closed: Condition,
    /// Whether the free-AI-removal notice modal is currently being shown.
    is_free_ai_removal_modal_open: bool,
    /// Whether the HOA onboarding flow is currently being shown.
    is_hoa_onboarding_open: bool,
    /// The feature-intro popover currently being shown, if any. Unlike the other
    /// one-time modals this is a non-blocking bottom-right popover, so it is
    /// intentionally excluded from `is_any_modal_open` (which suppresses terminal
    /// focus stealing) to keep the terminal usable while it is visible.
    active_feature_intro: Option<FeatureIntroId>,
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
            |me, _, event, ctx| {
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

        // The base-credit allowance that gates the free-AI-removal notice loads
        // asynchronously, so re-evaluate the notice whenever request usage updates.
        ctx.subscribe_to_model(&AIRequestUsageModel::handle(ctx), |me, _, event, ctx| {
            if let AIRequestUsageModelEvent::RequestUsageUpdated = event {
                me.maybe_recheck_free_ai_removal_modal(ctx);
            }
        });

        // Subscribe to auth manager events to automatically trigger modal when user becomes onboarded
        ctx.subscribe_to_model(&AuthManager::handle(ctx), |_, _, event, ctx| {
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
                    move |me, _, event, ctx| {
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
                    // New signups shouldn't see feature-intro popovers on their second
                    // startup, so pre-mark every registered feature intro as seen.
                    for intro in FEATURE_INTROS {
                        settings.mark_feature_intro_seen(intro.id.as_key(), ctx);
                    }
                });
                // Accounts created after the removal of free AI go through the new
                // onboarding and are treated as already-noticed (no modal).
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

        // The auto-handoff sleep modal starts closed, so its close condition
        // starts satisfied.
        let auto_handoff_sleep_modal_closed = Condition::new();
        auto_handoff_sleep_modal_closed.set();

        Self {
            is_build_plan_migration_modal_open: false,
            is_oz_launch_modal_open: false,
            is_openwarp_launch_modal_open: false,
            is_orchestration_launch_modal_open: false,
            is_auto_handoff_sleep_modal_open: false,
            auto_handoff_sleep_modal_closed,
            is_free_ai_removal_modal_open: false,
            is_hoa_onboarding_open: false,
            active_feature_intro: None,
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

    /// Returns the feature-intro popover currently being shown, if any.
    pub fn active_feature_intro(&self) -> Option<FeatureIntroId> {
        if self.target_window_id.is_some() {
            self.active_feature_intro
        } else {
            None
        }
    }

    pub fn mark_feature_intro_dismissed(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.set_active_feature_intro(None, ctx) {
            return;
        }
        // Feature intros sit ahead of HOA/build-plan in the startup queue and also
        // suppress free-AI rechecks while open. Resume those deferred paths so
        // lower-priority notices are not lost for the rest of the session.
        self.resume_modal_checks_after_feature_intro(ctx);
    }

    fn resume_modal_checks_after_feature_intro(&mut self, ctx: &mut ModelContext<Self>) {
        if self.check_and_trigger_free_ai_removal_modal(ctx) {
            return;
        }
        if self.check_and_trigger_hoa_onboarding(ctx) {
            return;
        }
        self.check_and_trigger_build_plan_migration_modal(ctx);
    }

    #[cfg(debug_assertions)]
    pub fn force_open_feature_intro(&mut self, id: FeatureIntroId, ctx: &mut ModelContext<Self>) {
        self.set_active_feature_intro(Some(id), ctx);
    }

    fn set_active_feature_intro(
        &mut self,
        intro: Option<FeatureIntroId>,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if self.active_feature_intro != intro {
            self.active_feature_intro = intro;
            // Bind the popover to the focused window as soon as it opens. The
            // workspace only renders / populates the view when
            // `target_window_id` matches, and `on_active_window_changed` may not
            // have run yet when the startup modal queue fires.
            if intro.is_some()
                && self.target_window_id.is_none()
                && let Some(window_id) = ctx.windows().active_window()
            {
                self.target_window_id = Some(window_id);
            }
            ctx.emit(OneTimeModalEvent::VisibilityChanged {
                is_open: intro.is_some(),
            });
            return true;
        }
        false
    }

    /// Returns whether the auto-handoff sleep discoverability modal is currently open.
    pub fn is_auto_handoff_sleep_modal_open(&self) -> bool {
        self.is_auto_handoff_sleep_modal_open && self.target_window_id.is_some()
    }

    pub fn mark_auto_handoff_sleep_modal_dismissed(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_auto_handoff_sleep_modal_open(false, ctx);
    }

    /// Triggers the auto-handoff sleep discoverability modal. Unlike the launch
    /// modals, this is not called on startup: the auto-handoff controller calls
    /// it on wake when a sleep interrupted an in-progress local agent run that
    /// would have been handed off had `auto_handoff_on_sleep_enabled` been on.
    /// Shows at most once per user (tracked by a synced private setting).
    /// Returns true when the modal was opened.
    pub fn check_and_trigger_auto_handoff_sleep_modal(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        let ai_settings = AISettings::as_ref(ctx);
        if *ai_settings.did_show_auto_handoff_sleep_modal {
            return false;
        }

        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            if let Err(e) = settings
                .did_show_auto_handoff_sleep_modal
                .set_value(true, ctx)
            {
                log::warn!("Failed to mark auto-handoff sleep modal as shown: {e}");
            }
        });

        let should_show = !matches!(ChannelState::channel(), Channel::Integration);
        self.set_auto_handoff_sleep_modal_open(should_show, ctx);
        should_show
    }

    /// Sets whether the auto-handoff sleep modal is open. `pub(crate)` so the
    /// debug palette action can force the modal open.
    pub(crate) fn set_auto_handoff_sleep_modal_open(
        &mut self,
        is_open: bool,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if self.is_auto_handoff_sleep_modal_open != is_open {
            self.is_auto_handoff_sleep_modal_open = is_open;
            if is_open {
                self.auto_handoff_sleep_modal_closed.reset();
            } else {
                self.auto_handoff_sleep_modal_closed.set();
            }
            ctx.emit(OneTimeModalEvent::VisibilityChanged { is_open });
            return true;
        }
        false
    }

    /// Returns a future that resolves immediately if the auto-handoff sleep
    /// modal is closed, or when it next closes if currently open. The future
    /// reads live modal state at poll time, so it can be created ahead of the
    /// modal opening.
    pub fn wait_until_auto_handoff_sleep_modal_closed(&self) -> impl Future<Output = ()> + use<> {
        self.auto_handoff_sleep_modal_closed.wait()
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
            || self.is_auto_handoff_sleep_modal_open
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
        // Feature intro is intentionally excluded from `is_any_modal_open`, so
        // track it separately. Without this, activating a window after the
        // startup queue already selected an intro never re-emits, and the
        // workspace never calls `show_feature_intro_modal`.
        let was_feature_intro_visible = self.active_feature_intro().is_some();
        let previous_target = self.target_window_id;
        self.target_window_id = Some(window_id);
        let is_any_modal_visible = self.is_any_modal_open();
        let is_feature_intro_visible = self.active_feature_intro().is_some();
        if was_any_modal_visible != is_any_modal_visible
            || was_feature_intro_visible != is_feature_intro_visible
            || (is_feature_intro_visible && previous_target != Some(window_id))
        {
            ctx.emit(OneTimeModalEvent::VisibilityChanged {
                is_open: is_any_modal_visible || is_feature_intro_visible,
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

        if self.check_and_trigger_feature_intro_modal(ctx) {
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
    /// when workspace billing data arrives after startup.
    fn maybe_recheck_free_ai_removal_modal(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.has_completed_initial_modal_checks
            || self.is_any_modal_open()
            || self.active_feature_intro.is_some()
        {
            return;
        }
        self.check_and_trigger_free_ai_removal_modal(ctx);
    }

    fn check_and_trigger_free_ai_removal_modal(&mut self, ctx: &mut ModelContext<Self>) -> bool {
        // Gated on the OpenWarpNewSettingsModes rollout flag (the server experiment
        // that previously gated this was removed in C1).
        if !FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
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
        let has_byok_or_byoe = ApiKeyManager::as_ref(ctx).has_any_key();
        let completed_new_onboarding = has_completed_local_onboarding(ctx);
        let has_zero_base_credits = AIRequestUsageModel::as_ref(ctx).request_limit() == 0;

        let decision = free_ai_removal_modal_decision(
            customer_type,
            is_warp_ai_enabled,
            has_byok_or_byoe,
            completed_new_onboarding,
            has_zero_base_credits,
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
            send_telemetry_from_ctx!(
                FreeAiRemovalModalTelemetryEvent::Shown {
                    variant: FreeAiRemovalModalVariant::Notice,
                },
                ctx
            );
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

    fn check_and_trigger_feature_intro_modal(&mut self, ctx: &mut ModelContext<Self>) -> bool {
        if !AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
            return false;
        }
        // Show the first registered feature intro that the user hasn't seen yet
        // (see `FEATURE_INTROS`).
        let next_id = FEATURE_INTROS
            .iter()
            .find(|intro| !AISettings::as_ref(ctx).is_feature_intro_seen(intro.id.as_key()))
            .map(|intro| intro.id);
        let Some(id) = next_id else {
            return false;
        };

        // Mark it seen up front so it shows at most once, even if suppressed below.
        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings.mark_feature_intro_seen(id.as_key(), ctx);
        });

        let should_show = !matches!(ChannelState::channel(), Channel::Integration);
        if should_show {
            self.set_active_feature_intro(Some(id), ctx);
        }
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
    /// Write the seen marker without showing the modal.
    MarkSeenSilently,
    /// Not enough data to decide; re-evaluate on the next billing/experiments update.
    Defer,
}

fn free_ai_removal_modal_decision(
    customer_type: Option<CustomerType>,
    is_warp_ai_enabled: bool,
    has_byok_or_byoe: bool,
    completed_new_onboarding: bool,
    has_zero_base_credits: bool,
    workspaces_fetched: bool,
) -> FreeAiRemovalModalDecision {
    if !is_warp_ai_enabled || has_byok_or_byoe || completed_new_onboarding {
        return FreeAiRemovalModalDecision::MarkSeenSilently;
    }
    // Restrict to a Free (or confirmed solo) user; anyone else is paid (silently
    // marked) or not-yet-known (deferred).
    match customer_type {
        Some(CustomerType::Free) => {}
        // A missing workspace usually means billing data hasn't loaded yet; only treat
        // it as a solo Free user once a server fetch has confirmed there is none, so a
        // paid user's modal decision never runs against absent data.
        None if workspaces_fetched => {}
        None | Some(CustomerType::Unknown) => return FreeAiRemovalModalDecision::Defer,
        Some(_) => return FreeAiRemovalModalDecision::MarkSeenSilently,
    }
    // Some ICPs still receive base AI credits on the Free plan; don't spook them with
    // the notice. Only show once the base allowance is gone, and defer (rather than
    // mark seen) otherwise so it re-evaluates if the allowance later drops to zero.
    if has_zero_base_credits {
        FreeAiRemovalModalDecision::Show
    } else {
        FreeAiRemovalModalDecision::Defer
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
