//! Stateful inline Markdown view for CreateDocuments and EditDocuments tool calls.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ai::agent::document_action_presentation::DocumentActionPresentation;
use markdown_parser::{parse_markdown_with_gfm_tables, FormattedText, FormattedTextLine};
use warp::tui_export::{
    AIAgentAction, AIAgentActionType, BlocklistAIActionEvent, BlocklistAIActionModel,
};
use warpui_core::elements::tui::{
    tui_collapsible, Modifier, TuiChildView, TuiContainer, TuiElement, TuiFlex, TuiParentElement,
    TuiText,
};
use warpui_core::elements::{CrossAxisAlignment, MouseStateHandle};
use warpui_core::{
    AppContext, Entity, EntityId, ModelHandle, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::agent_block_sections::tool_call_glyph_style;
use crate::tool_call_labels::{tool_call_display_state, tool_call_glyph, ToolCallDisplayState};
use crate::tui_builder::TuiUiBuilder;
use crate::tui_code_block_view::{TuiCodeBlockPayload, TuiCodeBlockView, TuiCodeBlockViewEvent};
use crate::tui_markdown::{render_formatted_text, TuiMarkdownBlockHooks, TuiMarkdownPalette};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TuiPlanCodeKey {
    document_index: usize,
    code_index: usize,
}

struct TuiPlanDocument {
    formatted: Option<Arc<FormattedText>>,
}

pub(super) enum TuiPlanViewEvent {
    LayoutChanged,
}

#[derive(Clone, Debug)]
pub(super) enum TuiPlanViewAction {
    SetCollapsed(bool),
}

pub(super) struct TuiPlanView {
    action: AIAgentAction,
    action_model: ModelHandle<BlocklistAIActionModel>,
    output_streaming: bool,
    presentation: DocumentActionPresentation,
    documents: Vec<TuiPlanDocument>,
    code_views: HashMap<TuiPlanCodeKey, ViewHandle<TuiCodeBlockView>>,
    collapsed: bool,
    header_mouse_state: MouseStateHandle,
}

impl TuiPlanView {
    pub(super) fn new(
        action: AIAgentAction,
        output_streaming: bool,
        action_model: &ModelHandle<BlocklistAIActionModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let presentation = DocumentActionPresentation::resolve(&action.action, None)
            .expect("TuiPlanView only supports document actions");
        let mut view = Self {
            action,
            action_model: action_model.clone(),
            output_streaming,
            presentation,
            documents: Vec::new(),
            code_views: HashMap::new(),
            collapsed: false,
            header_mouse_state: MouseStateHandle::default(),
        };
        view.sync_documents(ctx);

        ctx.subscribe_to_model(action_model, |me, _, event, ctx| {
            if matches!(
                event,
                BlocklistAIActionEvent::FinishedAction { action_id, .. }
                    if *action_id == me.action.id
            ) {
                me.sync_documents(ctx);
                me.invalidate_layout(ctx);
            }
        });
        view
    }

    pub(super) fn sync_action(
        &mut self,
        action: AIAgentAction,
        output_streaming: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let status_changed = self.output_streaming != output_streaming;
        let action_changed = self.action != action;
        self.action = action;
        self.output_streaming = output_streaming;
        let documents_changed = self.sync_documents(ctx);
        if status_changed || action_changed || documents_changed {
            self.invalidate_layout(ctx);
        }
    }

    pub(super) fn renders_rich_body(&self) -> bool {
        !self.documents.is_empty()
    }

    fn sync_documents(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let resolved = self.resolve_presentation(ctx);
        if self.presentation == resolved && self.documents.len() == resolved.documents.len() {
            return false;
        }

        self.documents = resolved
            .documents
            .iter()
            .map(|document| TuiPlanDocument {
                formatted: parse_markdown_with_gfm_tables(&document.content)
                    .ok()
                    .map(Arc::new),
            })
            .collect();
        self.presentation = resolved;
        self.sync_code_views(ctx);
        true
    }

    fn resolve_presentation(&self, app: &AppContext) -> DocumentActionPresentation {
        let result = self
            .action_model
            .as_ref(app)
            .get_action_result(&self.action.id)
            .map(|result| &result.result);
        DocumentActionPresentation::resolve(&self.action.action, result)
            .expect("TuiPlanView only supports document actions")
    }

    fn sync_code_views(&mut self, ctx: &mut ViewContext<Self>) {
        let mut descriptors = Vec::new();
        for (document_index, document) in self.documents.iter().enumerate() {
            let Some(formatted) = &document.formatted else {
                continue;
            };
            let mut code_index = 0;
            for line in &formatted.lines {
                if let FormattedTextLine::CodeBlock(code) = line {
                    descriptors.push((
                        TuiPlanCodeKey {
                            document_index,
                            code_index,
                        },
                        TuiCodeBlockPayload::new(
                            code.code.clone(),
                            (!code.lang.is_empty()).then(|| code.lang.clone()),
                        ),
                    ));
                    code_index += 1;
                }
            }
        }

        let active_keys = descriptors
            .iter()
            .map(|(key, _)| *key)
            .collect::<HashSet<_>>();
        self.code_views.retain(|key, _| active_keys.contains(key));

        for (key, payload) in descriptors {
            if let Some(view) = self.code_views.get(&key) {
                view.update(ctx, |view, ctx| {
                    view.sync(payload, ctx);
                });
                continue;
            }
            let view = ctx.add_tui_view(move |ctx| TuiCodeBlockView::new(payload, ctx));
            ctx.subscribe_to_view(&view, |me, _, event, ctx| match event {
                TuiCodeBlockViewEvent::LayoutChanged | TuiCodeBlockViewEvent::SyntaxUpdated => {
                    me.invalidate_layout(ctx)
                }
            });
            self.code_views.insert(key, view);
        }
    }

    fn display_state(&self, app: &AppContext) -> ToolCallDisplayState {
        let status = self
            .action_model
            .as_ref(app)
            .get_action_status(&self.action.id);
        tool_call_display_state(status.as_ref(), self.output_streaming, None)
    }

    fn document_subject(&self) -> String {
        if self.presentation.documents.len() == 1 {
            self.presentation.documents[0].title.clone()
        } else {
            format!("{} documents", self.presentation.documents.len())
        }
    }

    fn header_label(&self, state: ToolCallDisplayState) -> (&'static str, Option<String>) {
        if matches!(&self.action.action, AIAgentActionType::CreateDocuments(_)) {
            match state {
                ToolCallDisplayState::Constructing | ToolCallDisplayState::Running => {
                    ("Creating ", Some(self.document_subject()))
                }
                ToolCallDisplayState::Pending | ToolCallDisplayState::AwaitingApproval => {
                    ("Create plan", None)
                }
                ToolCallDisplayState::Succeeded => ("Created ", Some(self.document_subject())),
                ToolCallDisplayState::Failed => ("Failed to create plan", None),
                ToolCallDisplayState::Cancelled => ("Create plan cancelled", None),
            }
        } else {
            debug_assert!(matches!(
                &self.action.action,
                AIAgentActionType::EditDocuments(_)
            ));
            match state {
                ToolCallDisplayState::Constructing | ToolCallDisplayState::Running => {
                    ("Updating plan", None)
                }
                ToolCallDisplayState::Pending | ToolCallDisplayState::AwaitingApproval => {
                    ("Update plan", None)
                }
                ToolCallDisplayState::Succeeded => ("Updated plan", None),
                ToolCallDisplayState::Failed => ("Failed to update plan", None),
                ToolCallDisplayState::Cancelled => ("Update plan cancelled", None),
            }
        }
    }
    fn render_documents(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);
        let palette = TuiMarkdownPalette::from_builder(&builder);
        let mut documents =
            TuiFlex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        for (document_index, document) in self.documents.iter().enumerate() {
            if document_index > 0 {
                documents.add_child(TuiText::new(" ").truncate().finish());
            }
            if self.documents.len() > 1 {
                documents.add_child(
                    TuiText::new(self.presentation.documents[document_index].title.clone())
                        .with_style(builder.primary_text_style().add_modifier(Modifier::BOLD))
                        .finish(),
                );
            }
            let content = match &document.formatted {
                Some(formatted) => {
                    let render_code =
                        |code_index: usize, _code: &markdown_parser::CodeBlockText| {
                            self.code_views
                                .get(&TuiPlanCodeKey {
                                    document_index,
                                    code_index,
                                })
                                .map(|view| TuiChildView::new(view).finish())
                        };
                    render_formatted_text(
                        formatted,
                        palette,
                        &TuiMarkdownBlockHooks {
                            render_code: Some(&render_code),
                        },
                    )
                }
                None => TuiText::new(self.presentation.documents[document_index].content.clone())
                    .with_style(palette.body)
                    .finish(),
            };
            documents.add_child(content);
        }

        TuiContainer::new(documents.finish())
            .with_padding_left(2)
            .with_padding_y(1)
            .with_background(builder.plan_background())
            .finish()
    }

    fn invalidate_layout(&self, ctx: &mut ViewContext<Self>) {
        ctx.emit(TuiPlanViewEvent::LayoutChanged);
        ctx.notify();
    }
}

