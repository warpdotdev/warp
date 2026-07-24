//! Implementation for the omnibar - a floating menu for editor interactions
//! like formatting and changing block types.

use itertools::Itertools;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;
use warp_editor::content::text::{
    BlockType as ContentBlockType, BufferBlockStyle, BufferTextStyle, TextStyles,
    TextStylesWithMetadata,
};
use warp_editor::model::RichTextEditorModel;
use warp_editor::render::model::RenderState;
use warpui::accessibility::{AccessibilityContent, ActionAccessibilityContent, WarpA11yRole};
use warpui::elements::{
    AnchorPair, Border, ConstrainedBox, Container, CornerRadius, DropShadow, Flex, MainAxisSize,
    MouseStateHandle, OffsetPositioning, OffsetType, ParentElement, Point,
    PositionedElementOffsetBounds, PositioningAxis, Radius, Rect, XAxisAnchor, YAxisAnchor,
};
use warpui::presenter::ChildView;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, SizeConstraint, TypedActionView,
    View, ViewContext, ViewHandle,
};

use super::BlockType;
use super::model::{NotebooksEditorModel, RichTextEditorModelEvent};
use super::view::EditorViewAction;
use crate::appearance::Appearance;
use crate::menu::MenuVariant;
use crate::ui_components::buttons::icon_button;
use crate::ui_components::icons::Icon;
use crate::view_components::{CompactDropdown, CompactDropdownEvent, CompactDropdownItem};

const OMNIBAR_HEIGHT: f32 = 32.;
const OMNIBAR_PADDING: f32 = 4.;

const ACTION_BUTTON_SIZE: f32 = 24.;

pub enum OmnibarEvent {
    OpenLinkEditor,
}

/// View to render the omnibar.
pub struct Omnibar {
    model: ModelHandle<NotebooksEditorModel>,

    block_conversion_dropdown: ViewHandle<CompactDropdown<OmnibarAction>>,

    bold_button_state: MouseStateHandle,
    italicize_button_state: MouseStateHandle,
    underline_button_state: MouseStateHandle,
    strikethrough_button_state: MouseStateHandle,
    link_button_state: MouseStateHandle,
    inline_code_button_state: MouseStateHandle,

    active_text_styles: Option<TextStylesWithMetadata>,
    active_block_type: Option<ContentBlockType>,
}

