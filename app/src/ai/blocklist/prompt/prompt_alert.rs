use ai::api_keys::ApiKeyManager;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Flex, FormattedTextElement,
    HighlightedHyperlink, HyperlinkLens, MainAxisAlignment, MainAxisSize, ParentElement,
};
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::ai::blocklist::error_color;
use crate::ai::AIRequestUsageModel;
use crate::auth::AuthStateProvider;
use crate::network::NetworkStatus;
use crate::server::ids::ServerId;
use crate::settings::PrivacySettings;
use crate::settings_view::SettingsSection;
use crate::ui_components::icons::Icon;
use crate::workspace::WorkspaceAction;
use crate::workspaces::user_workspaces::UserWorkspaces;

const ANONYMOUS_USER_REQUEST_LIMIT_SOFT_GATE_PERCENTAGE: f32 = 0.5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptAlertAction {
    SignUpClickedForAnonymousUser,
    OpenSettingsClicked,
    OpenPrivacySettingsClicked,
    ManageBillingClicked { team_uid: ServerId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptAlertEvent {
    SignupAnonymousUser,
    OpenBillingAndUsagePage,
    OpenPrivacyPage,
    OpenBillingPortal { team_uid: ServerId },
}

/// The alert state of the chip that appears to the right of certain parts of the prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptAlertState {
    /// The user is offline (no connection).
    NoConnection,
    /// Telemetry is disabled and the user is on a free tier.
    /// Free tier users must enable telemetry or upgrade to use AI features.
    TelemetryDisabledOnFreeTier,
    /// An anonymous user has reached a certain percentage of requests used.
    /// This doesn't use a primary text to avoid being too in-your-face.
    AnonymousUserRequestLimitSoftGate,
    /// An anonymous user has reached the request limit.
    AnonymousUserRequestLimitHardGate,
    /// The user is delinquent due to a payment issue.
    DelinquentDueToPaymentIssue,
    /// Overages could be turned on, but aren't enabled.
    OveragesToggleableButNotEnabled,
    /// Overages are on, but the spend limit is too low.
    MonthlyOveragesSpendLimitReached,
    /// The user has reached the request limit.
    RequestLimitReached,
    /// No alert should be displayed.
    NoAlert,
}

pub struct PromptAlertView {
    state: PromptAlertState,
    action_hyperlink: HighlightedHyperlink,
}

impl PromptAlertView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let request_usage_model = AIRequestUsageModel::handle(ctx);
        let user_workspaces = UserWorkspaces::handle(ctx);
        let network_status = NetworkStatus::handle(ctx);
        let privacy_settings = PrivacySettings::handle(ctx);
        let api_key_manager = ApiKeyManager::handle(ctx);

        ctx.subscribe_to_model(&request_usage_model, |me, _, _, ctx| {
            me.state = Self::determine_state(ctx);
            ctx.notify();
        });

        ctx.subscribe_to_model(&user_workspaces, |me, _, _, ctx| {
            me.state = Self::determine_state(ctx);
            ctx.notify();
        });

        ctx.subscribe_to_model(&network_status, |me, _, _, ctx| {
            me.state = Self::determine_state(ctx);
            ctx.notify();
        });

        ctx.subscribe_to_model(&privacy_settings, |me, _, _, ctx| {
            me.state = Self::determine_state(ctx);
            ctx.notify();
        });

        ctx.subscribe_to_model(&api_key_manager, |me, _, _, ctx| {
            me.state = Self::determine_state(ctx);
            ctx.notify();
        });

        Self {
            state: Self::determine_state(ctx),
            action_hyperlink: Default::default(),
        }
    }

    pub fn determine_state(app: &AppContext) -> PromptAlertState {
        // First, if the user is offline, no AI features will work.
        if !NetworkStatus::as_ref(app).is_online() {
            return PromptAlertState::NoConnection;
        }

        // Check if telemetry is disabled for free tier users.
        // Free tier users must enable telemetry or upgrade to use AI features.
        let privacy_settings = PrivacySettings::as_ref(app);
        if !privacy_settings.is_telemetry_enabled {
            // Fail safe: if billing status is unknown, assume paid to avoid showing confusing message to paying users
            let is_on_paid_plan = UserWorkspaces::as_ref(app)
                .current_workspace()
                .map(|w| w.billing_metadata.is_user_on_paid_plan())
                .unwrap_or(true);

            if !is_on_paid_plan {
                return PromptAlertState::TelemetryDisabledOnFreeTier;
            }
        }

        let request_usage_model = AIRequestUsageModel::as_ref(app);
        let has_requests_remaining = request_usage_model.has_requests_remaining();
        let auth_state = AuthStateProvider::as_ref(app).get();

        // Next, if the user is anonymous, we check if they have reached a certain percentage of requests used.
        if auth_state
            .is_anonymous_user_feature_gated()
            .unwrap_or_default()
        {
            let percentage_used = request_usage_model.request_percentage_used();

            if percentage_used >= ANONYMOUS_USER_REQUEST_LIMIT_SOFT_GATE_PERCENTAGE {
                if has_requests_remaining {
                    return PromptAlertState::AnonymousUserRequestLimitSoftGate;
                } else {
                    return PromptAlertState::AnonymousUserRequestLimitHardGate;
                }
            }
        }

        // Next, make sure the user isn't delinquent in their plan.
        let workspace = UserWorkspaces::as_ref(app).current_workspace();
        if workspace.is_some_and(|w| w.billing_metadata.is_delinquent_due_to_payment_issue()) {
            return PromptAlertState::DelinquentDueToPaymentIssue;
        }

        // If there is ever any ai remaining, no alert
        if request_usage_model.has_any_ai_remaining(app) {
            return PromptAlertState::NoAlert;
        }

        // Check if overages are available.
        if let Some(workspace) = workspace {
            let are_overages_toggleable = workspace.are_overages_toggleable();
            let are_overages_enabled = workspace.are_overages_enabled();

            if are_overages_toggleable {
                if are_overages_enabled {
                    return PromptAlertState::MonthlyOveragesSpendLimitReached;
                } else {
                    return PromptAlertState::OveragesToggleableButNotEnabled;
                }
            }
        }

        // If overages aren't available, and since we already checked that the user
        // has no requests remaining, we can show the generic request limit reached alert.
        PromptAlertState::RequestLimitReached
    }

    pub fn is_no_alert(&self) -> bool {
        matches!(self.state, PromptAlertState::NoAlert)
    }

    pub fn state(&self) -> &PromptAlertState {
        &self.state
    }

    pub fn does_alert_block_ai_requests(app: &AppContext) -> bool {
        does_alert_block_ai_requests(&Self::determine_state(app))
    }

    fn primary_text(
        &self,
        state: &PromptAlertState,
        text_fragments: &mut Vec<FormattedTextFragment>,
    ) {
        // Add leading space to separate text from icon.
        //
        // Use this instead of hardcoded margin so it scales with font size and is consistent
        // with the space between this primary fragment and the option hyperlink fragment.
        text_fragments.push(FormattedTextFragment::plain_text("  "));
        match state {
            PromptAlertState::NoConnection => {
                text_fragments.push(FormattedTextFragment::plain_text(i18n::t(
                    "ai.prompt_alert.no_connection",
                )));
            }
            PromptAlertState::TelemetryDisabledOnFreeTier => {
                text_fragments.push(FormattedTextFragment::plain_text(i18n::t(
                    "ai.prompt_alert.telemetry_disabled_primary",
                )));
            }
            PromptAlertState::AnonymousUserRequestLimitSoftGate => {
                text_fragments.push(FormattedTextFragment::plain_text(String::new()));
            }
            PromptAlertState::AnonymousUserRequestLimitHardGate => {
                text_fragments.push(FormattedTextFragment::plain_text(i18n::t(
                    "ai.prompt_alert.anonymous_limit_hard_gate_primary",
                )));
            }
            PromptAlertState::DelinquentDueToPaymentIssue => {
                text_fragments.push(FormattedTextFragment::plain_text(i18n::t(
                    "ai.prompt_alert.delinquent_primary",
                )));
            }
            PromptAlertState::OveragesToggleableButNotEnabled
            | PromptAlertState::MonthlyOveragesSpendLimitReached
            | PromptAlertState::RequestLimitReached => {
                text_fragments.push(FormattedTextFragment::plain_text(i18n::t(
                    "ai.prompt_alert.out_of_credits",
                )));
            }
            PromptAlertState::NoAlert => {}
        }
    }

    fn action_hyperlink(
        &self,
        state: &PromptAlertState,
        text_fragments: &mut Vec<FormattedTextFragment>,
        app: &AppContext,
    ) {
        let auth_state = AuthStateProvider::as_ref(app).get();
        let current_team = UserWorkspaces::as_ref(app).current_team();
        let has_admin_permissions = current_team.is_some_and(|team| {
            team.has_admin_permissions(&auth_state.user_email().unwrap_or_default())
        });

        match state {
            PromptAlertState::NoConnection => {}
            PromptAlertState::TelemetryDisabledOnFreeTier => {
                // Show "enable analytics" action link
                text_fragments.push(FormattedTextFragment::plain_text("  "));
                text_fragments.push(FormattedTextFragment::hyperlink_action(
                    i18n::t("ai.prompt_alert.enable_analytics"),
                    PromptAlertAction::OpenPrivacySettingsClicked,
                ));

                // Show "or upgrade to Build" link
                text_fragments.push(FormattedTextFragment::plain_text(i18n::t("common.or")));
                let upgrade_url = if let Some(team) = UserWorkspaces::as_ref(app).current_team() {
                    UserWorkspaces::upgrade_link_for_team(team.uid)
                } else {
                    let user_id = auth_state.user_id().unwrap_or_default();
                    UserWorkspaces::upgrade_link(user_id)
                };
                text_fragments.push(FormattedTextFragment::hyperlink(
                    i18n::t("ai.prompt_alert.upgrade"),
                    upgrade_url,
                ));
                text_fragments.push(FormattedTextFragment::plain_text("."));
            }
            PromptAlertState::AnonymousUserRequestLimitSoftGate
            | PromptAlertState::AnonymousUserRequestLimitHardGate => {
                text_fragments.push(FormattedTextFragment::plain_text("  "));
                text_fragments.push(FormattedTextFragment::hyperlink_action(
                    i18n::t("ai.prompt_alert.sign_up_for_more_credits"),
                    PromptAlertAction::SignUpClickedForAnonymousUser,
                ));
            }
            PromptAlertState::DelinquentDueToPaymentIssue => {
                // Check if user is team admin with billing history
                let has_billing_history = current_team
                    .map(|team| team.has_billing_history)
                    .unwrap_or_default();
                if has_admin_permissions && has_billing_history {
                    text_fragments.push(FormattedTextFragment::plain_text("  "));
                    text_fragments.push(FormattedTextFragment::hyperlink_action(
                        i18n::t("ai.prompt_alert.manage_billing"),
                        PromptAlertAction::ManageBillingClicked {
                            team_uid: current_team.map(|team| team.uid).unwrap_or_default(),
                        },
                    ));
                } else {
                    text_fragments.push(FormattedTextFragment::plain_text(i18n::t(
                        "ai.prompt_alert.contact_team_admin",
                    )));
                }
            }
            PromptAlertState::OveragesToggleableButNotEnabled => {
                if has_admin_permissions {
                    text_fragments.push(FormattedTextFragment::plain_text("  "));
                    text_fragments.push(FormattedTextFragment::hyperlink_action(
                        i18n::t("ai.prompt_alert.enable_premium_overages"),
                        PromptAlertAction::OpenSettingsClicked,
                    ));
                } else {
                    text_fragments.push(FormattedTextFragment::plain_text(i18n::t(
                        "ai.prompt_alert.ask_admin_enable_overages",
                    )));
                }
            }
            PromptAlertState::MonthlyOveragesSpendLimitReached => {
                if has_admin_permissions {
                    text_fragments.push(FormattedTextFragment::plain_text("  "));
                    text_fragments.push(FormattedTextFragment::hyperlink_action(
                        i18n::t("ai.prompt_alert.increase_monthly_spend_limit"),
                        PromptAlertAction::OpenSettingsClicked,
                    ));
                } else {
                    text_fragments.push(FormattedTextFragment::plain_text(i18n::t(
                        "ai.prompt_alert.ask_admin_increase_overages",
                    )));
                }
            }
            PromptAlertState::RequestLimitReached => {
                text_fragments.push(FormattedTextFragment::plain_text("  "));
                if let Some(team) = UserWorkspaces::as_ref(app).current_team() {
                    if team.billing_metadata.can_upgrade_to_higher_tier_plan() {
                        let upgrade_url = UserWorkspaces::upgrade_link_for_team(team.uid);
                        let upgrade_text = if !has_admin_permissions {
                            i18n::t("ai.prompt_alert.compare_plans")
                        } else if team.billing_metadata.can_upgrade_to_build_plan() {
                            i18n::t("ai.prompt_alert.upgrade_to_build")
                        } else {
                            i18n::t("ai.prompt_alert.upgrade")
                        };

                        text_fragments
                            .push(FormattedTextFragment::hyperlink(upgrade_text, upgrade_url));
                    } else {
                        text_fragments.push(FormattedTextFragment::hyperlink(
                            i18n::t("ai.prompt_alert.contact_support"),
                            "mailto:support@warp.dev".to_owned(),
                        ));
                    }
                } else {
                    let user_id = auth_state.user_id().unwrap_or_default();
                    let upgrade_url = UserWorkspaces::upgrade_link(user_id);
                    let label =
                        if let Some(workspace) = UserWorkspaces::as_ref(app).current_workspace() {
                            if workspace.billing_metadata.can_upgrade_to_build_plan() {
                                i18n::t("ai.prompt_alert.upgrade_to_build")
                            } else {
                                i18n::t("ai.prompt_alert.upgrade")
                            }
                        } else {
                            i18n::t("ai.prompt_alert.upgrade")
                        };
                    text_fragments.push(FormattedTextFragment::hyperlink(label, upgrade_url));
                }
                if UserWorkspaces::as_ref(app).is_byo_api_key_enabled(app) {
                    text_fragments.push(FormattedTextFragment::plain_text(i18n::t("common.or")));
                    text_fragments.push(FormattedTextFragment::hyperlink_action(
                        i18n::t("ai.prompt_alert.use_own_api_keys"),
                        WorkspaceAction::ShowSettingsPageWithSearch {
                            search_query: "api".to_string(),
                            section: Some(SettingsSection::WarpAgent),
                        },
                    ));
                }
            }
            PromptAlertState::NoAlert => {}
        }
    }
}

