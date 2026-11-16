//! Module containing the definition of [`RendererState`].

use warpui::fonts::FamilyId;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

/// Helper singleton model that stores the icon font used to render native window controls on
/// Windows. Using a symbol font (as opposed to SVGs) produces windows controls that are better
/// aliased and more closely match the controls in other apps.
pub struct RendererState {
    icon_font_family: Option<FamilyId>,
}

impl RendererState {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        use windows::Wdk::System::SystemServices::RtlGetVersion;

        let mut version = unsafe { std::mem::zeroed() };
        let status = unsafe { RtlGetVersion(&mut version) };

        // "Segoue Fluent Icons" is the recommended symbol font on Windows 11 and is bundled with the
        // OS. On prior versions, the recommend symbol font is "Segoe MDL2 Assets".
        // See https://learn.microsoft.com/en-us/windows/apps/design/style/segoe-fluent-icons-font.
        let symbol_font = if status.is_ok() && version.dwBuildNumber >= 22000 {
            Self::load_symbol_font("Segoe Fluent Icons", ctx)
                .or_else(|| Self::load_symbol_font("Segoe MDL2 Assets", ctx))
        } else {
            Self::load_symbol_font("Segoe MDL2 Assets", ctx)
        };

        Self {
            icon_font_family: symbol_font,
        }
    }

    fn load_symbol_font(symbol_font_to_load: &str, ctx: &mut AppContext) -> Option<FamilyId> {
        warpui::fonts::Cache::handle(ctx).update(ctx, |font_cache, _| {
            match font_cache.get_or_load_system_font(symbol_font_to_load) {
                Ok(family) => Some(family),
                Err(err) => {
                    log::warn!("Failed to load windows symbol font due to error {err:?}");
                    None
                }
            }
        })
    }

    /// Returns the icon font family to use to render the window controls, or `None` if the font was
    /// not on the user's system for any reason.
    pub(super) fn icon_font_family(&self) -> Option<FamilyId> {
        self.icon_font_family
    }
}

impl Entity for RendererState {
    type Event = ();
}

impl SingletonEntity for RendererState {}