impl Omnibar {
    pub fn new(model: ModelHandle<NotebooksEditorModel>, ctx: &mut ViewContext<Self>) -> Self {
        let block_conversion_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = CompactDropdown::new(MenuVariant::Fixed, ctx);
            let appearance = Appearance::as_ref(ctx);
            dropdown.set_items(
                BlockType::all()
                    .map(|block_type| conversion_item(block_type, appearance))
                    .collect_vec(),
                ctx,
            );
            dropdown.set_icon_size(ACTION_BUTTON_SIZE - 2. * OMNIBAR_PADDING);
            dropdown
        });
        ctx.subscribe_to_view(&block_conversion_dropdown, Self::handle_dropdown_event);

        ctx.subscribe_to_model(&model, Self::handle_model_event);

        Self {
            model,
            block_conversion_dropdown,
            link_button_state: Default::default(),
            bold_button_state: Default::default(),
            strikethrough_button_state: Default::default(),
            italicize_button_state: Default::default(),
            underline_button_state: Default::default(),
            inline_code_button_state: Default::default(),
            active_text_styles: None,
            active_block_type: None,
        }
    }

    /// The relative positioning of the omnibar.
    ///
    /// The omnibar is positioned above the current text selection, clamped to the viewport. If
    /// no portion of the text selection is visible, the omnibar is not shown.
    pub fn positioning(render_state: &RenderState) -> OffsetPositioning {
        let selection_position = render_state.saved_positions().text_selection_id();

        OffsetPositioning::from_axes(
            PositioningAxis::relative_to_stack_child(
                &selection_position,
                PositionedElementOffsetBounds::ParentByPosition,
                OffsetType::Pixel(0.),
                AnchorPair::new(XAxisAnchor::Middle, XAxisAnchor::Middle),
            )
            .with_conditional_anchor(),
            PositioningAxis::relative_to_stack_child(
                &selection_position,
                PositionedElementOffsetBounds::WindowByPosition,
                OffsetType::Pixel(-4.),
                // TODO(ben): Decide if this should be above or below the cursor based
                //  on its location within the viewport.
                AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Bottom),
            )
            .with_conditional_anchor(),
        )
    }

    fn toggle_style(&mut self, style: TextStyles, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.toggle_style(style, ctx);
        });
        ctx.notify();
    }

    fn unset_link(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.unset_link(ctx);
        });
        ctx.notify();
    }

    fn convert_block(&mut self, style: BufferBlockStyle, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.convert_block(style, ctx);
        });
        ctx.notify();
    }

    fn render_action_button(
        &self,
        appearance: &Appearance,
        icon: Icon,
        action: OmnibarAction,
        active: bool,
        mouse_state: &MouseStateHandle,
    ) -> Box<dyn Element> {
        let active_background = appearance.theme().surface_3().into();
        let button = icon_button(appearance, icon, active, mouse_state.clone())
            .with_style(UiComponentStyles {
                width: Some(ACTION_BUTTON_SIZE),
                height: Some(ACTION_BUTTON_SIZE),
                // Explicitly override the default icon button padding of 4px.
                // With a button size of 24px, 1px of border, and 1px of padding, each icon should
                // be 20px.
                padding: Some(Coords::uniform(1.)),
                font_color: Some(appearance.theme().nonactive_ui_text_color().into()),
                ..Default::default()
            })
            .with_active_styles(UiComponentStyles {
                font_color: Some(
                    appearance
                        .theme()
                        .active_ui_text_color()
                        // .with_opacity(100)
                        .into_solid(),
                ),
                background: Some(active_background),
                border_color: None,
                ..Default::default()
            });

        let renderable_button = button
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
            .finish();

        Container::new(renderable_button)
            .with_margin_left(OMNIBAR_PADDING / 2.)
            .with_margin_right(OMNIBAR_PADDING / 2.)
            .finish()
    }

    fn render_separator(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Rect::new()
                    .with_background(appearance.theme().disabled_ui_text_color())
                    .finish(),
            )
            .with_width(1.)
            .finish(),
        )
        .with_margin_left(OMNIBAR_PADDING)
        .with_margin_right(OMNIBAR_PADDING)
        .finish()
    }

    /// Updates the omnibar state in response to rich text model changes.
    fn handle_model_event(
        &mut self,
        _handle: ModelHandle<NotebooksEditorModel>,
        event: &RichTextEditorModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if let RichTextEditorModelEvent::ActiveStylesChanged {
            selection_text_styles,
            block_type,
            ..
        } = event
        {
            // The omnibar only applies to selections, so we only care about
            // the selected text styles.
            self.active_text_styles = Some(selection_text_styles.clone());
            self.active_block_type = Some(block_type.clone());
            self.reset_conversion_menu(BlockType::from(block_type), ctx);
            ctx.notify();
        }
    }

    /// Reset the conversion dropdown to the selected block type.
    fn reset_conversion_menu(&self, block_type: BlockType, ctx: &mut ViewContext<Self>) {
        let block_name = block_type.label();
        self.block_conversion_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_selected_by_name(block_name, ctx);
        });
    }

    fn handle_dropdown_event(
        &mut self,
        _handle: ViewHandle<CompactDropdown<OmnibarAction>>,
        event: &CompactDropdownEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CompactDropdownEvent::Close => {
                // In case the menu was closed without converting to a new type of block, reset it to
                // the original block type. If the block _was_ converted, this will be overridden
                // by the incoming model event.
                if let Some(block_type) = &self.active_block_type {
                    self.reset_conversion_menu(BlockType::from(block_type), ctx);
                }
                // When the dropdown menu closes, restore focus to the parent editor view. Otherwise,
                // opening it (even if it's then dismissed) prevents typing.
                ctx.dispatch_typed_action(&EditorViewAction::Focus);
            }
        }
    }
}

