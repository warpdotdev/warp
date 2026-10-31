//! The Windows taskbar jump list: a "New Window" task followed by one task per tab config.

use anyhow::Context as _;
use warp_core::channel::ChannelState;
use warp_errors::report_error;
use warp_util::windows::com_initialized;
use warpui::{Entity, ModelContext, SingletonEntity};
use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PVCHF_DEFAULT, PropVariantChangeType,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink,
};
use windows::core::{HSTRING, Interface};

use crate::tab_configs::{TabConfig, display_names};
use crate::user_config::{WarpConfig, WarpConfigUpdateEvent};

/// A jump list task that relaunches Warp with `url` as its only argument.
#[derive(Clone, Debug, PartialEq, Eq)]
struct JumpListTask {
    title: String,
    url: String,
}

/// Keeps the jump list in sync with the user's tab configs.
pub(crate) struct JumpList {
    commit_in_flight: bool,
    refresh_pending: bool,
}

impl JumpList {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        // No initial refresh is needed: `WarpConfig` emits `TabConfigs` once its startup load of
        // the tab configs directory finishes, even when the directory is empty or missing.
        ctx.subscribe_to_model(&WarpConfig::handle(ctx), |me, _, event, ctx| match event {
            WarpConfigUpdateEvent::TabConfigs => me.refresh(ctx),
            WarpConfigUpdateEvent::Themes
            | WarpConfigUpdateEvent::LocalUserWorkflows
            | WarpConfigUpdateEvent::LaunchConfigs
            | WarpConfigUpdateEvent::TabConfigErrors(_)
            | WarpConfigUpdateEvent::ModelConfigs
            | WarpConfigUpdateEvent::ModelConfigErrors(_)
            | WarpConfigUpdateEvent::Settings
            | WarpConfigUpdateEvent::SettingsErrors(_)
            | WarpConfigUpdateEvent::SettingsErrorsCleared => {}
        });
        Self {
            commit_in_flight: false,
            refresh_pending: false,
        }
    }

    /// Commits at most one list at a time, so an older list can never overwrite a newer one.
    fn refresh(&mut self, ctx: &mut ModelContext<Self>) {
        if self.commit_in_flight {
            self.refresh_pending = true;
            return;
        }
        self.commit_in_flight = true;
        let tasks = jump_list_tasks(
            WarpConfig::as_ref(ctx).tab_configs(),
            ChannelState::url_scheme(),
        );
        ctx.spawn(async move { commit(&tasks) }, |me, result, ctx| {
            me.commit_in_flight = false;
            if let Err(err) = result {
                report_error!(err.context("Failed to update the Windows jump list"));
            }
            if std::mem::take(&mut me.refresh_pending) {
                me.refresh(ctx);
            }
        });
    }
}

impl Entity for JumpList {
    type Event = ();
}

impl SingletonEntity for JumpList {}

/// "New Window", then the tab configs in the order and with the labels of the `+` menu.
fn jump_list_tasks(tab_configs: &[TabConfig], scheme: &str) -> Vec<JumpListTask> {
    let new_window = JumpListTask {
        title: "New Window".to_owned(),
        url: format!("{scheme}://action/new_window?path=~"),
    };
    let tab_config_tasks = tab_configs
        .iter()
        .zip(display_names(tab_configs))
        .filter_map(|(config, title)| {
            let stem = config.source_path.as_deref()?.file_stem()?.to_str()?;
            Some(JumpListTask {
                title,
                url: format!("{scheme}://tab_config/{}", urlencoding::encode(stem)),
            })
        });
    std::iter::once(new_window)
        .chain(tab_config_tasks)
        .collect()
}

fn commit(tasks: &[JumpListTask]) -> anyhow::Result<()> {
    com_initialized()?;
    let exe = HSTRING::from(
        std::env::current_exe()
            .context("Failed to resolve the current executable")?
            .as_path(),
    );
    let list: ICustomDestinationList =
        unsafe { CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)? };
    // Warp sets its AppUserModelID explicitly, so Windows can't infer which list this is.
    unsafe { list.SetAppID(&HSTRING::from(ChannelState::app_id().to_string()))? };
    let mut min_slots = 0;
    // Users can't remove tasks, so the removed-destinations array is always irrelevant.
    let _: IObjectArray = unsafe { list.BeginList(&mut min_slots)? };
    let result = add_tasks(&list, &exe, tasks).and_then(|()| unsafe { list.CommitList() });
    if result.is_err() {
        let _ = unsafe { list.AbortList() };
    }
    Ok(result?)
}

fn add_tasks(
    list: &ICustomDestinationList,
    exe: &HSTRING,
    tasks: &[JumpListTask],
) -> windows::core::Result<()> {
    let collection: IObjectCollection =
        unsafe { CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)? };
    for task in tasks {
        unsafe { collection.AddObject(&shell_link(exe, task)?)? };
    }
    unsafe { list.AddUserTasks(&collection.cast::<IObjectArray>()?) }
}

fn shell_link(exe: &HSTRING, task: &JumpListTask) -> windows::core::Result<IShellLinkW> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(exe)?;
        link.SetArguments(&HSTRING::from(format!("\"{}\"", task.url)))?;
        link.SetIconLocation(exe, 0)?;
        // The shell reads the label from `PKEY_Title`, which must be a `VT_LPWSTR`; converting
        // from `&str` alone yields a `VT_BSTR`.
        let mut title = PROPVARIANT::default();
        PropVariantChangeType(
            &mut title,
            &PROPVARIANT::from(task.title.as_str()),
            PVCHF_DEFAULT,
            VT_LPWSTR,
        )?;
        let store: IPropertyStore = link.cast()?;
        store.SetValue(&PKEY_Title, &title)?;
        store.Commit()?;
        Ok(link)
    }
}

#[cfg(test)]
#[path = "jump_list_tests.rs"]
mod tests;
