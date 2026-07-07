//! Voltron (https://en.wikipedia.org/wiki/Voltron) in Warp is a code name for the UI element used
//! for workflows, ai commands and history search. As @charlespierce pointed out: it's 3 mini lions
//! (features) coming together to form one super bot (this view). Hence the name. Don't ask me, I
//! haven't watched it.
//!
//!
//! ██╗░░░██╗░█████╗░██╗░░░░░████████╗██████╗░░█████╗░███╗░░██╗
//! ██║░░░██║██╔══██╗██║░░░░░╚══██╔══╝██╔══██╗██╔══██╗████╗░██║
//! ╚██╗░██╔╝██║░░██║██║░░░░░░░░██║░░░██████╔╝██║░░██║██╔██╗██║
//! ░╚████╔╝░██║░░██║██║░░░░░░░░██║░░░██╔══██╗██║░░██║██║╚████║
//! ░░╚██╔╝░░╚█████╔╝███████╗░░░██║░░░██║░░██║╚█████╔╝██║░╚███║
//! ░░░╚═╝░░░░╚════╝░╚══════╝░░░╚═╝░░░╚═╝░░╚═╝░╚════╝░╚═╝░░╚══╝
//!
//!

use std::path::PathBuf;

use enclose::enclose;
use pathfinder_geometry::vector::Vector2F;
use vec1::Vec1;
use warpui::accessibility::AccessibilityContent;
use warpui::elements::{
    resizable_state_handle, Border, ChildAnchor, ChildView, Clipped, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, Dismiss, DispatchEventResult, Element, EventHandler, Flex,
    Icon, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Resizable, ResizableStateHandle, Shrinkable, Stack,
};
use warpui::geometry::vector::vec2f;
use warpui::keymap::{Context, FixedBinding};
use warpui::ui_components::button::{ButtonVariant, TextAndIcon, TextAndIconAlignment};
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::appearance::Appearance;
use crate::editor::{
    EditorView, Event as EditorEvent, PlainTextEditorViewAction, PropagateAndNoOpNavigationKeys,
    SingleLineEditorOptions,
};
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields};
use crate::terminal::input::MenuPositioning;
use crate::terminal::resizable_data::{ModalType, ResizableData, DEFAULT_VOLTRON_WIDTH};
use crate::util::bindings::{self, CustomAction};
use crate::workflows::categories::CategoriesView;

const DROPDOWN_BUTTON_WIDTH: f32 = 200.;
const DROPDOWN_PADDING: f32 = 6.;
const DROPDOWN_WIDTH: f32 = 170.;
const MAX_WIDTH_BOUND: f32 = 1.;
const VOLTRON_RIGHT_PADDING: f32 = 15.;

/// Padding between Voltron and the start of the editor
const EDITOR_PADDING_LEFT: f32 = 14.;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        VoltronAction::Close,
        id!(Voltron::ui_name()),
    )]);
}

/// Container view that implements the logic / UI for the Voltron itself, including colors
/// and main view rendering.
/// `feature_view_handle` stores the actual feature's implementation (view).
#[derive(Clone)]
pub struct VoltronFeatureView {
    pub name: VoltronItem,
    pub feature_view_handle: VoltronFeatureViewHandle,
}

/// Container enum for features implemented and added to Voltron.
#[derive(Clone)]
pub enum VoltronFeatureViewHandle {
    Workflows(ViewHandle<CategoriesView>),
}

/// Enum used to identify the item in Voltron.
#[derive(Clone, PartialEq, Eq, Copy, Debug)]
pub enum VoltronItem {
    AiCommands,
    Workflows,
    History,
}

impl VoltronItem {
    pub fn as_str(&self) -> &'static str {
        match self {
            VoltronItem::AiCommands => "A.I. Command Search",
            VoltronItem::Workflows => "Workflows",
            VoltronItem::History => "History Search",
        }
    }
}

/// Structure used by the `on_load` method to pass extra metadata to features.
#[derive(Clone, Default)]
pub struct VoltronMetadata {
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    pub active_session_path_if_local: Option<PathBuf>,

    /// Starting editor text for the buffer within Voltron.
    pub starting_editor_text: Option<String>,

    /// The keybinding context for updating shortcut strings in the Voltron menu.
    pub keymap_context: Context,

    /// How voltron is positioned relative to the input box
    pub menu_positioning: MenuPositioning,
}

/// Trait that each of the views used within Voltron should implement, so that all the events are
/// then properly propagated to corresponding views.
pub trait VoltronFeatureViewMeta {
    /// Placeholder text to show in the editor.
    fn editor_placeholder_text(&self) -> &'static str;

    /// Voltron captures all the editor events and passes them to the currently focused feature by
    /// calling this method. Note that it does not call `ctx.notify()`, so it's up to the feature
    /// to do so if needed.
    fn handle_editor_event(
        &mut self,
        event: &EditorEvent,
        current_editor_text: &str,
        ctx: &mut ViewContext<Self>,
    );