impl Entity for Omnibar {
    type Event = OmnibarEvent;
}

impl View for Omnibar {
    fn ui_name() -> &'static str {
        "Omnibar"
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut actions = Flex::row().with_main_axis_size(MainAxisSize::Min);

        let text_format_enabled = match self.active_block_type.as_ref() {
            Some(ContentBlockType::Item(_)) => false,
            Some(ContentBlockType::Text(block)) => block.allows_formatting(),
            None => true,
        };

        actions.add_child(
            Container::new(ChildView::new(&self.block_conversion_dropdown).finish())
                .with_margin_left(OMNIBAR_PADDING)
                .with_margin_right(OMNIBAR_PADDING)
                .finish(),
        );

        if text_format_enabled {
            actions.add_child(self.render_separator(appearance));

            actions.add_child(
                self.render_action_button(
                    appearance,
                    Icon::Bold,
                    OmnibarAction::BoldSelection,
                    self.active_text_styles
                        .as_ref()
                        .is_some_and(|s| !s.is_normal_weight()),
                    &self.bold_button_state,
                ),
            );

            actions.add_child(
                self.render_action_button(
                    appearance,
                    Icon::Italic,
                    OmnibarAction::ItalicizeSelection,
                    self.active_text_styles
                        .as_ref()
                        .is_some_and(|s| s.is_italic()),
                    &self.italicize_button_state,
                ),
            );

            actions.add_child(
                self.render_action_button(
                    appearance,
                    Icon::Underline,
                    OmnibarAction::UnderlineSelection,
                    self.active_text_styles
                        .as_ref()
                        .is_some_and(|s| s.is_underlined()),
                    &self.underline_button_state,
                ),
            );

            actions.add_child(
                self.render_action_button(
                    appearance,
                    Icon::Strikethrough,
                    OmnibarAction::StrikeThroughSelection,
                    self.active_text_styles
                        .as_ref()
                        .is_some_and(|s| s.is_strikethrough()),
                    &self.strikethrough_button_state,
                ),
            );

            let link_active = self
                .active_text_styles
                .as_ref()
                .is_some_and(|s| s.is_link());
            actions.add_child(self.render_action_button(
                appearance,
                Icon::Link,
                if link_active {
                    OmnibarAction::UnstyleLink
                } else {
                    OmnibarAction::OpenLinkEditor
                },
                link_active,
                &self.link_button_state,
            ));

            actions.add_child(
                self.render_action_button(
                    appearance,
                    Icon::InlineCode,
                    OmnibarAction::InlineCodeSelection,
                    self.active_text_styles
                        .as_ref()
                        .is_some_and(|s| s.is_inline_code()),
                    &self.inline_code_button_state,
                ),
            );
        }

        let bar = Container::new(
            ConstrainedBox::new(actions.finish())
                .with_height(OMNIBAR_HEIGHT - 2. * OMNIBAR_PADDING)
                .with_min_width(0.)
                .finish(),
        )
        .with_uniform_padding(OMNIBAR_PADDING)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .with_background(appearance.theme().surface_2())
        .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
        .with_drop_shadow(DropShadow::default())
        .finish();

