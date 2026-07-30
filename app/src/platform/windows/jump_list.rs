use warpui::{AddSingletonModel, Entity, ModelContext, SingletonEntity};
use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{
    CLSCTX_ALL, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink,
};
use windows::core::{HSTRING, Interface, PCWSTR};

use crate::ChannelState;
use crate::features::FeatureFlag;
use crate::tab_configs::TabConfig;
use crate::user_config::{WarpConfig, WarpConfigUpdateEvent};

pub struct JumpEntry {
    label: String,
    deeplink: String,
}

fn tab_configs_to_entries(configs: &[TabConfig], scheme: &str) -> Vec<JumpEntry> {
    let mut entries: Vec<JumpEntry> = configs
        .iter()
        .filter_map(|config| {
            let stem = config
                .source_path
                .as_ref()
                .and_then(|p| p.file_stem())
                .and_then(|s| s.to_str())?;
            if stem.is_empty() {
                return None;
            }
            Some(JumpEntry {
                label: config.name.clone(),
                deeplink: format!("{scheme}://tab_config/{}", urlencoding::encode(stem)),
            })
        })
        .collect();
    entries.sort_by(|a, b| a.label.cmp(&b.label));
    entries
}

pub fn refresh_jump_list(entries: &[JumpEntry], new_window: &str) {
    if !FeatureFlag::WindowsJumpList.is_enabled() {
        clear_jump_list();
        return;
    }

    log::info!("Refreshing jump list with {} entries", entries.len());

    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() {
            log::warn!("Failed to initialize COM: {hr:?}");
            return;
        }
    }

    // Runs on a dedicated thread (see spawn_refresh); a panic here unwinds only
    // this thread and never reaches startup. COM state is thread-local and dies
    // with the thread, so CoUninitialize is best-effort rather than required.
    refresh_jump_list_inner(entries, new_window);
    unsafe {
        CoUninitialize();
    }
}

fn refresh_jump_list_inner(entries: &[JumpEntry], new_window: &str) {
    unsafe {
        let destination_list: ICustomDestinationList =
            match CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER) {
                Ok(list) => list,
                Err(err) => {
                    log::warn!("Failed to create ICustomDestinationList: {err:?}");
                    return;
                }
            };

        let mut min_slots: u32 = 0;
        let removed: IObjectArray = match destination_list.BeginList(&mut min_slots) {
            Ok(removed) => removed,
            Err(err) => {
                log::warn!("Failed to begin jump list: {err:?}");
                return;
            }
        };

        let removed = removed_destinations(&removed);
        log::info!(
            "jumplist: BeginList min_slots={min_slots} removed_count={}",
            removed.len()
        );

        let exe = std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        // The Windows Tasks section holds only the static "New Window" entry.
        // Tasks are not subject to the destination slot count.
        let tasks: IObjectCollection = match create_object_collection() {
            Ok(tasks) => tasks,
            Err(err) => {
                log::warn!("Failed to create task collection: {err:?}");
                return;
            }
        };

        if let Some(link) =
            make_shell_link(&exe, new_window, "New Window", "Open a new Warp window")
        {
            let _ = tasks.AddObject(&link);
        } else {
            log::warn!("jumplist[TASKS]: failed to build New Window link");
        }

        if let Err(err) = destination_list.AddUserTasks(&tasks) {
            log::warn!("Failed to add user tasks: {err:?}");
            return;
        }

        // Tab Configs live in a custom destination category, capped to the slots
        // BeginList reports and filtered against user-removed destinations.
        // AppendCategory can fail under a privacy Group Policy (E_ACCESSDENIED);
        // per the Windows docs tasks still commit then, so we log and fall
        // through to CommitList.
        let destinations: IObjectCollection = match create_object_collection() {
            Ok(destinations) => destinations,
            Err(err) => {
                log::warn!("Failed to create destination collection: {err:?}");
                return;
            }
        };

        let mut shown: u32 = 0;
        for entry in entries.iter().filter(|e| !removed.contains(&e.deeplink)) {
            if shown >= min_slots {
                break;
            }
            if let Some(link) = make_shell_link(&exe, &entry.deeplink, &entry.label, &entry.label) {
                let _ = destinations.AddObject(&link);
                shown += 1;
            } else {
                log::warn!("jumplist[DESTS]: failed to build link for {}", entry.label);
            }
        }

        if shown > 0 {
            let category = HSTRING::from("Tab Configs");
            if let Err(err) = destination_list.AppendCategory(&category, &destinations) {
                log::warn!("Failed to append Tab Configs category: {err:?}");
            }
        }

        if let Err(err) = destination_list.CommitList() {
            log::warn!("Failed to commit jump list: {err:?}");
        } else {
            log::info!("Jump list committed successfully");
        }
    }
}

