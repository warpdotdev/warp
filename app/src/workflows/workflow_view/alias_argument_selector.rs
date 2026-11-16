use std::collections::HashMap;

use warp_core::ui::appearance::Appearance;
use warp_editor::editor::NavigationKey;
use warpui::elements::ChildView;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::cloud_object::model::persistence::CloudModel;
use crate::drive::workflows::enum_creation_dialog::WorkflowEnumData;
use crate::editor::{
    EditOrigin, EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys,
    SingleLineEditorOptions, TextOptions,
};
use crate::server::ids::SyncId;
use crate::view_components::{Dropdown, DropdownItem};
use crate::workflows::workflow::ArgumentType;
use crate::workflows::workflow_enum::EnumVariants;

/// Width of the argument editor in alias mode.
pub const ALIAS_ARGUMENT_EDITOR_WIDTH: f32 = 300.;
const EDITOR_FONT_SIZE: f32 = 14.;

#[derive(Debug, Clone, PartialEq)]
pub enum AliasArgumentSelectorAction {
    AliasValueSet(String),
}

/// Whether this argument is a text argument, a static enum argument, or a dynamic enum argument.
/// This is separate from ArgumentType because that requires a query to determine if the enum is
/// static or dynamic.
enum AliasArgumentType {
    Text,
    StaticEnum,
    DynamicEnum,
}

/// A widget to select the value for an alias argument to a workflow.
///
/// If the argument is a string type, the user can enter the value as a string.
/// If the argument is a static enum, the user can select a value from the list of options.
/// If the argument is a dynamic enum, the user can enter the value as a string.  The dynamic enum is environment
/// specific, so it doesn't make sense to choose from a set of options.
pub struct AliasArgumentSelector {
    string_argument_editor: ViewHandle<EditorView>,
    dropdown: ViewHandle<Dropdown<AliasArgumentSelectorAction>>,
    argument_type: AliasArgumentType,
}

impl AliasArgumentSelector {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let appearance = Appearance::as_ref(ctx);

        let text = TextOptions {
            font_size_override: Some(EDITOR_FONT_SIZE),
            font_family_override: Some(appearance.ui_font_family()),
            ..Default::default()
        };

        let editor = ctx.add_typed_action_view(|ctx| {
            EditorView::single_line(
                SingleLineEditorOptions {
                    text,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    ..Default::default()
                },
                ctx,
            )
        });

        ctx.subscribe_to_view(&editor, |_me, editor, event, ctx| match event {
            EditorEvent::Edited(origin) => {
                if *origin != EditOrigin::SystemEdit {
                    ctx.emit(AliasArgumentSelectorEvent::ValueSet(
                        editor.as_ref(ctx).buffer_text(ctx).clone(),
                    ));
                }
            }
            EditorEvent::Navigate(nav_key) => {
                ctx.emit(AliasArgumentSelectorEvent::Navigate(*nav_key));
            }
            _ => {}
        });

        let dropdown = ctx.add_typed_action_view(|ctx| {
            let mut d = Dropdown::new(ctx);
            d.set_menu_width(ALIAS_ARGUMENT_EDITOR_WIDTH, ctx);
            d.set_top_bar_max_width(ALIAS_ARGUMENT_EDITOR_WIDTH);
            d
        });

        AliasArgumentSelector {
            string_argument_editor: editor,
            dropdown,
            argument_type: AliasArgumentType::Text,
        }
    }

    /// Set the type of argument, and optionally the value of the argument.
    /// enum_data: This should be the map of unsaved enum data from the workflow editor.
    pub fn set_argument(
        &mut self,
        argument_type: &ArgumentType,
        value: Option<&String>,
        enum_data: &HashMap<SyncId, WorkflowEnumData>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.notify();
        match argument_type {
            ArgumentType::Text => {
                self.set_string_argument(value, ctx);
                self.argument_type = AliasArgumentType::Text;
            }
            ArgumentType::Enum { enum_id } => {
                let cloud_model = CloudModel::as_ref(ctx);

                // Get the variants from the unsaved enum data, if it exists.
                // Otherwise, pull it from the cloud model.
                let enum_variants = enum_data
                    .get(enum_id)
                    .and_then(|workflow_enum| workflow_enum.new_data.clone())
                    .or_else(|| {
                        cloud_model.get_workflow_enum(enum_id).map(|workflow_enum| {
                            workflow_enum.model().string_model.variants.clone()
                        })
                    });

                match enum_variants {
                    Some(EnumVariants::Static(variants)) => {
                        self.argument_type = AliasArgumentType::StaticEnum;

                        // Add the variants to the dropdown.
                        let items: Vec<_> = variants
                            .iter()
                            .map(|variant| {
                                DropdownItem::new(
                                    variant.clone(),
                                    AliasArgumentSelectorAction::AliasValueSet(variant.clone()),
                                )
                            })
                            .collect();

                        self.dropdown.update(ctx, |dropdown, ctx| {
                            dropdown.set_items(items, ctx);
                            if let Some(value) = value {
                                dropdown.set_selected_by_name(value, ctx);
                            } else {
                                dropdown.set_selected_to_none(ctx);
                            }
                        });
                    }
                    Some(EnumVariants::Dynamic(_)) => {
                        self.argument_type = AliasArgumentType::DynamicEnum;
                        self.set_string_argument(value, ctx);
                    }
                    None => {
                        log::info!("No enum variants found for enum_id: {enum_id:?}");
                        self.argument_type = AliasArgumentType::Text;
                        self.set_string_argument(value, ctx);
                    }
                }
            }
        }
    }

    /// Set the value of the string argument editor.
    fn set_string_argument(&mut self, value: Option<&String>, ctx: &mut ViewContext<Self>) {
        self.string_argument_editor.update(ctx, |editor, ctx| {
            if let Some(value) = value {
                editor.system_reset_buffer_text(value, ctx);
            } else {
                editor.system_clear_buffer(true, ctx);
            }
        });
    }
}

impl View for AliasArgumentSelector {
    fn ui_name() -> &'static str {
        "AliasArgumentSelector"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            match self.argument_type {
                AliasArgumentType::Text | AliasArgumentType::DynamicEnum => {
                    ctx.focus(&self.string_argument_editor);
                }
                AliasArgumentType::StaticEnum => {
                    self.dropdown.update(ctx, |dropdown, ctx| {
                        dropdown.toggle_expanded(ctx);
                    });
                }
            }
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        match self.argument_type {
            AliasArgumentType::Text | AliasArgumentType::DynamicEnum => {
                let appearance = Appearance::as_ref(app);
                appearance
                    .ui_builder()
                    .text_input(self.string_argument_editor.clone())
                    .with_style(UiComponentStyles {
                        padding: Some(Coords {
                            top: 5.,
                            bottom: 5.,
                            left: 12.,
                            right: 4.,
                        }),
                        ..Default::default()
                    })
                    .build()
                    .with_width(ALIAS_ARGUMENT_EDITOR_WIDTH)
                    .finish()
            }
            AliasArgumentType::StaticEnum => ChildView::new(&self.dropdown).finish(),
        }
    }
}

impl TypedActionView for AliasArgumentSelector {
    type Action = AliasArgumentSelectorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AliasArgumentSelectorAction::AliasValueSet(value) => {
                ctx.emit(AliasArgumentSelectorEvent::ValueSet(value.clone()));
                ctx.notify();
            }
        }
    }
}

pub enum AliasArgumentSelectorEvent {
    ValueSet(String),
    Navigate(NavigationKey),
}

impl Entity for AliasArgumentSelector {
    type Event = AliasArgumentSelectorEvent;
}
