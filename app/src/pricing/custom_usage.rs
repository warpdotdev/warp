use warp_graphql::billing::CustomUsageAmounts;
use warpui::elements::{
    Border, ChildView, Clipped, Container, Empty, Flex, MouseStateHandle, ParentElement, Text,
};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
    WeakViewHandle,
};

use crate::Appearance;
use crate::ai::blocklist::view_util::format_credits;
use crate::auth::AuthStateProvider;
use crate::editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions};
use crate::pricing::addon_pack::format_price;
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};

pub const LIMIT_WARNING: &str =
    "This purchase would exceed your monthly limit. Increase your limit to continue.";

pub fn parse_usage_dollars(text: &str, bounds: CustomUsageAmounts) -> Option<i32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let cents = text.parse::<i32>().ok()?.checked_mul(100)?;
    (cents >= bounds.min_usage_cents
        && cents <= bounds.max_usage_cents
        && bounds.step_usage_cents > 0
        && cents % bounds.step_usage_cents == 0)
        .then_some(cents)
}

pub struct CustomUsageEditor {
    handle: WeakViewHandle<Self>,
    editor: ViewHandle<EditorView>,
    expanded: bool,
    reload_only: bool,
    pending: bool,
    error: Option<String>,
    confirmation: Option<String>,
    awaiting_confirmation: bool,
    open_mouse: MouseStateHandle,
    buy_mouse: MouseStateHandle,
    reload_mouse: MouseStateHandle,
    limit_mouse: MouseStateHandle,
    cancel_mouse: MouseStateHandle,
}

#[derive(Clone, Debug)]
pub enum CustomUsageEvent {
    Selected,
    Completed,
    Confirmation(String),
    EditLimit,
}

#[derive(Clone, Debug)]
pub enum CustomUsageAction {
    Open,
    Cancel,
    Purchase,
    SaveReload,
    EditLimit,
}

impl CustomUsageEditor {
    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    pub fn is_pending(&self) -> bool {
        self.pending
    }

    pub fn set_reload_only(&mut self, reload_only: bool, ctx: &mut ViewContext<Self>) {
        self.reload_only = reload_only;
        ctx.notify();
    }

    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let editor = ctx.add_typed_action_view(|ctx| {
            let mut editor = EditorView::single_line(SingleLineEditorOptions::default(), ctx);
            editor.set_placeholder_text("10", ctx);
            editor
        });
        ctx.subscribe_to_view(&editor, |me, _, event, ctx| match event {
            EditorEvent::Edited(_) => {
                me.error = None;
                ctx.notify();
            }
            EditorEvent::Escape => {
                me.expanded = false;
                ctx.notify();
            }
            _ => {}
        });
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _, event, ctx| {
            if let UserWorkspacesEvent::PurchaseUsageConfirmed {
                usage_cents,
                price_usd_cents,
            } = event
                && me.awaiting_confirmation
            {
                let message = format!(
                    "Purchased {} of usage for {} USD before taxes.",
                    format_price(*usage_cents),
                    format_price(*price_usd_cents)
                );
                me.confirmation = Some(message.clone());
                me.awaiting_confirmation = false;
                ctx.emit(CustomUsageEvent::Confirmation(message));
            }
            if me.pending {
                match event {
                    UserWorkspacesEvent::PurchaseAddonCreditsSuccess
                    | UserWorkspacesEvent::UpdateWorkspaceSettingsSuccess => {
                        me.pending = false;
                        me.error = None;
                        ctx.emit(CustomUsageEvent::Completed);
                    }
                    UserWorkspacesEvent::PurchaseAddonCreditsCheckoutRequired { checkout_url } => {
                        me.pending = false;
                        me.awaiting_confirmation = false;
                        ctx.open_url(checkout_url);
                        ctx.emit(CustomUsageEvent::Completed);
                    }
                    UserWorkspacesEvent::PurchaseAddonCreditsRejected(error)
                    | UserWorkspacesEvent::UpdateWorkspaceSettingsRejected(error) => {
                        me.pending = false;
                        me.error = Some(error.to_string());
                        me.awaiting_confirmation = false;
                    }
                    _ => {}
                }
            }
            ctx.notify();
        });
        Self {
            handle: ctx.handle(),
            editor,
            expanded: false,
            reload_only: false,
            pending: false,
            error: None,
            confirmation: None,
            awaiting_confirmation: false,
            open_mouse: Default::default(),
            buy_mouse: Default::default(),
            reload_mouse: Default::default(),
            limit_mouse: Default::default(),
            cancel_mouse: Default::default(),
        }
    }

    pub fn open(&mut self, reload_only: bool, ctx: &mut ViewContext<Self>) {
        self.reload_only = reload_only;
        self.expanded = true;
        if self.editor.as_ref(ctx).buffer_text(ctx).is_empty()
            && let Some(cents) =
                UserWorkspaces::as_ref(ctx)
                    .current_workspace()
                    .and_then(|workspace| {
                        workspace
                            .settings
                            .addon_credits_settings
                            .selected_auto_reload_usage_cents
                    })
        {
            self.editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(&(cents / 100).to_string(), ctx);
            });
        }
        ctx.focus(&self.editor);
        ctx.emit(CustomUsageEvent::Selected);
        ctx.notify();
    }

    pub fn abandon(&mut self, ctx: &mut ViewContext<Self>) {
        self.expanded = false;
        self.error = None;
        self.editor
            .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
        ctx.notify();
    }

    fn amount(&self, app: &AppContext) -> Option<i32> {
        parse_usage_dollars(
            &self.editor.as_ref(app).buffer_text(app),
            UserWorkspaces::as_ref(app).custom_usage_amounts()?,
        )
    }

    fn can_manage(&self, app: &AppContext) -> bool {
        let workspaces = UserWorkspaces::as_ref(app);
        workspaces.custom_usage_amounts().is_some()
            && workspaces
                .team_for_view_handle(&self.handle, app)
                .is_some_and(|team| {
                    AuthStateProvider::as_ref(app)
                        .get()
                        .user_email()
                        .is_some_and(|email| team.has_admin_permissions(&email))
                })
    }

    fn can_purchase(&self, app: &AppContext) -> bool {
        let workspaces = UserWorkspaces::as_ref(app);
        workspaces.custom_usage_amounts().is_some()
            && workspaces
                .purchase_policy()
                .is_some_and(|policy| policy.allows_purchases())
            && workspaces
                .team_for_view_handle(&self.handle, app)
                .is_some_and(|team| {
                    AuthStateProvider::as_ref(app)
                        .get()
                        .user_email()
                        .is_some_and(|email| {
                            team.members
                                .iter()
                                .any(|member| member.email == email && !member.is_disabled)
                        })
                })
    }

    fn exceeds_limit(&self, cents: i32, app: &AppContext) -> bool {
        UserWorkspaces::as_ref(app)
            .current_workspace()
            .is_some_and(|workspace| workspace.would_addon_purchase_reach_limit(cents))
    }
}

