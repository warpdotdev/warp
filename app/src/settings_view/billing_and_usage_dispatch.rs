//! Dispatch wrapper that routes between the legacy and v2 billing & usage
//! pages.
use std::cell::RefCell;
use std::collections::HashMap;

use ::settings::Setting;
use warp_core::features::FeatureFlag;
use warp_core::ui::appearance::Appearance;
use warp_errors::{report_error, report_if_error};
use warpui::elements::{ChildView, Container, Flex, MouseStateHandle, ParentElement};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use super::SettingsSection;
use super::billing_and_usage_page::{BillingAndUsagePageEvent, BillingAndUsagePageView};
use super::billing_and_usage_page_v2::BillingAndUsagePageV2View;
use super::settings_page::{
    LocalOnlyIconState, MatchData, PageType, SettingsPageMeta, SettingsPageViewHandle,
    SettingsWidget, render_dropdown_item,
};
use crate::auth::{AuthManager, AuthStateProvider};
use crate::settings::{AISettings, AISettingsChangedEvent, UsageDisplayUnit};
use crate::view_components::{Dropdown, DropdownItem};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::Workspace;

pub struct BillingAndUsageDispatchView {
    page: PageType<Self>,
    v1: ViewHandle<BillingAndUsagePageView>,
    v2: ViewHandle<BillingAndUsagePageV2View>,
    local_only_icon_tooltip_states: RefCell<HashMap<String, MouseStateHandle>>,
    usage_display_unit_dropdown: ViewHandle<Dropdown<BillingAndUsageDispatchAction>>,
}

impl BillingAndUsageDispatchView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let v1 = ctx.add_typed_action_view(BillingAndUsagePageView::new);
        let v2 = ctx.add_typed_action_view(BillingAndUsagePageV2View::new);
        let usage_display_unit_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            let values = vec![UsageDisplayUnit::Credits, UsageDisplayUnit::Dollars];
            let current_value = AISettings::as_ref(ctx).usage_display_unit;
            let selected_index = values
                .iter()
                .position(|value| *value == current_value)
                .unwrap_or_else(|| {
                    report_error!(
                        "Could not find current UsageDisplayUnit value in dropdown option list"
                    );
                    0
                });

            dropdown.add_items(
                values
                    .into_iter()
                    .map(|value| {
                        DropdownItem::new(
                            value.display_name(),
                            BillingAndUsageDispatchAction::SetUsageDisplayUnit(value),
                        )
                    })
                    .collect(),
                ctx,
            );
            dropdown.set_selected_by_index(selected_index, ctx);
            dropdown
        });

        // Both children stay alive; only forward events from the active one
        // to avoid duplicate toasts.
        ctx.subscribe_to_view(&v1, |this, _, event, ctx| {
            if !this.use_v2(ctx) {
                ctx.emit(event.clone());
            }
        });
        ctx.subscribe_to_view(&v2, |this, _, event, ctx| {
            if this.use_v2(ctx) {
                ctx.emit(event.clone());
            }
        });

        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });
        ctx.subscribe_to_model(&AuthManager::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });
        ctx.subscribe_to_model(&AISettings::handle(ctx), |this, _, event, ctx| {
            if matches!(event, AISettingsChangedEvent::UsageDisplayUnit { .. }) {
                let current_value = AISettings::as_ref(ctx).usage_display_unit;
                this.usage_display_unit_dropdown
                    .update(ctx, |dropdown, ctx| {
                        dropdown.set_selected_by_action(
                            BillingAndUsageDispatchAction::SetUsageDisplayUnit(current_value),
                            ctx,
                        );
                    });
                ctx.notify();
            }
        });

        let page = PageType::new_monolith(BillingAndUsageWidget, Some("Billing and Usage"), true);

        Self {
            page,
            v1,
            v2,
            local_only_icon_tooltip_states: Default::default(),
            usage_display_unit_dropdown,
        }
    }

    fn use_v2(&self, ctx: &AppContext) -> bool {
        if !FeatureFlag::BillingAndUsagePageV2.is_enabled() {
            return false;
        }
        Self::workspace_uses_v2(UserWorkspaces::as_ref(ctx).current_workspace())
    }

    fn workspace_uses_v2(workspace: Option<&Workspace>) -> bool {
        workspace.is_none_or(|workspace| {
            let bm = &workspace.billing_metadata;
            bm.is_on_build_plan()
                || bm.is_on_build_max_plan()
                || bm.is_on_build_business_plan()
                || bm.is_enterprise_plan()
                || bm.is_free_plan()
        })
    }

    pub fn get_modal_content(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        if self.use_v2(app) {
            self.v2.read(app, |view, _| view.get_modal_content())
        } else {
            self.v1.read(app, |view, _| view.get_modal_content())
        }
    }
}