    /// Method called every time a specific feature is loaded. It passes metadata object.
    fn on_load(&mut self, _metadata: VoltronMetadata, _ctx: &mut ViewContext<Self>) {}

    /// Returns the custom action related to the given feature.
    fn custom_action() -> Option<CustomAction>;

    fn close(&mut self, _ctx: &mut ViewContext<Self>) {}
}

/// The actual Voltron view, which stores multiple VoltronFeatureViews and controls which one is
/// opened etc.
/// TODO remove voltron from the code given we are not using it anymore, and we have universal search instead.
pub struct Voltron {
    /// List of features available in Voltron
    features: Vec1<VoltronFeatureView>,
    /// Index of the currently showcased feature. If set to None - then Voltron is closed.
    current_feature_idx: usize,
    editor: ViewHandle<EditorView>,
    dropdown_mouse_state: MouseStateHandle,
    dropdown: ViewHandle<Menu<VoltronAction>>,
    expand_dropdown: bool,
    /// The last metadata then the view was loaded.
    metadata: VoltronMetadata,
    resizable_state_handle: ResizableStateHandle,
}

impl VoltronFeatureViewHandle {
    fn custom_action(&self) -> Option<CustomAction> {
        match self {
            VoltronFeatureViewHandle::Workflows(_) => CategoriesView::custom_action(),
        }
    }

    fn child_view(&self) -> Box<dyn Element> {
        use VoltronFeatureViewHandle::*;
        match self {
            Workflows(view_handle) => ChildView::new(view_handle).finish(),
        }
    }
}

impl VoltronFeatureView {
    pub fn new(name: VoltronItem, feature_view_handle: VoltronFeatureViewHandle) -> Self {
        Self {
            name,
            feature_view_handle,
        }
    }
}

impl Voltron {
    pub fn new(features: Vec1<VoltronFeatureView>, ctx: &mut ViewContext<Self>) -> Self {
        let editor = {
            ctx.add_typed_action_view(|ctx| {
                let options = SingleLineEditorOptions {
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    ..Default::default()
                };
                EditorView::single_line(options, ctx)
            })
        };
        ctx.subscribe_to_view(&editor, |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        let appearance = Appearance::as_ref(ctx);
        let theme = appearance.theme().clone();
        let dropdown = ctx.add_typed_action_view(|_ctx| {
            Menu::new()
                .with_width(DROPDOWN_WIDTH)
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
        });
        ctx.subscribe_to_view(&dropdown, move |me, _, event, ctx| {
            me.handle_menu_event(event, ctx);
        });

        let resizable_data_handle = ResizableData::handle(ctx);
        let resizable_state_handle = match resizable_data_handle
            .as_ref(ctx)
            .get_handle(ctx.window_id(), ModalType::VoltronWidth)
        {
            Some(handle) => handle,
            None => {
                log::error!("Couldn't retrieve voltron resizable state handle.");
                resizable_state_handle(DEFAULT_VOLTRON_WIDTH)
            }
        };

        Self {
            features,
            current_feature_idx: 0,
            editor,
            dropdown_mouse_state: Default::default(),
            dropdown,
            expand_dropdown: false,
            metadata: Default::default(),
            resizable_state_handle,
        }
    }

    fn handle_menu_event(&mut self, event: &MenuEvent, ctx: &mut ViewContext<Self>) {
        if let MenuEvent::Close { via_select_item: _ } = event {
            self.close_dropdown(ctx)
        }
    }

    fn close_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        self.expand_dropdown = false;
        ctx.notify();
    }

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        if let EditorEvent::Escape = event {
            self.close(ctx);
            return;
        }

