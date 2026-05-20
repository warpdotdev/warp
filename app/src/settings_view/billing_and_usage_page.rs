use chrono::Local;
use itertools::Itertools;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use thousands::Separable;
use warp_core::ui::theme::Fill;
use warp_core::{features::FeatureFlag, ui::appearance::Appearance};
use warp_graphql::billing::AddonCreditsOption;
use warpui::prelude::ChildView;
use warpui::{
    elements::{
        Align, Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
        Empty, Flex, FormattedTextElement, HighlightedHyperlink, Hoverable, HyperlinkUrl,
        MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
        ParentElement, ParentOffsetBounds, Radius, Shrinkable, Text, Wrap,
    },
    fonts::{Properties, Weight},
    platform::Cursor,
    ui_components::{
        button::{ButtonVariant, TextAndIcon, TextAndIconAlignment},
        components::{Coords, UiComponent, UiComponentStyles},
        switch::SwitchStateHandle,
    },
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, UpdateView, View,
    ViewContext, ViewHandle,
};

use settings::Setting;

use crate::{
    ai::AIRequestUsageModel,
    auth::{
        auth_manager::LoginGatedFeature, auth_state::AuthState, auth_view_modal::AuthViewVariant,
        AuthManager, AuthStateProvider, UserUid,
    },
    menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields},
    modal::{Modal, ModalEvent, ModalViewState},
    pricing::{PricingInfoModel, PricingInfoModelEvent},
    send_telemetry_from_ctx,
    server::{ids::ServerId, telemetry::TelemetryEvent},
    settings::ai::AISettings,
    settings_view::settings_page::TOGGLE_BUTTON_RIGHT_PADDING,
    ui_components::{
        blended_colors,
        buttons::icon_button,
        icons::Icon,
        menu_button::{icon_button_with_context_menu, MenuDirection},
        tab_selector::{self, SettingsTab},
    },
    view_components::{
        action_button::{ActionButton, PrimaryTheme, SecondaryTheme},
        ToastFlavor,
    },
    workspaces::{
        team::Team,
        update_manager::TeamUpdateManager,
        user_profiles::UserProfiles,
        user_workspaces::{UserWorkspaces, UserWorkspacesEvent},
        workspace::{CustomerType, Workspace},
    },
    WorkspaceAction,
};

use super::{
    admin_actions::AdminActions,
    billing_and_usage::{
        overage_limit_modal::{SpendingLimitModal, SpendingLimitModalEvent},
        usage_history_entry::UsageHistoryEntry,
        usage_history_model::UsageHistoryModel,
    },
    settings_page::{
        build_sub_header, render_body_item, render_customer_type_badge, render_info_icon,
        AdditionalInfo, Category, PageType, SettingsPageMeta, SettingsPageViewHandle,
        SettingsWidget, HEADER_PADDING,
    },
    MatchData, SettingsSection,
};

const HEADER_FONT_SIZE: f32 = 16.;
const OVERAGE_USAGE_LINK_TEXT: &str = "View details on overage usage";
const OVERAGE_TOGGLE_ADMIN_HEADER: &str = "Enable premium model usage overages";
const OVERAGE_TOGGLE_USER_HEADER_ENABLED: &str = "Premium model usage overages are enabled";
const OVERAGE_TOGGLE_USER_HEADER_DISABLED: &str = "Premium model usage overages are not enabled";
const OVERAGE_TOGGLE_DESCRIPTION: &str = "Continue using premium models beyond your plan's limits. Usage is charged in $20 increments up to your spending limit, with any remaining balance charged on your scheduled billing date.";
const OVERAGE_TOGGLE_USER_DESCRIPTION: &str =
    "Ask a team admin to enable overages for more AI usage.";

const SORT_MENU_ITEM_DISPLAY_NAME_A_Z_LABEL: &str = "A to Z";
const SORT_MENU_ITEM_DISPLAY_NAME_Z_A_LABEL: &str = "Z to A";
const SORT_MENU_ITEM_REQUEST_USAGE_ASCENDING_LABEL: &str = "Usage ascending";
const SORT_MENU_ITEM_REQUEST_USAGE_DESCENDING_LABEL: &str = "Usage descending";

const AUTO_RELOAD_EXCEED_LIMIT_WARNING_STRING: &str =
    "Auto reload is disabled, as the next reload would exceed your monthly spend limit. Increase your limit to use auto reload.";
const AUTO_RELOAD_DELINQUENT_WARNING_STRING: &str =
    "Restricted due to billing issue. Update your payment method to purchase add-on credits.";
const RESTRICTED_BILLING_USAGE_WARNING_STRING: &str =
    "Auto reload is disabled due to recent failed reload. Please update your payment method and try again.";

const OVERVIEW_TAB_TEXT: &str = "Overview";
const USAGE_HISTORY_TAB_TEXT: &str = "Usage History";

const ENTERPRISE_USAGE_CALLOUT_HEADER: &str = "Usage reporting is currently limited";
const ENTERPRISE_USAGE_CALLOUT_BODY_ADMIN_PREFIX: &str =
    "Enterprise credit usage isn't fully available in this view yet. For the most accurate spend tracking, ";
const ENTERPRISE_USAGE_CALLOUT_BODY_ADMIN_LINK: &str = "visit the admin panel";
const ENTERPRISE_USAGE_CALLOUT_BODY_ADMIN_SUFFIX: &str = ".";
const ENTERPRISE_USAGE_CALLOUT_BODY_NON_ADMIN: &str =
    "Enterprise credit usage isn't fully available in this view yet. Contact a team admin for detailed usage reporting.";

const ADDON_CREDITS_DESCRIPTION: &str = "Add-on credits are purchased in prepaid packages that roll over each billing cycle and expire after one year. The more you purchase, the better the per-credit rate. Once your base plan credits are used, add-on credits will be consumed.";
const ADDITIONAL_ADDON_CREDITS_DESCRIPTION_FOR_TEAM: &str =
    "Purchased add-on credits are shared across your team.";

// Cloud agent trial widget constants.
const AMBIENT_AGENT_TRIAL_TITLE: &str = "Cloud agent trial";
/// The threshold below which we only show the "Buy more" button (not "New agent").
use crate::ai::request_usage_model::AMBIENT_AGENT_TRIAL_CREDIT_THRESHOLD;

