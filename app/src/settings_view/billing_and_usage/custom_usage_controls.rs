use warp_graphql::billing::CustomUsageAmounts;
use warpui::elements::{Border, ChildView, Container, Flex, MouseStateHandle, ParentElement, Text};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
    WeakViewHandle,
};

use super::overage_limit_modal::{SpendingLimitModal, SpendingLimitModalEvent};
use crate::Appearance;
use crate::ai::blocklist::view_util::usage_display_unit;
use crate::auth::AuthStateProvider;
use crate::editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions};
use crate::settings::{AISettings, AISettingsChangedEvent, UsageDisplayUnit};
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};

#[derive(Clone, Copy)]
pub enum CustomUsagePurpose {
    Billing,
    Purchase,
    EnableReload,
}

pub enum CustomUsageEvent {
    Selected,
    PresetRestored,
    Saved,
}

#[derive(Clone, Debug)]
pub enum CustomUsageAction {
    Select,
    Purchase,
    SaveReload,
    DisableReload,
    EditLimit,
}

pub struct CustomUsageControls {
    handle: WeakViewHandle<Self>,
    editor: ViewHandle<EditorView>,
    limit_editor: ViewHandle<SpendingLimitModal>,
    editing_limit: bool,
    limit_update: bool,
    request_id: uuid::Uuid,
    purchase_pending: bool,
    submitted_amount: Option<i32>,
    message: Option<String>,
    purpose: CustomUsagePurpose,
    selected: bool,
    dirty: bool,
    loading: bool,
    error: Option<String>,
    select_mouse: MouseStateHandle,
    purchase_mouse: MouseStateHandle,
    save_mouse: MouseStateHandle,
    limit_mouse: MouseStateHandle,
}

pub fn parse_usage_dollars(text: &str, policy: CustomUsageAmounts) -> Option<i32> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let cents = text.parse::<i32>().ok()?.checked_mul(100)?;
    (policy.step_cents > 0
        && cents >= policy.minimum_cents
        && cents <= policy.maximum_cents
        && cents % policy.step_cents == 0)
        .then_some(cents)
}

