use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use chrono::Local;
use itertools::Itertools;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use settings::Setting;
use thousands::Separable;
use warp_core::features::FeatureFlag;
use warp_core::ui::appearance::Appearance;
use warp_graphql::billing::AddonCreditsOption;
use warpui::elements::{
    Align, Border, Clipped, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty,
    Expanded, Flex, FormattedTextElement, HighlightedHyperlink, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, ParentElement, Radius, Shrinkable, Text, Wrap,
};
use warpui::fonts::{Properties, Weight};
use warpui::prelude::ChildView;
use warpui::ui_components::button::{ButtonVariant, TextAndIcon, TextAndIconAlignment};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::switch::SwitchStateHandle;
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, UpdateView, View,
    ViewContext, ViewHandle,
};

use super::billing_and_usage::billing_cycle_usage_section::BillingCycleUsageSectionView;
use super::billing_and_usage::overage_limit_modal::{SpendingLimitModal, SpendingLimitModalEvent};
use super::billing_and_usage::usage_history_entry::UsageHistoryEntry;
use super::billing_and_usage::usage_history_model::UsageHistoryModel;
pub use super::billing_and_usage_page::BillingAndUsagePageEvent;
use super::billing_and_usage_page::{BillingAndUsagePageAction, BillingUsageTab};
use super::settings_page::{render_customer_type_badge, render_info_icon, AdditionalInfo};
use super::SettingsSection;
use crate::ai::request_usage_model::{
    BonusGrant, BonusGrantScope, BonusGrantType, AMBIENT_AGENT_TRIAL_CREDIT_THRESHOLD,
};
use crate::ai::AIRequestUsageModel;
use crate::auth::auth_state::AuthState;
use crate::auth::auth_view_modal::AuthViewVariant;
use crate::auth::{AuthManager, AuthStateProvider};
use crate::modal::{Modal, ModalEvent, ModalViewState};
use crate::pricing::PricingInfoModel;
use crate::server::ids::ServerId;
use crate::server::telemetry::TelemetryEvent;
use crate::settings::ai::AISettings;
use crate::ui_components::blended_colors;
use crate::ui_components::buttons::icon_button;
use crate::ui_components::icons::Icon;
use crate::ui_components::tab_selector::{self, SettingsTab};
use crate::view_components::action_button::{ActionButton, PrimaryTheme, SecondaryTheme};
use crate::view_components::ToastFlavor;
use crate::workspaces::update_manager::TeamUpdateManager;
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};
use crate::workspaces::workspace::{CustomerType, Workspace, WorkspaceUid};
use crate::{send_telemetry_from_ctx, WorkspaceAction};

const ADDON_CREDITS_DESCRIPTION: &str = "Add-on credits are purchased in prepaid packages that roll over each billing cycle and expire after one year. The more you purchase, the better the per-credit rate. Once your base plan credits are used, add-on credits will be consumed.";
const ADDITIONAL_ADDON_CREDITS_DESCRIPTION_FOR_TEAM: &str =
    "Purchased add-on credits are added to your personal balance.";
const MANAGED_AUTO_RELOAD_HEADER: &str = "Auto-reload is enabled";

const ADDON_CREDITS_DELINQUENT_WARNING_STRING: &str =
    "Restricted due to billing issue. Update your payment method to purchase add-on credits.";
const ADDON_CREDITS_NON_ADMIN_DELINQUENT_WARNING_STRING: &str =
    "Restricted due to billing issue. Contact your team admin to update their payment method.";
const RESTRICTED_BILLING_USAGE_WARNING_STRING: &str = "Auto reload is disabled due to recent failed reload. Please update your payment method and try again.";
const RESTRICTED_BILLING_USAGE_NON_ADMIN_WARNING_STRING: &str = "Auto reload is disabled due to recent failed reload. Contact your team admin to update their payment method.";

const HEADER_FONT_SIZE: f32 = 16.;

pub(super) const BASE_CREDITS_DOT_COLOR: ColorU = ColorU {
    r: 207,
    g: 145,
    b: 216,
    a: 255,
};
pub(super) const BONUS_CREDITS_DOT_COLOR: ColorU = ColorU {
    r: 236,
    g: 148,
    b: 85,
    a: 255,
};
pub(super) const PAYG_CREDITS_DOT_COLOR: ColorU = ColorU {
    r: 94,
    g: 177,
    b: 239,
    a: 255,
};
pub(super) const AMBIENT_CREDITS_DOT_COLOR: ColorU = ColorU {
    r: 99,
    g: 102,
    b: 241,
    a: 255,
};
pub(super) const AGGREGATE_CREDITS_DOT_COLOR: ColorU = ColorU {
    r: 140,
    g: 140,
    b: 140,
    a: 255,
};
const DEFAULT_MAX_MONTHLY_SPEND_CENTS: i32 = 20_000;
const AMBIENT_AGENT_TRIAL_TITLE: &str = "Cloud agent trial";

#[derive(Default)]
struct PlanSectionMouseStates {
    manage_billing_link: MouseStateHandle,
    open_admin_panel_link: MouseStateHandle,
    admin_panel_link: MouseStateHandle,
    refresh_button: MouseStateHandle,
}

#[derive(Default)]
struct BuyCreditsMouseStates {
    addon_info_icon: MouseStateHandle,
    edit_monthly_limit: MouseStateHandle,
    auto_reload_switch: SwitchStateHandle,
    auto_reload_info: MouseStateHandle,
    buy_button: MouseStateHandle,
}
#[derive(Default)]
struct AmbientTrialMouseStates {
    new_agent_button: MouseStateHandle,
    buy_more_button: MouseStateHandle,
    dismiss_button: MouseStateHandle,
}

#[derive(Default)]
struct TabMouseStates {
    overview: MouseStateHandle,
    usage_history: MouseStateHandle,
}

struct AddonCreditsState {
    selected_denomination: usize,
    options: Vec<AddonCreditsOption>,
    denomination_buttons: Vec<ViewHandle<ActionButton>>,
    purchase_loading: bool,
}
enum AddonCreditsPanelState {
    IneligiblePlan(AddonCreditsRestriction),
    AutoreloadNonAdmin {
        description_text: String,
        warning_text: Option<&'static str>,
    },
    Purchase(AddonCreditsPurchaseState),
}

enum AddonCreditsRestriction {
    UpgradeToBuild {
        link_text: &'static str,
        url: String,
    },
    ContactAccountExecutive,
    ContactTeamAdmin,
}

struct AddonCreditsPurchaseState {
    description_text: String,
    auto_reload_enabled: bool,
    has_admin_permissions: bool,
    purchase_disabled: bool,
    auto_reload_switch_disabled: bool,
    price_label: String,
    auto_reload_tooltip_text: String,
    warning_text: Option<&'static str>,
}

struct UsageHistoryState {
    model: ModelHandle<UsageHistoryModel>,
    expanded_entries: HashMap<String, bool>,
    entry_mouse_states: RefCell<HashMap<String, MouseStateHandle>>,
    tooltip_mouse_states: RefCell<HashMap<String, MouseStateHandle>>,
    load_more_button: ViewHandle<ActionButton>,
}

struct GrantBucket {
    grants: Vec<BonusGrant>,
}

impl GrantBucket {
    fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }

    fn total_balance(&self) -> i64 {
        self.grants
            .iter()
            .map(|g| g.request_credits_remaining as i64)
            .sum()
    }

    fn expiry_label(&self) -> String {
        let expiries: Vec<_> = self.grants.iter().filter_map(|g| g.expiration).collect();
        if expiries.is_empty() {
            return String::new();
        }
        let first = expiries[0];
        if expiries
            .iter()
            .all(|e| e.date_naive() == first.date_naive())
        {
            let local = first.with_timezone(&Local);
            format!("Expires {}", local.format("%b %d, %Y"))
        } else {
            String::new()
        }
    }
}

struct ClassifiedGrants {
    personal: GrantBucket,
    team: GrantBucket,
}

impl ClassifiedGrants {
    fn new(grants: &[BonusGrant], workspace_uid: Option<WorkspaceUid>) -> Self {
        let now = chrono::Utc::now();
        let mut personal = Vec::new();
        let mut team = Vec::new();

        for grant in grants {
            if grant.expiration.is_some_and(|exp| now >= exp) {
                continue;
            }
            if grant.request_credits_remaining <= 0 {
                continue;
            }
            let in_user_scope = grant.scope == BonusGrantScope::User;
            let in_workspace_scope =
                workspace_uid.is_some_and(|uid| grant.scope == BonusGrantScope::Workspace(uid));
            if grant.grant_type == BonusGrantType::AmbientOnly {
                continue;
            } else if in_user_scope {
                personal.push(grant.clone());
            } else if in_workspace_scope {
                team.push(grant.clone());
            }
        }

        Self {
            personal: GrantBucket { grants: personal },
            team: GrantBucket { grants: team },
        }
    }

