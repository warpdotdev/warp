//! Local TUI questionnaire for configuring the bottom statusline.

use std::collections::{HashMap, HashSet};

use warp::settings::{TuiStatuslineConfig, TuiStatuslineItem};
use warp::tui_export::{
    AskUserQuestionAction, AskUserQuestionItem, AskUserQuestionOption, AskUserQuestionSession,
    AskUserQuestionType, OptionRow, OptionSnapshot, OptionSourceStatus, QuestionDraft,
};
use warpui_core::elements::tui::{
    Modifier, TuiChildView, TuiContainer, TuiElement, TuiFlex, TuiText,
};
use warpui_core::keymap::macros::*;
use warpui_core::keymap::{EditableBinding, FixedBinding};
use warpui_core::{
    AppContext, Entity, EntityId, TuiView, TypedActionView, ViewContext, ViewHandle,
};

use crate::keybindings::{TUI_BINDING_GROUP, is_tui_owned_binding};
use crate::option_selector::{
    OptionSelectorPage, TuiOptionSelector, TuiOptionSelectorEvent, TuiOptionSelectorMoveDirection,
};
use crate::tui_builder::TuiUiBuilder;

const STATUSLINE_CONFIG_ACTIVE: &str = "TuiStatuslineConfigActive";
const STATUSLINE_REORDER_ACTIVE: &str = "TuiStatuslineReorderActive";
// The next stacked change mounts the picker and consumes this identifier.
#[allow(dead_code)]
const STATUSLINE_QUESTION_ID: &str = "statusline-items";

