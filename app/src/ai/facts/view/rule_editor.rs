use crate::cloud_object::model::generic_string_model::GenericStringObjectId;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObject, Revision};
use crate::editor::{
    EditorOptions, EditorView, EnterAction, EnterSettings, Event as EditorEvent,
    PropagateAndNoOpNavigationKeys, SingleLineEditorOptions, TextOptions,
};
use crate::network::NetworkStatus;
use crate::server::ids::SyncId;
use crate::ui_components::buttons::icon_button;
use crate::view_components::action_button::{ActionButton, DangerSecondaryTheme, PrimaryTheme};
use warp_core::ui::{appearance::Appearance, theme::color::internal_colors};
use warp_editor::editor::NavigationKey;
use warpui::elements::{Clipped, ConstrainedBox};
use warpui::{
    elements::{
        Border, ChildView, ClippedScrollStateHandle, ClippedScrollable, Container, CornerRadius,
        CrossAxisAlignment, Flex, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement,
        Radius, ScrollbarWidth,
    },
    platform::Cursor,
    ui_components::components::UiComponent,
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::{is_delete_allowed, style, AIFact, CloudAIFact, CloudAIFactModel};
use crate::ai::facts::AIMemory;
use crate::ui_components::icons::Icon;

const RULE_NAME_PLACEHOLDER_TEXT: &str = "e.g. Rust rules";
const RULE_DESCRIPTION_PLACEHOLDER_TEXT: &str = "e.g. Never use unwrap in Rust";

#[derive(Debug, Clone, Copy)]
enum EditorType {
    Name,
    Content,
}

#[derive(Debug, Clone)]
pub enum RuleEditorViewEvent {
    Back,
    Add {
        name: Option<String>,
        content: String,
    },
    Edit {
        name: Option<String>,
        content: String,
        sync_id: SyncId,
        revision_ts: Option<Revision>,
    },
    Delete {
        sync_id: SyncId,
    },
}

#[derive(Debug, Clone)]
pub enum RuleEditorViewAction {
    Back,
    Save,
    Delete,
}
pub struct RuleEditorView {
    // Is None if we are adding a new rule, otherwise it is the existing rule we are editing.
    ai_fact: Option<CloudAIFact>,

    current_editor: EditorType,
    name_editor: ViewHandle<EditorView>,
    content_editor: ViewHandle<EditorView>,

    save_button: ViewHandle<ActionButton>,
    delete_button: ViewHandle<ActionButton>,
    back_button: MouseStateHandle,
    clipped_scroll_state: ClippedScrollStateHandle,
}

impl RuleEditorView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let network_status = NetworkStatus::handle(ctx);
        ctx.subscribe_to_model(&network_status, |_me, _, _event, ctx| {
            ctx.notify();
        });

        let appearance = Appearance::as_ref(ctx);
        let font_family = appearance.ui_font_family();
        let text = TextOptions {
            font_size_override: Some(style::TEXT_FONT_SIZE),
            font_family_override: Some(font_family),
            ..Default::default()
        };
        let name_editor = ctx.add_typed_action_view(|ctx| {
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: text.clone(),
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text(RULE_NAME_PLACEHOLDER_TEXT, ctx);
            editor
        });
        ctx.subscribe_to_view(&name_editor, |me, _editor, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        let content_editor = ctx.add_typed_action_view(|ctx| {
            let mut editor = EditorView::new(
                EditorOptions {
                    text,
                    soft_wrap: true,
                    autogrow: true,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    supports_vim_mode: false,
                    single_line: false,
                    enter_settings: EnterSettings {
                        shift_enter: EnterAction::InsertNewLineIfMultiLine,
                        enter: EnterAction::InsertNewLineIfMultiLine,
                        alt_enter: EnterAction::InsertNewLineIfMultiLine,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text(RULE_DESCRIPTION_PLACEHOLDER_TEXT, ctx);
            editor
        });
        ctx.subscribe_to_view(&content_editor, |me, _editor, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        let save_button = ctx.add_typed_action_view(|ctx| {
            let mut button = ActionButton::new(crate::i18n::tr_static(ctx, "Save"), PrimaryTheme)
                .with_icon(Icon::Check)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(RuleEditorViewAction::Save);
                });
            // Disable the button until the user has entered a description
            button.set_disabled(true, ctx);
            button
        });

        let delete_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(
                crate::i18n::tr_static(ctx, "Delete rule"),
                DangerSecondaryTheme,
            )
            .with_icon(Icon::Trash)
            .on_click(|ctx| {
                ctx.dispatch_typed_action(RuleEditorViewAction::Delete);
            })
        });

        Self {
            ai_fact: None,
            current_editor: EditorType::Name,
            name_editor,
            content_editor,
            save_button,
            delete_button,
            back_button: Default::default(),
            clipped_scroll_state: Default::default(),
        }
    }

    pub fn set_ai_rule(&mut self, sync_id: Option<SyncId>, ctx: &mut ViewContext<Self>) {
        if let Some(sync_id) = sync_id {
            // Get the AIFact from the cloud model
            let Some(ai_fact) = CloudModel::as_ref(ctx)
                .get_object_of_type::<GenericStringObjectId, CloudAIFactModel>(&sync_id)
            else {
                return;
            };
            let AIFact::Memory(AIMemory { name, content, .. }) =
                ai_fact.model().string_model.clone();
            self.ai_fact = Some(ai_fact.clone());

            // Update the UI with the AIFact
            self.name_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(name.unwrap_or_default().as_str(), ctx);
            });
            self.content_editor.update(ctx, |editor, ctx| {
                editor.set_buffer_text(content.as_str(), ctx);
            });
        } else {
            self.ai_fact = None;
            self.name_editor.update(ctx, |editor, ctx| {
                editor.clear_buffer_and_reset_undo_stack(ctx);
            });
            self.content_editor.update(ctx, |editor, ctx| {
                editor.clear_buffer_and_reset_undo_stack(ctx);
            });
        }
        ctx.notify();
    }

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        let (current_editor, next_editor, next_editor_type) = match self.current_editor {
            EditorType::Name => (&self.name_editor, &self.content_editor, EditorType::Content),
            EditorType::Content => (&self.content_editor, &self.name_editor, EditorType::Name),
        };

        match event {
            EditorEvent::Focused => {
                self.current_editor = if self.name_editor.is_focused(ctx) {
                    EditorType::Name
                } else {
                    EditorType::Content
                };
            }
            EditorEvent::Navigate(NavigationKey::Tab)
            | EditorEvent::Navigate(NavigationKey::ShiftTab) => {
                self.current_editor = next_editor_type;
                ctx.focus(next_editor);
            }
            EditorEvent::Navigate(NavigationKey::Up) => {
                current_editor.update(ctx, |editor, ctx| {
                    editor.move_up(ctx);
                });
            }
            EditorEvent::Navigate(NavigationKey::Down) => {
                current_editor.update(ctx, |editor, ctx| {
                    editor.move_down(ctx);
                });
            }
            EditorEvent::Edited(_) => {
                // Disable the save button if the description is empty
                let is_disabled = self.content_editor.as_ref(ctx).buffer_text(ctx).is_empty();
                self.save_button.update(ctx, |button, ctx| {
                    button.set_disabled(is_disabled, ctx);
                });
            }
            _ => {}
        }
    }

    fn render_back_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let button = icon_button(appearance, Icon::ArrowLeft, false, self.back_button.clone());
        Container::new(
            button
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(RuleEditorViewAction::Back);
                })
                .with_cursor(Cursor::PointingHand)
                .finish(),
        )
        .with_margin_right(style::ICON_MARGIN)
        .finish()
    }

    fn render_save_button(&self, _appearance: &Appearance) -> Box<dyn Element> {
        Container::new(ChildView::new(&self.save_button).finish())
            .with_margin_left(style::SECTION_MARGIN)
            .finish()
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let title = if self.ai_fact.is_none() {
            "Add Rule"
        } else {
            "Edit Rule"
        };
        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(self.render_back_button(appearance))
                        .with_child(
                            appearance
                                .ui_builder()
                                .wrappable_text(title, true)
                                .with_style(style::header_text())
                                .build()
                                .finish(),
                        )
                        .finish(),
                )
                .with_child(self.render_save_button(appearance))
                .finish(),
        )
        .with_margin_bottom(style::ITEM_BOTTOM_MARGIN)
        .finish()
    }

    fn render_name_editor(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(Clipped::new(ChildView::new(&self.name_editor).finish()).finish())
            .with_background(appearance.theme().surface_2())
            .with_border(
                Border::all(1.).with_border_color(internal_colors::neutral_4(appearance.theme())),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_margin_bottom(style::ITEM_BOTTOM_MARGIN)
            .with_horizontal_padding(style::EDITOR_HORIZONTAL_PADDING)
            .with_vertical_padding(style::EDITOR_VERTICAL_PADDING)
            .finish()
    }

    fn render_content_editor(&self, appearance: &Appearance) -> Box<dyn Element> {
        ConstrainedBox::new(
            Container::new(
                ClippedScrollable::vertical(
                    self.clipped_scroll_state.clone(),
                    ConstrainedBox::new(ChildView::new(&self.content_editor).finish())
                        .with_min_height(style::EDITOR_MIN_HEIGHT)
                        .finish(),
                    ScrollbarWidth::Auto,
                    appearance.theme().nonactive_ui_detail().into(),
                    appearance.theme().active_ui_detail().into(),
                    warpui::elements::Fill::None,
                )
                .finish(),
            )
            .with_background(appearance.theme().surface_2())
            .with_border(
                Border::all(1.).with_border_color(internal_colors::neutral_4(appearance.theme())),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_margin_bottom(style::ITEM_BOTTOM_MARGIN)
            .with_padding_left(style::EDITOR_HORIZONTAL_PADDING)
            .with_vertical_padding(style::EDITOR_VERTICAL_PADDING)
            .finish(),
        )
        .with_max_height(style::EDITOR_MAX_HEIGHT)
        .finish()
    }

    fn render_form(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        Flex::column()
            .with_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .span(crate::i18n::tr_static(app, "Name"))
                        .build()
                        .finish(),
                )
                .with_margin_bottom(style::ITEM_BOTTOM_MARGIN)
                .finish(),
            )
            .with_child(self.render_name_editor(appearance))
            .with_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .span(crate::i18n::tr_static(app, "Rule"))
                        .build()
                        .finish(),
                )
                .with_margin_bottom(style::ITEM_BOTTOM_MARGIN)
                .finish(),
            )
            .with_child(self.render_content_editor(appearance))
            .finish()
    }
}