impl Entity for CustomUsageEditor {
    type Event = CustomUsageEvent;
}

impl TypedActionView for CustomUsageEditor {
    type Action = CustomUsageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CustomUsageAction::Open => self.open(self.reload_only, ctx),
            CustomUsageAction::Cancel => {
                if !self.pending {
                    self.abandon(ctx);
                }
            }
            CustomUsageAction::EditLimit => ctx.emit(CustomUsageEvent::EditLimit),
            CustomUsageAction::Purchase | CustomUsageAction::SaveReload => {
                if self.pending || !self.can_purchase(ctx) {
                    return;
                }
                let Some(cents) = self.amount(ctx) else {
                    return;
                };
                let workspaces = UserWorkspaces::as_ref(ctx);
                let Some(workspace) = workspaces.current_workspace() else {
                    return;
                };
                if self.exceeds_limit(cents, ctx)
                    || workspace
                        .billing_metadata
                        .is_delinquent_due_to_payment_issue()
                {
                    return;
                }
                let reload = matches!(action, CustomUsageAction::SaveReload);
                if reload && !self.can_manage(ctx) {
                    return;
                }
                if !reload
                    && (self.reload_only
                        || workspace
                            .settings
                            .addon_credits_settings
                            .auto_reload_enabled)
                {
                    return;
                }
                let Some(team_uid) = workspaces.team_uid_for_window(ctx.window_id()) else {
                    return;
                };
                self.pending = true;
                self.awaiting_confirmation = !reload;
                self.error = None;
                UserWorkspaces::handle(ctx).update(ctx, |workspaces, ctx| {
                    if reload {
                        workspaces.update_usage_reload_settings(team_uid, Some(true), cents, ctx);
                    } else {
                        workspaces.purchase_usage(Some(team_uid), cents, ctx);
                    }
                });
                ctx.notify();
            }
        }
    }
}