impl Entity for TuiPlanView {
    type Event = TuiPlanViewEvent;
}

impl TuiView for TuiPlanView {
    fn ui_name() -> &'static str {
        "TuiPlanView"
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<EntityId> {
        self.code_views.values().map(|view| view.id()).collect()
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);
        let state = self.display_state(app);
        let header_style = builder.primary_text_style().add_modifier(Modifier::BOLD);
        let (label, subject) = self.header_label(state);
        let mut header = vec![
            (
                format!("{} ", tool_call_glyph(state)),
                tool_call_glyph_style(state, &builder),
            ),
            (label.to_owned(), header_style),
        ];
        if let Some(subject) = subject {
            header.push((
                subject,
                builder.link_text_style().add_modifier(Modifier::BOLD),
            ));
        }
        let collapsed = self.collapsed;
        tui_collapsible(
            collapsed,
            header,
            header_style,
            self.header_mouse_state.clone(),
            || self.render_documents(app),
            move |event_ctx, _app| {
                event_ctx.dispatch_typed_action(TuiPlanViewAction::SetCollapsed(!collapsed));
            },
        )
    }
}

impl TypedActionView for TuiPlanView {
    type Action = TuiPlanViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TuiPlanViewAction::SetCollapsed(collapsed) => {
                self.collapsed = *collapsed;
                self.invalidate_layout(ctx);
            }
        }
    }
}

#[cfg(test)]
#[path = "tui_plan_view_tests.rs"]
mod tests;
