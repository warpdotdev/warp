use std::{borrow::Cow, mem, ops::Range, sync::Arc};

use string_offset::{ByteOffset, CharOffset};
use warp_completer::signatures::CommandRegistry;
use warp_editor::{
    content::{anchor::Anchor, buffer::Buffer, selection_model::BufferSelectionModel},
    editor::EmbeddedItemModel,
};
use warp_util::user_input::UserInput;
use warpui::{
    elements::{
        Align, Border, Container, CrossAxisAlignment, Empty, Flex, MainAxisAlignment,
        MouseStateHandle, ParentElement, Shrinkable,
    },
    platform::Cursor,
    ui_components::{button::ButtonVariant, components::UiComponent},
    AppContext, Element, Entity, ModelAsRef, ModelContext, ModelHandle, SingletonEntity,
};

use crate::{
    appearance::Appearance,
    cloud_object::{model::persistence::CloudModel, CloudObject},
    completer::SessionAgnosticContext,
    notebooks::{
        styles::block_footer_action_button,
        telemetry::{ActionEntrypoint, BlockInfo},
    },
    server::ids::{HashableId, ToServerId},
    settings::FontSettings,
    terminal::input::decorations::{parse_current_commands_and_tokens, ParsedTokensSnapshot},
    themes::theme::AnsiColorIdentifier,
    ui_components::icons::Icon,
    util::bindings::CustomAction,
    workflows::{CloudWorkflow, WorkflowId},
};

use super::{
    embedded_item::EmbeddedWorkflow,
    keys::{custom_action_to_display, NotebookKeybindings},
    model::ChildModelHandle,
    notebook_command::{parsed_token_to_color_style_ranges, transform_ansi_color_to_solid_color},
    rich_text_styles,
    view::EditorViewAction,
    NotebookWorkflow,
};

#[derive(Default)]
struct MouseStateHandles {
    insert_button_state: MouseStateHandle,
    copy_button_state: MouseStateHandle,
    edit_button_state: MouseStateHandle,
    remove_embedding_button_state: MouseStateHandle,
}

pub struct NotebookEmbed {
    start: Anchor,
    hashed_id: String,
    is_selected: bool,
    content: ModelHandle<Buffer>,
    selection_model: ModelHandle<BufferSelectionModel>,
    mouse_state_handles: MouseStateHandles,
    cached_syntax_color: Option<Vec<(Range<ByteOffset>, AnsiColorIdentifier)>>,
}

impl NotebookEmbed {
    pub fn new(
        start: CharOffset,
        hashed_id: String,
        content: ModelHandle<Buffer>,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let start = selection_model.update(ctx, |selection_model, ctx| {
            selection_model.anchor(start, ctx)
        });

        let embedding = Self {
            start,
            hashed_id,
            content,
            selection_model,
            is_selected: false,
            mouse_state_handles: Default::default(),
            cached_syntax_color: None,
        };

        embedding.highlight_syntax(ctx);
        embedding
    }

    pub fn highlight_syntax(&self, ctx: &mut ModelContext<Self>) {
        let completion_context = SessionAgnosticContext::new(CommandRegistry::global_instance());
        if let Some(command) = self
            .maybe_get_workflow(ctx)
            .and_then(|workflow| workflow.model().data.command())
        {
            let command = command.to_string();
            let _ = ctx.spawn(
                async move { parse_current_commands_and_tokens(command, &completion_context).await },
                |notebook_embed, parsed_tokens, ctx| {
                    notebook_embed.update_buffer_with_parsed_tokens(parsed_tokens, ctx);
                },
            );
        }
    }

    fn update_buffer_with_parsed_tokens(
        &mut self,
        parsed_tokens: ParsedTokensSnapshot,
        ctx: &mut ModelContext<Self>,
    ) {
        let colors = parsed_token_to_color_style_ranges(parsed_tokens.parsed_tokens);
        self.cached_syntax_color = Some(colors.clone());

        self.update_buffer_with_syntax_color(&colors, ctx);
    }