        if let Some(current_feature) = self.current_feature() {
            let current_editor_text = self.editor.as_ref(ctx).buffer_text(ctx);
            match current_feature.feature_view_handle {
                VoltronFeatureViewHandle::Workflows(view_handle) => view_handle
                    .update(ctx, |view, ctx| {
                        view.handle_editor_event(event, &current_editor_text, ctx)
                    }),
            }
        }
    }

    fn placeholder(&mut self, ctx: &mut ViewContext<Self>) -> Option<&'static str> {
        if let Some(current_feature) = self.current_feature() {
            Some(match current_feature.feature_view_handle {
                VoltronFeatureViewHandle::Workflows(view_handle) => {
                    view_handle.read(ctx, |view, _| view.editor_placeholder_text())
                }
            })
        } else {
            None
        }
    }

    fn load(&mut self, metadata: VoltronMetadata, ctx: &mut ViewContext<Self>) {
        self.metadata = metadata;
        if let Some(current_feature) = self.current_feature() {
            if let Some(editor_text) = self.metadata.starting_editor_text.as_ref() {
                self.editor.update(ctx, |view, ctx| {
                    view.clear_buffer_and_reset_undo_stack(ctx);
                    view.user_initiated_insert(
                        editor_text,
                        PlainTextEditorViewAction::SystemInsert,
                        ctx,
                    );
                });
            }

            match current_feature.feature_view_handle {
                VoltronFeatureViewHandle::Workflows(view_handle) => {
                    view_handle.update(ctx, |view, ctx| view.on_load(self.metadata.clone(), ctx))
                }
            }
        }
    }

    fn current_feature(&self) -> Option<VoltronFeatureView> {
        let feature_idx = if self.current_feature_idx >= self.features.len() {
            0
        } else {
            self.current_feature_idx
        };
        self.features.get(feature_idx).cloned()
    }

    fn feature_index(&self, feature_name: VoltronItem) -> Option<usize> {
        self.features
            .iter()
            .position(|feature| feature_name == feature.name)
    }

    fn select(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        self.current_feature_idx = index;

        let placeholder = self.placeholder(ctx);

        self.editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
            if let Some(placeholder) = placeholder {
                editor.set_placeholder_text(placeholder, ctx);
            }
        });
    }

    /// Selecting a feature by its name. No-op if there's no such feature.
    pub fn select_and_refresh_by_name(
        &mut self,
        feature_name: VoltronItem,
        metadata: VoltronMetadata,
        ctx: &mut ViewContext<Self>,
    ) {
        let idx = self.feature_index(feature_name);
        match idx {
            None => {
                log::info!(
                    "Trying to open {} in Voltron, but no such feature registered",
                    feature_name.as_str()
                );
                // Close voltron as a feature was requested that isn't registered.
                self.close(ctx);
            }
            Some(idx) => {
                self.select(idx, ctx);
                self.load(metadata, ctx);
                ctx.focus_self();
                ctx.notify();
            }
        }
    }

    fn update_menu_shortcuts(&mut self, ctx: &mut ViewContext<Self>) {
        let features = self.features.clone();
        let context = self.metadata.keymap_context.clone();
        self.dropdown.update(ctx, move |menu, ctx| {
            let items: Vec<MenuItem<VoltronAction>> = features
                .into_iter()
                .map(|view| {
                    let item = MenuItemFields::new(view.name.as_str())
                        .with_on_select_action(VoltronAction::SelectAndRefresh(view.name));
                    let label = view.feature_view_handle.custom_action().and_then(enclose!(
                        (context) | action | {
                            ctx.binding_for_custom_action(action.into(), vec![context])
                                .and_then(|binding| bindings::trigger_to_keystroke(binding.trigger))
                                .map(|keystroke| keystroke.displayed())
                        }
                    ));
                    item.with_key_shortcut_label(label).into_item()
                })
                .collect();
            menu.set_items(items, ctx);
        });
    }

    /// Closing Voltron if there is a currently selected feature. Otherwise this method is a no-op.
    pub fn close(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(current_feature) = self.current_feature() {
            match current_feature.feature_view_handle {
                VoltronFeatureViewHandle::Workflows(view_handle) => {
                    view_handle.update(ctx, |view, ctx| view.close(ctx))
                }
            }
        }
        self.close_dropdown(ctx);
        ctx.emit(VoltronEvent::Close);
        ctx.notify();
    }

    fn render_dropdown(&self, appearance: &Appearance) -> Box<dyn Element> {
        let current_feature = self
            .current_feature()
            .expect("Voltron is only rendered when it's visible and the feature exists");

        let icon_path = "bundled/svg/chevron-up.svg";
        let mut dropdown_stack = Stack::new().with_child(
            Container::new(
                ConstrainedBox::new(
                    // Why do we have event handler here wrapped around the button, you may
                    // wonder? Well, it's because dropdown has its own `Dismiss` which also
                    // means it'll close the dropdown on the _mouse down_ of the button, which
                    // caused weird behavior of `ToggleDropdown` action. This way, we ensure
                    // the order of events is correct and we open/close dropdown when needed.
                    EventHandler::new(
                        appearance
                            .ui_builder()
                            .button(ButtonVariant::Outlined, self.dropdown_mouse_state.clone())
                            .with_text_and_icon_label(
                                TextAndIcon::new(
                                    TextAndIconAlignment::TextFirst,
                                    current_feature.name.as_str().to_string(),
                                    Icon::new(icon_path, appearance.theme().active_ui_text_color()),
                                    MainAxisSize::Min,
                                    MainAxisAlignment::SpaceBetween,
                                    vec2f(15., 15.),
                                )
                                .with_inner_padding(10.),
                            )
                            // .with_text_label(current_feature.name.as_str().to_string())
                            .build()
                            .finish(),
                    )
                    .on_left_mouse_down(|ctx, _, _| {
                        ctx.dispatch_typed_action(VoltronAction::ToggleDropdown);
                        DispatchEventResult::StopPropagation
                    })
                    .finish(),
                )
                .with_max_width(DROPDOWN_BUTTON_WIDTH)
                .finish(),
            )
            .finish(),
        );

        if self.expand_dropdown {
            dropdown_stack.add_positioned_overlay_child(
                Container::new(ChildView::new(&self.dropdown).finish())
                    .with_padding_left(-(DROPDOWN_WIDTH - DROPDOWN_BUTTON_WIDTH))
                    .with_padding_bottom(DROPDOWN_PADDING)
                    .finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., -3.),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::TopRight,
                    ChildAnchor::BottomRight,
                ),
            );
        }

        dropdown_stack.finish()
    }

    /// Callback for computing width bounds of the voltron modal (min, max)
    /// Takes window size and returns (min, max) bounds to the resizable element.
    fn compute_panel_width_bounds(window_bounds: Vector2F) -> (f32, f32) {
        (
            400.,
            (window_bounds.x() * MAX_WIDTH_BOUND - VOLTRON_RIGHT_PADDING).max(400.),
        )
    }
}

