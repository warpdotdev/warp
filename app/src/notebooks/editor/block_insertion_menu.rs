use std::sync::Arc;

use serde::{Deserialize, Serialize};
use warp_editor::content::text::BufferBlockItem;
use warpui::{
    elements::{
        AnchorPair, Border, Container, CornerRadius, MouseStateHandle, OffsetPositioning,
        OffsetType, PositionedElementOffsetBounds, PositioningAxis, Radius, SavePosition, Stack,
        XAxisAnchor, YAxisAnchor,
    },
    presenter::ChildView,
    ui_components::{
        button::ButtonTooltipPosition,
        components::{UiComponent, UiComponentStyles},
    },
    AppContext, Element, SingletonEntity, ViewContext, ViewHandle,
};

use crate::{
    appearance::Appearance,
    cloud_object::{model::persistence::CloudModel, ObjectIdType, Space},
    drive::CloudObjectTypeAndId,
    menu::{self, Menu, MenuItemFields},
    notebooks::telemetry::EmbeddedObjectInfo,
    search::notebook_embedding::{
        searcher::EmbeddingSearchItemAction,
        view::{EmbeddingSearchEvent, EmbeddingSearchMenu},
    },
    server::ids::SyncId,
    themes::theme::Fill,
    ui_components::{buttons::icon_button, icons::Icon},
};

use super::{
    embedded_item::EmbeddedWorkflow,
    view::{EditorViewAction, EditorViewEvent, RichTextEditorView},
    BlockType,
};

/// The saved position ID for the block insertion button.
const BLOCK_INSERT_BUTTON_ID: &str = "notebook_block_insertion_button";

/// Where the block insertion menu was triggered from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockInsertionSource {
    AtCursor,
    BlockInsertionButton,
}

/// Editor view state related to the block insertion menu.
pub struct BlockInsertionMenuState {
    // If the menu is closed, this will be None.
    pub open_at_source: Option<BlockInsertionSource>,
    button_state: MouseStateHandle,
    // Whether the embedded object search menu is open.
    pub embedded_object_search_open: bool,
    /// The embedded object search menu, lazily created when embedded objects are enabled.
    embedded_object_search: Option<ViewHandle<EmbeddingSearchMenu>>,
    pub menu: ViewHandle<Menu<EditorViewAction>>,
}

impl BlockInsertionMenuState {
    pub fn new(ctx: &mut ViewContext<RichTextEditorView>, embedded_objects_enabled: bool) -> Self {
        let menu =
            ctx.add_typed_action_view(|ctx| Self::create_menu(embedded_objects_enabled, ctx));

        ctx.subscribe_to_view(&menu, RichTextEditorView::handle_block_insertion_menu_event);

        let embedded_object_search = if embedded_objects_enabled {
            let embedded_object_search = ctx.add_typed_action_view(EmbeddingSearchMenu::new);
            ctx.subscribe_to_view(
                &embedded_object_search,
                RichTextEditorView::handle_embedded_object_search_menu_event,
            );
            Some(embedded_object_search)
        } else {
            None
        };

        Self {
            open_at_source: None,
            button_state: Default::default(),
            embedded_object_search_open: false,
            embedded_object_search,
            menu,
        }
    }

    fn create_menu(
        embedded_objects_enabled: bool,
        ctx: &mut ViewContext<Menu<EditorViewAction>>,
    ) -> Menu<EditorViewAction> {
        let appearance = Appearance::as_ref(ctx);
        let mut menu = Menu::new().prevent_interaction_with_other_elements();

        for block_type in BlockType::code_block_types() {
            menu.add_item(
                MenuItemFields::new(block_type.label())
                    .with_icon(block_type.icon())
                    .with_on_select_action(EditorViewAction::InsertBlock(
                        warp_editor::content::text::BlockType::Text(block_type.into()),
                    ))
                    .into_item(),
            );
        }

        if embedded_objects_enabled {
            menu.add_item(
                MenuItemFields::new(crate::i18n::tr_static(ctx, "Embed"))
                    .with_icon(Icon::EmbedBlock)
                    .with_on_select_action(EditorViewAction::OpenEmbeddedObjectSearch)
                    .into_item(),
            );
        }

        for block_type in BlockType::text_block_types() {
            let mut item_fields = MenuItemFields::new(block_type.label())
                .with_icon(block_type.icon())
                .with_on_select_action(EditorViewAction::InsertBlock(
                    warp_editor::content::text::BlockType::Text(block_type.into()),
                ));
            if let Some(icon_fill) = block_type.icon_color(appearance) {
                item_fields = item_fields.with_override_icon_color(icon_fill);
            }
            menu.add_item(item_fields.into_item());
        }

        menu.add_item(
            MenuItemFields::new(crate::i18n::tr_static(ctx, "Divider"))
                .with_icon(Icon::HorizontalRuleBlock)
                .with_on_select_action(EditorViewAction::InsertBlock(
                    warp_editor::content::text::BlockType::Item(BufferBlockItem::HorizontalRule),
                ))
                .with_override_icon_color(Fill::Solid(appearance.theme().ui_warning_color()))
                .into_item(),
        );

        menu
    }

