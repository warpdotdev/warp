use std::sync::mpsc::SyncSender;

use warpui::{Entity, ModelHandle, SingletonEntity};

use crate::banner::BannerState;
use crate::persistence::ModelEvent;
use crate::referral_theme_status::ReferralThemeStatus;
use crate::resource_center::TipsCompleted;
use crate::settings::SettingsFileError;

/// Interfaces that allow us to interact with global resources owned by the main
/// thread that exist throughout the app including Model handles, channel senders,
/// channel receivers, and references.
///
/// Guidelines:
/// * If it doesn't need to be a global resource (e.g. it could be owned by a view),
///   it shouldn't go here.
///
/// * Use a Model if you have a global data model. You can use ModelHandle#update
///   to mutate it, ModelHandle#read to borrow it, and ViewContext#observe to
///   have a view respond to all updates to the model. A lot of things in the
///   app will be Models, but they won't necessarily need to be global in the app.
///   One example of a global model is the referral theme status: there will only
///   be one value throughout the app on whether the user has activated a referral
///   theme. One example of a model owned by a view is the terminal sessions model:
///   since this is metadata about particular shell sessions that are in a particular
///   pane, they belong to that pane's terminal view.
///
/// * Use a Sender if you need to send communication from the main thread to
///   another thread. For example, we send `ModelEvent`s to the sqlite writer
///   thread.
///
/// * Use a Receiver if you need to receive communication in the main thread
///   sent from another thread. This would need to be used in conjunction with
///   `ViewContext#spawn_stream_local` which polls the receiver for values.
///   One example use case could be receiving updates from the warp config watcher
///   thread. Note that `spawn_stream_local` is polling on the main thread, so
///   we should call this sparingly. It's easy to unintentionally call this from
///   a view that's instantiated many times in the app (e.g. EditorView). Instead
///   of doing that, we should ideally find a view that's only created once
///   per-window and propagate changes down the view hierarchy by calling update
///   on the view handles of that view's children.
///
/// * Use an Arc if you need to share data with another thread. Note that in
///   order to mutate anything under the Arc, you'll need a Mutex or RwLock.
///
/// * watch::Channel is not recommended as it creates a lot of implicit clones.
///   It actually also includes a Sender, Receiver, Arc, and a RwLock. Instead of
///   using watch::Channel, consider what of the use cases listed above are
///   relevant.
#[derive(Clone)]
pub struct GlobalResourceHandles {
    pub model_event_sender: Option<SyncSender<ModelEvent>>,
    pub tips_completed: ModelHandle<TipsCompleted>,
    pub referral_theme_status: ModelHandle<ReferralThemeStatus>,
    pub user_default_shell_unsupported_banner_model_handle: ModelHandle<BannerState>,
    pub settings_file_error: Option<SettingsFileError>,
}

impl GlobalResourceHandles {
    #[cfg(any(test, feature = "integration_tests"))]
    pub fn mock(app: &mut warpui::App) -> Self {
        let referral_theme_status = app.add_model(ReferralThemeStatus::new);
        let user_default_shell_unsupported_banner_model_handle =
            app.add_model(|_| BannerState::default());
        let tips_completed = app.add_model(|_| TipsCompleted::default());

        GlobalResourceHandles {
            model_event_sender: None,
            tips_completed,
            referral_theme_status,
            user_default_shell_unsupported_banner_model_handle,
            settings_file_error: None,
        }
    }
}

/// Singleton entity that provides access to a reference to the [`GlobalResourceHandles`].
pub struct GlobalResourceHandlesProvider {
    global_resources: GlobalResourceHandles,
}

impl GlobalResourceHandlesProvider {
    pub fn get(&self) -> &GlobalResourceHandles {
        &self.global_resources
    }

    pub(super) fn new(global_resources: GlobalResourceHandles) -> Self {
        Self { global_resources }
    }
}

impl Entity for GlobalResourceHandlesProvider {
    type Event = ();
}

impl SingletonEntity for GlobalResourceHandlesProvider {}
