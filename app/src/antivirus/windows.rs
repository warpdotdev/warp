use warp_core::send_telemetry_from_ctx;
use warp_util::windows::com_initialized;
use warpui::ModelContext;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::System::SecurityCenter::*;

use crate::antivirus::telemetry::AntivirusInfoTelemetryEvent;
use crate::antivirus::{AntivirusInfo, AntivirusInfoEvent};

impl AntivirusInfo {
    #[cfg(windows)]
    pub(super) async fn scan() -> anyhow::Result<Option<String>> {
        unsafe {
            com_initialized()?;

            // Read out all of the registered antivirus products.
            let pl: IWSCProductList = CoCreateInstance(&WSCProductList, None, CLSCTX_ALL)?;
            pl.Initialize(WSC_SECURITY_PROVIDER_ANTIVIRUS)?;

            let n = pl.Count().unwrap_or(0) as u32;
            for i in 0..n {
                let Ok(p) = pl.get_Item(i) else {
                    continue;
                };

                // If the product is on (meaning it's running), return it.
                if let Ok(WSC_SECURITY_PRODUCT_STATE_ON) = p.ProductState() {
                    return Ok(p.ProductName().ok().map(|s| s.to_string()));
                }
            }
        }

        Ok(None)
    }

    pub(super) fn on_scan_complete(
        &mut self,
        software: anyhow::Result<Option<String>>,
        ctx: &mut ModelContext<Self>,
    ) {
        let software = match software {
            Ok(software) => software,
            Err(err) => {
                log::warn!("Failed to scan for antivirus / EDR software: {err:#}");
                return;
            }
        };

        match software.as_ref() {
            None => {
                log::info!("No antivirus / EDR software detected");
            }
            Some(software) => {
                log::info!("Detected antivirus / EDR software {software:#?}");
                send_telemetry_from_ctx!(
                    AntivirusInfoTelemetryEvent::AntivirusDetected {
                        name: software.into()
                    },
                    ctx
                );
            }
        }

        self.0 = software;

        ctx.emit(AntivirusInfoEvent::ScannedComplete);
    }
}