    pub fn try_apply_cached_highlighting(&self, ctx: &mut ModelContext<Self>) {
        if let Some(colors) = &self.cached_syntax_color {
            self.update_buffer_with_syntax_color(colors, ctx);
        }
    }

    fn update_buffer_with_syntax_color(
        &self,
        colors: &[(Range<ByteOffset>, AnsiColorIdentifier)],
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(offset) = self.start_offset(ctx) else {
            return;
        };

        let appearance = Appearance::as_ref(ctx);
        let font_settings = FontSettings::as_ref(ctx);
        let terminal_colors_normal = appearance.theme().terminal_colors().normal.to_owned();
        let background_color = rich_text_styles(appearance, font_settings)
            .embedding_background
            .start_color();

        self.content.update(ctx, |buffer, ctx| {
            buffer.replace_embedding_at_offset(
                offset,
                Arc::new(
                    EmbeddedWorkflow::new(self.hashed_id.clone()).with_syntax_highlighting(
                        transform_ansi_color_to_solid_color(
                            colors,
                            &terminal_colors_normal,
                            background_color,
                        ),
                    ),
                ),
                self.selection_model.clone(),
                ctx,
            )
        });
    }

    pub fn hashed_id(&self) -> &str {
        self.hashed_id.as_str()
    }

    pub fn refresh_item_state(&self, ctx: &mut ModelContext<Self>) {
        let Some(offset) = self.start_offset(ctx) else {
            return;
        };

        self.content.update(ctx, |buffer, ctx| {
            buffer.replace_embedding_at_offset(
                offset,
                Arc::new(EmbeddedWorkflow::new(self.hashed_id.clone())),
                self.selection_model.clone(),
                ctx,
            )
        });

        // Re-highlight syntax since the command might have changed.
        self.highlight_syntax(ctx);
    }

    fn maybe_get_workflow<'a>(&self, ctx: &'a AppContext) -> Option<&'a CloudWorkflow> {
        let cloud_model = CloudModel::as_ref(ctx);

        // Currently we are only supporting embedded workflows. We could support
        // more drive objects in the future.
        let id = WorkflowId::from_hash(&self.hashed_id)?;
        cloud_model
            .get_by_uid(&id.to_server_id().uid())
            .and_then(|object| object.as_any().downcast_ref::<CloudWorkflow>())
            .and_then(|workflow| {
                if workflow.is_trashed(cloud_model) {
                    None
                } else {
                    Some(workflow)
                }
            })
    }

    pub fn start_offset(&self, ctx: &impl ModelAsRef) -> Option<CharOffset> {
        self.selection_model.as_ref(ctx).resolve_anchor(&self.start)
    }

    fn selectable(&self, ctx: &AppContext) -> bool {
        self.maybe_get_workflow(ctx).is_some()
    }

    fn render_footer_for_workflow(
        &self,
        workflow: &CloudWorkflow,
        appearance: &Appearance,
        ctx: &AppContext,
    ) -> Box<dyn Element> {
        let mut footer = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::End);

        let workflow_id = workflow.id;
        let workflow_info = NotebookWorkflow::from_cloud_workflow(Box::new(workflow.clone()));
        let block_info = BlockInfo::EmbeddedWorkflow {
            workflow_id: workflow_id.into_server().map(Into::into),
            team_uid: workflow.permissions.owner.into(),
        };

        let workflow_content = workflow.model().data.content().to_owned();
        footer.add_child(Shrinkable::new(1.0, Empty::new().finish()).finish());
        footer.add_child(
            Align::new(
                block_footer_action_button(
                    appearance,
                    Icon::Pencil,
                    self.mouse_state_handles.edit_button_state.clone(),
                    "Edit",
                    None,
                )
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(EditorViewAction::EditWorkflow(workflow_id));
                })
                .finish(),
            )
            .right()
            .finish(),
        );

        footer.add_child(
            Align::new(
                block_footer_action_button(
                    appearance,
                    Icon::Copy,
                    self.mouse_state_handles.copy_button_state.clone(),
                    "Copy",
                    custom_action_to_display(CustomAction::Copy),
                )
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(EditorViewAction::CopyTextToClipboard {
                        text: UserInput::new(workflow_content.clone()),
                        block: block_info,
                        entrypoint: ActionEntrypoint::Button,
                    });
                })
                .finish(),
            )
            .right()
            .finish(),
        );

        footer.add_child(
            Align::new(
                block_footer_action_button(
                    appearance,
                    Icon::TerminalInput,
                    self.mouse_state_handles.insert_button_state.clone(),
                    "Run in terminal",
                    NotebookKeybindings::as_ref(ctx).run_commands_keybinding(),
                )
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(EditorViewAction::RunWorkflow(workflow_info.clone()));
                })
                .finish(),
            )
            .right()
            .finish(),
        );
        footer.finish()
    }
}

