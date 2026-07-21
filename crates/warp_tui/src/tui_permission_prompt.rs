//! Reusable interaction for TUI tool-call permission requests.

use std::collections::HashMap;

use warp::tui_export::{
    AIAgentActionId, BlocklistAIActionEvent, BlocklistAIActionModel, OptionFooter, OptionRow,
    OptionSnapshot, OptionSourceStatus,
};
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::elements::tui::{
    Modifier, TuiChildView, TuiContainer, TuiElement, TuiFlex, TuiParentElement, TuiText,
};
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{EditableBinding, FixedBinding};
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::editor_view::TuiEditorView;
use crate::keybindings::{TUI_BINDING_GROUP, is_tui_owned_binding};
use crate::option_selector::{
    OptionSelectorPage, TuiOptionSelector, TuiOptionSelectorAction, TuiOptionSelectorEvent,
};
use crate::tui_builder::TuiUiBuilder;

const PERMISSION_PROMPT_ACTIVE: &str = "TuiPermissionPromptActive";
const YES_ID: &str = "yes";
const NO_ID: &str = "no";
const EDIT_ID: &str = "edit";

/// Registers controls used while a permission prompt owns focus.
pub(crate) fn init(app: &mut AppContext) {
    let predicate = id!(TuiPermissionPrompt::ui_name()) & id!(PERMISSION_PROMPT_ACTIVE);
    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        TuiPermissionPromptAction::CancelOrBack,
        predicate.clone(),
    )
    .with_group(TUI_BINDING_GROUP)]);
    app.register_editable_bindings([
        EditableBinding::new(
            "tui:permission-prompt:confirm",
            "Confirm the selected permission response",
            TuiPermissionPromptAction::Confirm,
        )
        .with_context_predicate(predicate.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("enter"),
        EditableBinding::new(
            "tui:permission-prompt:previous",
            "Select the previous permission response",
            TuiPermissionPromptAction::MoveUp,
        )
        .with_context_predicate(predicate.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("up"),
        EditableBinding::new(
            "tui:permission-prompt:next",
            "Select the next permission response",
            TuiPermissionPromptAction::MoveDown,
        )
        .with_context_predicate(predicate.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("down"),
        EditableBinding::new(
            "tui:permission-prompt:edit",
            "Edit or save the requested action",
            TuiPermissionPromptAction::EditBody,
        )
        .with_context_predicate(predicate)
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("ctrl-e"),
    ]);
    app.register_tui_binding_validator::<TuiPermissionPrompt>(is_tui_owned_binding);
}

/// Actions handled by the focused permission prompt.
#[derive(Clone, Debug)]
pub(crate) enum TuiPermissionPromptAction {
    /// Confirms the highlighted response.
    Confirm,
    /// Moves to the previous response, cycling through the optional body editor.
    MoveUp,
    /// Moves to the next response, cycling through the optional body editor.
    MoveDown,
    /// Focuses the optional editable action body.
    EditBody,
    /// Unwinds Other editing, otherwise rejects the request.
    CancelOrBack,
}

/// Events emitted to the tool-specific host view.
pub(crate) enum TuiPermissionPromptEvent {
    /// The user selected Yes.
    AcceptRequested,
    /// The user submitted replacement guidance from Other.
    ReplacementGuidanceSubmitted(String),
    /// The user selected No or cancelled the request.
    RejectRequested,
    /// The underlying action entered or left its blocking state.
    BlockingStateChanged,
    /// Selector content changed intrinsic height.
    LayoutChanged,
}

/// Reusable selector for one blocked tool action.
pub(crate) struct TuiPermissionPrompt {
    action_model: ModelHandle<BlocklistAIActionModel>,
    action_id: AIAgentActionId,
    selector: ViewHandle<TuiOptionSelector>,
    body_editor: Option<ViewHandle<TuiEditorView>>,
}

impl TuiPermissionPrompt {
    /// Creates the prompt and its retained option selector.
    pub(crate) fn new(
        action_model: ModelHandle<BlocklistAIActionModel>,
        action_id: AIAgentActionId,
        body_editor: Option<ViewHandle<TuiEditorView>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let selector = ctx.add_typed_action_tui_view(TuiOptionSelector::new);
        selector.update(ctx, |selector, ctx| {
            if let Some(body_editor) = body_editor.as_ref() {
                selector.set_leading_editor(body_editor.clone(), ctx);
            }
            let mut rows = vec![
                OptionRow {
                    id: YES_ID.to_owned(),
                    label: "yes".to_owned(),
                    harness: None,
                    badge: None,
                    disabled_reason: None,
                },
                OptionRow {
                    id: NO_ID.to_owned(),
                    label: "no".to_owned(),
                    harness: None,
                    badge: None,
                    disabled_reason: None,
                },
            ];
            if body_editor.is_some() {
                rows.push(OptionRow {
                    id: EDIT_ID.to_owned(),
                    label: "edit command".to_owned(),
                    harness: None,
                    badge: None,
                    disabled_reason: None,
                });
            }
            selector.set_page(
                OptionSelectorPage {
                    header: None,
                    snapshot: OptionSnapshot {
                        rows,
                        selected_id: Some(YES_ID.to_owned()),
                        status: OptionSourceStatus::Ready,
                        footer: body_editor.is_none().then(|| OptionFooter::CustomText {
                            label: "Other".to_owned(),
                        }),
                    },
                    searchable: false,
                    row_shortcuts: if body_editor.is_some() {
                        HashMap::from([(EDIT_ID.to_owned(), 'e')])
                    } else {
                        Default::default()
                    },
                },
                ctx,
            );
        });

        ctx.subscribe_to_view(&selector, |prompt, _, event, ctx| {
            prompt.handle_selector_event(event, ctx);
        });
        ctx.subscribe_to_model(&action_model, |prompt, _, event, ctx| {
            if event.action_id() != &prompt.action_id {
                return;
            }
            if matches!(
                event,
                BlocklistAIActionEvent::ActionBlockedOnUserConfirmation(_)
            ) {
                prompt.focus(ctx);
            }
            ctx.emit(TuiPermissionPromptEvent::BlockingStateChanged);
            prompt.invalidate_layout(ctx);
        });

        Self {
            action_model,
            action_id,
            selector,
            body_editor,
        }
    }

    /// Returns whether the associated action currently awaits confirmation.
    pub(crate) fn is_active(&self, app: &AppContext) -> bool {
        self.action_model
            .as_ref(app)
            .get_action_status(&self.action_id)
            .is_some_and(|status| status.is_blocked())
    }

    /// Focuses the option selector.
    pub(crate) fn focus(&self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.selector);
    }

    /// Restores Yes as the highlighted response after body editing.
    pub(crate) fn restore_options_focus(&self, ctx: &mut ViewContext<Self>) {
        self.selector.update(ctx, |selector, ctx| {
            selector.handle_action(&TuiOptionSelectorAction::SelectItemWithoutConfirm(0), ctx)
        });
    }
    #[cfg(test)]
    pub(crate) fn highlighted_index(&self, app: &AppContext) -> Option<usize> {
        self.selector.as_ref(app).highlighted_index()
    }

    /// Updates validation copy rendered below the optional body editor.
    pub(crate) fn set_body_error(&self, error: Option<String>, ctx: &mut ViewContext<Self>) {
        self.selector.update(ctx, |selector, ctx| {
            selector.set_leading_editor_error(error, ctx)
        });
    }
    /// Translates selector outcomes into tool-host permission events.
    fn handle_selector_event(
        &mut self,
        event: &TuiOptionSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TuiOptionSelectorEvent::Confirmed { id } if id == YES_ID => {
                ctx.emit(TuiPermissionPromptEvent::AcceptRequested);
            }
            TuiOptionSelectorEvent::Confirmed { id } if id == NO_ID => {
                ctx.emit(TuiPermissionPromptEvent::RejectRequested);
            }
            TuiOptionSelectorEvent::Confirmed { id } if id == EDIT_ID => {
                self.selector
                    .update(ctx, |selector, ctx| selector.focus_leading_editor(ctx));
            }
            TuiOptionSelectorEvent::CustomTextSubmitted { value } => {
                ctx.emit(TuiPermissionPromptEvent::ReplacementGuidanceSubmitted(
                    value.clone(),
                ));
            }
            TuiOptionSelectorEvent::Dismissed => {
                ctx.emit(TuiPermissionPromptEvent::RejectRequested);
            }
            TuiOptionSelectorEvent::LayoutInvalidated
            | TuiOptionSelectorEvent::CustomTextOpened => self.invalidate_layout(ctx),
            TuiOptionSelectorEvent::Confirmed { .. } | TuiOptionSelectorEvent::RetryRequested => {}
        }
    }
    /// Requests remeasurement by the tool host.
    fn invalidate_layout(&self, ctx: &mut ViewContext<Self>) {
        ctx.emit(TuiPermissionPromptEvent::LayoutChanged);
        ctx.notify();
    }

    /// Renders the context-sensitive interaction hints beneath the options.
    pub(crate) fn render_footer(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);
        let mut spans = vec![
            ("Esc".to_owned(), builder.primary_text_style()),
            (" to cancel  ".to_owned(), builder.muted_text_style()),
        ];
        if self.body_editor.is_some() {
            spans.extend([
                ("Ctrl+E".to_owned(), builder.primary_text_style()),
                (" to edit/save  ".to_owned(), builder.muted_text_style()),
            ]);
        }
        spans.extend([
            ("Enter".to_owned(), builder.primary_text_style()),
            (" to run".to_owned(), builder.muted_text_style()),
        ]);
        TuiText::from_spans(spans).truncate().finish()
    }
}

