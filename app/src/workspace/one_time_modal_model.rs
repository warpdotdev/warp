use std::future::Future;
use std::time::Duration;

use ai::api_keys::ApiKeyManager;
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warp_core::send_telemetry_from_ctx;
use warp_util::sync::Condition;
use warpui::r#async::{FutureExt as _, Spawnable, SpawnableOutput};
use warpui::{AppContext, Entity, ModelContext, SingletonEntity, WindowId};

use super::hoa_onboarding;
use super::view::factories_launch_modal::{
    FACTORIES_LAUNCH_SEEN_KEY, FactoriesLaunchModalTelemetryEvent,
};
use super::view::feature_intro_modal::{
    FEATURE_INTROS, FeatureIntroId, FeatureIntroModalTelemetryEvent,
};
use super::view::free_ai_removal_modal::{
    FreeAiRemovalModalTelemetryEvent, FreeAiRemovalModalVariant,
};
use crate::ai::blocklist::agent_view::toolbar_item::AgentToolbarItemKind;
use crate::ai::{AIRequestUsageModel, AIRequestUsageModelEvent};
use crate::auth::auth_manager::AuthManagerEvent;
use crate::auth::{AuthManager, AuthStateProvider};
use crate::channel::{Channel, ChannelState};
use crate::root_view::has_completed_local_onboarding;
use crate::server::experiments::{ServerExperiments, ServerExperimentsEvent};
use crate::server::server_api::ServerApiProvider;
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
    /// Whether the Warp Agent CLI launch modal is currently being shown.
    is_agent_cli_launch_modal_open: bool,
    /// Whether the auto-handoff sleep discoverability modal is currently being shown.
    is_auto_handoff_sleep_modal_open: bool,
    /// Set while the auto-handoff sleep modal is closed and reset while it is
    /// open, so async work (e.g. auto-resume-after-error) can wait for the
    /// modal to close. Mirrors the `Condition` pattern used by
    /// `NetworkStatus::pending_reconnect`.
    auto_handoff_sleep_modal_closed: Condition,
    /// Whether the free-AI-removal notice modal is currently being shown.
    is_free_ai_removal_modal_open: bool,
    /// Whether the Factories launch modal is currently being shown. Unlike the
    /// feature-intro popover, this is a centered, focus-stealing modal, so it
    /// participates in `is_any_modal_open`.
    is_factories_launch_modal_open: bool,
    /// Set while awaiting the atomic, cross-device impression claim for the
    /// Factories launch modal. Included in `is_any_modal_open` so the modal's
    /// slot is reserved for the whole claim round-trip: without this, a
    /// recheck of another modal (e.g. from an `AIRequestUsageModel` or
    /// `ExperimentsUpdated` event) could see no modal open yet and show its
    /// own modal, only for a winning claim to then open Factories on top of
    /// it, unfocused and unable to receive Escape. Also prevents a recheck
    /// that fires while the claim is in flight from starting a second,
    /// redundant claim.
    pending_factories_launch_claim: bool,
    /// Set when the cross-device claim for the Factories launch modal
    /// resolved as a win (`claimed == true`) while the feature-intro popover
    /// was open. Unlike the other one-time modals, the popover is
    /// intentionally excluded from `is_any_modal_open` (see
    /// `active_feature_intro`), so it isn't covered by
    /// `pending_factories_launch_claim`'s slot reservation. The win is real
    /// and must not be lost, but showing it immediately would stack the
    /// centered, focus-stealing modal on top of the popover, so it is held
    /// here and displayed by `check_and_trigger_factories_launch_modal` (or
    /// `maybe_display_pending_factories_launch_modal`) once the popover
    /// closes.
    factories_launch_pending_display: bool,
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