fn does_alert_block_ai_requests(state: &PromptAlertState) -> bool {
    match state {
        PromptAlertState::AnonymousUserRequestLimitSoftGate | PromptAlertState::NoAlert => false,
        PromptAlertState::NoConnection
        | PromptAlertState::TelemetryDisabledOnFreeTier
        | PromptAlertState::AnonymousUserRequestLimitHardGate
        | PromptAlertState::DelinquentDueToPaymentIssue
        | PromptAlertState::OveragesToggleableButNotEnabled
        | PromptAlertState::MonthlyOveragesSpendLimitReached
        | PromptAlertState::RequestLimitReached => true,
    }
}

impl Entity for PromptAlertView {
    type Event = PromptAlertEvent;
}

impl View for PromptAlertView {
    fn ui_name() -> &'static str {
        "PromptAlertView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let state = Self::determine_state(app);
        let mut text_fragments = vec![];

        self.primary_text(&state, &mut text_fragments);

        let auth_state = AuthStateProvider::as_ref(app).get();
        let current_team = UserWorkspaces::as_ref(app).current_team();
        let has_admin_permissions = auth_state
            .user_email()
            .zip(current_team)
            .is_some_and(|(email, team)| team.has_admin_permissions(&email));

        let can_purchase_addon_credits = current_team
            .and_then(|team| team.billing_metadata.tier.purchase_add_on_credits_policy)
            .is_some_and(|policy| policy.enabled);

