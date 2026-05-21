use crate::platform::WindowManager as _;
use crate::windowing::winit::window::WindowManager;
use crate::{DisplayId, DisplayIdx};
use anyhow::Result;
use itertools::Itertools as _;
use pathfinder_geometry::rect::RectF;
use std::sync::Arc;
use winit::monitor::MonitorHandle;
use winit::platform::windows::MonitorHandleExtWindows;
use winit::window::Window as WinitWindow;

use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use super::get_monitor_logical_bounds;

impl WindowManager {
    /// Returns the active Warp window. This will return an error if a different app's window is
    /// active.
    fn get_active_window_handle(&self) -> Result<Arc<WinitWindow>> {
        let window_id = &self
            .active_window_id()
            .ok_or(anyhow::anyhow!("No active window ID"))?;
        let ui_window = self
            .windows
            .get(window_id)
            .ok_or(anyhow::anyhow!("Window not found"))?;
        let winit_window_borrow = ui_window.inner.try_borrow()?;
        let winit_window_ref = winit_window_borrow
            .as_ref()
            .ok_or(anyhow::anyhow!("Unable to read Window information"))?;
        Ok(winit_window_ref.window.clone())
    }

    fn get_any_window_handle(&self) -> Result<Arc<WinitWindow>> {
        self.windows
            .values()
            .find_map(|window| {
                window
                    .inner
                    .try_borrow()
                    .ok()
                    .and_then(|borrow| borrow.as_ref().map(|inner| inner.window.clone()))
            })
            .ok_or_else(|| anyhow::anyhow!("No window handles available"))
    }

    /// Returns the monitor which contains the focused window ("key window" in MacOS parlance). It's
    /// the window that receives and handles the keypress events.
    fn get_foreground_monitor(&self) -> Result<MonitorHandle> {
        let any_window = self.get_any_window_handle()?;

        // Even if no window has foreground focus, MonitorFromWindow with
        // MONITOR_DEFAULTTONEAREST will return the nearest/primary monitor.
        let fg_hwnd = unsafe { GetForegroundWindow() };
        let target_hmonitor = unsafe { MonitorFromWindow(fg_hwnd, MONITOR_DEFAULTTONEAREST) };

        any_window
            .available_monitors()
            .find(|monitor| monitor.hmonitor() == target_hmonitor.0 as isize)
            .ok_or_else(|| anyhow::anyhow!("Could not match foreground window's monitor"))
    }

    fn get_active_monitor(&self) -> Result<MonitorHandle> {
        self.get_active_window_handle()
            .and_then(|w| {
                w.current_monitor()
                    .ok_or_else(|| anyhow::anyhow!("Unable to get current monitor"))
            })
            .or_else(|_| self.get_foreground_monitor())
    }

    pub(super) fn get_monitor_bounds_for_display_idx(&self, idx: DisplayIdx) -> Result<RectF> {
        let primary_monitor = self.get_primary_monitor_handle()?;
        let monitor = match idx {
            DisplayIdx::Primary => primary_monitor,
            DisplayIdx::External(numerical_index) => {
                let monitors = self.get_available_monitors()?;
                monitors
                    .iter()
                    .filter(|monitor| {
                        // Filter out the primary monitor.
                        monitor.hmonitor() != primary_monitor.hmonitor()
                    })
                    .nth(numerical_index)
                    .ok_or(anyhow::anyhow!(
                        "Could not find monitor handle for {numerical_index:?}"
                    ))?
                    .to_owned()
            }
        };
        Ok(get_monitor_logical_bounds(&monitor))
    }

    fn get_primary_monitor_handle(&self) -> Result<MonitorHandle> {
        let winit_window_ref = self.get_any_window_handle()?;
        winit_window_ref
            .primary_monitor()
            .ok_or(anyhow::anyhow!("No primary monitor found"))
    }

    pub(super) fn get_current_monitor_id(&self) -> Result<DisplayId> {
        let active_monitor = self.get_active_monitor()?;
        let active_monitor_id = active_monitor.hmonitor();
        Ok(DisplayId::from(active_monitor_id as usize))
    }

    fn get_available_monitors(&self) -> Result<Vec<MonitorHandle>> {
        let winit_window_ref = self.get_any_window_handle()?;
        Ok(winit_window_ref.available_monitors().collect_vec())
    }

    pub(super) fn get_available_monitor_count(&self) -> Result<usize> {
        let winit_window_ref = self.get_any_window_handle()?;
        Ok(winit_window_ref.available_monitors().count())
    }

    pub(super) fn get_active_monitor_logical_bounds(&self) -> Result<RectF> {
        let active_monitor = self.get_active_monitor()?;
        Ok(get_monitor_logical_bounds(&active_monitor))
    }
}
