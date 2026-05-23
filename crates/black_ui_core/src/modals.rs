use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{AppContext, View, ViewContext};

/// The data for displaying a platform-native alert dialog (modal).
pub struct AlertDialog {
    /// The primary text.
    pub message_text: String,
    /// Smaller, more detailed text.
    pub info_text: String,
    /// Each item is a button, and the String is the text for the button.
    pub buttons: Vec<String>,
}

impl AlertDialog {
    pub fn new(
        message_text: impl Into<String>,
        info_text: impl Into<String>,
        buttons: &[&str],
    ) -> Self {
        Self {
            message_text: message_text.into(),
            info_text: info_text.into(),
            buttons: buttons.iter().map(|s| String::from(*s)).collect(),
        }
    }
}

/// This is for requesting an [`AlertDialog`] on a [`AppContext`] or [`ViewContext`].
/// Unlike [`AlertDialog`], this data structure co-locates the handlers for each button with the
/// button text.
pub struct AlertDialogWithCallbacks<F> {
    /// The primary text.
    pub message_text: String,
    /// Smaller, more detailed text.
    pub info_text: String,
    /// Each item is a button, both its text and "on click" callback.
    pub button_data: Vec<ModalButton<F>>,
    /// Callback to run if the user clicks the "don't ask again" checkbox. This should prevent that
    /// type of modal in the future.
    pub on_disable: F,
}

/// Wraps the button text with its click handler.
pub struct ModalButton<F> {
    /// The text on the button.
    pub title: String,
    /// The click handler for this button.
    pub on_click: F,
}

/// The signature of the callback when requesting a modal from a [`AppContext`].
pub type AppModalCallback = Box<dyn FnOnce(&mut AppContext)>;
/// The signature of the callback when requesting a modal from a [`ViewContext`].
pub type ViewModalCallback<T> = Box<dyn FnOnce(&mut T, &mut ViewContext<T>)>;

impl ModalButton<AppModalCallback> {
    /// This constructor is for modals when you have a [`AppContext`]. If you're requesting
    /// a modal from a View and have access to a [`ViewContext`], use the [`ModalButton::for_view`]
    /// method instead!
    pub fn for_app<S, F>(title: S, on_click: F) -> Self
    where
        S: Into<String>,
        F: FnOnce(&mut AppContext) + 'static,
    {
        Self {
            title: title.into(),
            on_click: Box::new(on_click),
        }
    }
}

impl AlertDialogWithCallbacks<AppModalCallback> {
    /// This constructor is for modals when you have a [`AppContext`]. If you're requesting
    /// a modal from a View and have access to a [`ViewContext`], use the
    /// [`AlertDialogWithCallbacks::for_view`] method instead!
    pub fn for_app<F>(
        message_text: impl Into<String>,
        info_text: impl Into<String>,
        button_data: Vec<ModalButton<AppModalCallback>>,
        on_disable: F,
    ) -> Self
    where
        F: FnOnce(&mut AppContext) + 'static,
    {
        Self {
            message_text: message_text.into(),
            info_text: info_text.into(),
            button_data,
            on_disable: Box::new(on_disable),
        }
    }
}

impl<V: View> ModalButton<ViewModalCallback<V>> {
    /// This constructor is for modals when you have a [`ViewContext`]. If you're requesting a modal
    /// from a [`AppContext`], use the [`ModalButton::for_app`] method instead!
    pub fn for_view<S, F>(title: S, on_click: F) -> Self
    where
        S: Into<String>,
        F: FnOnce(&mut V, &mut ViewContext<V>) + 'static,
    {
        Self {
            title: title.into(),
            on_click: Box::new(on_click),
        }
    }
}

impl<V: View> AlertDialogWithCallbacks<ViewModalCallback<V>> {
    /// This constructor is for modals when you have a [`ViewContext`]. If you're requesting a modal
    /// from a [`AppContext`], use the [`AlertDialogWithCallbacks::for_app`] method instead!
    pub fn for_view<F>(
        message_text: impl Into<String>,
        info_text: impl Into<String>,
        button_data: Vec<ModalButton<ViewModalCallback<V>>>,
        on_disable: F,
    ) -> Self
    where
        F: FnOnce(&mut V, &mut ViewContext<V>) + 'static,
    {
        Self {
            message_text: message_text.into(),
            info_text: info_text.into(),
            button_data,
            on_disable: Box::new(on_disable),
        }
    }
}

/// This holds the data necessary for dispatching the response from a platform-native modal.
pub(super) struct PlatformModalResponseData {
    /// A list of callbacks for each button on the modal.
    /// The first callback (at index 0) is the callback for the first button being pressed, etc.
    pub button_callbacks: Vec<AppModalCallback>,
    /// The callback for if the "Don't ask again" checkbox is clicked.
    pub disable_callback: AppModalCallback,
}

/// A globally unique, incrementing integer to ID platform native modals.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct ModalId(usize);

static NEXT_MODAL_ID: AtomicUsize = AtomicUsize::new(0);

impl ModalId {
    /// Constructs a new globally unique modal ID.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let raw = NEXT_MODAL_ID.fetch_add(1, Ordering::Relaxed);
        Self(raw)
    }
}