pub enum VoltronEvent {
    Close,
}

impl Entity for Voltron {
    type Event = VoltronEvent;
}

impl View for Voltron {
    fn ui_name() -> &'static str {
        "Voltron"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.editor);
            ctx.notify();
        }
    }

    /// Voltron by itself doesn't provide any a11y features. Instead it delegates it to the
    /// currently selected feature's a11y methods.
    fn accessibility_contents(&self, ctx: &AppContext) -> Option<AccessibilityContent> {
        if let Some(current_feature) = self.current_feature() {
            // TODO create a delegate macro rather than having all those matches everywhere
            match current_feature.feature_view_handle {
                VoltronFeatureViewHandle::Workflows(view_handle) => {
                    view_handle.as_ref(ctx).accessibility_contents(ctx)
                }
            }
        } else {
            None
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn warpui::Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let current_feature = self
            .current_feature()
            .expect("Voltron is only rendered when it's visible and the feature exists");
        let editor = self.editor.as_ref(app);
        let height = editor.line_height(app.font_cache(), appearance);

        let editor =
            ConstrainedBox::new(Clipped::new(ChildView::new(&self.editor).finish()).finish())
                .with_height(height)
                .finish();

        let mut editor_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., editor).finish());
        if self.features.len() > 1 {
            editor_row.add_child(self.render_dropdown(appearance));
        }

        let editor_container = Container::new(
            ConstrainedBox::new(editor_row.finish())
                .with_min_height(20.)
                .finish(),
        )
        .with_padding_left(EDITOR_PADDING_LEFT)
        .with_padding_top(12.)
        .with_padding_right(12.)
        .with_padding_bottom(12.)
        .with_background(theme.surface_1())
        .finish();

        let feature_view =
            Shrinkable::new(1., current_feature.feature_view_handle.child_view()).finish();

        let voltron_content = match self.metadata.menu_positioning {
            MenuPositioning::AboveInputBox => Flex::column()
                .with_child(feature_view)
                .with_child(editor_container)
                .finish(),
            MenuPositioning::BelowInputBox => Flex::column()
                .with_child(editor_container)
                .with_child(feature_view)
                .finish(),
        };

        let container = Container::new(
            ConstrainedBox::new(voltron_content)
                .with_max_height(402.)
                .finish(),
        )
        .with_margin_top(117.)
        .with_background(theme.surface_2())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_border(Border::all(1.0).with_border_fill(theme.outline()));

        let resizable = Resizable::new(self.resizable_state_handle.clone(), container.finish())
            .on_resize(move |ctx, _| ctx.notify())
            .with_bounds_callback(Box::new(Self::compute_panel_width_bounds))
            .finish();

        Dismiss::new(resizable)
            .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(VoltronAction::Close))
            .finish()
    }
}

#[derive(Debug, Clone)]
pub enum VoltronAction {
    Close,
    SelectAndRefresh(VoltronItem),
    ToggleDropdown,
}

impl TypedActionView for Voltron {
    type Action = VoltronAction;

    fn handle_action(&mut self, action: &VoltronAction, ctx: &mut ViewContext<Self>) {
        match action {
            VoltronAction::Close => {
                self.close(ctx);
            }
            VoltronAction::SelectAndRefresh(name) => {
                self.select_and_refresh_by_name(*name, self.metadata.clone(), ctx);
                self.expand_dropdown = false;
                ctx.notify();
            }
            VoltronAction::ToggleDropdown => {
                self.expand_dropdown = !self.expand_dropdown;
                if self.expand_dropdown {
                    self.update_menu_shortcuts(ctx);
                }
                ctx.notify();
            }
        }
    }
}
