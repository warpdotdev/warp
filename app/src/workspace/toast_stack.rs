use warpui::{Entity, ModelContext, SingletonEntity, WindowId};

use crate::{
    view_components::{DismissibleToast, ToastType},
    workspace::WorkspaceAction,
};

/// A global model that provides an interface to open a workspace-level
/// toast. This allows callers to add a toast from any context that has
/// access to the AppContext.
#[derive(Copy, Clone, Debug)]
pub struct ToastStack;

impl From<ToastType> for DismissibleToast<WorkspaceAction> {
    fn from(value: ToastType) -> Self {
        match value {
            ToastType::CloudObjectNotFound => {
                DismissibleToast::error(String::from("Resource not found or access denied"))
            }
        }
    }
}

impl ToastStack {
    /// Adds an ephemeral toast to the Workspace in the window identified by `window_id`.
    pub fn add_ephemeral_toast(
        &mut self,
        toast: DismissibleToast<WorkspaceAction>,
        window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.emit(ToastStackEvent::AddEphemeralToast { window_id, toast });
    }

    /// Adds a persistent toast to the Workspace in the window identified by `window_id`.
    pub fn add_persistent_toast(
        &mut self,
        toast: DismissibleToast<WorkspaceAction>,
        window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.emit(ToastStackEvent::AddPersistentToast { window_id, toast });
    }

    pub fn add_ephemeral_toast_by_type(
        &mut self,
        toast_type: ToastType,
        window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) {
        let toast = match toast_type {
            ToastType::CloudObjectNotFound => DismissibleToast::error(String::from(
                crate::i18n::tr_static(ctx, "Resource not found or access denied"),
            )),
        };
        ctx.emit(ToastStackEvent::AddEphemeralToast { window_id, toast });
    }

    pub fn remove_toast_by_identifier(
        &mut self,
        identifier: String,
        window_id: WindowId,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.emit(ToastStackEvent::RemoveToast {
            window_id,
            identifier,
        });
    }
}

#[allow(clippy::enum_variant_names)]
pub enum ToastStackEvent {
    AddEphemeralToast {
        /// The window for which this event is for.
        window_id: WindowId,
        toast: DismissibleToast<WorkspaceAction>,
    },
    AddPersistentToast {
        /// The window for which this event is for.
        window_id: WindowId,
        toast: DismissibleToast<WorkspaceAction>,
    },
    RemoveToast {
        /// The window for which this event is for.
        window_id: WindowId,
        identifier: String,
    },
}

impl Entity for ToastStack {
    type Event = ToastStackEvent;
}

impl SingletonEntity for ToastStack {}
