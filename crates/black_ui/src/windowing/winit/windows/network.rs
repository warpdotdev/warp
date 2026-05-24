use anyhow::Context;
use windows::core::{implement, Interface};
use windows::Win32::Networking::NetworkListManager::{
    INetworkListManager, INetworkListManagerEvents, INetworkListManagerEvents_Impl,
    NetworkListManager, NLM_CONNECTIVITY, NLM_CONNECTIVITY_DISCONNECTED,
    NLM_CONNECTIVITY_IPV4_INTERNET, NLM_CONNECTIVITY_IPV6_INTERNET,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, IConnectionPoint, IConnectionPointContainer, CLSCTX_ALL,
    COINIT_APARTMENTTHREADED,
};

use crate::windowing::winit::app::CustomEvent;

/// Implements the INetworkListManagerEvents trait so we can pass along connectivity events from Windows
/// OS to our winit event loop.
#[implement(INetworkListManagerEvents)]
#[allow(non_camel_case_types)]
struct WindowsNetworkListener {
    event_loop: winit::event_loop::EventLoopProxy<CustomEvent>,
}

impl WindowsNetworkListener {
    fn new(event_loop: winit::event_loop::EventLoopProxy<CustomEvent>) -> Self {
        Self { event_loop }
    }
}

#[allow(non_snake_case)]
impl INetworkListManagerEvents_Impl for WindowsNetworkListener_Impl {
    fn ConnectivityChanged(&self, new_connectivity: NLM_CONNECTIVITY) -> windows::core::Result<()> {
        // The NLM_CONNECTIVITY parameter is a bitmap. When it contains NLM_CONNECTIVITY_IPV4_INTERNET
        // or NLM_CONNECTIVITY_IPV6_INTERNET, there's a connection. When it's equal to
        // NLM_CONNECTIVITY_DISCONNECTED, it's a disconnection. Other arbitrary network events are ignored.
        // https://learn.microsoft.com/en-us/windows/win32/api/netlistmgr/ne-netlistmgr-nlm_connectivity#syntax
        // let connected = new_connectivity.eq(&NLM_CONNECTIVITY_IPV6_INTERNET) || new_connectivity.eq(&NLM_CONNECTIVITY_IPV4_INTERNET);
        let connected = (new_connectivity.0
            & (NLM_CONNECTIVITY_IPV6_INTERNET.0 | NLM_CONNECTIVITY_IPV4_INTERNET.0))
            != 0;
        let disconnected = new_connectivity.eq(&NLM_CONNECTIVITY_DISCONNECTED);

        if connected {
            let _ = self.event_loop.send_event(CustomEvent::InternetConnected);
        } else if disconnected {
            let _ = self
                .event_loop
                .send_event(CustomEvent::InternetDisconnected);
        }
        Ok(())
    }
}

pub struct WindowsNetworkConnectionPoint {
    connection_point: IConnectionPoint,
    cookie: u32,

    #[allow(unused)]
    /// We keep the events interface around for the duration of the program because
    /// we're not sure we don't need it to keep living.
    events_interface: INetworkListManagerEvents,
}

impl WindowsNetworkConnectionPoint {
    pub fn clean_up(&self) {
        unsafe {
            if let Err(e) = self.connection_point.Unadvise(self.cookie) {
                log::warn!("Failed to clean up network connection point: {e:?}");
            }
        }
    }
}

pub fn add_network_connection_listener(
    event_loop_proxy: winit::event_loop::EventLoopProxy<CustomEvent>,
) -> anyhow::Result<WindowsNetworkConnectionPoint> {
    let network_listener = {
        unsafe {
            // This invocation matches winit exactly. We want to make sure we don't modify any winit invariants in the case that
            // winit also calls CoInitializeEx.
            // https://github.com/rust-windowing/winit/blob/953d9b426886749e2f88250f420c87db58080c97/src/platform_impl/windows/window.rs#L1386
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .context("Failed to initialize COM")?;

            let events_interface: INetworkListManagerEvents =
                WindowsNetworkListener::new(event_loop_proxy).into();

            let connection_point_container: IConnectionPointContainer =
                CoCreateInstance(&NetworkListManager, None, CLSCTX_ALL)
                    .and_then(|network_manager: INetworkListManager| network_manager.cast())
                    .context("Failed to construct IConnectionPointContainer")?;

            let connection_point: IConnectionPoint = connection_point_container
                .FindConnectionPoint(&INetworkListManagerEvents::IID)
                .context("Failed to construct IConnectionPoint")?;

            let cookie = connection_point
                .Advise(&events_interface)
                .context("Failed to attach point and sink")?;

            WindowsNetworkConnectionPoint {
                connection_point,
                cookie,
                events_interface,
            }
        }
    };
    Ok(network_listener)
}