fn create_object_collection() -> windows::core::Result<IObjectCollection> {
    unsafe {
        CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)
            .or_else(|_| CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_ALL))
    }
}

fn make_shell_link(
    exe: &str,
    arguments: &str,
    title: &str,
    description: &str,
) -> Option<IShellLinkW> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let exe_w = HSTRING::from(exe);
        if let Err(err) = link.SetPath(&exe_w) {
            log::warn!("Failed to set shell link path: {err:?}");
            return None;
        }
        let quoted = HSTRING::from(format!("\"{arguments}\""));
        if let Err(err) = link.SetArguments(&quoted) {
            log::warn!("Failed to set shell link arguments: {err:?}");
            return None;
        }
        // Windows requires an icon location on IShellLink entries placed in a
        // custom category. Pointing it at our own exe keeps every entry on the
        // Warp app icon (no per-entry icons in this iteration).
        if let Err(err) = link.SetIconLocation(&exe_w, 0) {
            log::warn!("Failed to set shell link icon: {err:?}");
        }
        let description_w = HSTRING::from(description);
        if let Err(err) = link.SetDescription(&description_w) {
            log::warn!("Failed to set shell link description: {err:?}");
            return None;
        }

        let store: IPropertyStore = link.cast().ok()?;
        let title_value: PROPVARIANT = title.into();
        if let Err(err) = store.SetValue(&PKEY_Title, &title_value) {
            log::warn!("Failed to set jump list title: {err:?}");
        }

        Some(link)
    }
}

fn removed_destinations(removed: &IObjectArray) -> Vec<String> {
    let mut deeplinks = Vec::new();
    unsafe {
        let Ok(count) = removed.GetCount() else {
            return deeplinks;
        };
        for i in 0..count {
            let Ok(item) = removed.GetAt::<windows::core::IUnknown>(i) else {
                continue;
            };
            let Ok(link): windows::core::Result<IShellLinkW> = item.cast() else {
                continue;
            };
            let mut buf = [0u16; 1024];
            if link.GetArguments(&mut buf).is_ok() {
                let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
                let args = String::from_utf16_lossy(&buf[..end]);
                deeplinks.push(args.trim_matches('"').to_string());
            }
        }
    }
    deeplinks
}

fn clear_jump_list() {
    unsafe {
        if CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_err() {
            return;
        }
        if let Ok(list) = CoCreateInstance::<_, ICustomDestinationList>(
            &DestinationList,
            None,
            CLSCTX_INPROC_SERVER,
        ) {
            let _ = list.DeleteList(PCWSTR::null());
        }
        log::info!("Jump list cleared");
        CoUninitialize();
    }
}

// A dedicated thread (rather than tokio::task::spawn_blocking) ensures COM is
// first init on this thread as apartment-threaded; a reused blocking-pool
// thread that a prior caller init'd as multi-threaded would reject STA init.
fn spawn_refresh(entries: Vec<JumpEntry>, new_window: String) {
    let _handle = std::thread::spawn(move || refresh_jump_list(&entries, &new_window));
}

pub struct JumpListManager;

impl JumpListManager {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&WarpConfig::handle(ctx), |_, _, event, ctx| {
            if let WarpConfigUpdateEvent::TabConfigs = event {
                let (entries, new_window) = Self::current_payload(ctx);
                spawn_refresh(entries, new_window);
            }
        });

        let (entries, new_window) = Self::current_payload(ctx);
        spawn_refresh(entries, new_window);
        Self
    }

    fn current_payload(ctx: &mut ModelContext<Self>) -> (Vec<JumpEntry>, String) {
        let scheme = ChannelState::url_scheme().to_string();
        let configs = WarpConfig::handle(ctx).as_ref(ctx).tab_configs().clone();
        let entries = tab_configs_to_entries(&configs, &scheme);
        let new_window = format!("{scheme}://action/new_window?path=~");
        (entries, new_window)
    }
}

impl Entity for JumpListManager {
    type Event = ();
}

impl SingletonEntity for JumpListManager {}

pub fn register(app: &mut impl AddSingletonModel) {
    if FeatureFlag::WindowsJumpList.is_enabled() {
        app.add_singleton_model(JumpListManager::new);
    } else {
        let _handle = std::thread::spawn(clear_jump_list);
    }
}

#[cfg(test)]
#[path = "jump_list_tests.rs"]
mod tests;