impl CustomUsageControls {
    pub fn new(purpose: CustomUsagePurpose, ctx: &mut ViewContext<Self>) -> Self {
        let editor = ctx.add_typed_action_view(|ctx| {
            let mut editor = EditorView::single_line(SingleLineEditorOptions::default(), ctx);
            editor.set_placeholder_text("Whole USD amount", ctx);
            editor
        });
        ctx.subscribe_to_view(&editor, |me, _, event, ctx| {
            if matches!(event, EditorEvent::Edited(origin) if origin.is_user()) {
                me.selected = true;
                me.dirty = true;
                me.error = None;
                ctx.emit(CustomUsageEvent::Selected);
                ctx.notify();
            }
        });
        let limit_editor = ctx.add_typed_action_view(SpendingLimitModal::new);
        ctx.subscribe_to_view(&limit_editor, |me, _, event, ctx| {
            match event {
                SpendingLimitModalEvent::Close => me.editing_limit = false,
                SpendingLimitModalEvent::Update { amount_cents }
                    if !me.loading && me.is_admin(ctx) =>
                {
                    let team_uid = UserWorkspaces::as_ref(ctx)
                        .team_for_view(ctx)
                        .map(|team| team.uid);
                    if let Some(team_uid) = team_uid {
                        me.loading = true;
                        me.limit_update = true;
                        UserWorkspaces::handle(ctx).update(ctx, |workspaces, ctx| {
                            workspaces.update_addon_usage_settings(
                                team_uid,
                                None,
                                None,
                                Some(*amount_cents as i32),
                                me.request_id,
                                ctx,
                            );
                        });
                    }
                }
                SpendingLimitModalEvent::Update { .. } => {}
            }
            ctx.notify();
        });
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _, event, ctx| {
            match event {
                UserWorkspacesEvent::TeamsChanged => {
                    if !me.dirty {
                        me.restore_saved(ctx);
                    }
                }
                UserWorkspacesEvent::CustomUsageRequestCompleted { request_id, result }
                    if *request_id == me.request_id =>
                {
                    me.loading = false;
                    match result {
                        Ok(Some(checkout_url)) => {
                            ctx.open_url(checkout_url);
                            me.message = Some(
                                "Complete checkout in your browser. Usage is added after payment."
                                    .to_string(),
                            );
                        }
                        Ok(None) if me.limit_update => me.editing_limit = false,
                        Ok(None) => {
                            if me.purchase_pending {
                                me.message = Some("Usage purchased successfully.".to_string());
                            } else {
                                me.dirty = me.amount(ctx) != me.submitted_amount;
                                me.message = Some("Auto-reload amount saved.".to_string());
                            }
                            ctx.emit(CustomUsageEvent::Saved);
                        }
                        Err(error) => me.error = Some(error.to_string()),
                    }
                    me.limit_update = false;
                    me.purchase_pending = false;
                }
                UserWorkspacesEvent::CurrentWorkspaceChanged => {
                    me.request_id = uuid::Uuid::new_v4();
                    me.loading = false;
                    me.limit_update = false;
                    me.purchase_pending = false;
                    me.submitted_amount = None;
                    me.editing_limit = false;
                    me.selected = false;
                    me.dirty = false;
                    me.error = None;
                    me.message = None;
                    me.restore_saved(ctx);
                }
                _ => {}
            }
            ctx.notify();
        });
        ctx.subscribe_to_model(&AISettings::handle(ctx), |_, _, event, ctx| {
            if matches!(event, AISettingsChangedEvent::UsageDisplayUnit { .. }) {
                ctx.notify();
            }
        });
        let mut me = Self {
            handle: ctx.handle(),
            editor,
            limit_editor,
            editing_limit: false,
            limit_update: false,
            request_id: uuid::Uuid::new_v4(),
            purchase_pending: false,
            submitted_amount: None,
            message: None,
            purpose,
            selected: false,
            dirty: false,
            loading: false,
            error: None,
            select_mouse: Default::default(),
            purchase_mouse: Default::default(),
            save_mouse: Default::default(),
            limit_mouse: Default::default(),
        };
        me.restore_saved(ctx);
        me
    }

    fn policy(app: &AppContext) -> Option<CustomUsageAmounts> {
        UserWorkspaces::as_ref(app)
            .current_workspace()?
            .billing_metadata
            .custom_usage_amounts
    }

    fn amount(&self, app: &AppContext) -> Option<i32> {
        parse_usage_dollars(
            &self.editor.as_ref(app).buffer_text(app),
            Self::policy(app)?,
        )
    }

    fn restore_saved(&mut self, ctx: &mut ViewContext<Self>) {
        let saved = UserWorkspaces::as_ref(ctx)
            .current_workspace()
            .and_then(|workspace| {
                workspace
                    .settings
                    .addon_credits_settings
                    .selected_auto_reload_usage_cents
            });
        if let Some(cents) = saved.filter(|cents| ![1000, 2000, 5000, 10000].contains(cents)) {
            self.editor.update(ctx, |editor, ctx| {
                editor.system_reset_buffer_text(&format!("{}", f64::from(cents) / 100.), ctx);
            });
            self.selected = true;
            self.dirty = false;
            ctx.emit(CustomUsageEvent::Selected);
        } else {
            self.selected = false;
            self.editor
                .update(ctx, |editor, ctx| editor.system_reset_buffer_text("", ctx));
            ctx.emit(CustomUsageEvent::PresetRestored);
        }
    }

    pub fn select_preset(&mut self, ctx: &mut ViewContext<Self>) {
        self.selected = false;
        self.dirty = true;
        ctx.notify();
    }

    pub fn is_selected(&self) -> bool {
        self.selected
    }

    fn is_admin(&self, app: &AppContext) -> bool {
        UserWorkspaces::as_ref(app)
            .team_for_view_handle(&self.handle, app)
            .is_some_and(|team| {
                AuthStateProvider::as_ref(app)
                    .get()
                    .user_email()
                    .is_some_and(|email| team.has_admin_permissions(&email))
            })
    }

    fn allowed(&self, app: &AppContext) -> bool {
        let Some(workspace) = UserWorkspaces::as_ref(app).current_workspace() else {
            return false;
        };
        self.amount(app).is_some_and(|cents| {
            workspace
                .bonus_grants_purchased_this_month
                .cents_spent
                .saturating_add(cents)
                <= workspace
                    .settings
                    .addon_credits_settings
                    .max_monthly_spend_cents
                    .unwrap_or(20_000)
        }) && !workspace
            .billing_metadata
            .is_delinquent_due_to_payment_issue()
            && !self.loading
    }
}

impl Entity for CustomUsageControls {
    type Event = CustomUsageEvent;
}