    fn has_any(&self) -> bool {
        !self.personal.is_empty() || !self.team.is_empty()
    }
}

pub struct BillingAndUsagePageV2View {
    auth_state: Arc<AuthState>,
    addon_credit_modal_state: ModalViewState<Modal<SpendingLimitModal>>,
    selected_tab: BillingUsageTab,
    usage_history: UsageHistoryState,
    addon_credits: AddonCreditsState,
    pending_auto_reload_toast: Option<String>,
    tab_mouse_states: TabMouseStates,
    plan_mouse_states: PlanSectionMouseStates,
    buy_credits_mouse_states: BuyCreditsMouseStates,
    ambient_trial_mouse_states: AmbientTrialMouseStates,
    billing_cycle_usage_section: ViewHandle<BillingCycleUsageSectionView>,
}

impl BillingAndUsagePageV2View {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _handle, event, ctx| {
            me.handle_workspaces_event(event, ctx);
            ctx.notify();
        });

        ctx.subscribe_to_model(&AuthManager::handle(ctx), |me, _, _, ctx| {
            me.refresh_addon_credits_settings(ctx);
            ctx.notify();
        });

        let team_update_manager = TeamUpdateManager::handle(ctx);
        ctx.subscribe_to_model(&team_update_manager, |_, _handle, _, ctx| {
            ctx.notify();
        });

        ctx.subscribe_to_model(&AIRequestUsageModel::handle(ctx), |_, _, _, ctx| {
            ctx.notify()
        });

        ctx.subscribe_to_model(
            &PricingInfoModel::handle(ctx),
            |me, _handle, _event, ctx| {
                me.update_addon_credits_options(ctx);
                me.refresh_addon_credits_settings(ctx);
                ctx.notify();
            },
        );

        let usage_history_model = ctx.add_model(UsageHistoryModel::new);
        ctx.subscribe_to_model(&usage_history_model, |_, _, _, ctx| {
            ctx.notify();
        });
        usage_history_model.update(ctx, |m, ctx| m.refresh_usage_history_async(ctx));

        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();

        let addon_credit_modal = ctx.add_typed_action_view(SpendingLimitModal::new);
        ctx.subscribe_to_view(&addon_credit_modal, |me, _, event, ctx| {
            me.handle_addon_credit_modal_event(event, ctx);
        });

        let addon_credit_modal_view = ctx.add_typed_action_view(|ctx| {
            Modal::new(
                Some("Monthly spending limit".to_string()),
                addon_credit_modal,
                ctx,
            )
            .with_header_style(UiComponentStyles {
                padding: Some(Coords::uniform(24.).bottom(16.)),
                ..Default::default()
            })
            .with_body_style(UiComponentStyles {
                padding: Some(Coords::uniform(24.).top(0.).bottom(12.)),
                ..Default::default()
            })
        });
        ctx.subscribe_to_view(&addon_credit_modal_view, |me, _, event, ctx| {
            me.handle_addon_credit_modal_close_event(event, ctx);
        });

        let load_more_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Load more", SecondaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(BillingAndUsagePageAction::RenderMoreUsageEntries);
            })
        });

        let billing_cycle_usage_section =
            ctx.add_typed_action_view(BillingCycleUsageSectionView::new);

        let mut me = Self {
            auth_state,
            addon_credit_modal_state: ModalViewState::new(addon_credit_modal_view),
            selected_tab: BillingUsageTab::Overview,
            usage_history: UsageHistoryState {
                model: usage_history_model,
                expanded_entries: HashMap::new(),
                entry_mouse_states: RefCell::new(HashMap::new()),
                tooltip_mouse_states: RefCell::new(HashMap::new()),
                load_more_button,
            },
            addon_credits: AddonCreditsState {
                selected_denomination: 0,
                options: Default::default(),
                denomination_buttons: Default::default(),
                purchase_loading: false,
            },
            pending_auto_reload_toast: None,
            tab_mouse_states: Default::default(),
            plan_mouse_states: Default::default(),
            buy_credits_mouse_states: Default::default(),
            ambient_trial_mouse_states: Default::default(),
            billing_cycle_usage_section,
        };
        me.update_addon_credits_options(ctx);
        me.refresh_addon_credits_settings(ctx);
        me
    }

    fn refresh_addon_credits_settings(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(workspace) = UserWorkspaces::as_ref(ctx).current_workspace() else {
            return;
        };
        let addon_credits_settings = &workspace.settings.addon_credits_settings;
        if addon_credits_settings.auto_reload_enabled {
            self.addon_credits.selected_denomination = addon_credits_settings
                .selected_auto_reload_credit_denomination
                .and_then(|amount| {
                    self.addon_credits
                        .options
                        .iter()
                        .find_position(|option| option.credits == amount)
                })
                .map_or(0, |pair| pair.0);
        }
        self.update_denomination_buttons_focus(ctx);
    }

    fn update_denomination_buttons_focus(&mut self, ctx: &mut ViewContext<Self>) {
        for (i, button_handle) in self.addon_credits.denomination_buttons.iter().enumerate() {
            ctx.update_view(button_handle, |button, ctx| {
                if i == self.addon_credits.selected_denomination {
                    button.set_theme(PrimaryTheme, ctx);
                } else {
                    button.set_theme(SecondaryTheme, ctx);
                }
            });
        }
    }

    fn handle_workspaces_event(
        &mut self,
        event: &UserWorkspacesEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            UserWorkspacesEvent::TeamsChanged => {
                self.update_addon_credit_modal(ctx);
            }
            UserWorkspacesEvent::UpdateWorkspaceSettingsSuccess => {
                self.update_addon_credit_modal(ctx);
                self.refresh_addon_credits_settings(ctx);
                if let Some(message) = self.pending_auto_reload_toast.take() {
                    self.show_toast(&message, ToastFlavor::Success, ctx);
                }
                ctx.notify();
            }
            UserWorkspacesEvent::UpdateWorkspaceSettingsRejected(_err) => {
                self.pending_auto_reload_toast = None;
                self.show_toast(
                    "Failed to update workspace settings",
                    ToastFlavor::Error,
                    ctx,
                );
            }
            UserWorkspacesEvent::AiOveragesUpdated => {
                ctx.notify();
            }
            UserWorkspacesEvent::PurchaseAddonCreditsSuccess => {
                self.addon_credits.purchase_loading = false;
                self.show_toast(
                    "Successfully purchased add-on credits",
                    ToastFlavor::Success,
                    ctx,
                );
                AIRequestUsageModel::handle(ctx)
                    .update(ctx, |m, ctx| m.refresh_request_usage_async(ctx));
            }
            UserWorkspacesEvent::PurchaseAddonCreditsRejected(err) => {
                self.addon_credits.purchase_loading = false;
                self.show_toast(&err.to_string(), ToastFlavor::Error, ctx);
            }
            _ => {}
        }
    }

    fn show_toast(&self, message: &str, flavor: ToastFlavor, ctx: &mut ViewContext<Self>) {
        ctx.emit(BillingAndUsagePageEvent::ShowToast {
            message: message.to_string(),
            flavor,
        });
    }

    pub fn get_modal_content(&self) -> Option<Box<dyn Element>> {
        if self.addon_credit_modal_state.is_open() {
            Some(self.addon_credit_modal_state.render())
        } else {
            None
        }
    }

    fn handle_addon_credit_modal_close_event(
        &mut self,
        event: &ModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ModalEvent::Close => {
                self.addon_credit_modal_state.close();
                ctx.emit(BillingAndUsagePageEvent::HideModal);
            }
        }
    }

    fn handle_addon_credit_modal_event(
        &mut self,
        event: &SpendingLimitModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SpendingLimitModalEvent::Close => {
                self.hide_addon_credit_modal(ctx);
            }
            SpendingLimitModalEvent::Update { amount_cents } => {
                let workspaces = UserWorkspaces::as_ref(ctx);
                let team_uid = workspaces.current_team_uid();

                if let Some(team_uid) = team_uid {
                    UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                        user_workspaces.update_addon_credits_settings(
                            team_uid,
                            None,
                            Some(*amount_cents as i32),
                            None,
                            ctx,
                        );
                    });
                    self.hide_addon_credit_modal(ctx);
                    ctx.notify();
                }
            }
        }
    }

    fn show_addon_credit_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.addon_credit_modal_state.open();
        self.addon_credit_modal_state
            .view
            .update(ctx, |modal, ctx| {
                modal.body().update(ctx, |body, ctx| {
                    body.focus_input(ctx);
                });
            });
        ctx.emit(BillingAndUsagePageEvent::ShowModal);
    }

    fn hide_addon_credit_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.addon_credit_modal_state.close();
        ctx.emit(BillingAndUsagePageEvent::HideModal);
    }

    fn update_addon_credit_modal(&mut self, ctx: &mut ViewContext<Self>) {
        let addon_limit = UserWorkspaces::as_ref(ctx)
            .current_workspace()
            .and_then(|ws| ws.settings.addon_credits_settings.max_monthly_spend_cents)
            .unwrap_or(DEFAULT_MAX_MONTHLY_SPEND_CENTS);

        self.addon_credit_modal_state
            .view
            .update(ctx, |modal, ctx| {
                modal.body().update(ctx, |body, ctx| {
                    body.update_amount_editor(addon_limit.max(0) as u32, ctx);
                });
            });
        ctx.notify();
    }

    fn update_addon_credits_options(&mut self, ctx: &mut ViewContext<Self>) {
        self.addon_credits.options = PricingInfoModel::as_ref(ctx)
            .addon_credits_options()
            .map(|options| options.to_vec())
            .unwrap_or_default();
        self.addon_credits.denomination_buttons = self
            .addon_credits
            .options
            .iter()
            .enumerate()
            .map(|(i, option)| {
                ctx.add_typed_action_view(move |_ctx| {
                    ActionButton::new(option.credits.separate_with_commas(), SecondaryTheme)
                        .with_icon(Icon::Credits)
                        .on_click(move |ctx| {
                            ctx.dispatch_typed_action(
                                BillingAndUsagePageAction::SelectTopupDenomination(i),
                            );
                        })
                })
            })
            .collect();
    }

    // ── Rendering ────────────────────────────────────────────────────────

    fn render_plan_section(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let mut plan_header = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);

        plan_header.add_child(
            Text::new_inline("Plan", appearance.ui_font_family(), HEADER_FONT_SIZE)
                .with_style(Properties::default().weight(Weight::Bold))
                .with_color(appearance.theme().active_ui_text_color().into())
                .finish(),
        );

        let mut right_side = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::End);

        let workspaces = UserWorkspaces::as_ref(app);

        if let Some(team) = workspaces.current_team() {
            if team.billing_metadata.customer_type != CustomerType::Unknown {
                right_side.add_child(
                    Container::new(render_customer_type_badge(
                        appearance,
                        team.billing_metadata.customer_type.to_display_string(),
                    ))
                    .with_margin_right(8.)
                    .finish(),
                );
            }

            let current_user_email = AuthStateProvider::as_ref(app)
                .get()
                .user_email()
                .unwrap_or_default();
            let has_admin_permissions = team.has_admin_permissions(&current_user_email);

            if has_admin_permissions {
                if team.billing_metadata.customer_type != CustomerType::Enterprise
                    && team.has_billing_history
                {
                    let team_uid = team.uid;
                    let fg_color = appearance.theme().active_ui_text_color();
                    right_side.add_child(
                        Container::new(
                            appearance
                                .ui_builder()
                                .button(
                                    ButtonVariant::Link,
                                    self.plan_mouse_states.manage_billing_link.clone(),
                                )
                                .with_text_and_icon_label(
                                    TextAndIcon::new(
                                        TextAndIconAlignment::IconFirst,
                                        "Manage billing",
                                        Icon::CoinsStacked.to_warpui_icon(fg_color),
                                        MainAxisSize::Min,
                                        MainAxisAlignment::Center,
                                        vec2f(14., 14.),
                                    )
                                    .with_inner_padding(4.),
                                )
                                .with_style(UiComponentStyles {
                                    font_color: Some(fg_color.into()),
                                    ..Default::default()
                                })
                                .build()
                                .on_click(move |ctx, _, _| {
                                    ctx.dispatch_typed_action(
                                        BillingAndUsagePageAction::GenerateStripeBillingPortalLink {
                                            team_uid,
                                        },
                                    );
                                })
                                .finish(),
                        )
                        .with_margin_left(8.)
                        .finish(),
                    );
                }

                if team.billing_metadata.is_enterprise_plan() {
                    let team_uid = team.uid;
                    let fg_color = appearance.theme().active_ui_text_color();
                    right_side.add_child(
                        Container::new(
                            appearance
                                .ui_builder()
                                .button(
                                    ButtonVariant::Link,
                                    self.plan_mouse_states.open_admin_panel_link.clone(),
                                )
                                .with_text_and_icon_label(
                                    TextAndIcon::new(
                                        TextAndIconAlignment::IconFirst,
                                        "Open admin panel",
                                        Icon::Users.to_warpui_icon(fg_color),
                                        MainAxisSize::Min,
                                        MainAxisAlignment::Center,
                                        vec2f(14., 14.),
                                    )
                                    .with_inner_padding(4.),
                                )
                                .with_style(UiComponentStyles {
                                    font_color: Some(fg_color.into()),
                                    ..Default::default()
                                })
                                .build()
                                .on_click(move |ctx, _, _| {
                                    ctx.dispatch_typed_action(
                                        BillingAndUsagePageAction::OpenAdminPanel { team_uid },
                                    );
                                })
                                .finish(),
                        )
                        .with_margin_left(8.)
                        .finish(),
                    );
                }
            }
        } else {
            let current_user_id = self.auth_state.user_id().unwrap_or_default();
            right_side.add_child(
                Container::new(render_customer_type_badge(appearance, "Free".into()))
                    .with_margin_right(8.)
                    .finish(),
            );
            right_side.add_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .button(
                            ButtonVariant::Link,
                            self.plan_mouse_states.admin_panel_link.clone(),
                        )
                        .with_text_and_icon_label(
                            TextAndIcon::new(
                                TextAndIconAlignment::IconFirst,
                                "Compare plans",
                                Icon::CoinsStacked
                                    .to_warpui_icon(appearance.theme().active_ui_text_color()),
                                MainAxisSize::Min,
                                MainAxisAlignment::Center,
                                vec2f(14., 14.),
                            )
                            .with_inner_padding(4.),
                        )
                        .with_style(UiComponentStyles {
                            font_color: Some(appearance.theme().active_ui_text_color().into()),
                            ..Default::default()
                        })
                        .build()
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(BillingAndUsagePageAction::Upgrade {
                                team_uid: None,
                                user_id: current_user_id,
                            });
                        })
                        .finish(),
                )
                .with_margin_left(8.)
                .finish(),
            );
        }

        right_side.add_child(
            Container::new(self.render_plan_refresh_button(appearance))
                .with_margin_left(8.)
                .finish(),
        );

        plan_header.add_child(right_side.finish());

        Container::new(plan_header.finish())
            .with_margin_bottom(24.)
            .finish()
    }

    fn render_plan_refresh_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let icon_color = theme.sub_text_color(theme.background());
        let mouse_state = self.plan_mouse_states.refresh_button.clone();
        warpui::elements::Hoverable::new(mouse_state, move |_| {
            Container::new(
                ConstrainedBox::new(Icon::Refresh.to_warpui_icon(icon_color).finish())
                    .with_width(16.)
                    .with_height(16.)
                    .finish(),
            )
            .with_uniform_padding(2.)
            .finish()
        })
        .with_cursor(warpui::platform::Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(BillingAndUsagePageAction::RefreshWorkspaceData);
        })
        .finish()
    }

    fn render_balance_section(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let theme = appearance.theme();
        let ai_model = AIRequestUsageModel::as_ref(app);

        let has_base_credits = ai_model.request_limit() > 0;

        let grants = ai_model.bonus_grants();
        let workspace_uid = UserWorkspaces::as_ref(app)
            .current_workspace()
            .map(|ws| ws.uid);
        let classified = ClassifiedGrants::new(grants, workspace_uid);

        if !has_base_credits && !classified.has_any() {
            return None;
        }

        let mut cards_row = Flex::row()
            .with_spacing(8.)
            .with_main_axis_size(MainAxisSize::Max);

        let outline_color = theme.outline().into_solid();

        if has_base_credits {
            let reset_str = ai_model
                .next_refresh_time_local()
                .format("Resets %b %d at %-I:%M %p")
                .to_string();
            let base_remaining = ai_model
                .request_limit()
                .saturating_sub(ai_model.requests_used()) as i64;
            let base_limit = (!ai_model.is_unlimited()).then(|| ai_model.request_limit() as i64);
            cards_row.add_child(
                Expanded::new(
                    1.,
                    render_balance_card(
                        appearance,
                        BASE_CREDITS_DOT_COLOR,
                        "Base credits",
                        &reset_str,
                        base_remaining,
                        base_limit,
                        outline_color,
                    ),
                )
                .finish(),
            );
        }

        if !classified.personal.is_empty() {
            cards_row.add_child(
                Expanded::new(
                    1.,
                    render_balance_card(
                        appearance,
                        BONUS_CREDITS_DOT_COLOR,
                        "Personal credits",
                        &classified.personal.expiry_label(),
                        classified.personal.total_balance(),
                        None,
                        outline_color,
                    ),
                )
                .finish(),
            );
        }

        if !classified.team.is_empty() {
            cards_row.add_child(
                Expanded::new(
                    1.,
                    render_balance_card(
                        appearance,
                        BONUS_CREDITS_DOT_COLOR,
                        "Team credits",
                        &classified.team.expiry_label(),
                        classified.team.total_balance(),
                        None,
                        outline_color,
                    ),
                )
                .finish(),
            );
        }

        Some(
            Flex::column()
                .with_child(
                    Container::new(
                        Text::new_inline("Balance", appearance.ui_font_family(), HEADER_FONT_SIZE)
                            .with_style(Properties::default().weight(Weight::Bold))
                            .with_color(theme.active_ui_text_color().into())
                            .finish(),
                    )
                    .with_margin_bottom(12.)
                    .finish(),
                )
                .with_child(cards_row.finish())
                .with_child(
                    ConstrainedBox::new(Empty::new().finish())
                        .with_height(24.)
                        .finish(),
                )
                .finish(),
        )
    }

    fn render_ambient_agent_trial_widget(
        &self,
        ai_model: &AIRequestUsageModel,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let credits_remaining = ai_model.ambient_only_credits_remaining()?;
        if credits_remaining == 0 {
            return None;
        }

        if *AISettings::as_ref(app).ambient_agent_trial_widget_dismissed {
            return None;
        }

        let theme = appearance.theme();
        let ui_builder = appearance.ui_builder();
        let fg = theme.foreground().into_solid();
        let bg = theme.background().into_solid();

        let title = Text::new_inline(AMBIENT_AGENT_TRIAL_TITLE, appearance.ui_font_family(), 14.)
            .with_color(theme.active_ui_text_color().into())
            .with_style(Properties::default().weight(Weight::Semibold))
            .finish();

        let credits_text = if credits_remaining == 1 {
            "1 credit remaining".to_string()
        } else {
            format!(
                "{} credits remaining",
                credits_remaining.separate_with_commas()
            )
        };
        let credits_label = Text::new_inline(credits_text, appearance.ui_font_family(), 12.)
            .with_color(blended_colors::text_sub(theme, theme.surface_1()))
            .finish();

        let left_side = Flex::row()
            .with_child(title)
            .with_child(Container::new(credits_label).with_margin_left(8.).finish())
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish();

        let mut right_side = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        if credits_remaining >= AMBIENT_AGENT_TRIAL_CREDIT_THRESHOLD {
            let new_agent_button = ui_builder
                .button(
                    ButtonVariant::Secondary,
                    self.ambient_trial_mouse_states.new_agent_button.clone(),
                )
                .with_text_label("New agent".to_string())
                .with_style(UiComponentStyles {
                    font_color: Some(bg),
                    background: Some(fg.into()),
                    font_size: Some(14.),
                    font_weight: Some(Weight::Semibold),
                    padding: Some(Coords {
                        top: 7.,
                        bottom: 7.,
                        left: 12.,
                        right: 12.,
                    }),
                    ..Default::default()
                })
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(WorkspaceAction::AddAmbientAgentTab);
                })
                .finish();
            right_side.add_child(
                Container::new(new_agent_button)
                    .with_margin_right(8.)
                    .finish(),
            );
        }

        let is_on_paid_plan = UserWorkspaces::as_ref(app)
            .current_team()
            .is_some_and(|team| team.billing_metadata.is_user_on_paid_plan());
        if !is_on_paid_plan {
            let user_id = AuthStateProvider::as_ref(app).get().user_id();
            let buy_more_button = ui_builder
                .button(
                    ButtonVariant::Secondary,
                    self.ambient_trial_mouse_states.buy_more_button.clone(),
                )
                .with_text_label("Buy more".to_string())
                .with_style(UiComponentStyles {
                    background: Some(bg.into()),
                    font_size: Some(14.),
                    font_weight: Some(Weight::Semibold),
                    padding: Some(Coords {
                        top: 7.,
                        bottom: 7.,
                        left: 12.,
                        right: 12.,
                    }),
                    ..Default::default()
                })
                .build()
                .on_click(move |ctx, _, _| {
                    if let Some(user_id) = user_id {
                        ctx.dispatch_typed_action(BillingAndUsagePageAction::Upgrade {
                            team_uid: None,
                            user_id,
                        });
                    }
                })
                .finish();
            right_side.add_child(buy_more_button);
        }

        if credits_remaining < AMBIENT_AGENT_TRIAL_CREDIT_THRESHOLD {
            let dismiss_button = icon_button(
                appearance,
                Icon::X,
                false,
                self.ambient_trial_mouse_states.dismiss_button.clone(),
            )
            .with_style(UiComponentStyles {
                width: Some(32.),
                height: Some(32.),
                padding: Some(Coords::uniform(8.)),
                ..Default::default()
            })
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(
                    BillingAndUsagePageAction::DismissAmbientAgentTrialWidget,
                );
            })
            .finish();
            right_side.add_child(Container::new(dismiss_button).with_margin_left(4.).finish());
        }

        let row_content = Flex::row()
            .with_child(Shrinkable::new(1., left_side).finish())
            .with_child(right_side.finish())
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .finish();

        let bright_blue: ColorU = theme.terminal_colors().bright.blue.into();
        let gradient_start = ColorU::transparent_black();
        let gradient_end = ColorU::new(bright_blue.r, bright_blue.g, bright_blue.b, 40);

        Some(
            Container::new(row_content)
                .with_horizontal_background_gradient(gradient_start, gradient_end)
                .with_border(Border::all(1.).with_border_color(theme.accent_overlay().into()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .with_margin_bottom(16.)
                .with_uniform_padding(12.)
                .finish(),
        )
    }

    fn render_addon_credits_panel(
        &self,
        workspace: &Workspace,
        team_uid: ServerId,
        has_admin_permissions: bool,
        delinquent: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        match self.addon_credits_panel_state(
            workspace,
            team_uid,
            has_admin_permissions,
            delinquent,
            app,
        ) {
            AddonCreditsPanelState::IneligiblePlan(restriction) => {
                self.render_addon_credits_ineligible_plan_card(restriction, appearance)
            }
            AddonCreditsPanelState::AutoreloadNonAdmin {
                description_text,
                warning_text,
            } => self.render_addon_credits_non_admin_auto_reload_card(
                appearance,
                description_text,
                warning_text,
            ),
            AddonCreditsPanelState::Purchase(state) => {
                self.render_addon_credits_purchase_card(workspace, team_uid, state, appearance)
            }
        }
    }

    fn addon_credits_panel_state(
        &self,
        workspace: &Workspace,
        team_uid: ServerId,
        has_admin_permissions: bool,
        delinquent: bool,
        app: &AppContext,
    ) -> AddonCreditsPanelState {
        let team_can_purchase = UserWorkspaces::as_ref(app)
            .current_team()
            .and_then(|t| t.billing_metadata.tier.purchase_add_on_credits_policy)
            .is_some_and(|p| p.enabled);
        let can_upgrade = workspace.billing_metadata.can_upgrade_to_build_plan();

        if !team_can_purchase {
            if !has_admin_permissions {
                return AddonCreditsPanelState::IneligiblePlan(
                    AddonCreditsRestriction::ContactTeamAdmin,
                );
            } else if can_upgrade {
                return AddonCreditsPanelState::IneligiblePlan(
                    AddonCreditsRestriction::UpgradeToBuild {
                        link_text: "Upgrade to Build",
                        url: UserWorkspaces::upgrade_link_for_team(team_uid),
                    },
                );
            }
            return AddonCreditsPanelState::IneligiblePlan(
                AddonCreditsRestriction::ContactAccountExecutive,
            );
        }

        let selected_credit_option = self
            .addon_credits
            .options
            .get(self.addon_credits.selected_denomination);
        let auto_reload_enabled = workspace
            .settings
            .addon_credits_settings
            .auto_reload_enabled;

        let team_count = UserWorkspaces::as_ref(app)
            .current_team()
            .map(|t| t.members.len())
            .unwrap_or(1);
        let description_text = if team_count > 1 {
            format!("{ADDON_CREDITS_DESCRIPTION} {ADDITIONAL_ADDON_CREDITS_DESCRIPTION_FOR_TEAM}")
        } else {
            ADDON_CREDITS_DESCRIPTION.to_string()
        };

        let would_exceed = selected_credit_option.is_some_and(|opt| {
            let limit = workspace
                .settings
                .addon_credits_settings
                .max_monthly_spend_cents
                .unwrap_or(DEFAULT_MAX_MONTHLY_SPEND_CENTS);
            (workspace.bonus_grants_purchased_this_month.cents_spent + opt.price_usd_cents) > limit
        });
        let purchase_disabled = self.addon_credits.purchase_loading
            || would_exceed
            || delinquent
            || auto_reload_enabled;
        let auto_reload_switch_disabled = !has_admin_permissions
            || delinquent
            || (!auto_reload_enabled && selected_credit_option.is_none());
        let price_label = selected_credit_option
            .map(|opt| {
                let credits = opt.credits.separate_with_commas();
                let dollars = format!("${:.2}", opt.price_usd_cents as f64 / 100.0);
                format!("{credits} credits / {dollars}")
            })
            .unwrap_or_default();
        let auto_reload_credit_amount = selected_credit_option
            .map(|o| format!("{} credits", o.credits.separate_with_commas()))
            .unwrap_or_else(|| "selected credit amount".to_string());
        let auto_reload_tooltip_text = format!(
            "When any member on your team’s credit balance reaches 100 credits remaining, \
            automatically purchase {auto_reload_credit_amount}."
        );
        let warning_text = if delinquent && has_admin_permissions {
            Some(ADDON_CREDITS_DELINQUENT_WARNING_STRING)
        } else if delinquent {
            Some(ADDON_CREDITS_NON_ADMIN_DELINQUENT_WARNING_STRING)
        } else if workspace
            .billing_metadata
            .has_failed_addon_credit_auto_reload_status()
        {
            Some(if has_admin_permissions {
                RESTRICTED_BILLING_USAGE_WARNING_STRING
            } else {
                RESTRICTED_BILLING_USAGE_NON_ADMIN_WARNING_STRING
            })
        } else if would_exceed {
            Some(match (auto_reload_enabled, has_admin_permissions) {
                (true, true) => {
                    "Auto-reload is paused because the next reload would exceed your monthly spend limit. Increase your limit to continue using auto-reload."
                }
                (true, false) => {
                    "Auto-reload is paused because the next reload would exceed your team’s monthly spend limit. Contact a team admin to increase it."
                }
                (false, true) => {
                    "This purchase would exceed your monthly limit. Increase your limit to continue."
                }
                (false, false) => {
                    "This purchase would exceed your team’s monthly spend limit. Contact a team admin to increase it."
                }
            })
        } else {
            None
        };

        if !has_admin_permissions && auto_reload_enabled {
            let configured_auto_reload_option = workspace
                .settings
                .addon_credits_settings
                .selected_auto_reload_credit_denomination
                .and_then(|credits| {
                    self.addon_credits
                        .options
                        .iter()
                        .find(|option| option.credits == credits)
                })
                .or(selected_credit_option);
            let description_text = match configured_auto_reload_option {
                Some(option) => {
                    let credits = option.credits.separate_with_commas();
                    let price = format!("${:.2}", option.price_usd_cents as f64 / 100.0);
                    format!(
                        "Your admin has enabled auto-reload for add-on credits. When your personal add-on credit balance runs low, Warp will automatically purchase {credits} credits for {price} and add them to your balance."
                    )
                }
                None => {
                    "Your admin has enabled auto-reload for add-on credits. When your personal add-on credit balance runs low, Warp will automatically purchase add-on credits and add them to your balance.".to_string()
                }
            };
            return AddonCreditsPanelState::AutoreloadNonAdmin {
                description_text,
                warning_text,
            };
        }

        AddonCreditsPanelState::Purchase(AddonCreditsPurchaseState {
            description_text,
            auto_reload_enabled,
            has_admin_permissions,
            purchase_disabled,
            auto_reload_switch_disabled,
            price_label,
            auto_reload_tooltip_text,
            warning_text,
        })
    }

    fn render_addon_credits_ineligible_plan_card(
        &self,
        restriction: AddonCreditsRestriction,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let bg = theme.background();
        let explanation = match restriction {
            AddonCreditsRestriction::UpgradeToBuild { link_text, url } => {
                FormattedTextElement::new(
                    FormattedText::new([FormattedTextLine::Line(vec![
                        FormattedTextFragment::hyperlink(link_text, url),
                        FormattedTextFragment::plain_text(" to purchase add-on credits."),
                    ])]),
                    appearance.ui_font_size(),
                    appearance.ui_font_family(),
                    appearance.ui_font_family(),
                    theme.sub_text_color(bg).into(),
                    HighlightedHyperlink::default(),
                )
                .with_hyperlink_font_color(theme.accent().into_solid())
                .register_default_click_handlers_with_action_support(
                    |lens, event, ctx| match lens {
                        warpui::elements::HyperlinkLens::Url(u) => ctx.open_url(u),
                        warpui::elements::HyperlinkLens::Action(a) => {
                            if let Some(act) =
                                a.as_any().downcast_ref::<BillingAndUsagePageAction>()
                            {
                                event.dispatch_typed_action(act.clone());
                            }
                        }
                    },
                )
                .finish()
            }
            AddonCreditsRestriction::ContactAccountExecutive => appearance
                .ui_builder()
                .paragraph("Contact your Account Executive for more add-on credits.")
                .with_style(UiComponentStyles {
                    font_color: Some(theme.sub_text_color(bg).into()),
                    ..Default::default()
                })
                .build()
                .finish(),
            AddonCreditsRestriction::ContactTeamAdmin => appearance
                .ui_builder()
                .paragraph("Contact a team admin to enable add-on credits.")
                .with_style(UiComponentStyles {
                    font_color: Some(theme.sub_text_color(bg).into()),
                    ..Default::default()
                })
                .build()
                .finish(),
        };
        let header = Text::new_inline("Buy credits", appearance.ui_font_family(), HEADER_FONT_SIZE)
            .with_color(theme.foreground().into())
            .with_style(Properties::default().weight(Weight::Medium))
            .finish();
        let card = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_children([
                Container::new(header).with_margin_bottom(8.).finish(),
                explanation,
            ])
            .finish();

        Container::new(card)
            .with_background_color(theme.surface_1().into_solid())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_margin_bottom(16.)
            .with_uniform_padding(16.)
            .finish()
    }

    fn render_addon_credits_non_admin_auto_reload_card(
        &self,
        appearance: &Appearance,
        description_text: String,
        warning_text: Option<&'static str>,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let bg = theme.background();
        let auto_reload_header = Text::new_inline(
            MANAGED_AUTO_RELOAD_HEADER,
            appearance.ui_font_family(),
            HEADER_FONT_SIZE,
        )
        .with_color(theme.foreground().into())
        .with_style(Properties::default().weight(Weight::Medium))
        .finish();
        let auto_reload_description = appearance
            .ui_builder()
            .paragraph(description_text)
            .with_style(UiComponentStyles {
                font_color: Some(theme.sub_text_color(bg).into()),
                ..Default::default()
            })
            .build()
            .finish();
        let mut card_children = vec![auto_reload_header, auto_reload_description];
        if let Some(warning_text) = warning_text {
            card_children.push(self.render_warning_row(appearance, warning_text.to_string()));
        }
        let card = Flex::column()
            .with_children(card_children)
            .with_spacing(8.)
            .finish();

        Container::new(card)
            .with_background_color(theme.surface_1().into_solid())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_margin_bottom(16.)
            .with_uniform_padding(16.)
            .finish()
    }

    fn render_addon_credits_purchase_card(
        &self,
        workspace: &Workspace,
        team_uid: ServerId,
        state: AddonCreditsPurchaseState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let card_upper = self.render_addon_credits_upper_section(workspace, &state, appearance);
        let card_lower = self.render_addon_credits_lower_section(team_uid, &state, appearance);

        Container::new(
            Flex::column()
                .with_children([card_upper, card_lower])
                .finish(),
        )
        .with_background_color(theme.surface_1().into_solid())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_margin_bottom(16.)
        .finish()
    }

    fn render_addon_credits_upper_section(
        &self,
        workspace: &Workspace,
        state: &AddonCreditsPurchaseState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let bg = theme.background();
        let ui_builder = appearance.ui_builder();
        let header = Text::new_inline("Buy credits", appearance.ui_font_family(), HEADER_FONT_SIZE)
            .with_color(theme.foreground().into())
            .with_style(Properties::default().weight(Weight::Medium))
            .finish();
        let paragraph = ui_builder
            .paragraph(state.description_text.clone())
            .with_style(UiComponentStyles {
                font_color: Some(theme.sub_text_color(bg).into()),
                ..Default::default()
            })
            .build()
            .finish();
        let mut upper_section = Flex::column()
            .with_children([header, paragraph])
            .with_spacing(8.);

        if state.has_admin_permissions {
            let info_icon = render_info_icon(
                appearance,
                AdditionalInfo::<BillingAndUsagePageAction> {
                    mouse_state: self.buy_credits_mouse_states.addon_info_icon.clone(),
                    on_click_action: None,
                    secondary_text: None,
                    tooltip_override_text: Some(
                        "Sets the monthly limit spent on add-on credits".to_string(),
                    ),
                },
            );
            let spend_limit = workspace
                .settings
                .addon_credits_settings
                .max_monthly_spend_cents
                .map(|c| format!("${:.2}", c as f64 / 100.0))
                .unwrap_or_else(|| "$200.00".to_string());
            let spend_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_children([
                    ui_builder.span("Monthly spend limit").build().finish(),
                    Shrinkable::new(1., Align::new(info_icon).left().finish()).finish(),
                    icon_button(
                        appearance,
                        Icon::Pencil,
                        false,
                        self.buy_credits_mouse_states.edit_monthly_limit.clone(),
                    )
                    .build()
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(BillingAndUsagePageAction::ShowAddOnCreditModal);
                    })
                    .finish(),
                    ui_builder.span(spend_limit).build().finish(),
                ])
                .finish();
            upper_section.add_child(spend_row);

            if let Some(purchased_row) =
                Self::render_purchased_this_month_row(workspace, appearance)
            {
                upper_section.add_child(purchased_row);
            }
        }

        if state.has_admin_permissions || !state.auto_reload_enabled {
            let denomination_button_elements = self
                .addon_credits
                .denomination_buttons
                .iter()
                .map(|button_handle| ChildView::new(button_handle).finish())
                .collect::<Vec<_>>();
            upper_section.add_child(
                Container::new(
                    Wrap::row()
                        .with_children(denomination_button_elements)
                        .with_spacing(8.)
                        .finish(),
                )
                .finish(),
            );
        }

        Container::new(upper_section.finish())
            .with_horizontal_padding(16.)
            .with_padding_top(16.)
            .with_padding_bottom(16.)
            .finish()
    }

    fn render_purchased_this_month_row(
        workspace: &Workspace,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let bonus_grants = &workspace.bonus_grants_purchased_this_month;
        if bonus_grants.total_credits_purchased == 0 {
            return None;
        }

        let credits_purchased = bonus_grants.total_credits_purchased;
        let cost_dollars = bonus_grants.cents_spent as f64 / 100.0;
        let theme = appearance.theme();

        let label = Text::new_inline("Purchased this month", appearance.ui_font_family(), 12.)
            .with_color(theme.active_ui_text_color().into())
            .finish();

        let credits_text = if credits_purchased == 1 {
            "1 credit".to_string()
        } else {
            format!("{} credits", credits_purchased.separate_with_commas())
        };

        let credits_component = Container::new(
            Text::new_inline(credits_text, appearance.ui_font_family(), 12.)
                .with_color(blended_colors::text_disabled(theme, theme.surface_1()))
                .finish(),
        )
        .with_margin_right(8.)
        .finish();

        let cost_component = Text::new_inline(
            format!("${cost_dollars:.2}"),
            appearance.ui_font_family(),
            12.,
        )
        .with_color(blended_colors::text_sub(theme, theme.surface_1()))
        .finish();

        Some(
            Container::new(
                Flex::row()
                    .with_child(label)
                    .with_child(
                        Flex::row()
                            .with_child(credits_component)
                            .with_child(cost_component)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .finish(),
                    )
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_main_axis_size(MainAxisSize::Max)
                    .finish(),
            )
            .with_margin_bottom(4.)
            .finish(),
        )
    }

    fn render_addon_credits_lower_section(
        &self,
        team_uid: ServerId,
        state: &AddonCreditsPurchaseState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let fg = theme.foreground();
        let auto_reload_enabled = state.auto_reload_enabled;
        let purchase_button_label = if self.addon_credits.purchase_loading {
            "Buying\u{2026}"
        } else {
            "One-time purchase"
        };
        let purchase_button_font_color = state
            .purchase_disabled
            .then_some(theme.disabled_text_color(theme.surface_3()).into());
        let purchase_button_background_color =
            state.purchase_disabled.then_some(theme.surface_3().into());
        let purchase_button_border_color = state
            .purchase_disabled
            .then_some(ColorU::transparent_black().into());
        let mut purchase_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                self.buy_credits_mouse_states.buy_button.clone(),
            )
            .with_text_label(purchase_button_label.to_string())
            .with_style(UiComponentStyles {
                font_size: Some(14.),
                font_weight: Some(Weight::Semibold),
                font_color: purchase_button_font_color,
                background: purchase_button_background_color,
                border_color: purchase_button_border_color,
                ..Default::default()
            })
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(BillingAndUsagePageAction::PurchaseAddonCredits {
                    team_uid,
                });
            });
        if state.purchase_disabled {
            purchase_button = purchase_button.disable();
        }
        let purchase_button = purchase_button.finish();
        let price_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Text::new_inline(state.price_label.clone(), appearance.ui_font_family(), 14.)
                    .with_color(fg.into())
                    .with_style(Properties::default().weight(Weight::Medium))
                    .finish(),
            );

        let mut right_group = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        if state.has_admin_permissions {
            let auto_reload_switch_element = {
                let switch_builder = appearance
                    .ui_builder()
                    .switch(self.buy_credits_mouse_states.auto_reload_switch.clone())
                    .check(auto_reload_enabled);
                if state.auto_reload_switch_disabled {
                    switch_builder.disable().build().finish()
                } else {
                    switch_builder
                        .build()
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(
                                BillingAndUsagePageAction::UpdateAutoReloadEnabled {
                                    team_uid,
                                    enabled: !auto_reload_enabled,
                                },
                            );
                        })
                        .finish()
                }
            };
            let auto_reload_info_icon = render_info_icon(
                appearance,
                AdditionalInfo::<BillingAndUsagePageAction> {
                    mouse_state: self.buy_credits_mouse_states.auto_reload_info.clone(),
                    on_click_action: None,
                    secondary_text: None,
                    tooltip_override_text: Some(state.auto_reload_tooltip_text.clone()),
                },
            );

            right_group.add_children([
                Text::new_inline("Auto-reload", appearance.ui_font_family(), 14.)
                    .with_color(fg.into())
                    .with_style(Properties::default().weight(Weight::Semibold))
                    .finish(),
                Container::new(auto_reload_info_icon)
                    .with_margin_left(4.)
                    .finish(),
                Container::new(auto_reload_switch_element)
                    .with_margin_left(8.)
                    .finish(),
            ]);
        }
        right_group.add_child(
            Container::new(purchase_button)
                .with_margin_left(if state.has_admin_permissions { 16. } else { 0. })
                .finish(),
        );
        let lower_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(price_row.finish())
            .with_child(right_group.finish());
        let mut lower_children: Vec<Box<dyn Element>> = vec![lower_row.finish()];

        if let Some(warning_text) = state.warning_text {
            lower_children.push(self.render_warning_row(appearance, warning_text.to_string()));
        }

        Container::new(
            Flex::column()
                .with_children(lower_children)
                .with_spacing(8.)
                .finish(),
        )
        .with_uniform_padding(16.)
        .with_border(Border::top(1.).with_border_color(theme.outline().into()))
        .finish()
    }

    fn render_warning_row(&self, appearance: &Appearance, msg: String) -> Box<dyn Element> {
        let theme = appearance.theme();
        let icon = ConstrainedBox::new(
            Icon::AlertTriangle
                .to_warpui_icon(theme.ui_error_color().into())
                .finish(),
        )
        .with_height(16.)
        .with_width(16.)
        .finish();
        let text = Text::new(msg, appearance.ui_font_family(), 12.)
            .with_color(theme.ui_error_color())
            .finish();
        Container::new(
            Flex::row()
                .with_child(Container::new(icon).with_margin_right(8.).finish())
                .with_child(Shrinkable::new(1.0, text).finish())
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        )
        .with_margin_top(8.)
        .finish()
    }

    fn render_overview_tab(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        let ai_model = AIRequestUsageModel::as_ref(app);
        if let Some(ambient_trial_widget) =
            self.render_ambient_agent_trial_widget(ai_model, appearance, app)
        {
            content.add_child(ambient_trial_widget);
        }
        if let Some(balance) = self.render_balance_section(appearance, app) {
            content.add_child(balance);
        }

        let delinquent = UserWorkspaces::as_ref(app)
            .current_team()
            .map(|t| t.billing_metadata.is_delinquent_due_to_payment_issue())
            .unwrap_or_default();

        if let (Some(ws), Some(team)) = (
            UserWorkspaces::as_ref(app).current_workspace(),
            UserWorkspaces::as_ref(app).current_team(),
        ) {
            let workspace_bonus_credits = ai_model.total_workspace_bonus_credits_remaining(ws.uid);
            let is_payg_zero = ws.billing_metadata.is_enterprise_pay_as_you_go_enabled()
                && workspace_bonus_credits == 0;

            if !is_payg_zero {
                let current_user_is_admin = {
                    let email = AuthStateProvider::as_ref(app)
                        .get()
                        .user_email()
                        .unwrap_or_default();
                    team.has_admin_permissions(&email)
                };
                content.add_child(self.render_addon_credits_panel(
                    ws,
                    team.uid,
                    current_user_is_admin,
                    delinquent,
                    app,
                ));
            }
        }

        let mut billing_cycle_usage_section =
            Container::new(ChildView::new(&self.billing_cycle_usage_section).finish());
        if !content.is_empty() {
            billing_cycle_usage_section = billing_cycle_usage_section
                .with_margin_top(16.)
                .with_padding_top(24.)
                .with_border(
                    Border::top(1.).with_border_color(appearance.theme().outline().into()),
                );
        }
        content.add_child(billing_cycle_usage_section.finish());

        content.finish()
    }

    fn render_usage_history_tab(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let history = self.usage_history.model.as_ref(app);
        if history.entries().is_empty() {
            return self.render_empty_usage_history(history.is_loading(), appearance, app);
        }

        let mut content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_child(
                Container::new(
                    Text::new_inline("Last 30 days", appearance.ui_font_family(), 14.)
                        .with_color(blended_colors::text_sub(
                            appearance.theme(),
                            appearance.theme().surface_1(),
                        ))
                        .finish(),
                )
                .with_vertical_margin(12.)
                .finish(),
            );

        let mut list = Flex::column().with_spacing(8.);
        for entry in history.entries().iter() {
            let expanded = self
                .usage_history
                .expanded_entries
                .get(&entry.conversation_id)
                .copied()
                .unwrap_or(false);
            let entry_mouse_state = self
                .usage_history
                .entry_mouse_states
                .borrow_mut()
                .entry(entry.conversation_id.clone())
                .or_default()
                .clone();
            let entry_tooltip_mouse_state = self
                .usage_history
                .tooltip_mouse_states
                .borrow_mut()
                .entry(entry.conversation_id.clone())
                .or_default()
                .clone();
            list.add_child(
                Container::new(
                    UsageHistoryEntry::new(
                        Some(entry.clone()),
                        expanded,
                        Some(entry_mouse_state),
                        entry_tooltip_mouse_state,
                    )
                    .render(appearance, app),
                )
                .finish(),
            );
        }
        content.add_child(list.finish());

        if history.has_more_entries() {
            content.add_child(
                Container::new(
                    Flex::row()
                        .with_child(self.usage_history.load_more_button.as_ref(app).render(app))
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_main_axis_size(MainAxisSize::Max)
                        .finish(),
                )
                .with_margin_top(24.)
                .finish(),
            );
        }

        content.finish()
    }

    fn render_empty_usage_history(
        &self,
        loading: bool,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_alignment(MainAxisAlignment::Center);

        if loading {
            let mut list = Flex::column().with_spacing(8.);
            for _ in 0..3 {
                list.add_child(
                    UsageHistoryEntry::new(None, false, None, MouseStateHandle::default())
                        .render(appearance, app),
                );
            }
            content.add_child(list.finish());
        } else {
            let zero = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Container::new(
                        ConstrainedBox::new(
                            Icon::Conversation
                                .to_warpui_icon(
                                    blended_colors::text_sub(
                                        appearance.theme(),
                                        appearance.theme().surface_1(),
                                    )
                                    .into(),
                                )
                                .finish(),
                        )
                        .with_width(24.)
                        .with_height(24.)
                        .finish(),
                    )
                    .with_margin_bottom(12.)
                    .finish(),
                )
                .with_child(
                    Container::new(
                        Text::new("No usage history", appearance.ui_font_family(), 14.)
                            .with_color(blended_colors::text_sub(
                                appearance.theme(),
                                appearance.theme().surface_1(),
                            ))
                            .finish(),
                    )
                    .with_margin_bottom(4.)
                    .finish(),
                )
                .with_child(
                    Text::new(
                        "Kick off an agent task to view usage history here.",
                        appearance.ui_font_family(),
                        14.,
                    )
                    .with_color(blended_colors::text_disabled(
                        appearance.theme(),
                        appearance.theme().surface_1(),
                    ))
                    .finish(),
                );
            content.add_child(
                Container::new(zero.finish())
                    .with_vertical_margin(160.)
                    .finish(),
            );
        }

        content.finish()
    }
}