/// How long to wait for the Factories launch modal's impression claim to resolve before
/// treating it as failed. See `claim_and_show_factories_launch_modal_with_timeout`.
const FACTORIES_LAUNCH_CLAIM_TIMEOUT: Duration = Duration::from_secs(15);

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

        // The Factories launch modal's eligibility (feature flag + validated CTA
        // URL) and other server-targeted intros only become true once a fresh
        // `Experiments`/user fetch arrives, which can land after the initial
        // modal-check pass already ran. Re-check so the intro isn't stuck unseen.
        // Some lightweight test harnesses don't register `ServerExperiments`
        // (see its `UserWorkspaces` subscription comment), so guard against that.
        if ctx.has_singleton_model::<ServerExperiments>() {
            ctx.subscribe_to_model(&ServerExperiments::handle(ctx), |me, _, event, ctx| {
                let ServerExperimentsEvent::ExperimentsUpdated = event;
                me.maybe_check_and_trigger_feature_intro_modal(ctx);
                me.maybe_check_and_trigger_factories_launch_modal(ctx);
            });
        }

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
                    if let Err(e) = settings
                        .did_check_to_trigger_agent_cli_launch_modal
                        .set_value(true, ctx)
                    {
                        log::warn!("Failed to mark Warp Agent CLI launch modal as dismissed: {e}");
                    }
                    // New signups shouldn't see feature-intro popovers on their second
                    // startup, so pre-mark every registered feature intro as seen.
                    for intro in FEATURE_INTROS {
                        settings.mark_feature_intro_seen(intro.id.as_key(), ctx);
                    }
                    // The Factories launch modal isn't a `FEATURE_INTROS` entry, so it
                    // needs its own pre-dismissal here for the same reason.
                    settings.mark_feature_intro_seen(FACTORIES_LAUNCH_SEEN_KEY, ctx);
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
            is_agent_cli_launch_modal_open: false,
            is_auto_handoff_sleep_modal_open: false,
            auto_handoff_sleep_modal_closed,
            is_free_ai_removal_modal_open: false,
            is_factories_launch_modal_open: false,
            pending_factories_launch_claim: false,
            factories_launch_pending_display: false,
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
        self.maybe_check_and_trigger_feature_intro_modal(ctx);
    }

    /// Returns whether the OpenWarp launch modal is currently open.
    pub fn is_openwarp_launch_modal_open(&self) -> bool {
        self.is_openwarp_launch_modal_open && self.target_window_id.is_some()
    }

    pub fn mark_openwarp_launch_modal_dismissed(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_openwarp_launch_modal_open(false, ctx);
        self.maybe_check_and_trigger_feature_intro_modal(ctx);
    }

    pub fn is_orchestration_launch_modal_open(&self) -> bool {
        self.is_orchestration_launch_modal_open && self.target_window_id.is_some()
    }

    pub fn mark_orchestration_launch_modal_dismissed(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_orchestration_launch_modal_open(false, ctx);
        self.maybe_check_and_trigger_feature_intro_modal(ctx);
    }

    pub fn is_agent_cli_launch_modal_open(&self) -> bool {
        self.is_agent_cli_launch_modal_open && self.target_window_id.is_some()
    }

    pub fn mark_agent_cli_launch_modal_dismissed(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_agent_cli_launch_modal_open(false, ctx);
        self.maybe_check_and_trigger_feature_intro_modal(ctx);
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
        // Factories sits immediately after feature intros in
        // `check_and_trigger_all_modals`'s priority order. Without this, an
        // eligible Factories claim (or a claim that already won and is held
        // as `factories_launch_pending_display`) would never be attempted
        // again once the last-registered feature intro has been seen, since
        // `check_and_trigger_all_modals` only runs once at startup.
        if self.check_and_trigger_factories_launch_modal(ctx) {
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

    /// Returns whether the Factories launch modal is currently open.
    pub fn is_factories_launch_modal_open(&self) -> bool {
        self.is_factories_launch_modal_open && self.target_window_id.is_some()
    }

    /// Dismissing this modal advances the queue to the checks that follow it
    /// in `check_and_trigger_all_modals` (HOA onboarding, then build-plan
    /// migration), mirroring how the other launch modals advance to the next
    /// step on their own dismissal.
    pub fn mark_factories_launch_modal_dismissed(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.set_factories_launch_modal_open(false, ctx) {
            return;
        }
        self.resume_modal_checks_after_factories_launch(ctx);
    }

    #[cfg(debug_assertions)]
    pub fn force_open_factories_launch_modal(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_factories_launch_modal_open(true, ctx);
    }

    fn set_factories_launch_modal_open(
        &mut self,
        is_open: bool,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if self.is_factories_launch_modal_open != is_open {
            self.is_factories_launch_modal_open = is_open;
            ctx.emit(OneTimeModalEvent::VisibilityChanged { is_open });
            return true;
        }
        false
    }

    /// Resumes the modal-check chain after the Factories launch modal's own
    /// slot, without retrying the Factories check itself (retrying inline
    /// from a lost/failed claim would immediately hammer the claim endpoint
    /// again; see `claim_and_show_factories_launch_modal`).
    fn resume_modal_checks_after_factories_launch(&mut self, ctx: &mut ModelContext<Self>) {
        if self.check_and_trigger_hoa_onboarding(ctx) {
            return;
        }
        self.check_and_trigger_build_plan_migration_modal(ctx);
    }

    /// Returns true if any one-time modal is currently open, or if the
    /// Factories launch modal's slot is reserved for an in-flight impression
    /// claim (see `pending_factories_launch_claim`).
    pub fn is_any_modal_open(&self) -> bool {
        (self.is_oz_launch_modal_open
            || self.is_openwarp_launch_modal_open
            || self.is_orchestration_launch_modal_open
            || self.is_agent_cli_launch_modal_open
            || self.is_auto_handoff_sleep_modal_open
            || self.is_build_plan_migration_modal_open
            || self.is_free_ai_removal_modal_open
            || self.is_factories_launch_modal_open
            || self.pending_factories_launch_claim
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

    #[cfg(debug_assertions)]
    pub fn force_open_agent_cli_launch_modal(&mut self, ctx: &mut ModelContext<Self>) {
        self.set_agent_cli_launch_modal_open(true, ctx);
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

    fn set_agent_cli_launch_modal_open(
        &mut self,
        is_open: bool,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if self.is_agent_cli_launch_modal_open != is_open {
            self.is_agent_cli_launch_modal_open = is_open;
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

        if self.check_and_trigger_agent_cli_launch_modal(ctx) {
            return;
        }

        if self.check_and_trigger_free_ai_removal_modal(ctx) {
            return;
        }

        if self.check_and_trigger_feature_intro_modal(ctx) {
            return;
        }

        if self.check_and_trigger_factories_launch_modal(ctx) {
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
        self.maybe_check_and_trigger_feature_intro_modal(ctx);
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
        // Never show one-time modals on WASM. `check_and_trigger_all_modals` already
        // guards its own call, but `maybe_recheck_free_ai_removal_modal` and
        // `resume_modal_checks_after_feature_intro` call this directly (e.g. from an
        // async billing/usage update), so the guard belongs here too.
        if cfg!(target_family = "wasm") {
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

    fn check_and_trigger_agent_cli_launch_modal(&mut self, ctx: &mut ModelContext<Self>) -> bool {
        if !FeatureFlag::AgentCliLaunchModal.is_enabled() {
            return false;
        }

        let ai_settings = AISettings::as_ref(ctx);
        if *ai_settings.did_check_to_trigger_agent_cli_launch_modal {
            return false;
        }

        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            if let Err(e) = settings
                .did_check_to_trigger_agent_cli_launch_modal
                .set_value(true, ctx)
            {
                log::warn!("Failed to mark Warp Agent CLI launch modal as dismissed: {e}");
            }
        });

        let should_show = !matches!(ChannelState::channel(), Channel::Integration);
        self.set_agent_cli_launch_modal_open(should_show, ctx);
        should_show
    }

    /// Re-runs `check_and_trigger_feature_intro_modal` outside the initial startup
    /// check, e.g. when a fresh experiments fetch makes a server-targeted intro
    /// newly eligible, or when a higher-priority modal ahead of it in
    /// `check_and_trigger_all_modals` is dismissed.
    fn maybe_check_and_trigger_feature_intro_modal(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.has_completed_initial_modal_checks
            || self.is_any_modal_open()
            || self.active_feature_intro.is_some()
        {
            return;
        }
        self.check_and_trigger_feature_intro_modal(ctx);
    }

    fn check_and_trigger_feature_intro_modal(&mut self, ctx: &mut ModelContext<Self>) -> bool {
        // Show the first registered, unseen feature intro that the user is currently
        // eligible for (see `FEATURE_INTROS`). An unseen but ineligible intro (e.g. a
        // server-targeted launch the user isn't enrolled in yet) is left unseen rather
        // than consumed, so it can still show once the user becomes eligible.
        let next = FEATURE_INTROS.iter().find(|intro| {
            !AISettings::as_ref(ctx).is_feature_intro_seen(intro.id.as_key())
                && (intro.eligible)(ctx)
        });
        let Some(intro) = next else {
            return false;
        };
        let id = intro.id;

        if matches!(ChannelState::channel(), Channel::Integration) {
            return false;
        }

        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings.mark_feature_intro_seen(id.as_key(), ctx);
        });
        self.set_active_feature_intro(Some(id), ctx);
        send_telemetry_from_ctx!(FeatureIntroModalTelemetryEvent::Shown { feature: id }, ctx);
        true
    }

    /// Re-runs `check_and_trigger_factories_launch_modal` outside the initial
    /// startup check, e.g. when a fresh experiments fetch makes the server-
    /// validated CTA URL newly available (see
    /// `UserWorkspaces::has_validated_factories_launch_modal_cta_url`).
    fn maybe_check_and_trigger_factories_launch_modal(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.has_completed_initial_modal_checks
            || self.is_any_modal_open()
            || self.pending_factories_launch_claim
            // The feature-intro popover is intentionally excluded from
            // `is_any_modal_open` (it's non-blocking), but Factories must
            // still wait for it: starting a claim now could resolve into a
            // centered, focus-stealing modal stacked on top of the popover.
            || self.active_feature_intro.is_some()
        {
            return;
        }
        self.check_and_trigger_factories_launch_modal(ctx);
    }

    /// Applies a Factories launch claim that already won
    /// (`factories_launch_pending_display`) but was held back because the
    /// feature-intro popover was open when it resolved. Never starts a new
    /// claim; it only releases a win that's already been decided. Returns
    /// `true` when it displayed the modal.
    fn maybe_display_pending_factories_launch_modal(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> bool {
        if !self.factories_launch_pending_display
            || self.is_any_modal_open()
            || self.active_feature_intro.is_some()
        {
            return false;
        }
        self.factories_launch_pending_display = false;
        self.set_factories_launch_modal_open(true, ctx);
        send_telemetry_from_ctx!(FactoriesLaunchModalTelemetryEvent::Shown, ctx);
        true
    }

    fn check_and_trigger_factories_launch_modal(&mut self, ctx: &mut ModelContext<Self>) -> bool {
        if self.maybe_display_pending_factories_launch_modal(ctx) {
            return true;
        }

        if !FeatureFlag::FactoriesLaunchModal.is_enabled() {
            return false;
        }

        if AISettings::as_ref(ctx).is_feature_intro_seen(FACTORIES_LAUNCH_SEEN_KEY) {
            return false;
        }

        if self.pending_factories_launch_claim {
            return false;
        }

        // Purely server-driven: the feature flag reflects cohort membership, and a
        // validated CTA URL ensures the modal never shows before a real booking
        // link is configured (never falls back to the generic Contact Sales page).
        if !UserWorkspaces::as_ref(ctx).has_validated_factories_launch_modal_cta_url() {
            return false;
        }

        if matches!(ChannelState::channel(), Channel::Integration) {
            return false;
        }

        self.claim_and_show_factories_launch_modal(ctx);
        true
    }

    /// Wins the atomic, cross-device impression claim before actually showing
    /// the Factories launch modal (see
    /// `AuthClient::claim_feature_intro_impression`). The one-time seen marker
    /// is written only once the outcome is known: on a win (`Ok(true)`) or a
    /// genuine loss to another device (`Ok(false)`), never on a request error,
    /// so a transient failure or being offline leaves the modal eligible to
    /// retry on the next recheck instead of silently burning the user's only
    /// impression.
    fn claim_and_show_factories_launch_modal(&mut self, ctx: &mut ModelContext<Self>) {
        let auth_client = ServerApiProvider::as_ref(ctx).get_auth_client();
        self.claim_and_show_factories_launch_modal_with_claim(
            move || async move {
                auth_client
                    .claim_feature_intro_impression(FACTORIES_LAUNCH_SEEN_KEY)
                    .await
            },
            FACTORIES_LAUNCH_CLAIM_TIMEOUT,
            ctx,
        );
    }

    /// The body of `claim_and_show_factories_launch_modal`, with the claim request and its
    /// timeout both injectable so tests can force the timeout path deterministically — with
    /// a claim future that never resolves and a very short timeout — instead of waiting out
    /// the real duration. The underlying GraphQL request has no transport-level timeout of
    /// its own, so without this bound a stalled request would never resolve, leaving
    /// `pending_factories_launch_claim` (and therefore `is_any_modal_open`) stuck `true` for
    /// the rest of the session and silently suppressing every other one-time modal.
    ///
    /// Every terminal outcome — success, a request error, a timeout, or the spawned future
    /// being aborted before it resolves — clears the reservation and resumes the modal queue.
    fn claim_and_show_factories_launch_modal_with_claim<F, Fut>(
        &mut self,
        claim: F,
        timeout: Duration,
        ctx: &mut ModelContext<Self>,
    ) where
        // `Spawnable`/`SpawnableOutput` drop the `Send` requirement on wasm, where the
        // underlying `AuthClient` future (backed by `wasm_bindgen_futures::JsFuture`) isn't
        // `Send` because there's no background thread to send it to.
        F: FnOnce() -> Fut + SpawnableOutput + 'static,
        Fut: Future<Output = Result<bool, anyhow::Error>> + Spawnable,
    {
        self.pending_factories_launch_claim = true;
        ctx.spawn_abortable(
            async move { claim().with_timeout(timeout).await },
            move |me, result, ctx| {
                me.pending_factories_launch_claim = false;
                match result {
                    Ok(Ok(claimed)) => {
                        AISettings::handle(ctx).update(ctx, |settings, ctx| {
                            settings.mark_feature_intro_seen(FACTORIES_LAUNCH_SEEN_KEY, ctx);
                        });
                        if claimed {
                            // The feature-intro popover is intentionally excluded from
                            // `is_any_modal_open`, so a win that resolves while it's open
                            // must be held rather than stacking the centered Factories
                            // modal on top of it.
                            if me.active_feature_intro.is_some() {
                                me.factories_launch_pending_display = true;
                            } else {
                                me.set_factories_launch_modal_open(true, ctx);
                                send_telemetry_from_ctx!(
                                    FactoriesLaunchModalTelemetryEvent::Shown,
                                    ctx
                                );
                            }
                        } else {
                            // Another device already won the claim; the modal has
                            // genuinely been shown, so it's correctly marked seen above.
                            me.resume_modal_checks_after_factories_launch(ctx);
                        }
                    }
                    Ok(Err(e)) => {
                        log::warn!("Failed to claim Factories launch modal impression: {e:#}");
                        me.resume_modal_checks_after_factories_launch(ctx);
                    }
                    Err(_timed_out) => {
                        // Accepted gap: this only stops waiting on the client side. If the
                        // server commits the claim just after the timeout elapses, this
                        // device's seen marker stays unset and a later retry receives
                        // `Ok(false)` like any non-winning caller, so the user never sees
                        // the modal despite having won the claim. Closing it needs a
                        // client-generated idempotency token or a claim-status lookup,
                        // which is disproportionate for a launch announcement given the
                        // failure is rare, costs at most one impression, and errs toward
                        // under- rather than over-showing.
                        log::warn!(
                            "Timed out waiting for the Factories launch modal impression claim"
                        );
                        me.resume_modal_checks_after_factories_launch(ctx);
                    }
                }
            },
            move |me, ctx| {
                // The claim was aborted before resolving. Nothing currently calls `abort()`
                // on this future, but releasing the reservation here keeps the invariant —
                // every terminal outcome resumes the queue — true even if a future teardown
                // path starts doing so.
                me.pending_factories_launch_claim = false;
                me.resume_modal_checks_after_factories_launch(ctx);
            },
        );
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
        let Some(target_window_id) = self
            .target_window_id
            .or_else(|| ctx.windows().active_window())
        else {
            return false;
        };
        let Some(current_team) = user_workspaces.team_for_window(target_window_id) else {
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
        let has_sunsetted_to_build = user_workspaces
            .current_workspace()
            .and_then(|workspace| workspace.billing_metadata.service_agreements.first())
            .is_some_and(|agreement| agreement.sunsetted_to_build_ts.is_some());

        if !has_sunsetted_to_build {
            return false;
        }

        // All conditions met, show the modal
        self.target_window_id = Some(target_window_id);
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
