mod context;
mod handle;

pub use self::context::*;
pub use self::handle::*;
use super::EntityId;
use crate::accessibility::{AccessibilityContent, ActionAccessibilityContent};
use crate::{keymap, Action, AppContext, CursorInfo, Element, Entity};

pub enum FocusContext {
    SelfFocused,
    DescendentFocused(EntityId),
}

impl FocusContext {
    pub fn is_self_focused(&self) -> bool {
        matches!(self, Self::SelfFocused)
    }
}

pub enum BlurContext {
    SelfBlurred,
    DescendentBlurred(EntityId),
}

impl BlurContext {
    pub fn is_self_blurred(&self) -> bool {
        matches!(self, Self::SelfBlurred)
    }
}

/// An interface for interactive, renderable UI components.
///
/// Conceptually, an implementation of [`View`] is analogous to a React
/// component - a structure that holds instance state and can be asked to render
/// itself, a process that produces a tree of rendering primitives (in [`warpui`](crate),
/// these are structures that implement [`Element`]; in React, these are DOM
/// elements).
///
/// # Example
///
/// ```
/// # use warpui_core::{*, elements::Rect};
///
/// struct MyView {}
///
/// impl Entity for MyView {
///     type Event = ();
/// }
///
/// impl View for MyView {
///     fn ui_name() -> &'static str { "MyView" }
///     fn render(&self, app: &AppContext) -> Box<dyn Element> {
///         Rect::new().finish()
///     }
/// }
/// ```
pub trait View: Entity {
    /// Returns a unique name for this implementation of View.
    fn ui_name() -> &'static str;

    /// Produces an [`Element`] tree representation of this view.
    fn render(&self, app: &AppContext) -> Box<dyn Element>;

    /// Handles the view or its descendent receiving focus.
    /// Which view received focus is indicated by the [`FocusContext`].
    fn on_focus(&mut self, _focus_ctx: &FocusContext, _ctx: &mut ViewContext<Self>) {}

    /// Accessibility (a11y) support for [`View`]s.
    ///
    /// Whenever the view is focused (i.e. [`View::on_focus`]), the provided a11y content
    /// is read out through the native screen reader (e.g. VoiceOver in MacOS).
    ///
    /// While the contents default to [`None`] (i.e. no a11y content), each view
    /// is encouraged to provide sensible a11y content so that visually-impaired users
    /// can follow along in the application.
    fn accessibility_contents(&self, _ctx: &AppContext) -> Option<AccessibilityContent> {
        None
    }

    /// Reports the active cursor position for the view, if any.
    /// This only applies to [`View`]s that have some sort of text editor.
    ///
    /// We intentionally provide _immutable_ access to the [`ViewContext`];
    /// querying the active cursor position shouldn't necessitate writes.
    fn active_cursor_position(&self, _ctx: &ViewContext<Self>) -> Option<CursorInfo> {
        None
    }

    /// Handles the view or its descendent losing focus.
    /// Which view lost focus is indicated by the [`BlurContext`].
    fn on_blur(&mut self, _blur_ctx: &BlurContext, _ctx: &mut ViewContext<Self>) {}

    /// Handles the view's containing window closing.
    fn on_window_closed(&mut self, _ctx: &mut ViewContext<Self>) {}

    /// Called when the view is transferred from one window to another.
    /// Views can override this to update any window-specific state.
    fn on_window_transferred(
        &mut self,
        _source_window_id: super::super::WindowId,
        _target_window_id: super::super::WindowId,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    /// Returns a representation of the current UI context for use in computing
    /// the set of valid actions/keyboard shortcuts.
    fn keymap_context(&self, _: &AppContext) -> keymap::Context {
        Self::default_keymap_context()
    }

    /// Returns the default context for a view.
    fn default_keymap_context() -> keymap::Context {
        let mut ctx = keymap::Context::default();
        ctx.set.insert(Self::ui_name());
        ctx
    }

    /// Allows a view to hook into any interactions with it or its children.
    ///
    /// A valid interaction must be handled by the view or its children, and includes:
    /// - all mouse events, except [`Event::MouseMoved`] and [`Event::ScrollWheel`]
    /// - all keyboard events, including all [`CustomAction`]s and [`StandardAction::Paste`]
    fn self_or_child_interacted_with(&self, _ctx: &mut ViewContext<Self>) {}

    /// Returns the current [`AccessibilityData`] for this view, if `Some`. Returning a valid
    /// [`AccessibilityData`] struct here indicates that this view should belong in the
    /// accessibility tree of this application.
    fn accessibility_data(&self, _ctx: &mut ViewContext<Self>) -> Option<AccessibilityData> {
        None
    }
}

/// The accessibility data of a current view.
pub struct AccessibilityData {
    /// The contents of the view.
    pub content: String,
}

/// An interface for a structure (typically a [`View`]) that handle actions
/// of a particular type.
pub trait TypedActionView {
    type Action: Action;

    /// Handles an action of type [`Self::Action`](TypedActionView::Action)
    /// that was dispatched from this view or any descendant.
    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {}

    /// TypedActionViews can implement another way to provide context about what’s going on with the app. After each `handle_action` call, the UI framework calls `action_accessibility_contents(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) -> ActionAccessibilityContent`.
    ///
    /// ### When and how to use it?
    /// This method should be implemented for all the meaningful Actions. For example, actions related to mouse movement are not meaningful, but action related to user copying the selected content - is.
    /// If your Action enum has tons of actions, and only some of them are meaningful, you can use helper implementations from the `warpui_core::accessibility` (like `ActionAccessibilityContent::Default()` or `ActionAccessibility_content::from_debug()`).
    fn action_accessibility_contents(
        &mut self,
        _action: &Self::Action,
        _ctx: &mut ViewContext<Self>,
    ) -> ActionAccessibilityContent {
        ActionAccessibilityContent::default()
    }
}