/// Renders a full-width permission card around a tool-specific body.
pub(crate) fn render_permission_card(
    prompt: &ViewHandle<TuiPermissionPrompt>,
    title: impl Into<String>,
    body: Option<Box<dyn TuiElement>>,
    app: &AppContext,
) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(app);
    let header = TuiContainer::new(
        TuiText::from_spans([
            ("■ ".to_owned(), builder.attention_glyph_style()),
            (
                title.into(),
                builder.primary_text_style().add_modifier(Modifier::BOLD),
            ),
        ])
        .finish(),
    )
    .with_background(builder.permission_header_background())
    .with_padding_x(1)
    .finish();
    let mut body_content = TuiFlex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    if let Some(body) = body {
        body_content.add_child(body);
        body_content.add_child(TuiText::new(" ").finish());
    }
    body_content.add_child(TuiChildView::new(prompt).finish());
    let body = TuiContainer::new(body_content.finish())
        .with_background(builder.permission_surface_background())
        .with_padding_x(3)
        .with_padding_y(1)
        .finish();
    TuiFlex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(header)
        .child(body)
        .child(
            TuiContainer::new(prompt.as_ref(app).render_footer(app))
                .with_padding_top(1)
                .finish(),
        )
        .finish()
}

impl Entity for TuiPermissionPrompt {
    type Event = TuiPermissionPromptEvent;
}