impl BillingAndUsagePageV2View {
    pub(super) fn on_page_selected(
        &mut self,
        _allow_steal_focus: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.addon_credits.purchase_loading = false;
        std::mem::drop(
            TeamUpdateManager::handle(ctx)
                .update(ctx, |mgr, ctx| mgr.refresh_workspace_metadata(ctx)),
        );
        AIRequestUsageModel::handle(ctx).update(ctx, |m, ctx| m.refresh_request_usage_async(ctx));
        self.usage_history
            .model
            .update(ctx, |m, ctx| m.refresh_usage_history_async(ctx));
        self.refresh_addon_credits_settings(ctx);
    }
}

impl Entity for BillingAndUsagePageV2View {
    type Event = BillingAndUsagePageEvent;
}

impl View for BillingAndUsagePageV2View {
    fn ui_name() -> &'static str {
        "Billing and usage v2"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut page = Flex::column();
        page.add_child(self.render_plan_section(appearance, app));

        let tabs = vec![
            SettingsTab::new(
                BillingUsageTab::Overview.label(),
                self.tab_mouse_states.overview.clone(),
            ),
            SettingsTab::new(
                BillingUsageTab::UsageHistory.label(),
                self.tab_mouse_states.usage_history.clone(),
            ),
        ];

        page.add_child(tab_selector::render_tab_selector(
            tabs,
            self.selected_tab.label(),
            |label, ctx| {
                ctx.dispatch_typed_action(BillingAndUsagePageAction::SelectTab(
                    BillingUsageTab::get_tab_from_label(label),
                ));
            },
            appearance,
        ));

