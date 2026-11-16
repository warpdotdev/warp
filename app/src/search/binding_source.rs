use std::sync::Arc;

use warpui::{Entity, EntityId, WindowId};

use crate::util::bindings::CommandBinding;

/// Type alias for the filter function that determines which command bindings to show
pub type BindingFilterFn = Option<Arc<dyn Fn(&CommandBinding) -> bool>>;

/// A model for tracking the current source of bindings for the command palette
///
/// This is necessary due to a quirk in how the UI Framework handles event handlers / callbacks:
///
/// In order to work around Rusts restriction on having two mutable references to the same data,
/// the framework _removes_ a view from the map of all views before calling a handler (it then
/// immediately re-inserts it into the map afterwards). This means then when a handler is being
/// executed in a given View, that View is _not_ in the global map. Since the Command Palette is
/// launched from the Workspace, which is the root of all terminal views, if we attempt to load the
/// key bindings from somewhere within that view (even by calling `command_palette.update()`), it
/// will fail with the Workspace missing from the map.
///
/// Instead, we create a small Model to cache the binding source information (window and view id)
/// and subscribe to any changes to that model from here. Then the model update handler is
/// scheduled after the event handler callback completes. This means that the update handler is
/// called on the CommandPalette directly, rather than the Workspace. This is safe because the
/// CommandPalette won't ever be the parent of any View that launches itself, so the fact that it
/// won't be in the view map won't affect our ability to load the key bindings for other views.
pub enum BindingSource {
    None,
    View {
        window_id: WindowId,
        view_id: EntityId,
        binding_filter_fn: BindingFilterFn,
    },
}

impl Entity for BindingSource {
    type Event = ();
}