pub(crate) fn init(app: &mut AppContext) {
    let active = id!(TuiStatuslineConfigView::ui_name()) & id!(STATUSLINE_CONFIG_ACTIVE);
    let reorder = active.clone() & id!(STATUSLINE_REORDER_ACTIVE);
    app.register_fixed_bindings([FixedBinding::new(
        "ctrl-c",
        TuiStatuslineConfigAction::Cancel,
        active.clone(),
    )
    .with_group(TUI_BINDING_GROUP)]);
    app.register_editable_bindings([
        EditableBinding::new(
            "tui:statusline:toggle",
            "Toggle the highlighted statusline item",
            TuiStatuslineConfigAction::Toggle,
        )
        .with_context_predicate(active.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("enter"),
        EditableBinding::new(
            "tui:statusline:save",
            "Save and close the statusline configuration",
            TuiStatuslineConfigAction::Save,
        )
        .with_context_predicate(active)
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("escape"),
        EditableBinding::new(
            "tui:statusline:move_left",
            "Move the highlighted statusline item left",
            TuiStatuslineConfigAction::MoveBackward,
        )
        .with_context_predicate(reorder.clone())
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("left"),
        EditableBinding::new(
            "tui:statusline:move_right",
            "Move the highlighted statusline item right",
            TuiStatuslineConfigAction::MoveForward,
        )
        .with_context_predicate(reorder)
        .with_group(TUI_BINDING_GROUP)
        .with_key_binding("right"),
    ]);
    app.register_tui_binding_validator::<TuiStatuslineConfigView>(is_tui_owned_binding);
}

#[derive(Clone, Debug)]
pub(crate) enum TuiStatuslineConfigAction {
    Toggle,
    Save,
    Cancel,
    MoveBackward,
    MoveForward,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TuiStatuslineConfigEvent {
    Saved(TuiStatuslineConfig),
    Cancelled,
    // The next stacked change mounts the picker and handles layout updates.
    #[allow(dead_code)]
    LayoutChanged,
}

pub(crate) struct TuiStatuslineConfigView {
    session: AskUserQuestionSession,
    selector: ViewHandle<TuiOptionSelector>,
}

// The next stacked change mounts the picker and consumes these lifecycle helpers.
#[allow(dead_code)]
impl TuiStatuslineConfigView {
    pub(crate) fn new(config: TuiStatuslineConfig, ctx: &mut ViewContext<Self>) -> Self {
        let config = config.normalized();
        let question = statusline_question();
        let selected_option_indices = config
            .enabled
            .iter()
            .filter_map(|item| {
                TuiStatuslineItem::ALL
                    .iter()
                    .position(|candidate| candidate == item)
            })
            .collect();
        let drafts = HashMap::from([(
            STATUSLINE_QUESTION_ID.to_owned(),
            QuestionDraft {
                selected_option_indices,
                ..Default::default()
            },
        )]);
        let session = AskUserQuestionSession::new_with_drafts(vec![question], drafts);
        let selector = ctx.add_typed_action_tui_view(TuiOptionSelector::new);
        let mut view = Self {
            session,
            selector: selector.clone(),
        };
        view.show_options(&config.order, ctx);
        ctx.subscribe_to_view(&selector, |view, _, event, ctx| {
            view.handle_selector_event(event, ctx);
        });
        ctx.focus(&selector);
        view
    }

    pub(crate) fn focus(&self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.selector);
    }

    fn show_options(&mut self, order: &[TuiStatuslineItem], ctx: &mut ViewContext<Self>) {
        let rows = order
            .iter()
            .filter_map(|item| {
                TuiStatuslineItem::ALL
                    .iter()
                    .position(|candidate| candidate == item)
                    .map(|index| OptionRow {
                        id: index.to_string(),
                        label: item.label().to_owned(),
                        harness: None,
                        badge: None,
                        disabled_reason: None,
                    })
            })
            .collect::<Vec<_>>();
        let selected_id = rows.first().map(|row| row.id.clone());
        self.selector.update(ctx, |selector, ctx| {
            selector.set_page(
                OptionSelectorPage {
                    header: None,
                    snapshot: OptionSnapshot {
                        rows,
                        selected_id,
                        status: OptionSourceStatus::Ready,
                        footer: None,
                    },
                    searchable: true,
                    row_shortcuts: Default::default(),
                },
                ctx,
            );
        });
        self.refresh_selection(ctx);
    }

    fn refresh_selection(&self, ctx: &mut ViewContext<Self>) {
        let selected_ids = self
            .session
            .draft_for_question(0)
            .map(|draft| {
                draft
                    .selected_option_indices
                    .iter()
                    .map(usize::to_string)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        self.selector.update(ctx, |selector, ctx| {
            selector.set_question_state(selected_ids, true, ctx);
        });
    }

    fn handle_selector_event(
        &mut self,
        event: &TuiOptionSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            TuiOptionSelectorEvent::Confirmed { id } => {
                let Ok(option_index) = id.parse::<usize>() else {
                    return;
                };
                let _ = self
                    .session
                    .apply(AskUserQuestionAction::ToggleOption { option_index });
                self.refresh_selection(ctx);
            }
            TuiOptionSelectorEvent::LayoutInvalidated
            | TuiOptionSelectorEvent::RowsReordered { .. } => {
                ctx.emit(TuiStatuslineConfigEvent::LayoutChanged);
                ctx.notify();
            }
            TuiOptionSelectorEvent::Dismissed => {
                ctx.emit(TuiStatuslineConfigEvent::Cancelled);
            }
            TuiOptionSelectorEvent::CustomTextSubmitted { .. }
            | TuiOptionSelectorEvent::CustomTextCleared
            | TuiOptionSelectorEvent::CustomTextOpened
            | TuiOptionSelectorEvent::CustomTextClosed
            | TuiOptionSelectorEvent::RetryRequested => {}
        }
    }

    fn current_config(&self, ctx: &AppContext) -> TuiStatuslineConfig {
        let order = self
            .selector
            .as_ref(ctx)
            .ordered_row_ids()
            .into_iter()
            .filter_map(|id| id.parse::<usize>().ok())
            .filter_map(|index| TuiStatuslineItem::ALL.get(index).copied())
            .collect::<Vec<_>>();
        let enabled_indices = self
            .session
            .draft_for_question(0)
            .map(|draft| &draft.selected_option_indices);
        let enabled = order
            .iter()
            .copied()
            .filter(|item| {
                TuiStatuslineItem::ALL
                    .iter()
                    .position(|candidate| candidate == item)
                    .is_some_and(|index| {
                        enabled_indices.is_some_and(|indices| indices.contains(&index))
                    })
            })
            .collect();
        TuiStatuslineConfig { order, enabled }.normalized()
    }

    fn render_footer(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);
        TuiText::from_spans([
            ("Enter ".to_owned(), builder.primary_text_style()),
            ("to toggle  ".to_owned(), builder.muted_text_style()),
            ("Esc ".to_owned(), builder.primary_text_style()),
            ("to save and close  ".to_owned(), builder.muted_text_style()),
            ("← → ".to_owned(), builder.primary_text_style()),
            ("to reorder".to_owned(), builder.muted_text_style()),
        ])
        .truncate()
        .finish()
    }
}

// The next stacked change mounts the picker and builds this question.
#[allow(dead_code)]
fn statusline_question() -> AskUserQuestionItem {
    AskUserQuestionItem {
        question_id: STATUSLINE_QUESTION_ID.to_owned(),
        question: "Configure statusline".to_owned(),
        question_type: AskUserQuestionType::MultipleChoice {
            is_multiselect: true,
            options: TuiStatuslineItem::ALL
                .iter()
                .map(|item| AskUserQuestionOption {
                    label: item.label().to_owned(),
                    recommended: false,
                })
                .collect(),
            supports_other: false,
        },
    }
}

impl Entity for TuiStatuslineConfigView {
    type Event = TuiStatuslineConfigEvent;
}

impl TuiView for TuiStatuslineConfigView {
    fn ui_name() -> &'static str {
        "TuiStatuslineConfigView"
    }

    fn child_view_ids(&self, _app: &AppContext) -> Vec<EntityId> {
        vec![self.selector.id()]
    }

    fn keymap_context(&self, app: &AppContext) -> warpui_core::keymap::Context {
        let mut context = Self::default_keymap_context();
        context.set.insert(STATUSLINE_CONFIG_ACTIVE);
        if self.selector.as_ref(app).list_is_focused(app) {
            context.set.insert(STATUSLINE_REORDER_ACTIVE);
        }
        context
    }

    fn render(&self, app: &AppContext) -> Box<dyn TuiElement> {
        let builder = TuiUiBuilder::from_app(app);
        let title = TuiText::new("Configure statusline")
            .with_style(builder.primary_text_style().add_modifier(Modifier::BOLD))
            .finish();
        let body = TuiFlex::column()
            .child(title)
            .child(TuiChildView::new(&self.selector).finish())
            .child(TuiText::new(" ").finish())
            .child(self.render_footer(app))
            .finish();
        TuiContainer::new(body)
            .with_padding(1)
            .with_background(builder.question_surface_background())
            .finish()
    }
}