        if self.selected_tab == BillingUsageTab::Overview {
            page.add_child(self.render_overview_tab(appearance, app));
        } else {
            page.add_child(self.render_usage_history_tab(appearance, app));
        }

        page.finish()
    }
}

impl TypedActionView for BillingAndUsagePageV2View {
    type Action = BillingAndUsagePageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        let is_login_gated = matches!(
            action,
            BillingAndUsagePageAction::Upgrade { .. }
                | BillingAndUsagePageAction::GenerateStripeBillingPortalLink { .. },
        );
        if AuthStateProvider::as_ref(ctx)
            .get()
            .is_anonymous_or_logged_out()
            && is_login_gated
        {
            AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                auth_manager.attempt_login_gated_feature(
                    action.into(),
                    AuthViewVariant::RequireLoginCloseable,
                    ctx,
                )
            });
            return;
        }

        match action {
            BillingAndUsagePageAction::Upgrade { team_uid, user_id } => match team_uid {
                Some(team_uid) => ctx.open_url(&UserWorkspaces::upgrade_link_for_team(*team_uid)),
                None => ctx.open_url(&UserWorkspaces::upgrade_link(*user_id)),
            },
            BillingAndUsagePageAction::GenerateStripeBillingPortalLink { team_uid } => {
                UserWorkspaces::handle(ctx).update(ctx, |ws, ctx| {
                    ws.generate_stripe_billing_portal_link(*team_uid, ctx);
                });
            }
            BillingAndUsagePageAction::OpenAdminPanel { team_uid } => {
                super::admin_actions::AdminActions::open_admin_panel(*team_uid, ctx);
            }
            BillingAndUsagePageAction::ContactSupport => {
                super::admin_actions::AdminActions::contact_support(ctx);
            }
            BillingAndUsagePageAction::SignupAnonymousUser => {
                ctx.emit(BillingAndUsagePageEvent::SignupAnonymousUser);
            }
            BillingAndUsagePageAction::AttemptLoginGatedUpgrade => {
                AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                    auth_manager.attempt_login_gated_feature(
                        action.into(),
                        AuthViewVariant::RequireLoginCloseable,
                        ctx,
                    )
                });
            }
            BillingAndUsagePageAction::OpenUrl(url) => ctx.open_url(&url.url),
            // Not applicable in v2
            BillingAndUsagePageAction::UpdateUsageBasedPricingSettings { .. }
            | BillingAndUsagePageAction::ShowOverageLimitModal => {}
            // Not applicable in v2
            BillingAndUsagePageAction::ToggleSortingMenu
            | BillingAndUsagePageAction::ChangeUsageSort { .. } => {}
            BillingAndUsagePageAction::RefreshWorkspaceData => {
                std::mem::drop(
                    TeamUpdateManager::handle(ctx)
                        .update(ctx, |mgr, ctx| mgr.refresh_workspace_metadata(ctx)),
                );
                AIRequestUsageModel::handle(ctx)
                    .update(ctx, |m, ctx| m.refresh_request_usage_async(ctx));
            }
            BillingAndUsagePageAction::SelectTab(tab) => {
                if self.selected_tab != *tab {
                    self.selected_tab = tab.clone();
                    ctx.notify();
                }
            }
            BillingAndUsagePageAction::ToggleUsageEntryExpanded { conversation_id } => {
                let expanded = self
                    .usage_history
                    .expanded_entries
                    .get(conversation_id)
                    .copied()
                    .unwrap_or(false);
                self.usage_history
                    .expanded_entries
                    .insert(conversation_id.clone(), !expanded);
                ctx.notify();
            }
            BillingAndUsagePageAction::RenderMoreUsageEntries => {
                self.usage_history
                    .model
                    .update(ctx, |m, ctx| m.load_more_usage_history_async(ctx));
            }
            BillingAndUsagePageAction::SelectTopupDenomination(i) => {
                self.addon_credits.selected_denomination = *i;
                self.update_denomination_buttons_focus(ctx);
                UserWorkspaces::handle(ctx).update(ctx, |ws, ctx| {
                    let has_admin_permissions = ws.current_team().is_some_and(|team| {
                        AuthStateProvider::as_ref(ctx)
                            .get()
                            .user_email()
                            .is_some_and(|email| team.has_admin_permissions(&email))
                    });
                    let team_uid = ws.current_team_uid();
                    if let Some((workspace, team_uid)) = ws.current_workspace().zip(team_uid) {
                        if has_admin_permissions
                            && workspace
                                .settings
                                .addon_credits_settings
                                .auto_reload_enabled
                        {
                            if let Some(opt) = self
                                .addon_credits
                                .options
                                .get(self.addon_credits.selected_denomination)
                            {
                                ws.update_addon_credits_settings(
                                    team_uid,
                                    None,
                                    None,
                                    Some(opt.credits),
                                    ctx,
                                );
                            }
                        }
                    }
                });
                ctx.notify();
            }
            BillingAndUsagePageAction::PurchaseAddonCredits { team_uid } => {
                if let Some(opt) = self
                    .addon_credits
                    .options
                    .get(self.addon_credits.selected_denomination)
                {
                    let credits = opt.credits;
                    let purchase_team_uid = *team_uid;
                    self.addon_credits.purchase_loading = true;
                    UserWorkspaces::handle(ctx).update(ctx, |ws, ctx| {
                        ws.purchase_addon_credits(purchase_team_uid, credits, ctx);
                    });
                    ctx.notify();
                }
            }
            BillingAndUsagePageAction::ShowAddOnCreditModal => {
                self.show_addon_credit_modal(ctx);
            }
            BillingAndUsagePageAction::UpdateAutoReloadEnabled { team_uid, enabled } => {
                let auto_reload_denomination_credits = if *enabled {
                    let Some(option) = self
                        .addon_credits
                        .options
                        .get(self.addon_credits.selected_denomination)
                    else {
                        self.show_toast(
                            "Unable to enable auto-reload until pricing options load.",
                            ToastFlavor::Error,
                            ctx,
                        );
                        return;
                    };
                    Some(option.credits)
                } else {
                    None
                };
                send_telemetry_from_ctx!(
                    TelemetryEvent::AutoReloadToggledFromBillingSettings {
                        enabled: *enabled,
                        banner_toggle_flag_enabled: FeatureFlag::BuildPlanAutoReloadBannerToggle
                            .is_enabled(),
                        post_purchase_modal_flag_enabled:
                            FeatureFlag::BuildPlanAutoReloadPostPurchaseModal.is_enabled(),
                    },
                    ctx
                );
                self.pending_auto_reload_toast = Some(if *enabled {
                    let credits = auto_reload_denomination_credits
                        .map(|c| c.separate_with_commas())
                        .unwrap_or_else(|| "your selected".to_string());
                    format!(
                        "Auto-reload enabled. We'll refill with {credits} credits when your balance runs low."
                    )
                } else {
                    "Auto-reload disabled.".to_string()
                });
                UserWorkspaces::handle(ctx).update(ctx, |ws, ctx| {
                    ws.update_addon_credits_settings(
                        *team_uid,
                        Some(*enabled),
                        None,
                        auto_reload_denomination_credits,
                        ctx,
                    );
                });
            }
            BillingAndUsagePageAction::DismissAmbientAgentTrialWidget => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings
                        .ambient_agent_trial_widget_dismissed
                        .set_value(true, ctx);
                });
                ctx.notify();
            }
            BillingAndUsagePageAction::NavigateToByokSettings => {
                ctx.dispatch_typed_action_deferred(WorkspaceAction::ShowSettingsPageWithSearch {
                    search_query: "api".to_string(),
                    section: Some(SettingsSection::WarpAgent),
                });
            }
        }
    }
}