pub fn create_discount_badge(discount: u32, appearance: &Appearance) -> Box<dyn Element> {
    if discount == 0 {
        return Empty::new().finish();
    }

    let theme = appearance.theme();
    let bg_color: Fill = theme.terminal_colors().normal.green.into();

    Container::new(
        Text::new_inline(format!("{discount}% off"), appearance.ui_font_family(), 10.)
            .with_color(theme.main_text_color(bg_color).into())
            .finish(),
    )
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
    .with_background(bg_color)
    .with_uniform_padding(4.)
    .finish()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BillingUsageTab {
    Overview,
    UsageHistory,
}
impl BillingUsageTab {
    pub fn get_tab_from_label(label: &str) -> Self {
        match label {
            OVERVIEW_TAB_TEXT => BillingUsageTab::Overview,
            USAGE_HISTORY_TAB_TEXT => BillingUsageTab::UsageHistory,
            _ => BillingUsageTab::Overview,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            BillingUsageTab::Overview => OVERVIEW_TAB_TEXT,
            BillingUsageTab::UsageHistory => USAGE_HISTORY_TAB_TEXT,
        }
    }
}

/// Represents a user item for sorting in the billing and usage page.
/// The display_name should already have email as fallback before creating this struct.
pub(crate) struct UserSortingCriteria<T> {
    pub display_name: String,
    pub requests_used: usize,
    pub data: T,
}

impl<T> UserSortingCriteria<T> {
    pub fn new(display_name: String, requests_used: usize, data: T) -> Self {
        Self {
            display_name,
            requests_used,
            data,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SortKey {
    DisplayName,
    Requests,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SortOrder {
    Asc,
    Desc,
}

pub(crate) struct ProratedRequestLimitsInfo {
    pub is_request_limit_prorated: bool,
    pub mouse_state: MouseStateHandle,
    pub is_current_user: bool,
}

pub struct BillingAndUsagePageView {
    page: PageType<Self>,
    auth_state: Arc<AuthState>,
    overage_limit_modal_state: ModalViewState<Modal<SpendingLimitModal>>,
    addon_credit_modal_state: ModalViewState<Modal<SpendingLimitModal>>,
    // Since UBP can take a second to enable due to needing to contact Stripe,
    // we allow the view to override the state of the toggle temporarily.
    usage_based_pricing_toggle_override: Option<bool>,
    usage_based_pricing_toggle_loading: bool,
    // Sorting menu state
    sorting_menu: ViewHandle<Menu<BillingAndUsagePageAction>>,
    sorting_menu_open: bool,
    // Current sort state for the members list
    current_sort_key: Option<SortKey>,
    current_sort_order: SortOrder,
    // Which view tab is currently selected
    selected_tab: BillingUsageTab,
    // Model for Usage History tab data
    usage_history_model: ModelHandle<UsageHistoryModel>,
    // Track which usage history entries have been expanded
    expanded_usage_entries: HashMap<String, bool>,
    // Persistent mouse states for usage history entries, keyed by conversation_id
    usage_entries_mouse_states: RefCell<HashMap<String, MouseStateHandle>>,
    // Persistent mouse states for tooltips in usage history entries, keyed by conversation_id
    usage_entries_tooltip_mouse_states: RefCell<HashMap<String, MouseStateHandle>>,
    // Action button for loading more usage history entries
    load_more_button: ViewHandle<ActionButton>,
    selected_addon_denomination: usize,
    addon_credits_options: Vec<AddonCreditsOption>,
    addon_credit_denomination_buttons: Vec<ViewHandle<ActionButton>>,
    purchase_addon_credits_loading: bool,
    prorated_request_limits_info_mouse_states: Vec<MouseStateHandle>,
}

impl BillingAndUsagePageView {
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

        ctx.subscribe_to_model(&PricingInfoModel::handle(ctx), |me, _handle, event, ctx| {
            #[allow(irrefutable_let_patterns)]
            if let PricingInfoModelEvent::PricingInfoUpdated = event {
                me.update_addon_credits_options(ctx);
                me.refresh_addon_credits_settings(ctx);
                ctx.notify();
            }
        });

        let usage_history_model = ctx.add_model(UsageHistoryModel::new);
        ctx.subscribe_to_model(&usage_history_model, |_, _, _, ctx| {
            ctx.notify();
        });
        // On page init, fetch the usage history for the current user.
        usage_history_model.update(ctx, |m, ctx| m.refresh_usage_history_async(ctx));

        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();

        let overage_limit_modal = ctx.add_typed_action_view(SpendingLimitModal::new);
        ctx.subscribe_to_view(&overage_limit_modal, |me, _, event, ctx| {
            me.handle_overage_limit_modal_event(event, ctx);
        });

        let overage_limit_modal_view = ctx.add_typed_action_view(|ctx| {
            Modal::new(
                Some("Overage spending limit".to_string()),
                overage_limit_modal,
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
        ctx.subscribe_to_view(&overage_limit_modal_view, |me, _, event, ctx| {
            me.handle_overage_modal_close_event(event, ctx);
        });

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

        // Sorting menu view and close handling
        let sorting_menu =
            ctx.add_typed_action_view(|_ctx| Menu::<BillingAndUsagePageAction>::new());
        ctx.subscribe_to_view(&sorting_menu, |me, _, event, ctx| {
            if let MenuEvent::Close { .. } = event {
                me.sorting_menu_open = false;
                ctx.notify();
            }
        });

        let load_more_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Load more", SecondaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(BillingAndUsagePageAction::RenderMoreUsageEntries);
            })
        });

        let mut me = Self {
            page: Self::build_page(),
            auth_state,
            overage_limit_modal_state: ModalViewState::new(overage_limit_modal_view),
            addon_credit_modal_state: ModalViewState::new(addon_credit_modal_view),
            usage_based_pricing_toggle_override: None,
            usage_based_pricing_toggle_loading: false,
            sorting_menu,
            sorting_menu_open: false,
            usage_history_model,
            current_sort_key: Some(SortKey::DisplayName),
            current_sort_order: SortOrder::Asc,
            selected_tab: BillingUsageTab::Overview,
            expanded_usage_entries: HashMap::new(),
            usage_entries_mouse_states: RefCell::new(HashMap::new()),
            usage_entries_tooltip_mouse_states: RefCell::new(HashMap::new()),
            load_more_button,
            selected_addon_denomination: 0,
            addon_credits_options: Default::default(),
            addon_credit_denomination_buttons: Default::default(),
            purchase_addon_credits_loading: false,
            prorated_request_limits_info_mouse_states: Default::default(),
        };
        me.update_addon_credits_options(ctx);
        me.refresh_addon_credits_settings(ctx);
        me.update_prorated_mouse_states(ctx);
        me
    }

    fn build_page() -> PageType<Self> {
        let categories = vec![Category::new(
            "Billing and usage",
            vec![
                Box::new(PlanWidget::default()),
                Box::new(UsageWidget::default()),
            ],
        )];

        PageType::new_categorized(categories, None)
    }

    fn refresh_addon_credits_settings(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(workspace) = UserWorkspaces::as_ref(ctx).current_workspace() else {
            return;
        };
        let addon_credits_settings = &workspace.settings.addon_credits_settings;
        if addon_credits_settings.auto_reload_enabled {
            self.selected_addon_denomination = addon_credits_settings
                .selected_auto_reload_credit_denomination
                .and_then(|amount| {
                    self.addon_credits_options
                        .iter()
                        .find_position(|option| option.credits == amount)
                })
                .map_or(0, |pair| pair.0);
        }
        self.update_denomination_buttons_focus(ctx);
    }

    fn update_denomination_buttons_focus(&mut self, ctx: &mut ViewContext<Self>) {
        for (i, button_handle) in self.addon_credit_denomination_buttons.iter().enumerate() {
            ctx.update_view(button_handle, |button, ctx| {
                if i == self.selected_addon_denomination {
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
                self.update_spending_limit_modals(ctx);
                self.update_prorated_mouse_states(ctx);
            }
            UserWorkspacesEvent::UpdateWorkspaceSettingsSuccess => {
                self.update_spending_limit_modals(ctx);
                self.refresh_addon_credits_settings(ctx);
                self.update_prorated_mouse_states(ctx);
                self.usage_based_pricing_toggle_override = None;
                self.usage_based_pricing_toggle_loading = false;
                ctx.notify();
            }
            UserWorkspacesEvent::UpdateWorkspaceSettingsRejected(_err) => {
                self.show_toast(
                    "Failed to update workspace settings",
                    ToastFlavor::Error,
                    ctx,
                );
                self.usage_based_pricing_toggle_override = None;
                self.usage_based_pricing_toggle_loading = false;
            }
            UserWorkspacesEvent::AiOveragesUpdated => {
                ctx.notify();
            }
            UserWorkspacesEvent::PurchaseAddonCreditsSuccess => {
                self.purchase_addon_credits_loading = false;
                self.show_toast(
                    "Successfully purchased add-on credits",
                    ToastFlavor::Success,
                    ctx,
                );
                AIRequestUsageModel::handle(ctx).update(ctx, |ai_request_usage_model, ctx| {
                    ai_request_usage_model.refresh_request_usage_async(ctx)
                });
            }
            UserWorkspacesEvent::PurchaseAddonCreditsRejected(err) => {
                self.purchase_addon_credits_loading = false;
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
        if self.overage_limit_modal_state.is_open() {
            Some(self.overage_limit_modal_state.render())
        } else if self.addon_credit_modal_state.is_open() {
            Some(self.addon_credit_modal_state.render())
        } else {
            None
        }
    }

    fn handle_overage_modal_close_event(
        &mut self,
        event: &ModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ModalEvent::Close => {
                self.overage_limit_modal_state.close();
                ctx.emit(BillingAndUsagePageEvent::HideModal);
            }
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

    fn handle_overage_limit_modal_event(
        &mut self,
        event: &SpendingLimitModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SpendingLimitModalEvent::Close => {
                self.hide_overage_limit_modal(ctx);
            }
            SpendingLimitModalEvent::Update { amount_cents } => {
                let workspaces = UserWorkspaces::as_ref(ctx);
                let team_uid = workspaces.current_team_uid();
                let usage_settings = workspaces.usage_based_pricing_settings();

                if let Some(team_uid) = team_uid {
                    self.update_usage_based_pricing_settings(
                        team_uid,
                        usage_settings.enabled,
                        Some(*amount_cents),
                        ctx,
                    );
                    self.hide_overage_limit_modal(ctx);
                    ctx.notify();
                }
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

    fn update_usage_based_pricing_settings(
        &mut self,
        team_uid: ServerId,
        enabled: bool,
        max_monthly_spend_cents: Option<u32>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.usage_based_pricing_toggle_loading = true;
        UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
            user_workspaces.update_usage_based_pricing_settings(
                team_uid,
                enabled,
                max_monthly_spend_cents,
                ctx,
            );
        });

        self.usage_based_pricing_toggle_override = Some(enabled);
    }

    fn show_overage_limit_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.overage_limit_modal_state.open();

        self.overage_limit_modal_state
            .view
            .update(ctx, |modal, ctx| {
                modal.body().update(ctx, |body, ctx| {
                    body.focus_input(ctx);
                });
            });

        ctx.emit(BillingAndUsagePageEvent::ShowModal);
    }

    fn hide_overage_limit_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.overage_limit_modal_state.close();
        ctx.emit(BillingAndUsagePageEvent::HideModal);
    }

    fn update_spending_limit_modals(&mut self, ctx: &mut ViewContext<Self>) {
        let workspaces = UserWorkspaces::as_ref(ctx);
        let usage_settings = workspaces.usage_based_pricing_settings();
        let overage_limit = usage_settings.max_monthly_spend_cents.unwrap_or(5000);
        let addon_limit = workspaces
            .current_workspace()
            .and_then(|workspace| {
                workspace
                    .settings
                    .addon_credits_settings
                    .max_monthly_spend_cents
            })
            .unwrap_or(20000);

        self.overage_limit_modal_state
            .view
            .update(ctx, |modal, ctx| {
                modal.body().update(ctx, |body, ctx| {
                    body.update_amount_editor(overage_limit, ctx);
                });
            });
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
        self.addon_credits_options = PricingInfoModel::as_ref(ctx)
            .addon_credits_options()
            .map(|opts| opts.to_vec())
            .unwrap_or_default();
        self.addon_credit_denomination_buttons = self
            .addon_credits_options
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

    fn update_prorated_mouse_states(&mut self, ctx: &mut ViewContext<Self>) {
        let workspace_members_count = UserWorkspaces::as_ref(ctx)
            .current_workspace()
            .map(|workspace| workspace.members.len())
            .unwrap_or(0);
        self.prorated_request_limits_info_mouse_states = (0..workspace_members_count)
            .map(|_| MouseStateHandle::default())
            .collect();
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
}

impl SettingsPageMeta for BillingAndUsagePageView {
    fn section() -> SettingsSection {
        SettingsSection::BillingAndUsage
    }

    fn should_render(&self, ctx: &AppContext) -> bool {
        let is_anonymous = AuthStateProvider::as_ref(ctx)
            .get()
            .is_anonymous_or_logged_out();

        !is_anonymous
    }

    fn on_page_selected(&mut self, _: bool, ctx: &mut ViewContext<Self>) {
        self.purchase_addon_credits_loading = false;
        std::mem::drop(
            TeamUpdateManager::handle(ctx)
                .update(ctx, |manager, ctx| manager.refresh_workspace_metadata(ctx)),
        );

        AIRequestUsageModel::handle(ctx).update(ctx, |ai_request_usage_model, ctx| {
            ai_request_usage_model.refresh_request_usage_async(ctx)
        });

        self.usage_history_model
            .update(ctx, |m, ctx| m.refresh_usage_history_async(ctx));

        self.refresh_addon_credits_settings(ctx);
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

#[derive(Debug, Clone)]
pub enum BillingAndUsagePageEvent {
    SignupAnonymousUser,
    ShowToast {
        message: String,
        flavor: ToastFlavor,
    },
    ShowModal,
    HideModal,
}

impl Entity for BillingAndUsagePageView {
    type Event = BillingAndUsagePageEvent;
}

impl View for BillingAndUsagePageView {
    fn ui_name() -> &'static str {
        "Billing and usage"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl TypedActionView for BillingAndUsagePageView {
    type Action = BillingAndUsagePageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        if AuthStateProvider::as_ref(ctx)
            .get()
            .is_anonymous_or_logged_out()
            && action.blocked_for_anonymous_user()
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
                Some(team_uid) => {
                    ctx.open_url(&UserWorkspaces::upgrade_link_for_team(*team_uid));
                }
                None => {
                    ctx.open_url(&UserWorkspaces::upgrade_link(*user_id));
                }
            },
            BillingAndUsagePageAction::GenerateStripeBillingPortalLink { team_uid } => {
                UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                    user_workspaces.generate_stripe_billing_portal_link(*team_uid, ctx);
                });
            }
            BillingAndUsagePageAction::OpenAdminPanel { team_uid } => {
                AdminActions::open_admin_panel(*team_uid, ctx);
            }
            BillingAndUsagePageAction::ContactSupport => {
                AdminActions::contact_support(ctx);
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
            BillingAndUsagePageAction::OpenUrl(url) => {
                ctx.open_url(&url.url);
            }
            BillingAndUsagePageAction::UpdateUsageBasedPricingSettings {
                team_uid,
                enabled,
                max_monthly_spend_cents,
            } => {
                self.update_usage_based_pricing_settings(
                    *team_uid,
                    *enabled,
                    *max_monthly_spend_cents,
                    ctx,
                );
            }
            BillingAndUsagePageAction::ShowOverageLimitModal => {
                self.show_overage_limit_modal(ctx);
            }
            BillingAndUsagePageAction::ToggleSortingMenu => {
                if self.sorting_menu_open {
                    self.sorting_menu_open = false;
                    ctx.focus_self();
                    ctx.notify();
                    return;
                }
                // Build four menu items with checkmark for selected state
                let sort_options = [
                    (
                        SORT_MENU_ITEM_DISPLAY_NAME_A_Z_LABEL,
                        SortKey::DisplayName,
                        SortOrder::Asc,
                    ),
                    (
                        SORT_MENU_ITEM_DISPLAY_NAME_Z_A_LABEL,
                        SortKey::DisplayName,
                        SortOrder::Desc,
                    ),
                    (
                        SORT_MENU_ITEM_REQUEST_USAGE_ASCENDING_LABEL,
                        SortKey::Requests,
                        SortOrder::Asc,
                    ),
                    (
                        SORT_MENU_ITEM_REQUEST_USAGE_DESCENDING_LABEL,
                        SortKey::Requests,
                        SortOrder::Desc,
                    ),
                ];

                let items: Vec<MenuItem<BillingAndUsagePageAction>> = sort_options
                    .iter()
                    .map(|(label, key, order)| {
                        let is_selected = matches!(
                            (self.current_sort_key, self.current_sort_order),
                            (Some(k), o) if k == *key && o == *order
                        );

                        let mut menu_item = MenuItemFields::new(*label).with_on_select_action(
                            BillingAndUsagePageAction::ChangeUsageSort {
                                key: *key,
                                order: *order,
                            },
                        );

                        menu_item = if is_selected {
                            menu_item.with_icon(Icon::Check)
                        } else {
                            menu_item.with_indent()
                        };

                        menu_item.into_item()
                    })
                    .collect();

                ctx.update_view(&self.sorting_menu, |menu, ctx| menu.set_items(items, ctx));
                self.sorting_menu_open = true;
                ctx.focus(&self.sorting_menu);
                ctx.notify();
            }
            BillingAndUsagePageAction::RefreshWorkspaceData => {
                std::mem::drop(
                    TeamUpdateManager::handle(ctx)
                        .update(ctx, |manager, ctx| manager.refresh_workspace_metadata(ctx)),
                );

                AIRequestUsageModel::handle(ctx).update(ctx, |ai_request_usage_model, ctx| {
                    ai_request_usage_model.refresh_request_usage_async(ctx)
                });
            }
            BillingAndUsagePageAction::ChangeUsageSort { key, order } => {
                self.current_sort_key = Some(*key);
                self.current_sort_order = *order;
                self.sorting_menu_open = false;
                ctx.focus_self();
                ctx.notify();
            }
            BillingAndUsagePageAction::SelectTab(tab) => {
                if self.selected_tab != *tab {
                    self.selected_tab = tab.clone();
                    ctx.notify();
                }
            }
            BillingAndUsagePageAction::ToggleUsageEntryExpanded { conversation_id } => {
                let is_expanded = self
                    .expanded_usage_entries
                    .get(conversation_id)
                    .copied()
                    .unwrap_or(false);

                self.expanded_usage_entries
                    .insert(conversation_id.clone(), !is_expanded);
                ctx.notify();
            }
            BillingAndUsagePageAction::RenderMoreUsageEntries => {
                self.usage_history_model
                    .update(ctx, |m, ctx| m.load_more_usage_history_async(ctx));
            }
            BillingAndUsagePageAction::SelectTopupDenomination(i) => {
                self.selected_addon_denomination = *i;
                self.update_denomination_buttons_focus(ctx);
                UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                    let team_uid = user_workspaces.current_team_uid();
                    if let Some((workspace, team_uid)) =
                        user_workspaces.current_workspace().zip(team_uid)
                    {
                        if workspace
                            .settings
                            .addon_credits_settings
                            .auto_reload_enabled
                        {
                            if let Some(option) = self
                                .addon_credits_options
                                .get(self.selected_addon_denomination)
                            {
                                user_workspaces.update_addon_credits_settings(
                                    team_uid,
                                    None,
                                    None,
                                    Some(option.credits),
                                    ctx,
                                );
                            }
                        }
                    }
                });
                ctx.notify();
            }
            BillingAndUsagePageAction::PurchaseAddonCredits { team_uid } => {
                if let Some(option) = self
                    .addon_credits_options
                    .get(self.selected_addon_denomination)
                {
                    let credits = option.credits;
                    let team_uid = *team_uid;
                    self.purchase_addon_credits_loading = true;
                    UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                        user_workspaces.purchase_addon_credits(team_uid, credits, ctx);
                    });
                    ctx.notify();
                }
            }
            BillingAndUsagePageAction::ShowAddOnCreditModal => {
                self.show_addon_credit_modal(ctx);
            }
            BillingAndUsagePageAction::UpdateAutoReloadEnabled { team_uid, enabled } => {
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

                let selected_auto_reload_value = if *enabled {
                    self.addon_credits_options
                        .get(self.selected_addon_denomination)
                        .map(|option| option.credits)
                } else {
                    None
                };
                UserWorkspaces::handle(ctx).update(ctx, |user_workspaces, ctx| {
                    user_workspaces.update_addon_credits_settings(
                        *team_uid,
                        Some(*enabled),
                        None,
                        selected_auto_reload_value,
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

impl From<ViewHandle<BillingAndUsagePageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<BillingAndUsagePageView>) -> Self {
        SettingsPageViewHandle::BillingAndUsage(view_handle)
    }
}

#[derive(Debug, Clone)]
pub enum BillingAndUsagePageAction {
    OpenUrl(HyperlinkUrl),
    Upgrade {
        team_uid: Option<ServerId>,
        user_id: UserUid,
    },
    GenerateStripeBillingPortalLink {
        team_uid: ServerId,
    },
    OpenAdminPanel {
        team_uid: ServerId,
    },
    ContactSupport,
    SignupAnonymousUser,
    AttemptLoginGatedUpgrade,
    UpdateUsageBasedPricingSettings {
        team_uid: ServerId,
        enabled: bool,
        max_monthly_spend_cents: Option<u32>,
    },
    ShowOverageLimitModal,
    RefreshWorkspaceData,
    ToggleSortingMenu,
    ChangeUsageSort {
        key: SortKey,
        order: SortOrder,
    },
    SelectTab(BillingUsageTab),
    ToggleUsageEntryExpanded {
        conversation_id: String,
    },
    RenderMoreUsageEntries,
    SelectTopupDenomination(usize),
    PurchaseAddonCredits {
        team_uid: ServerId,
    },
    ShowAddOnCreditModal,
    UpdateAutoReloadEnabled {
        team_uid: ServerId,
        enabled: bool,
    },
    DismissAmbientAgentTrialWidget,
    NavigateToByokSettings,
}

impl BillingAndUsagePageAction {
    fn blocked_for_anonymous_user(&self) -> bool {
        use BillingAndUsagePageAction::*;
        matches!(
            self,
            Upgrade { .. } | GenerateStripeBillingPortalLink { .. },
        )
    }
}

impl From<&BillingAndUsagePageAction> for LoginGatedFeature {
    fn from(val: &BillingAndUsagePageAction) -> LoginGatedFeature {
        use BillingAndUsagePageAction::*;
        match val {
            Upgrade { .. } => "Upgrade Plan",
            GenerateStripeBillingPortalLink { .. } => "Generate Stripe Billing Portal Link",
            _ => "Unknown reason",
        }
    }
}

#[derive(Default)]
struct UsageWidget {
    requests_highlight_index: HighlightedHyperlink,
    ubp_switch_state: SwitchStateHandle,
    ubp_info_icon_mouse_state: MouseStateHandle,
    pencil_icon_mouse_state: MouseStateHandle,
    overage_usage_link_mouse_state: MouseStateHandle,
    // Mouse state for the inline "Increase your limit" link inside the warning row
    exceed_limit_link_mouse_state: MouseStateHandle,
    refresh_icon_mouse_state: MouseStateHandle,
    sort_icon_mouse_state: MouseStateHandle,
    overview_tab_mouse_state: MouseStateHandle,
    usage_history_tab_mouse_state: MouseStateHandle,
    addon_info_icon_mouse_state: MouseStateHandle,
    edit_monthly_limit: MouseStateHandle,
    auto_reload_switch: SwitchStateHandle,
    buy_button: MouseStateHandle,
    // Ambient agent trial widget buttons.
    ambient_trial_new_agent_button: MouseStateHandle,
    ambient_trial_buy_more_button: MouseStateHandle,
    ambient_trial_dismiss_button: MouseStateHandle,
}

#[derive(Copy, Clone, Debug)]
enum Divisor {
    Unlimited,
    Limit(usize),
}

impl UsageWidget {
    /// Renders the ambient agent trial widget showing remaining credits and action buttons.
    /// Returns None if the user has no ambient-only credits (None value from server),
    /// or if the widget has been dismissed (only dismissible when below threshold).
    fn render_ambient_agent_trial_widget(
        &self,
        ai_request_usage_model: &AIRequestUsageModel,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let credits_remaining = ai_request_usage_model.ambient_only_credits_remaining()?;

        // Check if the widget has been dismissed.
        let is_dismissed = *AISettings::as_ref(app).ambient_agent_trial_widget_dismissed;
        if is_dismissed {
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

        // Only show "New agent" button if credits >= threshold.
        if credits_remaining >= AMBIENT_AGENT_TRIAL_CREDIT_THRESHOLD {
            let new_agent_button = ui_builder
                .button(
                    ButtonVariant::Secondary,
                    self.ambient_trial_new_agent_button.clone(),
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

        // Only show "Buy more" button for users not on a paid plan.
        let is_on_paid_plan = UserWorkspaces::as_ref(app)
            .current_team()
            .is_some_and(|team| team.billing_metadata.is_user_on_paid_plan());
        if !is_on_paid_plan {
            let user_id = AuthStateProvider::as_ref(app).get().user_id();
            let buy_more_button = ui_builder
                .button(
                    ButtonVariant::Secondary,
                    self.ambient_trial_buy_more_button.clone(),
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

        // Show dismiss button only when credits are below threshold.
        if credits_remaining < AMBIENT_AGENT_TRIAL_CREDIT_THRESHOLD {
            let dismiss_button = icon_button(
                appearance,
                Icon::X,
                false,
                self.ambient_trial_dismiss_button.clone(),
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

        let card = Container::new(row_content)
            .with_horizontal_background_gradient(gradient_start, gradient_end)
            .with_border(Border::all(1.).with_border_color(theme.accent_overlay().into()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_margin_bottom(16.)
            .with_uniform_padding(12.)
            .finish();

        Some(card)
    }

    fn render_usage_based_pricing_section(
        &self,
        enabled: bool,
        team: &Team,
        appearance: &Appearance,
        app: &AppContext,
        has_admin_permissions: bool,
        ubp_toggle_loading: bool,
    ) -> Box<dyn Element> {
        let is_delinquent = team.billing_metadata.is_delinquent_due_to_payment_issue();
        let enabled_and_not_delinquent = enabled && !is_delinquent;

        let (header_text, description_text) = if has_admin_permissions {
            (OVERAGE_TOGGLE_ADMIN_HEADER, OVERAGE_TOGGLE_DESCRIPTION)
        } else if enabled {
            (
                OVERAGE_TOGGLE_USER_HEADER_ENABLED,
                OVERAGE_TOGGLE_DESCRIPTION,
            )
        } else {
            (
                OVERAGE_TOGGLE_USER_HEADER_DISABLED,
                OVERAGE_TOGGLE_USER_DESCRIPTION,
            )
        };

        let header = Text::new_inline(header_text, appearance.ui_font_family(), 14.)
            .with_color(appearance.theme().active_ui_text_color().into())
            .finish();

        let description = appearance
            .ui_builder()
            .paragraph(description_text)
            .with_style(UiComponentStyles {
                font_color: Some(blended_colors::text_sub(
                    appearance.theme(),
                    appearance.theme().surface_1(),
                )),
                font_size: Some(12.),
                margin: Some(Coords {
                    top: 4.,
                    bottom: 0.,
                    left: 0.,
                    right: 0.,
                }),
                ..Default::default()
            })
            .build()
            .finish();

        let mut column = Flex::column();

        if has_admin_permissions {
            let team_uid = team.uid;
            let toggle = appearance
                .ui_builder()
                .switch(self.ubp_switch_state.clone())
                .check(enabled_and_not_delinquent)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(
                        BillingAndUsagePageAction::UpdateUsageBasedPricingSettings {
                            team_uid,
                            enabled: !enabled,
                            max_monthly_spend_cents: None,
                        },
                    );
                });

            let toggle = if ubp_toggle_loading || is_delinquent {
                toggle.disable().finish()
            } else {
                toggle.finish()
            };

            column.add_child(
                Flex::row()
                    .with_child(header)
                    .with_child(Container::new(toggle).with_margin_left(16.).finish())
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_main_axis_size(MainAxisSize::Max)
                    .finish(),
            );
        } else {
            column.add_child(header);
        }

        column.add_child(Container::new(description).with_margin_right(100.).finish());

        if enabled_and_not_delinquent || team.billing_metadata.has_overages_used() {
            column.add_child(self.render_monthly_overage_spending_limit(
                appearance,
                app,
                has_admin_permissions,
            ));
            column.add_child(self.render_total_overages_row(appearance, app));
            if let Some(manage_link) =
                self.render_manage_overages_link(appearance, team.uid, has_admin_permissions)
            {
                column.add_child(manage_link);
            }
        }

        column.finish()
    }

    fn render_monthly_overage_spending_limit(
        &self,
        appearance: &Appearance,
        app: &AppContext,
        has_admin_permissions: bool,
    ) -> Box<dyn Element> {
        let workspaces = UserWorkspaces::as_ref(app);
        let usage_settings = workspaces.usage_based_pricing_settings();

        let spend_limit_text = if let Some(cents) = usage_settings.max_monthly_spend_cents {
            format!("${:.2}", cents as f64 / 100.0)
        } else {
            "Not set".to_string()
        };

        let info_icon = render_info_icon(
            appearance,
            AdditionalInfo::<BillingAndUsagePageAction> {
                mouse_state: self.ubp_info_icon_mouse_state.clone(),
                on_click_action: None,
                secondary_text: None,
                tooltip_override_text: Some(
                    "Sets the monthly overage spending limit beyond the plan amount".to_string(),
                ),
            },
        );

        let label = Text::new_inline(
            "Monthly overage spending limit",
            appearance.ui_font_family(),
            12.,
        )
        .with_color(appearance.theme().active_ui_text_color().into())
        .finish();

        let value = Text::new_inline(spend_limit_text, appearance.ui_font_family(), 12.)
            .with_color(blended_colors::text_sub(
                appearance.theme(),
                appearance.theme().surface_1(),
            ))
            .finish();

        let mut right_side = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        if has_admin_permissions {
            let pencil_icon = icon_button(
                appearance,
                Icon::Pencil,
                false,
                self.pencil_icon_mouse_state.clone(),
            )
            .with_style(UiComponentStyles {
                width: Some(20.),
                height: Some(20.),
                ..Default::default()
            })
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(BillingAndUsagePageAction::ShowOverageLimitModal);
            })
            .finish();

            right_side.add_child(Container::new(pencil_icon).with_margin_right(8.).finish());
        }

        right_side.add_child(value);

        Container::new(
            Flex::row()
                .with_child(
                    Flex::row()
                        .with_child(label)
                        .with_child(Container::new(info_icon).with_margin_left(4.).finish())
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .finish(),
                )
                .with_child(right_side.finish())
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_main_axis_size(MainAxisSize::Max)
                .finish(),
        )
        .with_margin_top(16.)
        .finish()
    }

    fn render_manage_overages_link(
        &self,
        appearance: &Appearance,
        team_uid: ServerId,
        has_admin_permissions: bool,
    ) -> Option<Box<dyn Element>> {
        if has_admin_permissions {
            Some(
                appearance
                    .ui_builder()
                    .link(
                        OVERAGE_USAGE_LINK_TEXT.to_string(),
                        None,
                        Some(Box::new(move |ctx| {
                            ctx.dispatch_typed_action(
                                BillingAndUsagePageAction::GenerateStripeBillingPortalLink {
                                    team_uid,
                                },
                            );
                        })),
                        self.overage_usage_link_mouse_state.clone(),
                    )
                    .build()
                    .with_margin_top(16.)
                    .finish(),
            )
        } else {
            None
        }
    }

    fn render_warning_icon(appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        ConstrainedBox::new(
            Icon::AlertTriangle
                .to_warpui_icon(theme.ui_error_color().into())
                .finish(),
        )
        .with_height(16.)
        .with_width(16.)
        .finish()
    }

    fn render_warning_row_with_content(
        appearance: &Appearance,
        content: Box<dyn Element>,
    ) -> Box<dyn Element> {
        Container::new(
            Flex::row()
                .with_child(
                    Container::new(Self::render_warning_icon(appearance))
                        .with_margin_right(8.)
                        .finish(),
                )
                .with_child(Shrinkable::new(1.0, content).finish())
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        )
        .with_margin_top(8.) // 8px from spacing + 8px here = 16px total
        .finish()
    }

    fn render_warning_row(
        &self,
        appearance: &Appearance,
        warning_string: String,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let warning_text = Text::new(warning_string, appearance.ui_font_family(), 12.)
            .with_color(theme.ui_error_color())
            .finish();

        Self::render_warning_row_with_content(appearance, warning_text)
    }

    fn render_warning_row_with_link(
        &self,
        appearance: &Appearance,
        text_fragments: Vec<FormattedTextFragment>,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        // Build: [plain text] [always-underlined link] [plain text]
        let mut children: Vec<Box<dyn Element>> = Vec::new();
        let ui_builder = appearance.ui_builder();

        for fragment in text_fragments {
            match fragment.styles.hyperlink {
                Some(markdown_parser::Hyperlink::Url(url)) => {
                    let link = ui_builder
                        .link(
                            fragment.text,
                            Some(url),
                            None,
                            self.exceed_limit_link_mouse_state.clone(),
                        )
                        .with_style(UiComponentStyles {
                            // Make it look like a link in the error row
                            font_size: Some(12.),
                            font_color: Some(theme.ui_error_color()),
                            border_color: Some(theme.ui_error_color().into()), // always underline
                            border_width: Some(1.),
                            ..Default::default()
                        })
                        .build()
                        .finish();
                    children.push(link);
                }
                Some(markdown_parser::Hyperlink::Action(action)) => {
                    // Downcast to our action type and dispatch on click
                    let maybe_action = action
                        .as_any()
                        .downcast_ref::<BillingAndUsagePageAction>()
                        .cloned();
                    let link = ui_builder
                        .link(
                            fragment.text,
                            None,
                            maybe_action.map(|act| {
                                Box::new(move |ctx: &mut warpui::EventContext| {
                                    ctx.dispatch_typed_action(act.clone());
                                })
                                    as Box<dyn Fn(&mut warpui::EventContext)>
                            }),
                            self.exceed_limit_link_mouse_state.clone(),
                        )
                        .with_style(UiComponentStyles {
                            font_size: Some(12.),
                            font_color: Some(theme.ui_error_color()),
                            border_color: Some(theme.ui_error_color().into()),
                            border_width: Some(1.),
                            ..Default::default()
                        })
                        .build()
                        .finish();
                    children.push(link);
                }
                None => {
                    // Plain text in error color
                    let text = Text::new_inline(fragment.text, appearance.ui_font_family(), 12.)
                        .with_color(theme.ui_error_color())
                        .finish();
                    children.push(text);
                }
            }
        }

        let content = Flex::row().with_children(children).finish();
        Self::render_warning_row_with_content(appearance, content)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_addon_credits_panel(
        &self,
        selected_topup_denomination: usize,
        workspace: &Workspace,
        team_uid: ServerId,
        has_admin_permissions: bool,
        bonus_credit_balance: i32,
        addon_credits_options: &[AddonCreditsOption],
        addon_credit_denomination_buttons: &[ViewHandle<ActionButton>],
        purchase_addon_credits_loading: bool,
        delinquent_due_to_payment_issue: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let fg = appearance.theme().foreground();
        let bg = appearance.theme().background();
        let ui_builder = appearance.ui_builder();
        let theme = appearance.theme();

        let header = Text::new_inline("Add-on credits", appearance.ui_font_family(), 16.)
            .with_color(fg.into())
            .with_style(Properties::default().weight(Weight::Bold))
            .finish();

        let credits_value = Text::new_inline(
            bonus_credit_balance.separate_with_commas(),
            appearance.ui_font_family(),
            16.,
        )
        .with_color(fg.into())
        .finish();

        let icon = Container::new(
            ConstrainedBox::new(Icon::Credits.to_warpui_icon(fg).finish())
                .with_height(16.)
                .with_width(16.)
                .finish(),
        )
        .with_margin_right(12.)
        .finish();

        let card_header = Flex::row()
            .with_child(Shrinkable::new(1., Align::new(header).left().finish()).finish())
            .with_child(icon)
            .with_child(credits_value)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish();

        let team_can_purchase_addon_credits = UserWorkspaces::as_ref(app)
            .current_team()
            .and_then(|team| team.billing_metadata.tier.purchase_add_on_credits_policy)
            .is_some_and(|policy| policy.enabled);
        let can_upgrade_to_build = UserWorkspaces::as_ref(app)
            .current_workspace()
            .map(|workspace| workspace.billing_metadata.can_upgrade_to_build_plan())
            .unwrap_or(false);

        let no_credits_access_explanation = match (
            team_can_purchase_addon_credits,
            can_upgrade_to_build,
            has_admin_permissions,
        ) {
            // If the team can purchase addon credits (which implies they're already on a Build-like plan)
            // and the current user is a team admin, don't show any explanation, so that we show the
            // fuller experience with the rest of the settings below this.
            (true, _, true) => None,
            // If the team cannot purchase addon credits, but they can upgrade to a Build-like plan,
            // and the current user is an admin, then we show them a nudge to switch to Build.
            (false, true, true) => {
                let upgrade_url = UserWorkspaces::upgrade_link_for_team(team_uid);
                let is_legacy_paid = UserWorkspaces::handle(app)
                    .as_ref(app)
                    .current_team()
                    .is_some_and(|team| team.billing_metadata.is_on_legacy_paid_plan());
                let (link_text, suffix) = if is_legacy_paid {
                    ("Switch to the Build plan", " to purchase add-on credits.")
                } else {
                    ("Upgrade to the Build plan", " to purchase add-on credits.")
                };

                let text_fragments = vec![
                    FormattedTextFragment::hyperlink(link_text, upgrade_url),
                    FormattedTextFragment::plain_text(suffix),
                ];

                Some(
                    FormattedTextElement::new(
                        FormattedText::new([FormattedTextLine::Line(text_fragments)]),
                        appearance.ui_font_size(),
                        appearance.ui_font_family(),
                        appearance.ui_font_family(),
                        theme.sub_text_color(bg).into(),
                        HighlightedHyperlink::default(),
                    )
                    .with_hyperlink_font_color(theme.accent().into_solid())
                    .register_default_click_handlers_with_action_support(
                        |hyperlink_lens, event, ctx| match hyperlink_lens {
                            warpui::elements::HyperlinkLens::Url(url) => {
                                ctx.open_url(url);
                            }
                            warpui::elements::HyperlinkLens::Action(action_ref) => {
                                if let Some(action) = action_ref
                                    .as_any()
                                    .downcast_ref::<BillingAndUsagePageAction>()
                                {
                                    event.dispatch_typed_action(action.clone());
                                }
                            }
                        },
                    )
                    .finish(),
                )
            }
            // If the team cannot purchase addon credits, and they can't upgrade to Build, that means
            // they're on an Enterprise-like plan. For admins, we show them a message to contact their
            // Account Executive.
            (false, false, true) => {
                let paragraph_text = "Contact your Account Executive for more add-on credits.";
                Some(
                    ui_builder
                        .paragraph(paragraph_text)
                        .with_style(UiComponentStyles {
                            font_color: Some(theme.sub_text_color(bg).into()),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
            }
            // Every other case relates to not being a team admin. If you aren't an admin, we show
            // a generic message telling you to talk to them.
            (_, _, false) => {
                let paragraph_text = "Contact a team admin to purchase add-on credits.";
                Some(
                    ui_builder
                        .paragraph(paragraph_text)
                        .with_style(UiComponentStyles {
                            font_color: Some(theme.sub_text_color(bg).into()),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
            }
        };

        // If we have an explanation, render it + return early, since the rest of the content
        // here (monthly spend limits, ad-hoc purchasing of credits) isn't relevant.
        if let Some(no_credits_access_explanation) = no_credits_access_explanation {
            let card_content = Flex::column()
                .with_children([
                    Container::new(card_header).with_margin_bottom(8.).finish(),
                    no_credits_access_explanation,
                ])
                .finish();
            return Container::new(card_content)
                .with_background_color(theme.surface_1().into_solid())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .with_margin_bottom(16.)
                .with_uniform_padding(16.)
                .finish();
        }

        let team_member_count = UserWorkspaces::as_ref(app)
            .current_team()
            .map(|team| team.members.len())
            .unwrap_or(1);

        let paragraph_text = if team_member_count > 1 {
            format!("{ADDON_CREDITS_DESCRIPTION} {ADDITIONAL_ADDON_CREDITS_DESCRIPTION_FOR_TEAM}")
        } else {
            ADDON_CREDITS_DESCRIPTION.to_string()
        };
        let paragraph = ui_builder
            .paragraph(paragraph_text)
            .with_style(UiComponentStyles {
                font_color: Some(theme.sub_text_color(bg).into()),
                ..Default::default()
            })
            .build()
            .finish();

        let info_icon = render_info_icon(
            appearance,
            AdditionalInfo::<BillingAndUsagePageAction> {
                mouse_state: self.addon_info_icon_mouse_state.clone(),
                on_click_action: None,
                secondary_text: None,
                tooltip_override_text: Some(
                    "Sets the monthly limit spent on add-on credits".to_string(),
                ),
            },
        );

        let spend_limit_text = workspace
            .settings
            .addon_credits_settings
            .max_monthly_spend_cents
            .map(|cents| format!("${:.2}", cents as f64 / 100.0))
            .unwrap_or_else(|| "$200.00".to_string());

        let monthly_spend_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_children([
                ui_builder.span("Monthly spend limit").build().finish(),
                Shrinkable::new(1., Align::new(info_icon).left().finish()).finish(),
                icon_button(
                    appearance,
                    Icon::Pencil,
                    false,
                    self.edit_monthly_limit.clone(),
                )
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(BillingAndUsagePageAction::ShowAddOnCreditModal);
                })
                .finish(),
                ui_builder.span(spend_limit_text).build().finish(),
            ])
            .finish();

        let bonus_grants_purchased = UserWorkspaces::as_ref(app)
            .current_workspace()
            .map(|workspace| workspace.bonus_grants_purchased_this_month.clone());

        let purchased_this_month_row = if let Some(bonus_grants) = bonus_grants_purchased {
            if bonus_grants.total_credits_purchased == 0 {
                None
            } else {
                let credits_purchased = bonus_grants.total_credits_purchased;
                let cost_cents = bonus_grants.cents_spent;
                let cost_dollars = cost_cents as f64 / 100.0;

                let label =
                    Text::new_inline("Purchased this month", appearance.ui_font_family(), 12.)
                        .with_color(appearance.theme().active_ui_text_color().into())
                        .finish();

                let credits_text = if credits_purchased == 1 {
                    "1 credit".to_string()
                } else {
                    format!("{} credits", credits_purchased.separate_with_commas())
                };

                let credits_component = Container::new(
                    Text::new_inline(credits_text, appearance.ui_font_family(), 12.)
                        .with_color(blended_colors::text_disabled(
                            appearance.theme(),
                            appearance.theme().surface_1(),
                        ))
                        .finish(),
                )
                .with_margin_right(8.)
                .finish();

                let cost_component = Text::new_inline(
                    format!("${cost_dollars:.2}"),
                    appearance.ui_font_family(),
                    12.,
                )
                .with_color(blended_colors::text_sub(
                    appearance.theme(),
                    appearance.theme().surface_1(),
                ))
                .finish();

                let right_side = Flex::row()
                    .with_child(credits_component)
                    .with_child(cost_component)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .finish();

                Some(
                    Container::new(
                        Flex::row()
                            .with_child(label)
                            .with_child(right_side)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                            .with_main_axis_size(MainAxisSize::Max)
                            .finish(),
                    )
                    .with_margin_bottom(4.)
                    .finish(),
                )
            }
        } else {
            None
        };

        let selected_option = addon_credits_options.get(selected_topup_denomination);

        let auto_reload_enabled = workspace
            .settings
            .addon_credits_settings
            .auto_reload_enabled;

        let auto_reload_amount = selected_option
            .map(|option| option.credits.to_string())
            .filter(|_| auto_reload_enabled)
            .unwrap_or("your selected".to_string());
        let auto_reload_switch = ui_builder
            .switch(self.auto_reload_switch.clone())
            .check(auto_reload_enabled);
        let auto_reload_switch = if delinquent_due_to_payment_issue {
            auto_reload_switch.disable().build().finish()
        } else {
            auto_reload_switch
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(BillingAndUsagePageAction::UpdateAutoReloadEnabled {
                        team_uid,
                        enabled: !auto_reload_enabled,
                    });
                })
                .finish()
        };

        let auto_reload_switch = Container::new(render_body_item::<BillingAndUsagePageAction>(
            "Auto reload".into(),
            None,
            Default::default(),
            Default::default(),
            appearance,
            auto_reload_switch,
            Some(format!(
                "When enabled, auto reload will automatically purchase {auto_reload_amount} \
                credits when your add-on credit balance reaches 100 credits remaining."
            )),
        ))
        .with_padding_right(-TOGGLE_BUTTON_RIGHT_PADDING)
        .finish();

        let denomination_buttons = addon_credit_denomination_buttons
            .iter()
            .map(|button_handle| ChildView::new(button_handle).finish())
            .collect::<Vec<Box<dyn Element>>>();
        let denominations = Wrap::row()
            .with_children(denomination_buttons)
            .with_spacing(8.)
            .finish();

        let mut card_content_upper = Flex::column()
            .with_children([card_header, paragraph, monthly_spend_row])
            .with_spacing(8.);

        if let Some(purchased_row) = purchased_this_month_row {
            card_content_upper.add_child(purchased_row);
        }
        card_content_upper.add_child(auto_reload_switch);

        let base_rate = addon_credits_options
            .first()
            .map_or(0., |option| option.rate());

        let (rendered_price, discount_badge) = match selected_option {
            Some(option) => {
                let price_dollars = option.price_usd_cents as f64 / 100.0;
                let rendered_price = Container::new(
                    Text::new_inline(
                        format!("${price_dollars:.2}"),
                        appearance.ui_font_family(),
                        16.,
                    )
                    .with_color(fg.into())
                    .finish(),
                )
                .with_margin_right(16.)
                .finish();

                let discount_percent = if base_rate > 0.0 {
                    let actual_rate = option.rate();
                    ((base_rate - actual_rate) / base_rate * 100.0).round() as u32
                } else {
                    0
                };

                let discount_badge =
                    Container::new(create_discount_badge(discount_percent, appearance))
                        .with_margin_right(8.)
                        .finish();
                (rendered_price, discount_badge)
            }
            None => (Empty::new().finish(), Empty::new().finish()),
        };

        let button_text = if purchase_addon_credits_loading {
            "Buying…".to_string()
        } else {
            "Buy".to_string()
        };

        let would_exceed_limit = selected_option.is_some_and(|option| {
            let purchase_cost_cents = option.price_usd_cents;
            let monthly_limit_cents = workspace
                .settings
                .addon_credits_settings
                .max_monthly_spend_cents
                .unwrap_or(20000); // Default $200 limit

            let already_spent_cents = workspace.bonus_grants_purchased_this_month.cents_spent;

            (already_spent_cents + purchase_cost_cents) > monthly_limit_cents
        });

        let is_buy_button_disabled =
            purchase_addon_credits_loading || would_exceed_limit || delinquent_due_to_payment_issue;

        let button_font_color = is_buy_button_disabled.then_some(
            appearance
                .theme()
                .disabled_text_color(appearance.theme().surface_3())
                .into(),
        );
        let button_bg_color =
            is_buy_button_disabled.then_some(appearance.theme().surface_3().into());
        let button_border = is_buy_button_disabled.then_some(ColorU::transparent_black().into());
        let mut buy_button = ui_builder
            .button(ButtonVariant::Accent, self.buy_button.clone())
            .with_text_label(button_text)
            .with_style(UiComponentStyles {
                font_size: Some(14.),
                font_weight: Some(Weight::Semibold),
                font_color: button_font_color,
                background: button_bg_color,
                border_color: button_border,
                ..Default::default()
            })
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(BillingAndUsagePageAction::PurchaseAddonCredits {
                    team_uid,
                });
            });

        if is_buy_button_disabled {
            buy_button = buy_button.disable();
        }

        let buy_button = buy_button.finish();

        let mut buy_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_children([
                Shrinkable::new(1., denominations).finish(),
                Flex::row()
                    .with_children([discount_badge, rendered_price])
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .finish(),
            ]);

        if auto_reload_enabled {
            card_content_upper.add_child(buy_row.finish());
            if delinquent_due_to_payment_issue {
                card_content_upper.add_child(self.render_warning_row(
                    appearance,
                    AUTO_RELOAD_DELINQUENT_WARNING_STRING.to_string(),
                ));
            } else if would_exceed_limit {
                card_content_upper.add_child(self.render_warning_row(
                    appearance,
                    AUTO_RELOAD_EXCEED_LIMIT_WARNING_STRING.to_string(),
                ));
            }
            let card_upper = Container::new(card_content_upper.finish())
                .with_uniform_padding(16.)
                .finish();
            Container::new(card_upper)
                .with_background_color(theme.surface_1().into_solid())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .with_margin_bottom(16.)
                .finish()
        } else {
            buy_row.add_child(buy_button);
            let card_upper = Container::new(card_content_upper.finish())
                .with_horizontal_padding(16.)
                .with_padding_top(16.)
                .finish();

            let mut card_content_lower_children = vec![
                ui_builder.span("One-time purchase").build().finish(),
                buy_row.finish(),
            ];

            if delinquent_due_to_payment_issue {
                card_content_lower_children.push(self.render_warning_row(
                    appearance,
                    AUTO_RELOAD_DELINQUENT_WARNING_STRING.to_string(),
                ));
            } else if workspace
                .billing_metadata
                .has_failed_addon_credit_auto_reload_status()
            {
                card_content_lower_children.push(self.render_warning_row(
                    appearance,
                    RESTRICTED_BILLING_USAGE_WARNING_STRING.to_string(),
                ));
            } else if would_exceed_limit {
                let warning_fragments = vec![
                    FormattedTextFragment::plain_text(
                        "Reloading would exceed your monthly limit. ",
                    ),
                    FormattedTextFragment::hyperlink_action(
                        "Increase your limit",
                        BillingAndUsagePageAction::ShowAddOnCreditModal,
                    ),
                    FormattedTextFragment::plain_text(" to continue."),
                ];
                card_content_lower_children
                    .push(self.render_warning_row_with_link(appearance, warning_fragments));
            }

            let card_content_lower = Flex::column()
                .with_children(card_content_lower_children)
                .with_spacing(8.)
                .finish();

            let card_lower = Container::new(card_content_lower)
                .with_uniform_padding(16.)
                .with_border(Border::top(1.).with_border_color(theme.outline().into()))
                .finish();

            let card_content = Flex::column()
                .with_children([card_upper, card_lower])
                .finish();

            Container::new(card_content)
                .with_background_color(theme.surface_1().into_solid())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .with_margin_bottom(16.)
                .finish()
        }
    }

    fn render_total_overages_row(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let billing_metadata = UserWorkspaces::as_ref(app)
            .current_team()
            .map(|team| team.billing_metadata.clone())
            .unwrap_or_default();
        let ai_overages = billing_metadata.ai_overages.as_ref();
        let is_period_over_now = ai_overages
            .map(|overages| overages.current_period_end < chrono::Utc::now())
            .unwrap_or(false);

        let (total_overages_count, total_overages_cost, total_overages_period_end) =
            if is_period_over_now {
                (Some(0), Some(0), None)
            } else {
                (
                    ai_overages.map(|o| o.current_monthly_requests_used),
                    ai_overages.map(|o| o.current_monthly_request_cost_cents),
                    ai_overages.map(|overages| overages.current_period_end),
                )
            };

        let (request_count_label, cost_label) =
            if let (Some(count), Some(cost)) = (total_overages_count, total_overages_cost) {
                if count == 1 {
                    (
                        "1 credit".to_string(),
                        format!("${:.2}", cost as f64 / 100.0),
                    )
                } else {
                    (
                        format!("{} credits", count.separate_with_commas()),
                        format!("${:.2}", cost as f64 / 100.0),
                    )
                }
            } else {
                ("0 credits".to_string(), "$0.00".to_string())
            };

        let mut left_side_component =
            Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        let label = Text::new_inline("Total overages", appearance.ui_font_family(), 12.)
            .with_color(appearance.theme().active_ui_text_color().into())
            .finish();

        left_side_component.add_child(Container::new(label).with_margin_right(8.).finish());

        let request_count_component = Container::new(
            Text::new_inline(request_count_label, appearance.ui_font_family(), 12.)
                .with_color(blended_colors::text_disabled(
                    appearance.theme(),
                    appearance.theme().surface_1(),
                ))
                .finish(),
        )
        .with_margin_right(8.)
        .finish();

        let cost_component = Text::new_inline(cost_label, appearance.ui_font_family(), 12.)
            .with_color(blended_colors::text_sub(
                appearance.theme(),
                appearance.theme().surface_1(),
            ))
            .finish();

        if let Some(period_end) = total_overages_period_end {
            let local_period_end = period_end.with_timezone(&Local);
            let formatted_date = local_period_end.format("%b %d at %-I:%M %p").to_string();
            let billing_date_text = format!("Usage resets on {formatted_date}");
            left_side_component.add_child(
                Container::new(
                    Text::new_inline(billing_date_text, appearance.ui_font_family(), 12.)
                        .with_color(blended_colors::text_disabled(
                            appearance.theme(),
                            appearance.theme().surface_1(),
                        ))
                        .finish(),
                )
                .with_margin_right(8.)
                .finish(),
            );
        };

        let right_side_components = Flex::row()
            .with_child(request_count_component)
            .with_child(cost_component)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        Container::new(
            Flex::row()
                .with_child(left_side_component.finish())
                .with_child(right_side_components.finish())
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_main_axis_size(MainAxisSize::Max)
                .finish(),
        )
        .with_margin_top(16.)
        .finish()
    }

    fn render_request_usage_count(
        &self,
        used: usize,
        divisor: Option<Divisor>,
        workspace_is_delinquent_due_to_payment_issue: bool,
        appearance: &Appearance,
        prorated_request_limits_info: Option<ProratedRequestLimitsInfo>,
    ) -> Box<dyn warpui::Element> {
        let mut row = Flex::row();

        let show_alert = workspace_is_delinquent_due_to_payment_issue
            || matches!(divisor, Some(Divisor::Limit(limit)) if used >= limit);

        if let Some(info) = prorated_request_limits_info {
            if info.is_request_limit_prorated {
                row.add_child(render_info_icon(
                appearance,
                AdditionalInfo::<BillingAndUsagePageAction> {
                    mouse_state: info.mouse_state,
                    on_click_action: None,
                    secondary_text: None,
                    tooltip_override_text: match info.is_current_user {
                        true => Some("Your credit limit is prorated because you joined midway through the billing cycle.".to_string()),
                        false => Some("This credit limit is prorated because this user joined midway through the billing cycle.".to_string()),
                    },
                },
            ))
            }
        }

        if show_alert {
            row.add_child(
                ConstrainedBox::new(
                    Icon::AlertTriangle
                        .to_warpui_icon(appearance.theme().ui_error_color().into())
                        .finish(),
                )
                .with_height(16.)
                .with_width(16.)
                .finish(),
            )
        }

        let request_count_label = if workspace_is_delinquent_due_to_payment_issue {
            "Restricted due to billing issue".to_string()
        } else {
            match divisor {
                Some(Divisor::Unlimited) => {
                    format!("{}/Unlimited", used.separate_with_commas())
                }
                Some(Divisor::Limit(limit)) => format!(
                    "{}/{}",
                    used.separate_with_commas(),
                    limit.separate_with_commas()
                ),
                None => used.separate_with_commas(),
            }
        };

        row.add_child(
            appearance
                .ui_builder()
                .paragraph(request_count_label)
                .with_style(UiComponentStyles {
                    font_color: {
                        if show_alert {
                            Some(appearance.theme().ui_error_color())
                        } else {
                            Some(blended_colors::text_sub(
                                appearance.theme(),
                                appearance.theme().surface_1(),
                            ))
                        }
                    },
                    font_size: Some(14.),
                    margin: Some(Coords {
                        top: 0.,
                        bottom: 0.,
                        left: 8.,
                        right: 0.,
                    }),
                    ..Default::default()
                })
                .build()
                .finish(),
        );

        row.finish()
    }

    /// Renders a row of what is being limited, along with the current used/limit.
    #[allow(clippy::too_many_arguments)]
    fn render_ai_usage_limit_row(
        &self,
        name: String,
        used: usize,
        divisor: Option<Divisor>,
        refresh_duration: String,
        workspace_is_delinquent_due_to_payment_issue: bool,
        appearance: &Appearance,
        prorated_request_limits_info: Option<ProratedRequestLimitsInfo>,
    ) -> Box<dyn warpui::Element> {
        let request_usage_details = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::End)
            .with_child(self.render_request_usage_count(
                used,
                divisor,
                workspace_is_delinquent_due_to_payment_issue,
                appearance,
                prorated_request_limits_info,
            ));

        let left_side = if !name.is_empty() {
            Shrinkable::new(
                2.,
                Container::new(
                    Text::new_inline(name, appearance.ui_font_family(), 14.)
                        .with_style(Properties::default())
                        .with_color(
                            appearance
                                .theme()
                                .main_text_color(appearance.theme().surface_2())
                                .into(),
                        )
                        .finish(),
                )
                .with_margin_bottom(20.)
                .finish(),
            )
            .finish()
        } else {
            let header = "Credits";
            let description =
                format!("This is the {refresh_duration} limit of AI credits for your account.");

            let request_usage_description = FormattedTextElement::from_str(
                description,
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(blended_colors::text_sub(
                appearance.theme(),
                appearance.theme().surface_1(),
            ));

            Shrinkable::new(
                2.,
                Container::new(
                    Flex::column()
                        .with_child(
                            appearance
                                .ui_builder()
                                .paragraph(header)
                                .with_style(UiComponentStyles {
                                    font_color: Some(blended_colors::text_main(
                                        appearance.theme(),
                                        appearance.theme().surface_1(),
                                    )),
                                    margin: Some(Coords {
                                        top: 0.,
                                        bottom: 4.,
                                        left: 0.,
                                        right: 0.,
                                    }),
                                    ..Default::default()
                                })
                                .build()
                                .finish(),
                        )
                        .with_child(request_usage_description.finish())
                        .finish(),
                )
                .with_margin_bottom(16.)
                .finish(),
            )
            .finish()
        };

        Flex::row()
            .with_child(left_side)
            .with_child(
                Shrinkable::new(
                    1.,
                    Container::new(request_usage_details.finish())
                        .with_margin_bottom(16.)
                        .finish(),
                )
                .finish(),
            )
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .finish()
    }
}

impl SettingsWidget for UsageWidget {
    type View = BillingAndUsagePageView;

    fn search_terms(&self) -> &str {
        "a.i. ai usage limit plan"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_request_usage_model = AIRequestUsageModel::as_ref(app);
        let formatted_next_refresh_time = ai_request_usage_model
            .next_refresh_time_local()
            .format("%b %d at %-I:%M %p")
            .to_string();
        let workspace_is_delinquent_due_to_payment_issue = UserWorkspaces::as_ref(app)
            .current_team()
            .map(|team| team.billing_metadata.is_delinquent_due_to_payment_issue())
            .unwrap_or_default();

        let mut usage = Flex::column();

        let tabs = vec![
            SettingsTab::new(
                BillingUsageTab::Overview.label(),
                self.overview_tab_mouse_state.clone(),
            ),
            SettingsTab::new(
                BillingUsageTab::UsageHistory.label(),
                self.usage_history_tab_mouse_state.clone(),
            ),
        ];

        let tab_selector = tab_selector::render_tab_selector(
            tabs,
            view.selected_tab.label(),
            // On click, set clicked tab as selected
            |label, ctx| {
                ctx.dispatch_typed_action(BillingAndUsagePageAction::SelectTab(
                    BillingUsageTab::get_tab_from_label(label),
                ));
            },
            appearance,
        );
        usage.add_child(tab_selector);

        // Render correct page based on selected tab
        if view.selected_tab == BillingUsageTab::Overview {
            let usage_content = self.render_usage_content(
                view,
                appearance,
                app,
                ai_request_usage_model,
                &formatted_next_refresh_time,
                workspace_is_delinquent_due_to_payment_issue,
                &view.prorated_request_limits_info_mouse_states,
            );
            usage.add_child(usage_content);
        } else {
            usage.add_child(self.render_usage_history_content(view, appearance, app));
        }

        usage.finish()
    }
}

impl UsageWidget {
    fn render_usage_history_content(
        &self,
        view: &BillingAndUsagePageView,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let usage_history = view.usage_history_model.as_ref(app);
        if usage_history.entries().is_empty() {
            return self.render_empty_usage_history_content(
                usage_history.is_loading(),
                appearance,
                app,
            );
        }

        let mut content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_child(
                Container::new(
                    Text::new_inline("Last 30 days".to_string(), appearance.ui_font_family(), 14.)
                        .with_color(blended_colors::text_sub(
                            appearance.theme(),
                            appearance.theme().surface_1(),
                        ))
                        .finish(),
                )
                .with_vertical_margin(12.)
                .finish(),
            );

        let mut usage_history_list = Flex::column().with_spacing(8.);
        let entries = usage_history.entries();
        for entry in entries.iter() {
            let is_expanded = view
                .expanded_usage_entries
                .get(&entry.conversation_id)
                .copied()
                .unwrap_or(false);

            let mouse_state = view
                .usage_entries_mouse_states
                .borrow_mut()
                .entry(entry.conversation_id.clone())
                .or_default()
                .clone();

            let tooltip_mouse_state = view
                .usage_entries_tooltip_mouse_states
                .borrow_mut()
                .entry(entry.conversation_id.clone())
                .or_default()
                .clone();

            usage_history_list.add_child(
                Container::new(
                    UsageHistoryEntry::new(
                        Some(entry.clone()),
                        is_expanded,
                        Some(mouse_state),
                        tooltip_mouse_state,
                    )
                    .render(appearance, app),
                )
                .finish(),
            );
        }
        content.add_child(usage_history_list.finish());

        if usage_history.has_more_entries() {
            let load_more = view.load_more_button.as_ref(app).render(app);
            content.add_child(
                Container::new(
                    Flex::row()
                        .with_child(load_more)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_main_axis_size(MainAxisSize::Max)
                        .finish(),
                )
                .with_margin_top(24.)
                .finish(),
            );
        }

        Container::new(content.finish()).finish()
    }

    /// Renders default views to show when there is no usage history,
    /// either because the history is loading or because there is no history.
    fn render_empty_usage_history_content(
        &self,
        is_loading: bool,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_alignment(MainAxisAlignment::Center);

        if is_loading {
            let mut loading_usage_entry_list = Flex::column().with_spacing(8.);
            for _ in 0..3 {
                loading_usage_entry_list.add_child(
                    UsageHistoryEntry::new(None, false, None, MouseStateHandle::default())
                        .render(appearance, app),
                );
            }
            content.add_child(loading_usage_entry_list.finish());
        } else {
            let res = Flex::column()
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
                Container::new(res.finish())
                    .with_vertical_margin(160.)
                    .finish(),
            );
        }

        content.finish()
    }

    /// Renders usage reporting callout card for Enterprise users indicating that usage reporting
    /// is currently limited. Admins see a link to the admin panel; non-admins see a message to
    /// contact their admin.
    pub fn render_enterprise_usage_card(
        &self,
        team_uid: ServerId,
        has_admin_permissions: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let bg = theme.background();

        let icon = Container::new(
            ConstrainedBox::new(
                Icon::AlertCircle
                    .to_warpui_icon(blended_colors::text_sub(theme, theme.surface_1()).into())
                    .finish(),
            )
            .with_width(16.)
            .with_height(16.)
            .finish(),
        )
        .with_margin_right(8.)
        .finish();

        let header = Text::new_inline(
            ENTERPRISE_USAGE_CALLOUT_HEADER,
            appearance.ui_font_family(),
            16.,
        )
        .with_color(theme.foreground().into())
        .with_style(Properties::default().weight(Weight::Bold))
        .finish();

        let card_header = Flex::row()
            .with_child(icon)
            .with_child(Shrinkable::new(1., Align::new(header).left().finish()).finish())
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish();

        // Body text differs for admin vs non-admin
        let body = if has_admin_permissions {
            let admin_panel_url = AdminActions::admin_panel_link_for_team(team_uid);
            let text_fragments = vec![
                FormattedTextFragment::plain_text(ENTERPRISE_USAGE_CALLOUT_BODY_ADMIN_PREFIX),
                FormattedTextFragment::hyperlink(
                    ENTERPRISE_USAGE_CALLOUT_BODY_ADMIN_LINK,
                    admin_panel_url,
                ),
                FormattedTextFragment::plain_text(ENTERPRISE_USAGE_CALLOUT_BODY_ADMIN_SUFFIX),
            ];
            FormattedTextElement::new(
                FormattedText::new([FormattedTextLine::Line(text_fragments)]),
                12.,
                appearance.ui_font_family(),
                appearance.ui_font_family(),
                theme.sub_text_color(bg).into(),
                HighlightedHyperlink::default(),
            )
            .with_hyperlink_font_color(theme.accent().into_solid())
            .register_default_click_handlers(|hyperlink, _event_ctx, app_ctx| {
                app_ctx.open_url(&hyperlink.url);
            })
            .finish()
        } else {
            appearance
                .ui_builder()
                .paragraph(ENTERPRISE_USAGE_CALLOUT_BODY_NON_ADMIN)
                .with_style(UiComponentStyles {
                    font_color: Some(theme.sub_text_color(bg).into()),
                    font_size: Some(12.),
                    ..Default::default()
                })
                .build()
                .finish()
        };

        let card_content = Flex::column()
            .with_children([
                Container::new(card_header).with_margin_bottom(8.).finish(),
                body,
            ])
            .finish();

        let card_container = Container::new(card_content)
            .with_background_color(theme.surface_1().into_solid())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_uniform_padding(16.)
            .with_margin_bottom(16.)
            .finish();

        Flex::column().with_child(card_container).finish()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_usage_content(
        &self,
        view: &BillingAndUsagePageView,
        appearance: &Appearance,
        app: &AppContext,
        ai_request_usage_model: &AIRequestUsageModel,
        formatted_next_refresh_time: &str,
        workspace_is_delinquent_due_to_payment_issue: bool,
        prorated_request_limits_info_mouse_states: &[MouseStateHandle],
    ) -> Box<dyn Element> {
        let mut usage = Flex::column();

        let workspace = UserWorkspaces::as_ref(app).current_workspace();
        // Check if we should show the sort button (admin with team size > 1)
        let workspace_team_members = workspace
            .map(|workspace| workspace.members.clone())
            .unwrap_or_default();
        let current_user_email = AuthStateProvider::as_ref(app)
            .get()
            .user_email()
            .unwrap_or_default();
        let team = UserWorkspaces::as_ref(app).current_team();
        let has_admin_permissions =
            team.is_some_and(|team| team.has_admin_permissions(&current_user_email));

        let mut usage_header_right_side = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                appearance
                    .ui_builder()
                    .paragraph(format!("Resets {formatted_next_refresh_time}"))
                    .with_style(UiComponentStyles {
                        font_color: Some(blended_colors::text_sub(
                            appearance.theme(),
                            appearance.theme().surface_1(),
                        )),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_child(
                Container::new(
                    icon_button(
                        appearance,
                        Icon::Refresh,
                        false,
                        self.refresh_icon_mouse_state.clone(),
                    )
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(BillingAndUsagePageAction::RefreshWorkspaceData);
                    })
                    .finish(),
                )
                .with_margin_left(8.)
                .finish(),
            );

        // only show sort button if user is admin and has more than 1 member
        if has_admin_permissions && workspace_team_members.len() > 1 {
            usage_header_right_side.add_child(
                Container::new({
                    let mut button = icon_button_with_context_menu(
                        Icon::Sort,
                        move |ctx, _, _| {
                            ctx.dispatch_typed_action(BillingAndUsagePageAction::ToggleSortingMenu)
                        },
                        self.sort_icon_mouse_state.clone(),
                        &view.sorting_menu,
                        view.sorting_menu_open,
                        MenuDirection::Right,
                        Some(Cursor::PointingHand),
                        None,
                        appearance,
                    );

                    let hoverable =
                        Hoverable::new(self.sort_icon_mouse_state.clone(), |mouse_state| {
                            if mouse_state.is_hovered() {
                                let tooltip =
                                    appearance.ui_builder().tool_tip("Sort by".to_string());

                                button.add_positioned_overlay_child(
                                    tooltip.build().finish(),
                                    OffsetPositioning::offset_from_parent(
                                        vec2f(0., 4.),
                                        ParentOffsetBounds::Unbounded,
                                        ParentAnchor::BottomMiddle,
                                        ChildAnchor::TopMiddle,
                                    ),
                                );
                            }
                            button.finish()
                        });

                    hoverable.finish()
                })
                .with_margin_left(8.)
                .finish(),
            );
        }

        // Render the ambient agent trial widget if the user has ambient-only credits.
        if let Some(ambient_trial_widget) =
            self.render_ambient_agent_trial_widget(ai_request_usage_model, appearance, app)
        {
            usage.add_child(ambient_trial_widget);
        }

        if let (Some(workspace), Some(team)) = (workspace, team) {
            let bonus_credit_balance =
                ai_request_usage_model.total_workspace_bonus_credits_remaining(workspace.uid);

            // Hide addon credits panel for Enterprise PAYG users when they have 0 credits.
            let is_enterprise_payg_with_zero_credits = workspace
                .billing_metadata
                .is_enterprise_pay_as_you_go_enabled()
                && bonus_credit_balance == 0;

            if !is_enterprise_payg_with_zero_credits {
                usage.add_child(self.render_addon_credits_panel(
                    view.selected_addon_denomination,
                    workspace,
                    team.uid,
                    has_admin_permissions,
                    bonus_credit_balance,
                    &view.addon_credits_options,
                    &view.addon_credit_denomination_buttons,
                    view.purchase_addon_credits_loading,
                    workspace_is_delinquent_due_to_payment_issue,
                    app,
                ));
            }
        }

        let usage_header = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    build_sub_header(
                        appearance,
                        "Usage",
                        Some(
                            appearance
                                .theme()
                                .main_text_color(appearance.theme().surface_2()),
                        ),
                    )
                    .finish(),
                )
                .with_child(usage_header_right_side.finish())
                .finish(),
        )
        .with_padding_bottom(HEADER_PADDING)
        .finish();

        usage.add_child(usage_header);

        // For enterprise plan users with base limit = 0, show a limited usage reporting callout
        // as this is not applicable to them
        if let Some(t) = team {
            if t.billing_metadata.customer_type == CustomerType::Enterprise
                && t.billing_metadata
                    .tier
                    .warp_ai_policy
                    .is_some_and(|p| p.limit == 0)
            {
                usage.add_child(self.render_enterprise_usage_card(
                    t.uid,
                    has_admin_permissions,
                    appearance,
                ));
                return usage.finish();
            }
        }

        // Show a summed "Team total" row first.
        let num_team_members = workspace_team_members.len();
        let should_show_team_total = num_team_members > 1 && has_admin_permissions;
        if should_show_team_total {
            let team_total_used: usize = workspace_team_members
                .iter()
                .map(|m| m.usage_info.requests_used_since_last_refresh as usize)
                .sum();
            let is_unlimited = ai_request_usage_model.is_unlimited();

            let team_divisor = if is_unlimited {
                Some(Divisor::Unlimited)
            } else {
                None
            };

            usage.add_child(self.render_ai_usage_limit_row(
                "Team total".to_string(),
                team_total_used,
                team_divisor,
                ai_request_usage_model.refresh_duration_to_string(),
                workspace_is_delinquent_due_to_payment_issue,
                appearance,
                None,
            ));
            let divider = Container::new(
                ConstrainedBox::new(Empty::new().finish())
                    .with_height(1.)
                    .finish(),
            )
            .with_background_color(appearance.theme().outline().into_solid())
            .with_margin_bottom(16.)
            .finish();

            usage.add_child(divider);
        }

        // Build UserSortingCriteria with (display_name, requests_used, rendered_row)
        // Note: display_name already has email as fallback
        let mut user_information = if !has_admin_permissions {
            vec![]
        } else {
            workspace_team_members
                .iter()
                .enumerate()
                .map(|(i, member)| {
                    // Compute effective display name (fallback to email if missing or empty)
                    let display_name = UserProfiles::as_ref(app)
                        .profile_for_uid(member.uid)
                        .and_then(|profile| profile.display_name.clone())
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| member.email.clone());

                    let requests_used = member.usage_info.requests_used_since_last_refresh as usize;

                    let row = self.render_ai_usage_limit_row(
                        display_name.clone(),
                        requests_used,
                        if member.usage_info.is_unlimited {
                            Some(Divisor::Unlimited)
                        } else {
                            Some(Divisor::Limit(member.usage_info.request_limit as usize))
                        },
                        ai_request_usage_model.refresh_duration_to_string(),
                        workspace_is_delinquent_due_to_payment_issue,
                        appearance,
                        Some(ProratedRequestLimitsInfo {
                            is_request_limit_prorated: member.usage_info.is_request_limit_prorated,
                            mouse_state: prorated_request_limits_info_mouse_states[i].clone(),
                            is_current_user: member.email == current_user_email,
                        }),
                    );

                    UserSortingCriteria::new(display_name, requests_used, row)
                })
                .collect_vec()
        };

        if user_information.is_empty() {
            let display_name = AuthStateProvider::as_ref(app)
                .get()
                .display_name()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| current_user_email.clone());
            let user_workspace_member = workspace_team_members
                .iter()
                .find(|m| m.email == current_user_email);

            let row = self.render_ai_usage_limit_row(
                display_name.clone(),
                ai_request_usage_model.requests_used(),
                if ai_request_usage_model.is_unlimited() {
                    Some(Divisor::Unlimited)
                } else {
                    Some(Divisor::Limit(ai_request_usage_model.request_limit()))
                },
                ai_request_usage_model.refresh_duration_to_string(),
                workspace_is_delinquent_due_to_payment_issue,
                appearance,
                user_workspace_member.map(|member| ProratedRequestLimitsInfo {
                    is_request_limit_prorated: member.usage_info.is_request_limit_prorated,
                    mouse_state: prorated_request_limits_info_mouse_states[0].clone(), // We know the workspace has at least one member, so just take the first mouse state handle since we don't use the others.
                    is_current_user: true,
                }),
            );
            user_information.push(UserSortingCriteria::new(
                display_name,
                ai_request_usage_model.requests_used(),
                row,
            ));
        }

        // Apply sort according to current_sort_key/current_sort_order via shared helper
        // Get current user's display name (with email fallback) for pinning
        let current_user_display_name = AuthStateProvider::as_ref(app)
            .get()
            .display_name()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| current_user_email.clone());

        // TODO: move sorting once per initial load or sort option change https://github.com/warpdotdev/warp-internal/pull/18288/files#r2392139761
        sort_user_items_in_place(
            &mut user_information,
            &current_user_display_name,
            view.current_sort_key,
            view.current_sort_order,
        );

        let user_information = user_information
            .into_iter()
            .map(|item| item.data)
            .collect_vec();

        usage.extend(user_information);

        let auth_state = AuthStateProvider::as_ref(app).get();

        let upgrade_cta_text_fragments = if let Some(team) =
            UserWorkspaces::as_ref(app).current_team()
        {
            if workspace_is_delinquent_due_to_payment_issue {
                if has_admin_permissions {
                    vec![
                        FormattedTextFragment::hyperlink_action(
                            "Manage billing",
                            BillingAndUsagePageAction::GenerateStripeBillingPortalLink {
                                team_uid: team.uid,
                            },
                        ),
                        FormattedTextFragment::plain_text(" to regain access to AI features."),
                    ]
                } else {
                    // Non-admin team member - show message to contact admin
                    vec![FormattedTextFragment::plain_text(
                        "Contact your team admin to resolve billing issues.",
                    )]
                }
            } else if team.billing_metadata.can_upgrade_to_higher_tier_plan() {
                let upgrade_url = UserWorkspaces::upgrade_link_for_team(team.uid);
                if has_admin_permissions {
                    if team.billing_metadata.can_upgrade_to_build_plan() {
                        if team.billing_metadata.is_on_legacy_paid_plan() {
                            vec![
                                FormattedTextFragment::hyperlink(
                                    "Switch to the Build plan",
                                    upgrade_url,
                                ),
                                FormattedTextFragment::plain_text(
                                    " for a more flexible pricing model.",
                                ),
                            ]
                        } else {
                            let mut fragments = vec![FormattedTextFragment::hyperlink(
                                "Upgrade to the Build plan",
                                upgrade_url,
                            )];
                            if team.billing_metadata.is_byo_api_key_enabled() {
                                fragments.push(FormattedTextFragment::plain_text(" or "));
                                fragments.push(FormattedTextFragment::hyperlink_action(
                                    "bring your own key",
                                    BillingAndUsagePageAction::NavigateToByokSettings,
                                ));
                            }
                            fragments.push(FormattedTextFragment::plain_text(
                                " for increased access to AI features.",
                            ));
                            fragments
                        }
                    } else {
                        let upgrade_text = match team.billing_metadata.customer_type {
                            CustomerType::Prosumer => "Upgrade to Turbo plan",
                            CustomerType::Turbo => "Upgrade to Lightspeed plan",
                            _ => "Upgrade",
                        };
                        vec![
                            FormattedTextFragment::hyperlink(upgrade_text, upgrade_url),
                            FormattedTextFragment::plain_text(" to get more AI usage."),
                        ]
                    }
                } else {
                    vec![]
                }
            } else if team.billing_metadata.is_on_build_plan() {
                vec![
                    FormattedTextFragment::hyperlink(
                        "Upgrade to Max",
                        UserWorkspaces::upgrade_link_for_team(team.uid),
                    ),
                    FormattedTextFragment::plain_text(" for more AI credits."),
                ]
            } else if team.billing_metadata.is_on_build_max_plan() {
                vec![
                    FormattedTextFragment::hyperlink(
                        "Switch to Business",
                        UserWorkspaces::upgrade_link_for_team(team.uid),
                    ),
                    FormattedTextFragment::plain_text(
                        " for security features like SSO and automatically applied zero data retention.",
                    ),
                ]
            } else if team.billing_metadata.is_on_build_business_plan() {
                vec![
                    FormattedTextFragment::hyperlink(
                        "Upgrade to Enterprise",
                        "mailto:sales@warp.dev",
                    ),
                    FormattedTextFragment::plain_text(" for custom limits and dedicated support."),
                ]
            } else if !team.billing_metadata.is_usage_based_pricing_toggleable() {
                vec![
                    FormattedTextFragment::hyperlink("Contact support", "mailto:support@warp.dev"),
                    FormattedTextFragment::plain_text(" for more AI usage."),
                ]
            } else {
                vec![]
            }
        } else {
            let user_id = auth_state.user_id().unwrap_or_default();
            let upgrade_url = UserWorkspaces::upgrade_link(user_id);
            let mut fragments = vec![FormattedTextFragment::hyperlink(
                "Upgrade to the Build plan",
                upgrade_url,
            )];
            if UserWorkspaces::as_ref(app).is_byo_api_key_enabled(app) {
                fragments.push(FormattedTextFragment::plain_text(" or "));
                fragments.push(FormattedTextFragment::hyperlink_action(
                    "bring your own key",
                    BillingAndUsagePageAction::NavigateToByokSettings,
                ));
            }
            fragments.push(FormattedTextFragment::plain_text(
                " for more credits and access to more models.",
            ));
            fragments
        };

        let mut upgrade_cta = FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(upgrade_cta_text_fragments)]),
            appearance.ui_font_size(),
            appearance.ui_font_family(),
            appearance.ui_font_family(),
            blended_colors::text_sub(appearance.theme(), appearance.theme().surface_1()),
            self.requests_highlight_index.clone(),
        )
        .with_hyperlink_font_color(appearance.theme().accent().into_solid());

        if AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out()
        {
            upgrade_cta = upgrade_cta.register_default_click_handlers(|_, ctx, _| {
                ctx.dispatch_typed_action(BillingAndUsagePageAction::AttemptLoginGatedUpgrade);
            });
        } else {
            upgrade_cta = upgrade_cta.register_default_click_handlers_with_action_support(
                |hyperlink_lens, event, ctx| match hyperlink_lens {
                    warpui::elements::HyperlinkLens::Url(url) => {
                        ctx.open_url(url);
                    }
                    warpui::elements::HyperlinkLens::Action(action_ref) => {
                        if let Some(action) = action_ref
                            .as_any()
                            .downcast_ref::<BillingAndUsagePageAction>()
                        {
                            event.dispatch_typed_action(action.clone());
                        }
                    }
                },
            );
        };

        usage.add_child(
            Container::new(upgrade_cta.finish())
                .with_margin_bottom(16.)
                .finish(),
        );

        let workspaces = UserWorkspaces::as_ref(app);
        if let Some(team) = workspaces.current_team() {
            if team.billing_metadata.is_usage_based_pricing_toggleable() {
                let usage_based_pricing_settings = workspaces.usage_based_pricing_settings();

                let enabled = view
                    .usage_based_pricing_toggle_override
                    .unwrap_or(usage_based_pricing_settings.enabled);

                usage.add_child(
                    Container::new(self.render_usage_based_pricing_section(
                        enabled,
                        team,
                        appearance,
                        app,
                        has_admin_permissions,
                        view.usage_based_pricing_toggle_loading,
                    ))
                    .with_margin_bottom(16.)
                    .finish(),
                );
            }
        }

        usage.finish()
    }
}

/// Sorts items in-place. The current user is always pinned first.
/// For display name sort, comparison is case-insensitive on display name.
/// For requests sort, ties are broken by display name ascending.
pub(crate) fn sort_user_items_in_place<T>(
    items: &mut [UserSortingCriteria<T>],
    display_name: &str,
    key: Option<SortKey>,
    order: SortOrder,
) {
    let pin_current_user =
        |a: &UserSortingCriteria<T>, b: &UserSortingCriteria<T>| -> Option<std::cmp::Ordering> {
            if a.display_name == display_name {
                Some(std::cmp::Ordering::Less)
            } else if b.display_name == display_name {
                Some(std::cmp::Ordering::Greater)
            } else {
                None
            }
        };

    let compare_by_name = |a: &UserSortingCriteria<T>,
                           b: &UserSortingCriteria<T>,
                           ascending: bool|
     -> std::cmp::Ordering {
        let a_key = a.display_name.to_lowercase();
        let b_key = b.display_name.to_lowercase();
        if ascending {
            a_key.cmp(&b_key)
        } else {
            b_key.cmp(&a_key)
        }
    };

    items.sort_by(|a, b| {
        if let Some(ordering) = pin_current_user(a, b) {
            return ordering;
        }

        match (key, order) {
            (Some(SortKey::DisplayName), SortOrder::Asc) => compare_by_name(a, b, true),
            (Some(SortKey::DisplayName), SortOrder::Desc) => compare_by_name(a, b, false),
            (Some(SortKey::Requests), SortOrder::Asc) => {
                let primary = a.requests_used.cmp(&b.requests_used);
                if primary == std::cmp::Ordering::Equal {
                    compare_by_name(a, b, true)
                } else {
                    primary
                }
            }
            (Some(SortKey::Requests), SortOrder::Desc) => {
                let primary = b.requests_used.cmp(&a.requests_used);
                if primary == std::cmp::Ordering::Equal {
                    compare_by_name(a, b, true)
                } else {
                    primary
                }
            }
            _ => a.display_name.cmp(&b.display_name),
        }
    });
}

#[derive(Default)]
struct PlanWidgetStateHandles {
    upgrade_link: MouseStateHandle,
    anonymous_user_sign_up_button: MouseStateHandle,
    enterprise_contact_us_link: MouseStateHandle,
    stripe_billing_portal_link: MouseStateHandle,
    admin_panel_link: MouseStateHandle,
}

#[derive(Default)]
struct PlanWidget {
    ui_state_handles: PlanWidgetStateHandles,
}

impl PlanWidget {
    fn render_anonymous_account_info(
        &self,
        auth_state: &AuthState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let button_styles = UiComponentStyles {
            font_size: Some(14.),
            font_weight: Some(Weight::Semibold),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
            padding: Some(Coords {
                top: 12.,
                bottom: 12.,
                left: 40.,
                right: 40.,
            }),
            ..Default::default()
        };

        let user_info = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                self.ui_state_handles.anonymous_user_sign_up_button.clone(),
            )
            .with_style(button_styles)
            .with_text_label("Sign up".to_owned())
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(BillingAndUsagePageAction::SignupAnonymousUser);
            })
            .finish();

        let mut plan_info = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::SpaceEvenly)
            .with_cross_axis_alignment(CrossAxisAlignment::End);
        let current_user_id = auth_state.user_id().unwrap_or_default();

        plan_info.add_child(render_customer_type_badge(appearance, "Free".into()));
        plan_info.add_child(
            Container::new(
                appearance
                    .ui_builder()
                    .button(
                        ButtonVariant::Link,
                        self.ui_state_handles.upgrade_link.clone(),
                    )
                    .with_text_and_icon_label(
                        TextAndIcon::new(
                            TextAndIconAlignment::IconFirst,
                            "Compare plans",
                            Icon::CoinsStacked.to_warpui_icon(appearance.theme().accent()),
                            MainAxisSize::Min,
                            MainAxisAlignment::Center,
                            vec2f(14., 14.),
                        )
                        .with_inner_padding(4.),
                    )
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(BillingAndUsagePageAction::Upgrade {
                            team_uid: None,
                            user_id: current_user_id,
                        });
                    })
                    .finish(),
            )
            .with_margin_top(8.)
            .finish(),
        );

        Flex::row()
            .with_child(
                Shrinkable::new(
                    1.0,
                    Flex::row()
                        .with_child(user_info)
                        .with_main_axis_alignment(MainAxisAlignment::Start)
                        .with_main_axis_size(MainAxisSize::Max)
                        .finish(),
                )
                .finish(),
            )
            .with_child(Align::new(plan_info.finish()).right().finish())
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .finish()
    }

    fn render_plan_header_text(&self, appearance: &Appearance) -> Box<dyn Element> {
        Text::new_inline("Plan", appearance.ui_font_family(), HEADER_FONT_SIZE)
            .with_style(Properties::default().weight(Weight::Bold))
            .with_color(appearance.theme().active_ui_text_color().into())
            .finish()
    }

    fn render_team_admin_actions(
        &self,
        team: &Team,
        _current_user_id: UserUid,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        if team.billing_metadata.customer_type == CustomerType::Enterprise
            || !team.has_billing_history
        {
            return None;
        }

        let team_uid = team.uid;
        let content = Container::new(
            appearance
                .ui_builder()
                .button(
                    ButtonVariant::Link,
                    self.ui_state_handles.enterprise_contact_us_link.clone(),
                )
                .with_text_and_icon_label(
                    TextAndIcon::new(
                        TextAndIconAlignment::IconFirst,
                        "Manage billing",
                        Icon::CoinsStacked.to_warpui_icon(appearance.theme().accent()),
                        MainAxisSize::Min,
                        MainAxisAlignment::Center,
                        vec2f(14., 14.),
                    )
                    .with_inner_padding(4.),
                )
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(
                        BillingAndUsagePageAction::GenerateStripeBillingPortalLink { team_uid },
                    );
                })
                .finish(),
        )
        .with_margin_left(12.)
        .finish();

        Some(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(content)
                .finish(),
        )
    }

    fn render_plan_badge_for_team(
        &self,
        team: &Team,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        if team.billing_metadata.customer_type != CustomerType::Unknown {
            Some(
                Container::new(render_customer_type_badge(
                    appearance,
                    team.billing_metadata.customer_type.to_display_string(),
                ))
                .with_margin_right(12.)
                .finish(),
            )
        } else {
            None
        }
    }

    fn render_admin_panel_button(
        &self,
        team_uid: ServerId,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            appearance
                .ui_builder()
                .button(
                    ButtonVariant::Link,
                    self.ui_state_handles.stripe_billing_portal_link.clone(),
                )
                .with_text_and_icon_label(
                    TextAndIcon::new(
                        TextAndIconAlignment::IconFirst,
                        "Open admin panel",
                        Icon::Users.to_warpui_icon(appearance.theme().accent()),
                        MainAxisSize::Min,
                        MainAxisAlignment::Center,
                        vec2f(14., 14.),
                    )
                    .with_inner_padding(4.),
                )
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(BillingAndUsagePageAction::OpenAdminPanel {
                        team_uid,
                    });
                })
                .finish(),
        )
        .with_margin_left(12.)
        .finish()
    }

    fn render_non_team_user_actions(
        &self,
        auth_state: &AuthState,
        appearance: &Appearance,
    ) -> (Box<dyn Element>, Box<dyn Element>) {
        let current_user_id = auth_state.user_id().unwrap_or_default();

        let plan_badge = render_customer_type_badge(appearance, "Free".into());

        let badge_element = Container::new(plan_badge).with_margin_right(16.).finish();

        let compare_plans_button = Container::new(
            appearance
                .ui_builder()
                .button(
                    ButtonVariant::Link,
                    self.ui_state_handles.admin_panel_link.clone(),
                )
                .with_text_and_icon_label(
                    TextAndIcon::new(
                        TextAndIconAlignment::IconFirst,
                        "Compare plans",
                        Icon::CoinsStacked.to_warpui_icon(appearance.theme().accent()),
                        MainAxisSize::Min,
                        MainAxisAlignment::Center,
                        vec2f(14., 14.),
                    )
                    .with_inner_padding(4.),
                )
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(BillingAndUsagePageAction::Upgrade {
                        team_uid: None,
                        user_id: current_user_id,
                    });
                })
                .finish(),
        )
        .with_margin_left(12.)
        .finish();

        (badge_element, compare_plans_button)
    }

    fn render_account_info(
        &self,
        auth_state: &AuthState,
        app: &AppContext,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut plan_header = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);

        plan_header.add_child(self.render_plan_header_text(appearance));

        let mut right_side = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::End);

        let current_user_id = auth_state.user_id().unwrap_or_default();
        let workspaces = UserWorkspaces::as_ref(app);

        if let Some(team) = workspaces.current_team() {
            if let Some(plan_badge) = self.render_plan_badge_for_team(team, appearance) {
                right_side.add_child(plan_badge);
            }

            let current_user_email = auth_state.user_email().unwrap_or_default();
            let has_admin_permissions = team.has_admin_permissions(&current_user_email);

            if has_admin_permissions {
                if let Some(admin_actions) =
                    self.render_team_admin_actions(team, current_user_id, appearance)
                {
                    right_side.add_child(admin_actions);
                }

                let admin_panel_button = self.render_admin_panel_button(team.uid, appearance);
                right_side.add_child(admin_panel_button);
            }
        } else {
            let (plan_badge, compare_plans_button) =
                self.render_non_team_user_actions(auth_state, appearance);
            right_side.add_child(plan_badge);
            right_side.add_child(compare_plans_button);
        }

        plan_header.add_child(right_side.finish());
        plan_header.finish()
    }
}

impl SettingsWidget for PlanWidget {
    type View = BillingAndUsagePageView;

    fn search_terms(&self) -> &str {
        "plan billing"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let account_info = if view.auth_state.is_anonymous_or_logged_out() {
            self.render_anonymous_account_info(view.auth_state.as_ref(), appearance)
        } else {
            self.render_account_info(view.auth_state.as_ref(), app, appearance)
        };

        let mut col = Flex::column();

        col.add_child(
            Container::new(account_info)
                .with_margin_bottom(HEADER_PADDING)
                .finish(),
        );

        col.finish()
    }
}

#[cfg(test)]
#[path = "billing_and_usage_page_tests.rs"]
mod tests;