impl View for CustomUsageControls {
    fn ui_name() -> &'static str {
        "Custom usage amount"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let Some(policy) = Self::policy(app) else {
            let mut column = Flex::column().with_spacing(8.);
            if let Some(settings) = UserWorkspaces::as_ref(app)
                .current_workspace()
                .map(|workspace| &workspace.settings.addon_credits_settings)
                && let Some(cents) = settings.selected_auto_reload_usage_cents
                && settings.selected_auto_reload_credit_denomination.is_none()
            {
                column.add_child(Text::new(format!("Saved auto-reload amount: ${:.2} of usage. Custom auto-reload is paused on this billing plan.", f64::from(cents) / 100.), appearance.ui_font_family(), 12.).finish());
                if settings.auto_reload_enabled && self.is_admin(app) {
                    let mut button = appearance
                        .ui_builder()
                        .button(ButtonVariant::Secondary, self.save_mouse.clone())
                        .with_text_label("Disable auto-reload".to_string())
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(CustomUsageAction::DisableReload)
                        });
                    if self.loading {
                        button = button.disable();
                    }
                    column.add_child(button.finish());
                }
            }
            if let Some(error) = &self.error {
                column.add_child(
                    Text::new(error.clone(), appearance.ui_font_family(), 12.)
                        .with_color(appearance.theme().ui_error_color())
                        .finish(),
                );
            }
            return column.finish();
        };
        let workspaces = UserWorkspaces::as_ref(app);
        let workspace = workspaces.current_workspace();
        let admin = self.is_admin(app);
        let mut column = Flex::column().with_spacing(8.);
        if let Some(cents) = workspace.and_then(|workspace| {
            workspace
                .settings
                .addon_credits_settings
                .selected_auto_reload_usage_cents
        }) {
            column.add_child(
                Text::new(
                    format!(
                        "Saved auto-reload amount: ${:.2} of usage.",
                        f64::from(cents) / 100.
                    ),
                    appearance.ui_font_family(),
                    12.,
                )
                .finish(),
            );
        }
        if !admin
            && workspace.is_some_and(|workspace| {
                workspace
                    .settings
                    .addon_credits_settings
                    .auto_reload_enabled
            })
        {
            return column.finish();
        }
        let mut choose = appearance
            .ui_builder()
            .button(ButtonVariant::Secondary, self.select_mouse.clone())
            .with_text_label("Custom amount".to_string())
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(CustomUsageAction::Select));
        if self.loading {
            choose = choose.disable();
        }
        column.add_child(choose.finish());
        if !self.selected {
            return column.finish();
        }
        column
            .add_child(Text::new("Custom amount (USD)", appearance.ui_font_family(), 12.).finish());
        column.add_child(
            Container::new(ChildView::new(&self.editor).finish())
                .with_uniform_padding(8.)
                .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
                .finish(),
        );
        let amount = self.amount(app);
        let text = amount.map_or_else(
            || {
                format!(
                    "Enter whole USD from ${} through ${}.",
                    policy.minimum_cents / 100,
                    policy.maximum_cents / 100
                )
            },
            |cents| {
                format!(
                    "Get ${} of usage for ${} before tax.",
                    cents / 100,
                    cents / 100
                )
            },
        );
        column.add_child(Text::new(text, appearance.ui_font_family(), 12.).finish());
        if usage_display_unit(app) == UsageDisplayUnit::Credits
            && let Some(cents) = amount
        {
            let credits = (f64::from(cents) / 1.8).floor() as i32;
            column.add_child(
                Text::new(
                    format!("Approximately {credits} display credits."),
                    appearance.ui_font_family(),
                    12.,
                )
                .finish(),
            );
        }
        let reload_enabled = workspace.is_some_and(|workspace| {
            workspace
                .settings
                .addon_credits_settings
                .auto_reload_enabled
        });
        let enabled = self.allowed(app);
        let mut buttons = Flex::row().with_spacing(8.);
        if !matches!(self.purpose, CustomUsagePurpose::EnableReload) {
            let mut button = appearance
                .ui_builder()
                .button(ButtonVariant::Accent, self.purchase_mouse.clone())
                .with_text_label(
                    if self.loading {
                        "Buying…"
                    } else {
                        "One-time purchase"
                    }
                    .to_string(),
                )
                .build()
                .on_click(|ctx, _, _| ctx.dispatch_typed_action(CustomUsageAction::Purchase));
            if !enabled || reload_enabled {
                button = button.disable();
            }
            buttons.add_child(button.finish());
        }
        if admin && !matches!(self.purpose, CustomUsagePurpose::Purchase) {
            let mut button = appearance
                .ui_builder()
                .button(ButtonVariant::Accent, self.save_mouse.clone())
                .with_text_label(
                    if reload_enabled {
                        "Save auto-reload amount"
                    } else {
                        "Enable auto-reload"
                    }
                    .to_string(),
                )
                .build()
                .on_click(|ctx, _, _| ctx.dispatch_typed_action(CustomUsageAction::SaveReload));
            if !enabled {
                button = button.disable();
            }
            buttons.add_child(button.finish());
        }
        column.add_child(buttons.finish());
        if admin {
            column.add_child(
                appearance
                    .ui_builder()
                    .button(ButtonVariant::Secondary, self.limit_mouse.clone())
                    .with_text_label("Edit monthly spend limit".to_string())
                    .build()
                    .on_click(|ctx, _, _| ctx.dispatch_typed_action(CustomUsageAction::EditLimit))
                    .finish(),
            );
            if self.editing_limit {
                column.add_child(ChildView::new(&self.limit_editor).finish());
            }
        }
        if amount.is_some() && !enabled && !self.loading {
            let warning = if workspace.is_some_and(|workspace| {
                workspace
                    .billing_metadata
                    .is_delinquent_due_to_payment_issue()
            }) {
                "Billing is restricted. Update your payment method in Billing & usage."
            } else if admin {
                "This purchase would exceed your monthly limit. Increase your limit to continue."
            } else {
                "This purchase would exceed your monthly limit. Contact your admin to increase it."
            };
            column.add_child(Text::new(warning, appearance.ui_font_family(), 12.).finish());
        }
        if reload_enabled {
            column.add_child(
                Text::new(
                    "Disable auto-reload before making a one-time purchase.",
                    appearance.ui_font_family(),
                    12.,
                )
                .finish(),
            );
        }
        if let Some(message) = &self.message {
            column.add_child(Text::new(message.clone(), appearance.ui_font_family(), 12.).finish());
        }
        if let Some(error) = &self.error {
            column.add_child(
                Text::new(error.clone(), appearance.ui_font_family(), 12.)
                    .with_color(appearance.theme().ui_error_color())
                    .finish(),
            );
        }
        column.finish()
    }
}