impl Entity for NotebookEmbed {
    type Event = ();
}

impl EmbeddedItemModel for NotebookEmbed {
    fn render_item_footer(&self, ctx: &AppContext) -> Option<Box<dyn Element>> {
        // Currently we are only supporting embedded workflows. We could support
        // more drive objects in the future.
        let workflow = self.maybe_get_workflow(ctx);
        let appearance = Appearance::as_ref(ctx);

        workflow.map(|workflow| self.render_footer_for_workflow(workflow, appearance, ctx))
    }

    fn border(&self, app: &AppContext) -> Option<Border> {
        if self.is_selected {
            let border_fill = Appearance::as_ref(app).theme().accent();
            Some(Border::all(3.).with_border_fill(border_fill))
        } else {
            None
        }
    }

    fn render_remove_embedding_button(&self, ctx: &AppContext) -> Option<Box<dyn Element>> {
        let appearance = Appearance::as_ref(ctx);
        let offset = self.start_offset(ctx)?;
        Some(
            Container::new(
                appearance
                    .ui_builder()
                    .button(
                        ButtonVariant::Text,
                        self.mouse_state_handles
                            .remove_embedding_button_state
                            .clone(),
                    )
                    .with_text_label(crate::i18n::tr_static(ctx, "Remove").to_string())
                    .build()
                    .with_cursor(Cursor::Arrow)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(EditorViewAction::RemoveEmbeddingAt(offset));
                    })
                    .finish(),
            )
            .with_margin_right(12.)
            .finish(),
        )
    }
}

impl ChildModelHandle for ModelHandle<NotebookEmbed> {
    fn start_offset(&self, app: &AppContext) -> Option<CharOffset> {
        self.as_ref(app).start_offset(app)
    }

    fn end_offset(&self, app: &AppContext) -> Option<CharOffset> {
        // Embedding should always take one character offset.
        self.as_ref(app).start_offset(app).map(|offset| offset + 1)
    }

    fn selectable(&self, app: &AppContext) -> bool {
        self.as_ref(app).selectable(app)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn executable_workflow(&self, app: &AppContext) -> Option<NotebookWorkflow> {
        // Currently we are only supporting embedded workflows. We could support
        // more drive objects in the future.
        self.as_ref(app)
            .maybe_get_workflow(app)
            .map(|workflow| NotebookWorkflow::from_cloud_workflow(Box::new(workflow.clone())))
    }

    fn executable_command<'a>(&'a self, app: &'a AppContext) -> Option<Cow<'a, str>> {
        self.as_ref(app)
            .maybe_get_workflow(app)
            .map(|workflow| workflow.model().data.content().into())
    }

    fn selected(&self, app: &AppContext) -> bool {
        self.as_ref(app).is_selected
    }

    fn set_selected(&self, selected: bool, ctx: &mut AppContext) -> bool {
        self.update(ctx, |model, _ctx| {
            mem::replace(&mut model.is_selected, selected)
        })
    }

    fn clone_boxed(&self) -> Box<dyn ChildModelHandle> {
        Box::new(self.clone())
    }
}
