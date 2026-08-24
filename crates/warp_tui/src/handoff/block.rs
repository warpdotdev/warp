//! TUI card for presenting and configuring a local-to-cloud handoff.
//!
//! The card renders each handoff phase, from initial acceptance and the
//! environment/model selectors through launch progress and the created cloud
//! run. It owns keyboard actions, focus transitions, selector coordination,
//! links, and layout invalidation for the embedded transcript surface.
//!
//! Handoff state, validation, environment-catalog updates, and asynchronous
//! execution remain in [`super::model::TuiHandoffModel`]. This module
//! translates that model state into terminal elements and forwards user intent
//! back to the model.

use std::cell::Cell;

use warp::tui_export::{AIConversationId, OZ_ENVIRONMENTS_URL};
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::elements::tui::{
    Modifier, TuiChildView, TuiConstraint, TuiContainer, TuiElement, TuiFlex, TuiLayoutContext,
    TuiSize, TuiText,
};
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{self, FixedBinding};
use warpui_core::{
    AppContext, Entity, EntityId, FocusContext, ModelHandle, TuiView, TypedActionView, ViewContext,
    ViewHandle,
};

use super::model::{
    TuiHandoffEditableState, TuiHandoffModel, TuiHandoffModelEvent, TuiHandoffPhase,
    TuiHandoffSelectorKind,
};
use crate::keybindings::TUI_BINDING_GROUP;
use crate::link::TuiLink;
use crate::option_selector::{
    OptionSelectorHeader, OptionSelectorPage, TuiOptionSelector, TuiOptionSelectorEvent,
};
use crate::transcript_view::BLOCK_TOP_PADDING_ROWS;
use crate::tui_ask_question_view::PageNavigationDirection;
use crate::tui_builder::TuiUiBuilder;
use crate::ui::horizontally_centered;

const HANDOFF_TITLE: &str = "Hand off to cloud";
const EMPTY_CONVERSATION_HANDOFF_EXPLANATION: &str =
    "The agent will work on this session in the cloud.";
const EXISTING_CONVERSATION_HANDOFF_EXPLANATION: &str = "The agent will continue working on your session in the cloud. You will be able to continue the conversation here at any point.";
const HANDOFF_PAGE_SEQUENCE: [TuiHandoffSelectorKind; 2] = [
    TuiHandoffSelectorKind::Environment,
    TuiHandoffSelectorKind::Model,
];
const ACCEPTANCE_CONTEXT_FLAG: &str = "TuiHandoffBlockAcceptance";
const CONFIGURING_CONTEXT_FLAG: &str = "TuiHandoffBlockConfiguring";
const NO_ENVIRONMENT_CONTEXT_FLAG: &str = "TuiHandoffBlockNoEnvironment";
const COMMITTED_CONTEXT_FLAG: &str = "TuiHandoffBlockCommitted";
const CREATED_CONTEXT_FLAG: &str = "TuiHandoffBlockCreated";