fn render_balance_card(
    appearance: &Appearance,
    dot_color: ColorU,
    label: &str,
    date: &str,
    remaining: i64,
    total: Option<i64>,
    border_color: ColorU,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let sub_color = blended_colors::text_sub(theme, theme.background());

    let status_dot = ConstrainedBox::new(
        Container::new(Empty::new().finish())
            .with_background_color(dot_color)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .finish(),
    )
    .with_width(8.)
    .with_height(8.)
    .finish();

    let label_text = Text::new_inline(label.to_string(), appearance.ui_font_family(), 12.)
        .with_color(sub_color)
        .with_style(Properties::default().weight(Weight::Semibold))
        .finish();

    let date_text = Clipped::new(
        Text::new_inline(date.to_string(), appearance.ui_font_family(), 10.)
            .with_color(sub_color)
            .finish(),
    )
    .finish();

    let header = Flex::row()
        .with_child(status_dot)
        .with_child(
            Container::new(Shrinkable::new(1., label_text).finish())
                .with_margin_left(8.)
                .with_margin_right(8.)
                .finish(),
        )
        .with_child(Shrinkable::new(1., Align::new(date_text).right().finish()).finish())
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Max)
        .finish();

    let credit_count = Text::new_inline(
        remaining.separate_with_commas(),
        appearance.ui_font_family(),
        24.,
    )
    .with_color(theme.active_ui_text_color().into())
    .with_style(Properties::default().weight(Weight::Semibold))
    .finish();

    let remaining_label_text = match total {
        Some(limit) => format!("/ {} remaining", limit.separate_with_commas()),
        None => "remaining".to_string(),
    };
    let remaining_label = Text::new_inline(remaining_label_text, appearance.ui_font_family(), 14.)
        .with_color(sub_color)
        .finish();

    let value_row = Flex::row()
        .with_child(credit_count)
        .with_child(
            Container::new(remaining_label)
                .with_margin_left(4.)
                .with_padding_bottom(1.)
                .finish(),
        )
        .with_cross_axis_alignment(CrossAxisAlignment::End);

    Container::new(
        Flex::column()
            .with_child(header)
            .with_child(value_row.finish())
            .with_spacing(8.)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .finish(),
    )
    .with_border(Border::all(1.).with_border_color(border_color))
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
    .with_horizontal_padding(16.)
    .with_vertical_padding(12.)
    .finish()
}
