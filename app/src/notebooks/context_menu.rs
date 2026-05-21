//! Shared context menu implementation for notebooks.

use pathfinder_geometry::vector::Vector2F;
use warp_core::context_flag::ContextFlag;
use warpui::{
    elements::{ChildAnchor, OffsetPositioning, ParentAnchor, ParentOffsetBounds, Stack},
    keymap::Trigger,
    presenter::ChildView,
    Action, Element, EventContext, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::{
    editor::EditorView,
    i18n::{self, I18nKey},
    menu::{self, Menu, MenuItem, MenuItemFields},
    pane_group::{focus_state::PaneFocusHandle, PaneEvent, SplitPaneState},
    util::bindings::{keybinding_name_to_display_string, trigger_to_keystroke, CustomAction},
};

use super::{
    editor::{keys::custom_action_to_display, view::RichTextEditorView},
    telemetry::ActionEntrypoint,
};

#[cfg(test)]
#[path = "context_menu_tests.rs"]
mod tests;

const CONTEXT_MENU_WIDTH: f32 = 200.;

pub struct ContextMenuState<V: TypedActionView + View>
where
    V::Action: Clone + From<ContextMenuAction>,
{
    /// The kind of menu that's open. If `None`, the menu is closed.
    source: Option<MenuSource>,
    menu: ViewHandle<Menu<V::Action>>,
    /// Focus state of the pane containing this context menu.
    focus_handle: Option<PaneFocusHandle>,
}

#[derive(Debug, Clone)]
pub enum MenuSource {
    RichTextEditor {
        parent_offset: Vector2F,
        editor: ViewHandle<RichTextEditorView>,
    },
    TextEditor {
        parent_offset: Vector2F,
        editor: ViewHandle<EditorView>,
    },
}

impl<V: TypedActionView + View> ContextMenuState<V>
where
    V::Event: From<PaneEvent>,
    V::Action: Clone + From<ContextMenuAction>,
{
    pub fn new(ctx: &mut ViewContext<V>) -> Self {
        let menu = ctx.add_typed_action_view(|_| Menu::new().with_width(CONTEXT_MENU_WIDTH));

        ctx.subscribe_to_view(&menu, |view, _, event, ctx| match event {
            menu::Event::ItemSelected | menu::Event::ItemHovered => (),
            menu::Event::Close { via_select_item } => {
                view.handle_action(
                    &V::Action::from(ContextMenuAction::Close {
                        via_select_item: *via_select_item,
                    }),
                    ctx,
                );
            }
        });

        Self {
            source: None,
            menu,
            focus_handle: None,
        }
    }

    pub(super) fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle) {
        self.focus_handle = Some(focus_handle);
    }

    /// Renders the context menu, if it's open.
    pub fn render(&self, stack: &mut Stack) {
        let offset = match self.source {
            Some(MenuSource::RichTextEditor { parent_offset, .. }) => parent_offset,
            Some(MenuSource::TextEditor { parent_offset, .. }) => parent_offset,
            None => return,
        };

        stack.add_positioned_overlay_child(
            ChildView::new(&self.menu).finish(),
            OffsetPositioning::offset_from_parent(
                offset,
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::TopLeft,
                ChildAnchor::TopLeft,
            ),
        );
    }

    /// Show the context menu.
    pub fn show_context_menu(&mut self, source: MenuSource, ctx: &mut ViewContext<V>) {
        let mut items = vec![];

        // Section 1: text selection actions.
        let (has_selection, can_edit) = match &source {
            MenuSource::RichTextEditor { editor, .. } => {
                let editor = editor.as_ref(ctx);
                (
                    !editor.selection_is_single_cursor(ctx) || editor.has_command_selection(ctx),
                    editor.is_editable(ctx),
                )
            }
            MenuSource::TextEditor { editor, .. } => editor.read(ctx, |editor, ctx| {
                (!editor.selected_text(ctx).is_empty(), editor.can_edit(ctx))
            }),
        };

        if has_selection && can_edit {
            let item = MenuItemFields::new(i18n::tr(ctx, I18nKey::CommonCut))
                .with_on_select_action(V::Action::from(ContextMenuAction::CutSelectedText))
                .with_key_shortcut_label(custom_action_to_display(CustomAction::Cut));
            items.push(item.into_item());
        }
        if has_selection {
            let item = MenuItemFields::new(i18n::tr(ctx, I18nKey::CommonCopy))
                .with_on_select_action(V::Action::from(ContextMenuAction::CopySelectedText))
                .with_key_shortcut_label(custom_action_to_display(CustomAction::Copy));
            items.push(item.into_item());
        }
        if can_edit {
            let item = MenuItemFields::new(i18n::tr(ctx, I18nKey::CommonPaste))
                .with_on_select_action(V::Action::from(ContextMenuAction::Paste))
                .with_key_shortcut_label(custom_action_to_display(CustomAction::Paste));
            items.push(item.into_item());
        }

        // Section 2: Split-pane actions
        let split_pane_menu_items = self.split_pane_menu_items(ctx);
        if !items.is_empty() && !split_pane_menu_items.is_empty() {
            items.push(MenuItem::Separator);
        }
        if !split_pane_menu_items.is_empty() {
            items.extend(split_pane_menu_items);
        }

        self.menu.update(ctx, move |menu, ctx| {
            menu.set_items(items, ctx); // This also resets the selection.
        });
        self.source = Some(source);
        ctx.focus(&self.menu);
        ctx.notify();
    }

    fn split_pane_menu_items(&self, ctx: &mut ViewContext<V>) -> Vec<MenuItem<V::Action>> {
        let mut items = vec![];
        if ContextFlag::CreateNewSession.is_enabled() {
            items.extend([
                MenuItemFields::new(i18n::tr(ctx, I18nKey::CommonSplitPaneRight))
                    .with_on_select_action(V::Action::from(ContextMenuAction::EmitPaneEvent(
                        PaneEvent::SplitRight(None),
                    )))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "pane_group:add_right",
                        ctx,
                    ))
                    .into_item(),
                MenuItemFields::new(i18n::tr(ctx, I18nKey::CommonSplitPaneLeft))
                    .with_on_select_action(V::Action::from(ContextMenuAction::EmitPaneEvent(
                        PaneEvent::SplitLeft(None),
                    )))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "pane_group:add_left",
                        ctx,
                    ))
                    .into_item(),
                MenuItemFields::new(i18n::tr(ctx, I18nKey::CommonSplitPaneDown))
                    .with_on_select_action(V::Action::from(ContextMenuAction::EmitPaneEvent(
                        PaneEvent::SplitDown(None),
                    )))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "pane_group:add_down",
                        ctx,
                    ))
                    .into_item(),
                MenuItemFields::new(i18n::tr(ctx, I18nKey::CommonSplitPaneUp))
                    .with_on_select_action(V::Action::from(ContextMenuAction::EmitPaneEvent(
                        PaneEvent::SplitUp(None),
                    )))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "pane_group:add_up",
                        ctx,
                    ))
                    .into_item(),
            ]);
        }

        let split_pane_state = self
            .focus_handle
            .as_ref()
            .map_or(SplitPaneState::NotInSplitPane, |h| h.split_pane_state(ctx));
        if split_pane_state.is_in_split_pane() {
            let is_maximized = split_pane_state.is_maximized();
            items.push(
                MenuItemFields::toggle_pane_action(is_maximized)
                    .with_on_select_action(V::Action::from(ContextMenuAction::EmitPaneEvent(
                        PaneEvent::ToggleMaximized,
                    )))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "pane_group:toggle_maximize_pane",
                        ctx,
                    ))
                    .into_item(),
            );

            items.push(
                MenuItemFields::new(i18n::tr(ctx, I18nKey::CommonClosePane))
                    .with_on_select_action(V::Action::from(ContextMenuAction::EmitPaneEvent(
                        PaneEvent::Close,
                    )))
                    .with_key_shortcut_label(
                        trigger_to_keystroke(&Trigger::Custom(
                            CustomAction::CloseCurrentSession.into(),
                        ))
                        .map(|keystroke| keystroke.displayed()),
                    )
                    .into_item(),
            );
        }
        items
    }

    /// Close the context menu. If `focus_parent` is true, the parent view (either the editor that
    /// triggered the context menu or the notebook view) will be focused.
    pub fn close_context_menu(&mut self, focus_parent: bool, ctx: &mut ViewContext<V>) {
        if focus_parent {
            match &self.source {
                Some(MenuSource::RichTextEditor { editor, .. }) => ctx.focus(editor),
                Some(MenuSource::TextEditor { editor, .. }) => ctx.focus(editor),
                None => ctx.focus_self(),
            }
        }
        self.source = None;
        ctx.notify();
    }

    #[cfg(test)]
    /// List out the context menu items by name.
    pub fn item_names<'a>(&self, ctx: &'a impl warpui::ViewAsRef) -> Vec<&'a str> {
        self.menu
            .as_ref(ctx)
            .items()
            .iter()
            .map(|item| match item {
                MenuItem::Item(item) => item.label(),
                MenuItem::Separator => "----",
                MenuItem::ItemsRow { .. } => panic!("ItemsRow not supported"),
                MenuItem::Submenu { fields, .. } => fields.label(),
                MenuItem::Header { fields, .. } => fields.label(),
            })
            .collect()
    }

    pub fn handle_action(&mut self, action: &ContextMenuAction, ctx: &mut ViewContext<V>) {
        match action {
            ContextMenuAction::Open(source) => self.show_context_menu(source.clone(), ctx),
            ContextMenuAction::Close { via_select_item } => {
                self.close_context_menu(!*via_select_item, ctx)
            }
            ContextMenuAction::CopySelectedText => match &self.source {
                Some(MenuSource::RichTextEditor { editor, .. }) => {
                    editor.update(ctx, |editor, ctx| editor.copy(ActionEntrypoint::Menu, ctx))
                }
                Some(MenuSource::TextEditor { editor, .. }) => {
                    editor.update(ctx, |editor, ctx| editor.copy(ctx))
                }
                None => (),
            },
            ContextMenuAction::CutSelectedText => match &self.source {
                Some(MenuSource::RichTextEditor { editor, .. }) => {
                    ctx.focus(editor);
                    editor.update(ctx, |editor, ctx| editor.cut(ActionEntrypoint::Menu, ctx));
                }
                Some(MenuSource::TextEditor { editor, .. }) => {
                    ctx.focus(editor);
                    editor.update(ctx, |editor, ctx| editor.cut(ctx))
                }
                None => (),
            },
            ContextMenuAction::Paste => match &self.source {
                Some(MenuSource::RichTextEditor { editor, .. }) => {
                    ctx.focus(editor);
                    editor.update(ctx, |editor, ctx| editor.paste(ctx))
                }
                Some(MenuSource::TextEditor { editor, .. }) => {
                    ctx.focus(editor);
                    editor.update(ctx, |editor, ctx| editor.paste(ctx))
                }
                None => (),
            },
            ContextMenuAction::EmitPaneEvent(event) => ctx.emit(V::Event::from(event.clone())),
        }
    }
}