impl View for CustomUsageEditor {
    fn ui_name() -> &'static str {
        "CustomUsageEditor"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let workspaces = UserWorkspaces::as_ref(app);
        let Some(bounds) = workspaces.custom_usage_amounts() else {
            return Empty::new().finish();
        };
        let appearance = Appearance::as_ref(app);
        let ui = appearance.ui_builder();
        let theme = appearance.theme();
        let saved = workspaces.current_workspace().and_then(|workspace| {
            workspace
                .settings
                .addon_credits_settings
                .selected_auto_reload_usage_cents
        });
        let mut column = Flex::column().with_spacing(8.);
        if let Some(cents) = saved {
            column.add_child(
                ui.paragraph(format!(
                    "Saved auto-reload amount: {} USD of usage",
                    format_price(cents)
                ))
                .build()
                .finish(),
            );
        }
        if let Some(confirmation) = &self.confirmation {
            column.add_child(ui.paragraph(confirmation.clone()).build().finish());
        }
        if !self.can_purchase(app) {
            return column.finish();
        }
        if !self.expanded {
            return column
                .with_child(
                    ui.button(ButtonVariant::Secondary, self.open_mouse.clone())
                        .with_text_label("Custom amount".to_string())
                        .build()
                        .on_click(|ctx, _, _| ctx.dispatch_typed_action(CustomUsageAction::Open))
                        .finish(),
                )
                .finish();
        }
        column.add_child(
            ui.paragraph("Custom amount (USD, whole dollars)")
                .build()
                .finish(),
        );
        column.add_child(
            Container::new(Clipped::new(ChildView::new(&self.editor).finish()).finish())
                .with_uniform_padding(8.)
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .finish(),
        );
        let amount = self.amount(app);
        let over_limit = amount.is_some_and(|cents| self.exceeds_limit(cents, app));
        let invalid = amount.is_none();
        let guidance = match amount {
            Some(cents) => format!(
                "Receive {} of usage (≈ {}). Price: {} USD before taxes and applicable discounts.",
                format_price(cents),
                format_credits(cents as f32 / 1.8),
                format_price(cents)
            ),
            None => format!(
                "Enter a whole-dollar USD amount from {} to {}.",
                format_price(bounds.min_usage_cents),
                format_price(bounds.max_usage_cents)
            ),
        };
        column.add_child(
            Text::new(
                guidance,
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(theme.foreground().into())
            .finish(),
        );
        if over_limit {
            column.add_child(ui.paragraph(if self.can_manage(app) { LIMIT_WARNING } else { "This purchase would exceed your monthly limit. Contact a team admin to increase it." }).build().finish());
            if self.can_manage(app) {
                column.add_child(
                    ui.button(ButtonVariant::Secondary, self.limit_mouse.clone())
                        .with_text_label("Increase your limit".to_string())
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(CustomUsageAction::EditLimit)
                        })
                        .finish(),
                );
            }
        }
        if let Some(error) = &self.error {
            column.add_child(ui.paragraph(error.clone()).build().finish());
        }
        let disabled = invalid
            || over_limit
            || self.pending
            || workspaces.current_workspace().is_some_and(|workspace| {
                workspace
                    .billing_metadata
                    .is_delinquent_due_to_payment_issue()
            });
        let reload_enabled = workspaces.current_workspace().is_some_and(|workspace| {
            workspace
                .settings
                .addon_credits_settings
                .auto_reload_enabled
        });
        let mut buttons = Flex::row().with_spacing(8.);
        if !self.reload_only && !reload_enabled {
            let mut button = ui
                .button(ButtonVariant::Accent, self.buy_mouse.clone())
                .with_text_label(
                    if self.pending {
                        "Buying…"
                    } else {
                        "Buy custom amount"
                    }
                    .to_string(),
                );
            if disabled {
                button = button.disabled();
            }
            buttons.add_child(
                button
                    .build()
                    .on_click(|ctx, _, _| ctx.dispatch_typed_action(CustomUsageAction::Purchase))
                    .finish(),
            );
        }
        let mut save = ui
            .button(ButtonVariant::Secondary, self.reload_mouse.clone())
            .with_text_label(
                if self.pending {
                    "Saving…"
                } else if reload_enabled {
                    "Save auto-reload amount"
                } else {
                    "Enable auto-reload"
                }
                .to_string(),
            );
        if disabled {
            save = save.disabled();
        }
        if self.can_manage(app) {
            buttons.add_child(
                save.build()
                    .on_click(|ctx, _, _| ctx.dispatch_typed_action(CustomUsageAction::SaveReload))
                    .finish(),
            );
        }
        buttons.add_child(
            ui.button(ButtonVariant::Secondary, self.cancel_mouse.clone())
                .with_text_label("Cancel".to_string())
                .build()
                .on_click(|ctx, _, _| ctx.dispatch_typed_action(CustomUsageAction::Cancel))
                .finish(),
        );
        column.with_child(buttons.finish()).finish()
    }
}

#[cfg(test)]
#[path = "custom_usage_tests.rs"]
mod tests;
