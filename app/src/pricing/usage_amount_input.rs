use thousands::Separable;
use warp_graphql::billing::{AddonCreditsOption, AddonUsagePurchaseRange};
use warpui::elements::{
    Border, ChildView, Clipped, Container, CrossAxisAlignment, Expanded, Flex, ParentElement, Text,
};
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity, View, ViewContext, ViewHandle,
};

use super::PricingInfoModel;
use super::addon_pack::PackAmount;
use crate::Appearance;
use crate::editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions};
use crate::settings::UsageDisplayUnit;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::workspaces::workspace::{ChargeUnit, WorkspaceUid};

pub fn custom_usage_enabled(ctx: &AppContext) -> bool {
    UserWorkspaces::as_ref(ctx).charge_unit() == ChargeUnit::Cents
}

#[derive(Clone)]
pub enum AddonPurchaseAmount {
    Credits(AddonCreditsOption),
    UsageCents(i32),
}

impl AddonPurchaseAmount {
    pub fn label(&self, unit: UsageDisplayUnit) -> String {
        match self {
            Self::Credits(option) => PackAmount::of(option, unit).label(),
            Self::UsageCents(cents) => PackAmount::UsageCents(*cents).label(),
        }
    }

    pub fn price_usd_cents_with_premium(&self, premium_bps: i32) -> i32 {
        let cents = match self {
            Self::Credits(option) => option.price_usd_cents,
            Self::UsageCents(cents) => *cents,
        };
        let surcharge = (i64::from(cents) * i64::from(premium_bps.max(0)) + 9_999) / 10_000;
        cents + surcharge as i32
    }

    pub fn discount_percent(&self, base_rate: f32) -> u32 {
        match self {
            Self::Credits(option) => option.discount_percent(base_rate),
            Self::UsageCents(_) => 0,
        }
    }
}

pub struct UsageAmountInput {
    editor: ViewHandle<EditorView>,
    synced_amount: Option<i32>,
    synced_workspace: Option<WorkspaceUid>,
}

pub enum UsageAmountInputEvent {
    Changed,
    Commit(i32),
}

impl UsageAmountInput {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let editor = ctx.add_typed_action_view(|ctx| {
            let mut editor = EditorView::single_line(SingleLineEditorOptions::default(), ctx);
            editor.system_reset_buffer_text("10", ctx);
            editor
        });
        ctx.subscribe_to_view(&editor, |me, _, event, ctx| {
            match event {
                EditorEvent::Edited(_) => ctx.emit(UsageAmountInputEvent::Changed),
                EditorEvent::Enter | EditorEvent::Blurred => {
                    if let Some(cents) = me.amount(ctx) {
                        ctx.emit(UsageAmountInputEvent::Commit(cents));
                    }
                }
                _ => {}
            }
            ctx.notify();
        });
        Self {
            editor,
            synced_amount: None,
            synced_workspace: None,
        }
    }

    pub fn amount(&self, ctx: &AppContext) -> Option<i32> {
        parse_usage_amount(
            &self.editor.as_ref(ctx).buffer_text(ctx),
            PricingInfoModel::as_ref(ctx).addon_usage_purchase_range()?,
        )
    }

    pub fn sync_workspace(
        &mut self,
        workspace: Option<(WorkspaceUid, Option<i32>)>,
        ctx: &mut ViewContext<Self>,
    ) {
        let uid = workspace.map(|(uid, _)| uid);
        if self.synced_workspace != uid {
            self.synced_workspace = uid;
            self.synced_amount = None;
        }
        self.sync_amount(workspace.and_then(|(_, cents)| cents).unwrap_or(1000), ctx);
    }

    pub fn sync_amount(&mut self, cents: i32, ctx: &mut ViewContext<Self>) {
        if self.synced_amount == Some(cents) {
            return;
        }
        self.synced_amount = Some(cents);
        self.editor.update(ctx, |editor, ctx| {
            editor.system_reset_buffer_text(&(cents / 100).to_string(), ctx);
        });
        ctx.notify();
    }
}

fn parse_usage_amount(text: &str, range: &AddonUsagePurchaseRange) -> Option<i32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let cents = text.parse::<i32>().ok()?.checked_mul(100)?;
    (range.step_usage_cents > 0
        && cents >= range.min_usage_cents
        && cents <= range.max_usage_cents
        && (cents - range.min_usage_cents) % range.step_usage_cents == 0)
        .then_some(cents)
}

impl Entity for UsageAmountInput {
    type Event = UsageAmountInputEvent;
}

impl warpui::TypedActionView for UsageAmountInput {
    type Action = ();
}

impl View for UsageAmountInput {
    fn ui_name() -> &'static str {
        "UsageAmountInput"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.editor);
        }
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(ctx);
        let theme = appearance.theme();
        let valid = self.amount(ctx).is_some();
        let helper = PricingInfoModel::as_ref(ctx)
            .addon_usage_purchase_range()
            .map_or("Loading usage pricing…".to_string(), |range| {
                format!(
                    "Enter an amount from ${} to ${}",
                    (range.min_usage_cents / 100).separate_with_commas(),
                    (range.max_usage_cents / 100).separate_with_commas()
                )
            });
        let input = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(
                Text::new_inline("$", appearance.ui_font_family(), 14.)
                    .with_color(theme.foreground().into())
                    .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    Clipped::new(ChildView::new(&self.editor).finish()).finish(),
                )
                .finish(),
            )
            .finish();
        Flex::column()
            .with_spacing(6.)
            .with_child(
                Text::new_inline("Usage amount", appearance.ui_font_family(), 14.)
                    .with_color(theme.foreground().into())
                    .finish(),
            )
            .with_child(
                Container::new(input)
                    .with_uniform_padding(8.)
                    .with_border(Border::all(1.).with_border_fill(if valid {
                        theme.outline()
                    } else {
                        theme.ui_error_color().into()
                    }))
                    .finish(),
            )
            .with_child(
                Text::new(helper, appearance.ui_font_family(), 12.)
                    .with_color(if valid {
                        theme.sub_text_color(theme.surface_1()).into()
                    } else {
                        theme.ui_error_color()
                    })
                    .finish(),
            )
            .finish()
    }
}

#[cfg(test)]
#[path = "usage_amount_input_tests.rs"]
mod tests;