impl TuiView for TuiPermissionPrompt {
    fn ui_name() -> &'static str {
        "TuiPermissionPrompt"
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<EntityId> {
        vec![self.selector.id()]
    }

    fn keymap_context(&self, app: &AppContext) -> warpui_core::keymap::Context {
        let mut context = Self::default_keymap_context();
        if self.is_active(app) {
            context.set.insert(PERMISSION_PROMPT_ACTIVE);
        }
        context
    }

    fn render(&self, _app: &AppContext) -> Box<dyn TuiElement> {
        TuiChildView::new(&self.selector).finish()
    }
}

impl TypedActionView for TuiPermissionPrompt {
    type Action = TuiPermissionPromptAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        if !self.is_active(ctx) {
            return;
        }
        match action {
            TuiPermissionPromptAction::Confirm => {
                let body_editor_focused = self
                    .body_editor
                    .as_ref()
                    .is_some_and(|editor| editor.as_ref(ctx).is_focused());
                self.selector.update(ctx, |selector, ctx| {
                    if body_editor_focused {
                        selector.handle_action(
                            &TuiOptionSelectorAction::SelectItemWithoutConfirm(0),
                            ctx,
                        );
                    } else {
                        selector.confirm_selected(ctx);
                    }
                });
            }
            TuiPermissionPromptAction::MoveUp => {
                self.selector
                    .update(ctx, |selector, ctx| selector.move_up(ctx));
            }
            TuiPermissionPromptAction::MoveDown => {
                self.selector
                    .update(ctx, |selector, ctx| selector.move_down(ctx));
            }
            TuiPermissionPromptAction::EditBody => {
                self.selector
                    .update(ctx, |selector, ctx| selector.focus_leading_editor(ctx));
            }
            TuiPermissionPromptAction::CancelOrBack => {
                let handled = self
                    .selector
                    .update(ctx, |selector, ctx| selector.handle_back(ctx));
                if !handled {
                    ctx.emit(TuiPermissionPromptEvent::RejectRequested);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tui_permission_prompt_tests.rs"]
mod tests;