pub(crate) fn init(app: &mut AppContext) {
    let card = || id!(TuiHandoffBlock::ui_name());
    let acceptance = || card() & id!(ACCEPTANCE_CONTEXT_FLAG);
    let configuring = || card() & id!(CONFIGURING_CONTEXT_FLAG);
    let no_environment = || card() & id!(NO_ENVIRONMENT_CONTEXT_FLAG);
    let committed = || card() & id!(COMMITTED_CONTEXT_FLAG);
    let created = || card() & id!(CREATED_CONTEXT_FLAG);
    app.register_fixed_bindings([
        FixedBinding::new("enter", TuiHandoffBlockAction::Confirm, acceptance())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("numpadenter", TuiHandoffBlockAction::Confirm, acceptance())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("ctrl-e", TuiHandoffBlockAction::Configure, acceptance())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "enter",
            TuiHandoffBlockAction::OpenEnvironments,
            no_environment(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "numpadenter",
            TuiHandoffBlockAction::OpenEnvironments,
            no_environment(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "r",
            TuiHandoffBlockAction::RefreshEnvironments,
            no_environment(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("escape", TuiHandoffBlockAction::Back, configuring())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "left",
            TuiHandoffBlockAction::CommitAndPreviousPage,
            configuring(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "right",
            TuiHandoffBlockAction::CommitAndNextPage,
            configuring(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("tab", TuiHandoffBlockAction::NextPage, configuring())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "ctrl-c",
            TuiHandoffBlockAction::Cancel,
            acceptance() | configuring() | no_environment(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new(
            "ctrl-c",
            TuiHandoffBlockAction::ConsumeInterrupt,
            committed(),
        )
        .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("escape", TuiHandoffBlockAction::Cancel, committed())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("enter", TuiHandoffBlockAction::OpenRun, created())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("numpadenter", TuiHandoffBlockAction::OpenRun, created())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("c", TuiHandoffBlockAction::ContinueLocally, created())
            .with_group(TUI_BINDING_GROUP),
        FixedBinding::new("n", TuiHandoffBlockAction::StartNewConversation, created())
            .with_group(TUI_BINDING_GROUP),
    ]);
}

/// Events owned by the view rather than the handoff model.
#[derive(Clone)]
pub(crate) enum TuiHandoffBlockEvent {
    LayoutInvalidated,
}

/// Keyboard actions supported by the handoff card.
#[derive(Clone, Debug)]
pub(crate) enum TuiHandoffBlockAction {
    Confirm,
    Configure,
    CommitAndPreviousPage,
    CommitAndNextPage,
    NextPage,
    OpenEnvironments,
    RefreshEnvironments,
    Back,
    Cancel,
    ConsumeInterrupt,
    OpenRun,
    ContinueLocally,
    StartNewConversation,
}

/// Keyboard-focused presentation for a [`TuiHandoffModel`].
pub(crate) struct TuiHandoffBlock {
    model: ModelHandle<TuiHandoffModel>,
    selector: ViewHandle<TuiOptionSelector>,
    pending_page_navigation: Option<PageNavigationDirection>,
    link: TuiLink,
    last_measured_width: Cell<Option<u16>>,
}

impl TuiHandoffBlock {
    pub(crate) fn new(model: ModelHandle<TuiHandoffModel>, ctx: &mut ViewContext<Self>) -> Self {
        let selector = ctx.add_typed_action_tui_view(TuiOptionSelector::new);
        ctx.subscribe_to_view(&selector, |block, _, event, ctx| {
            block.handle_selector_event(event, ctx);
        });
        ctx.subscribe_to_model(&model, |block, _, event, ctx| {
            block.handle_model_event(event, ctx);
        });
        Self {
            model,
            selector,
            pending_page_navigation: None,
            link: TuiLink::default(),
            last_measured_width: Cell::new(None),
        }
    }

    pub(crate) fn is_active(&self, ctx: &AppContext) -> bool {
        self.model.as_ref(ctx).is_active()
    }

    pub(crate) fn source_conversation_id(&self, ctx: &AppContext) -> Option<AIConversationId> {
        self.model.as_ref(ctx).source_conversation_id()
    }

    fn handle_model_event(&mut self, event: &TuiHandoffModelEvent, ctx: &mut ViewContext<Self>) {
        if let TuiHandoffModelEvent::Changed { .. } = event {
            self.refresh_selector(ctx);
            ctx.notify();
        }
    }

    fn finish_page_confirmation(
        &mut self,
        page: TuiHandoffSelectorKind,
        ctx: &mut ViewContext<Self>,
    ) {
        let sequence = HANDOFF_PAGE_SEQUENCE;
        let Some(index) = sequence.iter().position(|candidate| *candidate == page) else {
            self.return_to_acceptance(ctx);
            return;
        };
        let navigation = self.pending_page_navigation.take();
        let target = match navigation {
            Some(PageNavigationDirection::Previous) => {
                index.checked_sub(1).and_then(|index| sequence.get(index))
            }
            Some(PageNavigationDirection::Next) | None => sequence.get(index + 1),
        };
        match target.copied() {
            Some(target) => self.open_page(target, ctx),
            None if navigation.is_some() => self.open_page(page, ctx),
            None => self.return_to_acceptance(ctx),
        }
    }

    fn navigate_page(&mut self, direction: PageNavigationDirection, ctx: &mut ViewContext<Self>) {
        let TuiHandoffPhase::Editable {
            state: TuiHandoffEditableState::Configuring { page },
            ..
        } = self.model.as_ref(ctx).phase()
        else {
            return;
        };
        let page = *page;
        let sequence = HANDOFF_PAGE_SEQUENCE;
        let Some(index) = sequence.iter().position(|candidate| *candidate == page) else {
            return;
        };
        let target = match direction {
            PageNavigationDirection::Previous => {
                index.checked_sub(1).and_then(|index| sequence.get(index))
            }
            PageNavigationDirection::Next => sequence.get(index + 1),
        };
        if let Some(target) = target.copied() {
            self.open_page(target, ctx);
        }
    }

    fn handle_configure(&mut self, ctx: &mut ViewContext<Self>) {
        if matches!(
            self.model.as_ref(ctx).phase(),
            TuiHandoffPhase::Editable {
                state: TuiHandoffEditableState::Acceptance { .. },
                ..
            }
        ) && !self.model.as_ref(ctx).no_environments(ctx)
        {
            self.open_page(TuiHandoffSelectorKind::Environment, ctx);
        }
    }

    fn handle_arrow_navigation(
        &mut self,
        navigation: PageNavigationDirection,
        ctx: &mut ViewContext<Self>,
    ) {
        self.pending_page_navigation = Some(navigation);
        let confirmation_started = self
            .selector
            .update(ctx, |selector, ctx| selector.confirm_selected(ctx));
        if !confirmation_started {
            self.pending_page_navigation = None;
            self.navigate_page(navigation, ctx);
        }
    }

    fn open_page(&mut self, page: TuiHandoffSelectorKind, ctx: &mut ViewContext<Self>) {
        let opened = self
            .model
            .update(ctx, |model, ctx| model.open_page(page, ctx));
        if !opened {
            return;
        }
        let snapshot = self.model.as_ref(ctx).selector_snapshot(page, ctx);
        let sequence = HANDOFF_PAGE_SEQUENCE;
        let position = sequence
            .iter()
            .position(|candidate| *candidate == page)
            .unwrap_or(0)
            + 1;
        self.selector.update(ctx, |selector, ctx| {
            selector.set_page(
                OptionSelectorPage {
                    header: Some(OptionSelectorHeader {
                        field_label: "Edit handoff configuration".to_owned(),
                        position: (position, sequence.len()),
                        prompt: page.question().to_owned(),
                    }),
                    snapshot,
                    searchable: true,
                    row_shortcuts: Default::default(),
                },
                ctx,
            );
        });
        ctx.focus(&self.selector);
        ctx.notify();
    }

    fn return_to_acceptance(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.return_to_acceptance(ctx);
        });
        self.pending_page_navigation = None;
        ctx.focus_self();
        ctx.notify();
    }

    fn refresh_selector(&mut self, ctx: &mut ViewContext<Self>) {
        let TuiHandoffPhase::Editable {
            state: TuiHandoffEditableState::Configuring { page },
            ..
        } = self.model.as_ref(ctx).phase()
        else {
            return;
        };
        let snapshot = self.model.as_ref(ctx).selector_snapshot(*page, ctx);
        self.selector.update(ctx, |selector, ctx| {
            selector.refresh_snapshot(snapshot, ctx);
        });
    }

    fn handle_selector_event(
        &mut self,
        event: &TuiOptionSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TuiOptionSelectorEvent::Confirmed { id } => {
                let TuiHandoffPhase::Editable {
                    state: TuiHandoffEditableState::Configuring { page },
                    ..
                } = self.model.as_ref(ctx).phase()
                else {
                    return;
                };
                let page = *page;
                let applied = self
                    .model
                    .update(ctx, |model, ctx| model.apply_selection(page, id, ctx));
                if applied {
                    self.finish_page_confirmation(page, ctx);
                }
            }
            TuiOptionSelectorEvent::Dismissed => self.return_to_acceptance(ctx),
            TuiOptionSelectorEvent::CustomTextSubmitted { .. }
            | TuiOptionSelectorEvent::RowsReordered { .. }
            | TuiOptionSelectorEvent::CustomTextCleared
            | TuiOptionSelectorEvent::CustomTextOpened
            | TuiOptionSelectorEvent::CustomTextClosed
            | TuiOptionSelectorEvent::RetryRequested => {}
            TuiOptionSelectorEvent::LayoutInvalidated => {
                ctx.emit(TuiHandoffBlockEvent::LayoutInvalidated);
            }
        }
    }

    fn confirm(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| model.confirm(ctx));
        if matches!(
            self.model.as_ref(ctx).phase(),
            TuiHandoffPhase::Committed { .. }
        ) {
            ctx.focus_self();
        }
        ctx.notify();
    }

    fn handle_back(&mut self, ctx: &mut ViewContext<Self>) {
        self.pending_page_navigation = None;
        let handled = self
            .selector
            .update(ctx, |selector, ctx| selector.handle_back(ctx));
        if !handled {
            self.return_to_acceptance(ctx);
        }
    }

    fn render_configuration(
        &self,
        ctx: &AppContext,
        builder: &TuiUiBuilder,
    ) -> Box<dyn TuiElement> {
        let model = self.model.as_ref(ctx);
        let explanation = if model.forked_existing_conversation() {
            EXISTING_CONVERSATION_HANDOFF_EXPLANATION
        } else {
            EMPTY_CONVERSATION_HANDOFF_EXPLANATION
        };
        let explanation_style = builder.primary_text_style().add_modifier(Modifier::BOLD);
        if model.no_environments(ctx) {
            return TuiFlex::column()
                .child(
                    TuiText::new(explanation)
                        .with_style(explanation_style)
                        .finish(),
                )
                .child(TuiText::new(" ").finish())
                .child(
                    TuiText::new("A cloud environment is required to hand off this conversation.")
                        .with_style(builder.primary_text_style())
                        .finish(),
                )
                .child(
                    TuiText::new("Create one in Oz, then refresh this card.")
                        .with_style(builder.muted_text_style())
                        .finish(),
                )
                .finish();
        }
        let mut content = TuiFlex::column()
            .child(
                TuiText::new(explanation)
                    .with_style(explanation_style)
                    .finish(),
            )
            .child(TuiText::new(" ").finish())
            .child(render_metadata_line(
                model.environment_label(ctx),
                model.model_label(ctx),
                builder,
            ));
        if let Some(error) = model.validation_error() {
            content = content.child(
                TuiText::new(error.to_owned())
                    .with_style(builder.error_text_style())
                    .finish(),
            );
        }
        content.finish()
    }

    fn render_body(&self, ctx: &AppContext, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
        match self.model.as_ref(ctx).phase() {
            TuiHandoffPhase::Editable {
                state: TuiHandoffEditableState::Acceptance { .. },
                ..
            } => self.render_configuration(ctx, builder),
            TuiHandoffPhase::Editable {
                state: TuiHandoffEditableState::Configuring { .. },
                ..
            } => TuiChildView::new(&self.selector).finish(),
            TuiHandoffPhase::Committed { .. } => TuiText::from_spans([
                ("● ".to_owned(), builder.attention_glyph_style()),
                (
                    "Creating cloud run…".to_owned(),
                    builder.primary_text_style(),
                ),
            ])
            .finish(),
            TuiHandoffPhase::Created { url, .. } => TuiFlex::column()
                .child(
                    TuiText::new("Cloud run created.")
                        .with_style(builder.primary_text_style())
                        .finish(),
                )
                .child(self.link.render(
                    url.clone(),
                    builder.muted_text_style(),
                    move |event_ctx, _| {
                        event_ctx.dispatch_typed_action(TuiHandoffBlockAction::OpenRun);
                    },
                ))
                .finish(),
            TuiHandoffPhase::Persisted { .. } => TuiFlex::column().finish(),
        }
    }

    fn render_footer(&self, ctx: &AppContext, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
        let model = self.model.as_ref(ctx);
        let spans = match model.phase() {
            TuiHandoffPhase::Editable {
                state: TuiHandoffEditableState::Acceptance { .. },
                ..
            } if model.no_environments(ctx) => vec![
                ("Enter ".to_owned(), builder.primary_text_style()),
                ("open environments  ".to_owned(), builder.muted_text_style()),
                ("R ".to_owned(), builder.primary_text_style()),
                ("refresh  ".to_owned(), builder.muted_text_style()),
                ("Ctrl + C".to_owned(), builder.primary_text_style()),
                (" to cancel".to_owned(), builder.muted_text_style()),
            ],
            TuiHandoffPhase::Editable {
                state: TuiHandoffEditableState::Acceptance { .. },
                ..
            } => vec![
                ("Enter ".to_owned(), builder.primary_text_style()),
                ("to hand off  ".to_owned(), builder.muted_text_style()),
                ("Ctrl + E".to_owned(), builder.primary_text_style()),
                (" to edit  ".to_owned(), builder.muted_text_style()),
                ("Ctrl + C".to_owned(), builder.primary_text_style()),
                (" to cancel".to_owned(), builder.muted_text_style()),
            ],
            TuiHandoffPhase::Editable {
                state: TuiHandoffEditableState::Configuring { .. },
                ..
            } => vec![
                ("Enter ".to_owned(), builder.primary_text_style()),
                ("to accept  ".to_owned(), builder.muted_text_style()),
                ("Tab or ← →".to_owned(), builder.primary_text_style()),
                (" to navigate  ".to_owned(), builder.muted_text_style()),
                ("Esc ".to_owned(), builder.primary_text_style()),
                ("to go back".to_owned(), builder.muted_text_style()),
            ],
            TuiHandoffPhase::Committed { .. } => {
                vec![("esc to cancel".to_owned(), builder.muted_text_style())]
            }
            TuiHandoffPhase::Created { .. } => {
                let mut spans = vec![
                    ("Enter ".to_owned(), builder.primary_text_style()),
                    ("open cloud run  ".to_owned(), builder.muted_text_style()),
                ];
                if model.forked_existing_conversation() {
                    spans.extend([
                        ("C ".to_owned(), builder.primary_text_style()),
                        ("continue locally  ".to_owned(), builder.muted_text_style()),
                    ]);
                }
                spans.extend([
                    ("N ".to_owned(), builder.primary_text_style()),
                    ("new conversation".to_owned(), builder.muted_text_style()),
                ]);
                spans
            }
            TuiHandoffPhase::Persisted { .. } => Vec::new(),
        };
        TuiText::from_spans(spans).finish()
    }

    fn render_completed(
        &self,
        url: &str,
        completed_at: &str,
        continuing_locally: bool,
        ctx: &AppContext,
    ) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(ctx);
        let content = TuiFlex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(horizontally_centered(
                TuiText::from_spans([
                    ("⟣ ".to_owned(), builder.option_selector_selected_style()),
                    (
                        format!(
                            "Conversation forked to cloud on {completed_at}{}",
                            if continuing_locally {
                                "; continuing locally"
                            } else {
                                ""
                            }
                        ),
                        builder.primary_text_style(),
                    ),
                ])
                .finish(),
            ))
            .child(horizontally_centered(self.link.render(
                url.to_owned(),
                builder.muted_text_style(),
                move |event_ctx, _| {
                    event_ctx.dispatch_typed_action(TuiHandoffBlockAction::OpenRun);
                },
            )))
            .finish();
        let banner = TuiContainer::new(content)
            .with_background(builder.orchestration_header_background())
            .finish();
        TuiContainer::new(banner)
            .with_padding_top(BLOCK_TOP_PADDING_ROWS)
            .finish()
    }

    pub(crate) fn needs_height_measurement(&self, width: u16) -> bool {
        self.last_measured_width.get() != Some(width)
    }

    pub(crate) fn record_height_measurement(&self, width: u16) {
        self.last_measured_width.set(Some(width));
    }

    pub(crate) fn desired_height(
        &self,
        width: u16,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> usize {
        let mut element = self.render(app);
        usize::from(
            element
                .layout(
                    TuiConstraint::loose(TuiSize::new(width, u16::MAX)),
                    ctx,
                    app,
                )
                .height,
        )
    }
}

fn render_metadata_line(
    environment: String,
    model: String,
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    TuiText::from_spans([
        ("Environment: ".to_owned(), builder.primary_text_style()),
        (environment, builder.orchestration_selected_value_style()),
        ("  •  ".to_owned(), builder.muted_text_style()),
        ("Model: ".to_owned(), builder.primary_text_style()),
        (model, builder.orchestration_selected_value_style()),
    ])
    .finish()
}

impl Entity for TuiHandoffBlock {
    type Event = TuiHandoffBlockEvent;
}

impl TuiView for TuiHandoffBlock {
    fn ui_name() -> &'static str {
        "TuiHandoffBlock"
    }

    fn child_view_ids(&self, _ctx: &AppContext) -> Vec<EntityId> {
        vec![self.selector.id()]
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused()
            && matches!(
                self.model.as_ref(ctx).phase(),
                TuiHandoffPhase::Editable {
                    state: TuiHandoffEditableState::Configuring { .. },
                    ..
                }
            )
        {
            ctx.focus(&self.selector);
        }
    }

    fn keymap_context(&self, ctx: &AppContext) -> keymap::Context {
        let mut context = keymap::Context::default();
        context.set.insert(Self::ui_name());
        let model = self.model.as_ref(ctx);
        match model.phase() {
            TuiHandoffPhase::Editable {
                state: TuiHandoffEditableState::Acceptance { .. },
                ..
            } if model.no_environments(ctx) => {
                context.set.insert(NO_ENVIRONMENT_CONTEXT_FLAG);
            }
            TuiHandoffPhase::Editable {
                state: TuiHandoffEditableState::Acceptance { .. },
                ..
            } => {
                context.set.insert(ACCEPTANCE_CONTEXT_FLAG);
            }
            TuiHandoffPhase::Editable {
                state: TuiHandoffEditableState::Configuring { .. },
                ..
            } => {
                context.set.insert(CONFIGURING_CONTEXT_FLAG);
            }
            TuiHandoffPhase::Committed { .. } => {
                context.set.insert(COMMITTED_CONTEXT_FLAG);
            }
            TuiHandoffPhase::Created { .. } => {
                context.set.insert(CREATED_CONTEXT_FLAG);
            }
            TuiHandoffPhase::Persisted { .. } => {}
        }
        context
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        if let TuiHandoffPhase::Persisted {
            url,
            completed_at,
            continuing_locally,
        } = self.model.as_ref(ctx).phase()
        {
            return self.render_completed(url, completed_at, *continuing_locally, ctx);
        }
        let builder = TuiUiBuilder::from_app(ctx);
        let header = TuiContainer::new(
            TuiText::from_spans([
                ("■ ".to_owned(), builder.option_selector_selected_style()),
                (HANDOFF_TITLE.to_owned(), builder.primary_text_style()),
            ])
            .finish(),
        )
        .with_background(builder.orchestration_header_background())
        .with_padding_x(1)
        .finish();
        let body = TuiContainer::new(self.render_body(ctx, &builder))
            .with_background(builder.orchestration_surface_background())
            .with_padding_x(3)
            .with_padding_y(1)
            .finish();
        TuiContainer::new(
            TuiFlex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(header)
                .child(body)
                .child(
                    TuiContainer::new(self.render_footer(ctx, &builder))
                        .with_padding_top(1)
                        .finish(),
                )
                .finish(),
        )
        .with_padding_top(BLOCK_TOP_PADDING_ROWS)
        .finish()
    }
}

impl TypedActionView for TuiHandoffBlock {
    type Action = TuiHandoffBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TuiHandoffBlockAction::Confirm => self.confirm(ctx),
            TuiHandoffBlockAction::Configure => self.handle_configure(ctx),
            TuiHandoffBlockAction::CommitAndPreviousPage => {
                self.handle_arrow_navigation(PageNavigationDirection::Previous, ctx)
            }
            TuiHandoffBlockAction::CommitAndNextPage => {
                self.handle_arrow_navigation(PageNavigationDirection::Next, ctx)
            }
            TuiHandoffBlockAction::NextPage => {
                self.navigate_page(PageNavigationDirection::Next, ctx)
            }
            TuiHandoffBlockAction::OpenEnvironments => ctx.open_url(OZ_ENVIRONMENTS_URL),
            TuiHandoffBlockAction::RefreshEnvironments => {
                self.model
                    .update(ctx, |model, ctx| model.refresh_environments(ctx));
            }
            TuiHandoffBlockAction::Back => self.handle_back(ctx),
            TuiHandoffBlockAction::Cancel => {
                self.model.update(ctx, |model, ctx| model.cancel(ctx));
            }
            TuiHandoffBlockAction::ConsumeInterrupt => {}
            TuiHandoffBlockAction::OpenRun => {
                if let Some(url) = self.model.as_ref(ctx).url() {
                    ctx.open_url(url);
                }
            }
            TuiHandoffBlockAction::ContinueLocally => {
                self.model
                    .update(ctx, |model, ctx| model.continue_locally(ctx));
                if matches!(
                    self.model.as_ref(ctx).phase(),
                    TuiHandoffPhase::Persisted { .. }
                ) {
                    self.last_measured_width.set(None);
                    ctx.emit(TuiHandoffBlockEvent::LayoutInvalidated);
                    ctx.notify();
                }
            }
            TuiHandoffBlockAction::StartNewConversation => {
                self.model
                    .update(ctx, |model, ctx| model.start_new_conversation(ctx));
            }
        }
    }
}

#[cfg(test)]
#[path = "block_tests.rs"]
mod tests;
