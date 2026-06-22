use warp_core::send_telemetry_from_ctx;
use warpui::ModelContext;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
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

/// Helper struct to properly enforce reference counting of the Windows COM library.
///
/// Per the Windows docs (https://learn.microsoft.com/en-us/windows/win32/api/combaseapi/nf-combaseapi-coinitializeex)
/// each call to [`CoInitializeEx`] must be paired with a call to [`CoUninitialize`] in order for
/// the COM library to be gracefully uninitialized.
// TODO(alokedesai): Move this to a shared place in `core` so we can use it in other places in the
// app.
struct ComInitialized;

impl Drop for ComInitialized {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

thread_local! {
    static COM_INITIALIZED: Result<ComInitialized, windows::core::Error> = {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
            Ok(ComInitialized)
        }
    };
}

fn com_initialized() -> Result<(), windows::core::Error> {
    COM_INITIALIZED.with(|initialized| {
        initialized
            .as_ref()
            .map(|_| ())
            .map_err(|error| error.clone())
    })
}