impl Entity for RuleEditorView {
    type Event = RuleEditorViewEvent;
}

impl View for RuleEditorView {
    fn ui_name() -> &'static str {
        "RuleEditorView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            match self.current_editor {
                EditorType::Name => ctx.focus(&self.name_editor),
                EditorType::Content => ctx.focus(&self.content_editor),
            }
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut col = Flex::column()
            .with_child(self.render_header(appearance))
            .with_child(self.render_form(appearance, app));

        if let Some(ai_fact) = &self.ai_fact {
            if is_delete_allowed(ai_fact.clone(), app) {
                col.add_child(ChildView::new(&self.delete_button).finish());
            }
        }
        col.finish()
    }
}

impl TypedActionView for RuleEditorView {
    type Action = RuleEditorViewAction;

    fn handle_action(&mut self, action: &RuleEditorViewAction, ctx: &mut ViewContext<Self>) {
        match action {
            RuleEditorViewAction::Back => {
                ctx.emit(RuleEditorViewEvent::Back);
            }
            RuleEditorViewAction::Save => {
                let name = self.name_editor.as_ref(ctx).buffer_text(ctx);
                let name = if name.is_empty() { None } else { Some(name) };
                let content = self.content_editor.as_ref(ctx).buffer_text(ctx);
                if let Some(ai_fact) = &self.ai_fact {
                    ctx.emit(RuleEditorViewEvent::Edit {
                        name,
                        content,
                        sync_id: ai_fact.sync_id(),
                        revision_ts: ai_fact.metadata().revision.clone(),
                    });
                } else {
                    // Using AIMemory with is_autogenerated set to false to represent a manually created rule
                    ctx.emit(RuleEditorViewEvent::Add { name, content });
                }
            }
            RuleEditorViewAction::Delete => {
                if let Some(ai_fact) = &self.ai_fact {
                    ctx.emit(RuleEditorViewEvent::Delete {
                        sync_id: ai_fact.sync_id(),
                    });
                }
            }
        }
    }
}