        let suggest_buy_credits = can_purchase_addon_credits
            && has_admin_permissions
            && matches!(
                state,
                PromptAlertState::RequestLimitReached
                    | PromptAlertState::OveragesToggleableButNotEnabled
                    | PromptAlertState::MonthlyOveragesSpendLimitReached
            );

        if suggest_buy_credits {
            text_fragments.push(FormattedTextFragment::plain_text("  "));
            text_fragments.push(FormattedTextFragment::hyperlink_action(
                i18n::t("ai.prompt_alert.add_credits"),
                WorkspaceAction::ShowSettingsPage(SettingsSection::BillingAndUsage),
            ));
        } else {
            self.action_hyperlink(&state, &mut text_fragments, app);
        }

        let formatted_text_element = FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(text_fragments)]),
            appearance.ui_font_size(),
            appearance.ui_font_family(),
            appearance.ui_font_family(),
            error_color(appearance.theme()),
            self.action_hyperlink.clone(),
        )
        .with_line_height_ratio(1.)
        .with_hyperlink_font_color(appearance.theme().ansi_fg_blue())
        .with_no_text_wrapping()
        .register_default_click_handlers_with_action_support(|hyperlink_lens, event, ctx| {
            match hyperlink_lens {
                HyperlinkLens::Url(url) => {
                    ctx.open_url(url);
                }
                HyperlinkLens::Action(action_ref) => {
                    if let Some(action) = action_ref.as_any().downcast_ref::<PromptAlertAction>() {
                        event.dispatch_typed_action(action.clone());
                    } else if let Some(action) =
                        action_ref.as_any().downcast_ref::<WorkspaceAction>()
                    {
                        event.dispatch_typed_action(action.clone());
                    }
                }
            }
        })
        .finish();

        let icon_size = appearance.ui_font_size();

        let mut chip_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::End);
        if does_alert_block_ai_requests(&self.state) {
            chip_row.add_child(
                ConstrainedBox::new(
                    Icon::AlertTriangle
                        .to_warpui_icon(error_color(appearance.theme()).into())
                        .finish(),
                )
                .with_width(icon_size)
                .with_height(icon_size)
                .finish(),
            )
        }

        chip_row.add_child(formatted_text_element);

        Container::new(chip_row.finish())
            .with_margin_right(16.)
            .finish()
    }
}

impl TypedActionView for PromptAlertView {
    type Action = PromptAlertAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            PromptAlertAction::SignUpClickedForAnonymousUser => {
                ctx.emit(PromptAlertEvent::SignupAnonymousUser);
            }
            PromptAlertAction::OpenSettingsClicked => {
                ctx.emit(PromptAlertEvent::OpenBillingAndUsagePage);
            }
            PromptAlertAction::OpenPrivacySettingsClicked => {
                ctx.emit(PromptAlertEvent::OpenPrivacyPage);
            }
            PromptAlertAction::ManageBillingClicked { team_uid } => {
                ctx.emit(PromptAlertEvent::OpenBillingPortal {
                    team_uid: *team_uid,
                });
            }
        }
    }
}