    pub fn reset_selection(&mut self, ctx: &mut AppContext) {
        self.menu.update(ctx, |menu, ctx| {
            menu.reset_selection(ctx);
        })
    }
}

impl RichTextEditorView {
    /// Open the block insertion menu.
    pub(super) fn open_block_insertion_menu(
        &mut self,
        source: BlockInsertionSource,
        ctx: &mut ViewContext<Self>,
    ) {
        // Reset selection if we are opening a new block insertion menu or opening
        // the menu from a different source.
        if self.insertion_menu_state.open_at_source != Some(source) {
            self.insertion_menu_state.reset_selection(ctx);
            ctx.notify();
        }
        self.insertion_menu_state.open_at_source = Some(source);
        // By default we should show the block insertion menu.
        self.insertion_menu_state.embedded_object_search_open = false;
        ctx.focus(&self.insertion_menu_state.menu);
        ctx.emit(EditorViewEvent::OpenedBlockInsertionMenu(source));
    }

    pub(super) fn open_embedded_object_search(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(embedded_object_search) = &self.insertion_menu_state.embedded_object_search else {
            return;
        };
        self.insertion_menu_state.embedded_object_search_open = true;
        // Reset the filter state.
        embedded_object_search.update(ctx, |menu, ctx| {
            menu.reset_state(ctx);
        });
        ctx.focus(embedded_object_search);
        ctx.emit(EditorViewEvent::OpenedEmbeddedObjectSearch);
    }

    /// Set the space containing this notebook.
    pub fn set_space(&mut self, space: Space, ctx: &mut ViewContext<Self>) {
        if let Some(embedded_object_search) = &self.insertion_menu_state.embedded_object_search {
            embedded_object_search.update(ctx, |menu, ctx| menu.set_embedding_space(space, ctx));
        }
    }

    /// Close the block insertion menu.
    pub(super) fn close_block_insertion_menu(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_block_insertion_menu_open() {
            ctx.notify();
        }
        self.insertion_menu_state.open_at_source = None;
        self.insertion_menu_state.embedded_object_search_open = false;
        ctx.focus_self();
    }

    /// Whether the block insertion menu is open.
    pub(super) fn is_block_insertion_menu_open(&self) -> bool {
        self.insertion_menu_state.open_at_source.is_some()
    }