/// Dispatch an action to show the notebook context menu for a rich text editor view.
pub fn show_rich_editor_context_menu<A>(
    ctx: &mut EventContext,
    position: Vector2F,
    parent_position_id: &str,
    editor: &ViewHandle<RichTextEditorView>,
) where
    A: Action + From<ContextMenuAction>,
{
    if let Some(parent_bounds) = ctx.element_position_by_id(parent_position_id) {
        let offset = position - parent_bounds.origin();
        ctx.dispatch_typed_action(A::from(ContextMenuAction::Open(
            MenuSource::RichTextEditor {
                parent_offset: offset,
                editor: editor.clone(),
            },
        )));
    }
}

/// Dispatch an action to show the notebook context menu for a plain text editor view.
pub fn show_text_editor_context_menu<A>(
    ctx: &mut EventContext,
    position: Vector2F,
    parent_position_id: &str,
    editor: &ViewHandle<EditorView>,
) where
    A: Action + From<ContextMenuAction>,
{
    if let Some(parent_bounds) = ctx.element_position_by_id(parent_position_id) {
        let offset = position - parent_bounds.origin();
        ctx.dispatch_typed_action(A::from(ContextMenuAction::Open(MenuSource::TextEditor {
            parent_offset: offset,
            editor: editor.clone(),
        })));
    }
}

#[derive(Debug, Clone)]
pub enum ContextMenuAction {
    Open(MenuSource),
    Close { via_select_item: bool },
    CopySelectedText,
    CutSelectedText,
    Paste,
    EmitPaneEvent(PaneEvent),
}