impl TypedActionView for CustomUsageControls {
    type Action = CustomUsageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        if self.loading {
            return;
        }
        if matches!(action, CustomUsageAction::Select) {
            self.selected = true;
            self.dirty = true;
            ctx.focus(&self.editor);
            ctx.emit(CustomUsageEvent::Selected);
            ctx.notify();
            return;
        }
        if matches!(action, CustomUsageAction::DisableReload) {
            if !self.is_admin(ctx) {
                return;
            }
            let Some(team_uid) = UserWorkspaces::as_ref(ctx)
                .team_for_view(ctx)
                .map(|team| team.uid)
            else {
                return;
            };
            self.loading = true;
            UserWorkspaces::handle(ctx).update(ctx, |workspaces, ctx| {
                workspaces.update_addon_usage_settings(
                    team_uid,
                    Some(false),
                    None,
                    None,
                    self.request_id,
                    ctx,
                );
            });
            ctx.notify();
            return;
        }
        if matches!(action, CustomUsageAction::EditLimit) {
            if !self.is_admin(ctx) {
                return;
            }
            self.editing_limit = true;
            self.limit_editor
                .update(ctx, |view, ctx| view.focus_input(ctx));
            ctx.notify();
            return;
        }
        if !self.allowed(ctx) {
            return;
        }
        let Some(cents) = self.amount(ctx) else {
            return;
        };
        let workspaces = UserWorkspaces::as_ref(ctx);
        let Some(team) = workspaces.team_for_view(ctx) else {
            return;
        };
        let team_uid = team.uid;
        let admin = AuthStateProvider::as_ref(ctx)
            .get()
            .user_email()
            .is_some_and(|email| team.has_admin_permissions(&email));
        match action {
            CustomUsageAction::Select
            | CustomUsageAction::EditLimit
            | CustomUsageAction::DisableReload => {}
            CustomUsageAction::Purchase => {
                if workspaces.current_workspace().is_some_and(|workspace| {
                    workspace
                        .settings
                        .addon_credits_settings
                        .auto_reload_enabled
                }) {
                    return;
                }
                self.loading = true;
                self.purchase_pending = true;
                self.error = None;
                self.message = None;
                UserWorkspaces::handle(ctx).update(ctx, |workspaces, ctx| {
                    workspaces.purchase_addon_usage(Some(team_uid), cents, self.request_id, ctx)
                });
            }
            CustomUsageAction::SaveReload if admin => {
                self.loading = true;
                self.submitted_amount = Some(cents);
                self.error = None;
                self.message = None;
                UserWorkspaces::handle(ctx).update(ctx, |workspaces, ctx| {
                    workspaces.update_addon_usage_settings(
                        team_uid,
                        Some(true),
                        Some(cents),
                        None,
                        self.request_id,
                        ctx,
                    )
                });
            }
            CustomUsageAction::SaveReload => {}
        }
        ctx.notify();
    }
}

#[cfg(test)]
#[path = "custom_usage_controls_tests.rs"]
mod tests;