    fn handle_embedded_object_search_menu_event(
        &mut self,
        _handle: ViewHandle<EmbeddingSearchMenu>,
        event: &EmbeddingSearchEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EmbeddingSearchEvent::Close => self.close_block_insertion_menu(ctx),
            EmbeddingSearchEvent::ItemSelected { payload } => match payload.as_ref() {
                EmbeddingSearchItemAction::AcceptWorkflow(id) => {
                    self.insert_embedded_workflow(id, ctx)
                }
                EmbeddingSearchItemAction::AcceptNotebook(id) => {
                    self.insert_embedded_notebook(id, ctx)
                }
            },
        }
    }

    /// Insert an embedded workflow block at the current insertion menu source.
    fn insert_embedded_workflow(&mut self, id: &SyncId, ctx: &mut ViewContext<Self>) {
        self.insert_block(
            warp_editor::content::text::BlockType::Item(BufferBlockItem::Embedded {
                item: Arc::new(EmbeddedWorkflow::new(
                    id.sqlite_uid_hash(ObjectIdType::Workflow),
                )),
            }),
            ctx,
        );
        let team_uid = CloudModel::as_ref(ctx)
            .get_workflow(id)
            .and_then(|workflow| workflow.permissions.owner.into());
        ctx.emit(EditorViewEvent::InsertedEmbeddedObject(
            EmbeddedObjectInfo::Workflow {
                workflow_id: id.into_server().map(Into::into),
                team_uid,
            },
        ))
    }

    /// Insert an embedded notebook inline view at the current insertion menu source.
    fn insert_embedded_notebook(&mut self, id: &SyncId, ctx: &mut ViewContext<Self>) {
        let (title, link) = CloudModel::handle(ctx).read(ctx, |model, _| {
            let title = model
                .get_notebook(id)
                .map(|notebook| notebook.model().title.clone())
                .unwrap_or_else(|| "Untitled".to_string());
            let link = model
                .get_by_uid(&CloudObjectTypeAndId::Notebook(*id).uid())
                .and_then(|object| object.object_link());
            (title, link)
        });

        if let Some(link) = link {
            self.insert_embedded_notebook_view(title, link, ctx);
        }
    }

    /// Callback for events on the block insertion menu.
    fn handle_block_insertion_menu_event(
        &mut self,
        _menu: ViewHandle<Menu<EditorViewAction>>,
        event: &menu::Event,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            menu::Event::ItemSelected | menu::Event::ItemHovered => (),
            menu::Event::Close { via_select_item } => {
                // Don't close the block insertion menu if the embedded object
                // search menu is open. Handle the close event emitted from
                // embedded object search menu instead.
                if self.insertion_menu_state.embedded_object_search_open {
                    return;
                }
                self.close_block_insertion_menu(ctx);
                if !*via_select_item {
                    ctx.focus_self()
                }
            }
        }
    }

    /// Renders controls for the block insertion menu.
    pub(super) fn render_block_insertion_menu(&self, stack: &mut Stack, app: &AppContext) {
        if self.disable_block_insertion_menu() {
            return;
        }

        if self.can_edit_app(app) {
            self.render_button(stack, app);
        }

        if let Some(source) = self.insertion_menu_state.open_at_source {
            self.render_menu(source, stack, app);
        }
    }

    /// Renders a button that opens the block insertion menu when clicked.
    fn render_button(&self, stack: &mut Stack, app: &AppContext) {
        let appearance = Appearance::as_ref(app);
        let ui_builder = appearance.ui_builder().clone();
        let insert_block_tooltip = crate::i18n::tr_static(app, "Insert block").to_string();
        let button = icon_button(
            appearance,
            Icon::Plus,
            self.insertion_menu_state.open_at_source
                == Some(BlockInsertionSource::BlockInsertionButton),
            self.insertion_menu_state.button_state.clone(),
        )
        .with_active_styles(UiComponentStyles {
            background: Some(appearance.theme().surface_2().into()),
            border_color: Some(appearance.theme().surface_3().into()),
            ..Default::default()
        })
        .with_tooltip(move || {
            ui_builder
                .tool_tip(insert_block_tooltip.clone())
                .build()
                .finish()
        })
        // Position the tooltip above the insertion button to ensure they don't overlap if the
        // button is towards the bottom of the screen.
        .with_tooltip_position(ButtonTooltipPosition::Above)
        .build()
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(EditorViewAction::OpenBlockInsertionMenu))
        .finish();

        let render_state = self.model.as_ref(app).render_state();
        let hovered_block_id = render_state
            .as_ref(app)
            .saved_positions()
            .hovered_block_start();

        stack.add_positioned_child(
            SavePosition::new(button, BLOCK_INSERT_BUTTON_ID).finish(),
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    &hovered_block_id,
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(-4.),
                    AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Right),
                )
                .with_conditional_anchor(),
                PositioningAxis::relative_to_stack_child(
                    hovered_block_id,
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(YAxisAnchor::Middle, YAxisAnchor::Middle),
                )
                .with_conditional_anchor(),
            ),
        );
    }

    /// Renders a menu for inserting new kinds of blocks.
    fn render_menu(&self, source: BlockInsertionSource, stack: &mut Stack, app: &AppContext) {
        let appearance = Appearance::as_ref(app);
        let render_state = self.model.as_ref(app).render_state.as_ref(app);

        let (container, bounds) = if !self.insertion_menu_state.embedded_object_search_open {
            let menu = ChildView::new(&self.insertion_menu_state.menu).finish();
            (
                Container::new(menu)
                    .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                    .finish(),
                PositionedElementOffsetBounds::ParentByPosition,
            )
        } else if let Some(embedded_object_search) =
            &self.insertion_menu_state.embedded_object_search
        {
            (
                ChildView::new(embedded_object_search).finish(),
                // Embedded object search menu is not bounded by the editor.
                PositionedElementOffsetBounds::WindowByPosition,
            )
        } else {
            // Embedded object search is open but no menu exists - shouldn't happen.
            return;
        };

        let positioning = match source {
            BlockInsertionSource::BlockInsertionButton => OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    BLOCK_INSERT_BUTTON_ID,
                    bounds,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                ),
                PositioningAxis::relative_to_stack_child(
                    BLOCK_INSERT_BUTTON_ID,
                    bounds,
                    OffsetType::Pixel(4.),
                    AnchorPair::new(YAxisAnchor::Bottom, YAxisAnchor::Top),
                ),
            ),
            BlockInsertionSource::AtCursor => {
                let cursor_position = render_state.saved_positions().cursor_id();
                OffsetPositioning::from_axes(
                    PositioningAxis::relative_to_stack_child(
                        &cursor_position,
                        bounds,
                        OffsetType::Pixel(0.),
                        AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                    )
                    .with_conditional_anchor(),
                    PositioningAxis::relative_to_stack_child(
                        &cursor_position,
                        PositionedElementOffsetBounds::WindowByPosition,
                        OffsetType::Pixel(4.),
                        // TODO: Decide if this should be above or below the cursor based
                        // on its location within the viewport.
                        AnchorPair::new(YAxisAnchor::Bottom, YAxisAnchor::Top),
                    )
                    .with_conditional_anchor(),
                )
            }
        };

        stack.add_positioned_overlay_child(container, positioning);
    }
}