impl TypedActionView for TuiStatuslineConfigView {
    type Action = TuiStatuslineConfigAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TuiStatuslineConfigAction::Toggle => {
                self.selector
                    .update(ctx, |selector, ctx| selector.confirm_selected(ctx));
            }
            TuiStatuslineConfigAction::Save => {
                ctx.emit(TuiStatuslineConfigEvent::Saved(self.current_config(ctx)));
            }
            TuiStatuslineConfigAction::Cancel => {
                ctx.emit(TuiStatuslineConfigEvent::Cancelled);
            }
            TuiStatuslineConfigAction::MoveBackward => {
                self.selector.update(ctx, |selector, ctx| {
                    let Some(row_id) = selector.selected_row_id() else {
                        return;
                    };
                    selector.move_row(&row_id, TuiOptionSelectorMoveDirection::Backward, ctx);
                });
            }
            TuiStatuslineConfigAction::MoveForward => {
                self.selector.update(ctx, |selector, ctx| {
                    let Some(row_id) = selector.selected_row_id() else {
                        return;
                    };
                    selector.move_row(&row_id, TuiOptionSelectorMoveDirection::Forward, ctx);
                });
            }
        }
    }
}

#[cfg(test)]
#[path = "statusline_config_view_tests.rs"]
mod tests;