#[cfg(test)]
#[path = "billing_and_usage_dispatch_tests.rs"]
mod tests;

impl Entity for BillingAndUsageDispatchView {
    type Event = BillingAndUsagePageEvent;
}
#[derive(Clone, Debug, PartialEq)]
pub enum BillingAndUsageDispatchAction {
    SetUsageDisplayUnit(UsageDisplayUnit),
}

impl TypedActionView for BillingAndUsageDispatchView {
    type Action = BillingAndUsageDispatchAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            BillingAndUsageDispatchAction::SetUsageDisplayUnit(value) => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.usage_display_unit.set_value(*value, ctx));
                });
                ctx.notify();
            }
        }
    }
}

impl View for BillingAndUsageDispatchView {
    fn ui_name() -> &'static str {
        "Billing and usage"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl SettingsPageMeta for BillingAndUsageDispatchView {
    fn section() -> SettingsSection {
        SettingsSection::BillingAndUsage
    }

    fn should_render(&self, ctx: &AppContext) -> bool {
        !AuthStateProvider::as_ref(ctx)
            .get()
            .is_anonymous_or_logged_out()
    }

    fn on_page_selected(&mut self, allow_steal_focus: bool, ctx: &mut ViewContext<Self>) {
        if self.use_v2(ctx) {
            self.v2.update(ctx, |view, ctx| {
                view.on_page_selected(allow_steal_focus, ctx)
            });
        } else {
            self.v1.update(ctx, |view, ctx| {
                view.on_page_selected(allow_steal_focus, ctx)
            });
        }
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id);
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<BillingAndUsageDispatchView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<BillingAndUsageDispatchView>) -> Self {
        SettingsPageViewHandle::BillingAndUsage(view_handle)
    }
}

#[derive(Default)]
struct BillingAndUsageWidget;

impl SettingsWidget for BillingAndUsageWidget {
    type View = BillingAndUsageDispatchView;

    fn search_terms(&self) -> &str {
        if FeatureFlag::PricingTransparency.is_enabled() {
            "plan billing a.i. ai usage limit credits dollars cost spend display unit pricing transparency balance overview"
        } else {
            "plan billing a.i. ai usage limit credits balance overview"
        }
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let billing_page = if view.use_v2(app) {
            ChildView::new(&view.v2).finish()
        } else {
            ChildView::new(&view.v1).finish()
        };
        let mut page = Flex::column();

        if FeatureFlag::PricingTransparency.is_enabled() {
            page.add_child(
                Container::new(render_dropdown_item(
                    appearance,
                    "Usage display unit",
                    Some("Select the unit for usage and spend amounts."),
                    None,
                    LocalOnlyIconState::for_setting(
                        UsageDisplayUnit::storage_key(),
                        UsageDisplayUnit::sync_to_cloud(),
                        &mut view.local_only_icon_tooltip_states.borrow_mut(),
                        app,
                    ),
                    None,
                    &view.usage_display_unit_dropdown,
                ))
                .with_margin_bottom(24.)
                .finish(),
            );
        }

        page.with_child(billing_page).finish()
    }
}