        Compact::new(bar).finish()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum OmnibarAction {
    /// Toggle bold styling on the selected text.
    BoldSelection,
    /// Toggle italic styling on the selected text.
    ItalicizeSelection,
    UnderlineSelection,
    StrikeThroughSelection,
    InlineCodeSelection,
    OpenLinkEditor,
    UnstyleLink,
    /// Convert the selected text to a particular kind of block.
    ConvertBlock(BufferBlockStyle),
}

impl TypedActionView for Omnibar {
    type Action = OmnibarAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            OmnibarAction::BoldSelection => self.toggle_style(TextStyles::default().bold(), ctx),
            OmnibarAction::ItalicizeSelection => {
                self.toggle_style(TextStyles::default().italic(), ctx)
            }
            OmnibarAction::UnderlineSelection => {
                self.toggle_style(TextStyles::default().underline(), ctx)
            }
            OmnibarAction::StrikeThroughSelection => {
                self.toggle_style(TextStyles::default().strikethrough(), ctx)
            }
            OmnibarAction::InlineCodeSelection => {
                self.toggle_style(TextStyles::default().inline_code(), ctx)
            }
            OmnibarAction::OpenLinkEditor => ctx.emit(OmnibarEvent::OpenLinkEditor),
            OmnibarAction::UnstyleLink => self.unset_link(ctx),
            OmnibarAction::ConvertBlock(style) => {
                self.convert_block(style.clone(), ctx);
            }
        }
    }

    fn action_accessibility_contents(
        &mut self,
        action: &Self::Action,
        ctx: &mut ViewContext<Self>,
    ) -> ActionAccessibilityContent {
        match action {
            OmnibarAction::BoldSelection => self
                .model
                .as_ref(ctx)
                .style_toggle_a11y(BufferTextStyle::bold()),
            OmnibarAction::ItalicizeSelection => self
                .model
                .as_ref(ctx)
                .style_toggle_a11y(BufferTextStyle::Italic),
            OmnibarAction::UnderlineSelection => self
                .model
                .as_ref(ctx)
                .style_toggle_a11y(BufferTextStyle::Underline),
            OmnibarAction::StrikeThroughSelection => self
                .model
                .as_ref(ctx)
                .style_toggle_a11y(BufferTextStyle::StrikeThrough),
            OmnibarAction::InlineCodeSelection => self
                .model
                .as_ref(ctx)
                .style_toggle_a11y(BufferTextStyle::InlineCode),
            OmnibarAction::ConvertBlock(style) => {
                ActionAccessibilityContent::Custom(AccessibilityContent::new_without_help(
                    format!("Convert to {}", BlockType::from(style).label()),
                    WarpA11yRole::UserAction,
                ))
            }
            OmnibarAction::OpenLinkEditor => ActionAccessibilityContent::from_debug(),
            OmnibarAction::UnstyleLink => ActionAccessibilityContent::Custom(
                AccessibilityContent::new_without_help("Remove link", WarpA11yRole::UserAction),
            ),
        }
    }
}

/// Creates a dropdown item for converting to the given block type.
fn conversion_item(
    block_type: BlockType,
    appearance: &Appearance,
) -> CompactDropdownItem<OmnibarAction> {
    let action = OmnibarAction::ConvertBlock(block_type.into());
    let mut item = CompactDropdownItem::new(block_type.icon(), block_type.label(), action);
    if let Some(icon_fill) = block_type.icon_color(appearance) {
        item = item.with_icon_color(icon_fill);
    }
    item
}

/// Small UI element that disregards the parent's minimum size constraint. This
/// lets its child shrink to its content size. It's useful for offset-positioned
/// [`Flex`] elements, which often have a minimum size constraint of their parent's
/// size, and would otherwise expand to fill it.
struct Compact {
    child: Box<dyn Element>,
}

impl Compact {
    fn new(child: Box<dyn Element>) -> Self {
        Self { child }
    }
}

impl Element for Compact {
    fn layout(
        &mut self,
        constraint: warpui::SizeConstraint,
        ctx: &mut warpui::LayoutContext,
        app: &warpui::AppContext,
    ) -> Vector2F {
        self.child.layout(
            SizeConstraint {
                min: Vector2F::zero(),
                max: constraint.max,
            },
            ctx,
            app,
        )
    }

    fn paint(
        &mut self,
        origin: Vector2F,
        ctx: &mut warpui::PaintContext,
        app: &warpui::AppContext,
    ) {
        self.child.paint(origin, ctx, app)
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn dispatch_event(
        &mut self,
        event: &warpui::event::DispatchedEvent,
        ctx: &mut warpui::EventContext,
        app: &warpui::AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut warpui::AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app)
    }

    fn z_index(&self) -> Option<warpui::elements::ZIndex> {
        self.child.z_index()
    }

    fn bounds(&self) -> Option<RectF> {
        self.child.bounds()
    }

    fn parent_data(&self) -> Option<&dyn std::any::Any> {
        self.child.parent_data()
    }
}
