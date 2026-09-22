use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, Setting as _, SupportedPlatforms, SyncToCloud};
use warp_errors::report_if_error;
use warpui::platform::WindowBackdrop;
use warpui::{AppContext, SingletonEntity, WindowId};

define_settings_group!(WindowSettings, settings: [
    background_blur_radius: BackgroundBlurRadius {
        type: u8,
        default: 1,
        supported_platforms: SupportedPlatforms::MAC,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        storage_key: "OverrideBlur",
        toml_path: "appearance.window.override_blur",
        description: "The blur radius applied to the window background.",
    },
    background_backdrop: BackgroundBackdrop {
        type: WindowBackdrop,
        default: WindowBackdrop::None,
        supported_platforms: SupportedPlatforms::WINDOWS,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.window.backdrop",
        description: "The system backdrop material applied to the window background.",
    },
    legacy_override_blur_texture: LegacyOverrideBlurTexture {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::WINDOWS,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        storage_key: "OverrideBlurTexture",
        toml_path: "appearance.window.override_blur_texture",
        description: "Deprecated legacy setting for the Acrylic window backdrop.",
    },
    background_opacity: BackgroundOpacity {
        type: u8,
        default: 100,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        storage_key: "OverrideOpacity",
        toml_path: "appearance.window.override_opacity",
        description: "The opacity of the window background, from 1 to 100 percent.",
    },
    open_windows_at_custom_size: OpenWindowsAtCustomSize {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.window.open_windows_at_custom_size",
        description: "Whether to open new windows at a custom size instead of the default.",
    },
    new_windows_num_columns: NewWindowsNumColumns {
        type: u16,
        default: 80,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.window.new_windows_num_columns",
        description: "The number of columns for new windows when using a custom size.",
    },
    new_windows_num_rows: NewWindowsNumRows {
        type: u16,
        default: 40,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.window.new_windows_num_rows",
        description: "The number of rows for new windows when using a custom size.",
    },
    left_panel_visibility_across_tabs: LeftPanelVisibilityAcrossTabs {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.window.left_panel_visibility_across_tabs",
        description: "Whether the left panel visibility is shared across all tabs.",
    },
    zoom_level: ZoomLevel {
        type: u16,
        default: 100,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "appearance.window.zoom_level",
        description: "The zoom level for the window, as a percentage.",
    },
]);

pub(crate) fn stage_legacy_background_backdrop(ctx: &mut AppContext) {
    WindowSettings::handle(ctx).update(ctx, |settings, ctx| {
        if settings.background_backdrop.is_value_explicitly_set()
            || !*settings.legacy_override_blur_texture
        {
            return;
        }
        report_if_error!(settings.background_backdrop.load_value(
            WindowBackdrop::Acrylic,
            false,
            ctx
        ));
    });
}

pub(crate) fn migrate_legacy_background_backdrop(ctx: &mut AppContext) {
    WindowSettings::handle(ctx).update(ctx, |settings, ctx| {
        if settings.background_backdrop.is_value_explicitly_set() {
            return;
        }
        if *settings.legacy_override_blur_texture {
            report_if_error!(
                settings
                    .background_backdrop
                    .set_value(WindowBackdrop::Acrylic, ctx)
            );
        } else {
            report_if_error!(settings.background_backdrop.load_value(
                WindowBackdrop::None,
                false,
                ctx
            ));
        }
    });
}

impl ZoomLevel {
    /// Available zoom values (percent): 50, 60, 70, 80, 90, 100, 110, 125, 150, 175, 200, 225, 250, 300, 350.
    /// This corresponds to a zoom factor range of [0.5, 3.5].
    /// Used for zoom level adjustments and the zoom dropdown in appearance settings.
    pub const VALUES: [u16; 15] = [
        50, 60, 70, 80, 90, 100, 110, 125, 150, 175, 200, 225, 250, 300, 350,
    ];

    /// Returns the current [`ZoomLevel`] as a percentage (so that it be can be used as a zoom factor).
    pub fn as_zoom_factor(&self) -> f32 {
        self.inner as f32 / 100.0
    }
}

impl BackgroundBlurRadius {
    pub const MIN: u8 = 1;
    pub const MAX: u8 = 64;

    fn validate(&self, new_value: u8) -> u8 {
        if new_value < Self::MIN {
            log::warn!(
                "Window background blur radius should not be smaller than {}",
                Self::MIN
            );
            Self::MIN
        } else if new_value > Self::MAX {
            log::warn!(
                "Window background blur radius should not be smaller than {}",
                Self::MAX
            );
            Self::MAX
        } else {
            new_value
        }
    }
}

impl BackgroundOpacity {
    // Capping min opacity at 1 for now as for some reason the rendered assets from
    // last frame will start showing up in the current frame in metal when opacity is at 0.
    pub const MIN: u8 = 1;
    pub const MAX: u8 = 100;

    /// Returns the effective background opacity for the window.
    ///
    /// When native window decorations are enabled (e.g. as a GPU driver workaround) on Windows,
    /// the native frame adds a white background that bleeds through any transparent areas, so we
    /// force full opacity.
    pub fn effective_opacity(&self, window_id: WindowId, app: &AppContext) -> u8 {
        if self.is_configurable(window_id, app) {
            **self
        } else {
            Self::MAX
        }
    }

    pub fn is_configurable(&self, window_id: WindowId, app: &AppContext) -> bool {
        let disable_transparency = app
            .windows()
            .platform_window(window_id)
            .is_some_and(|w| w.uses_native_window_decorations())
            && cfg!(windows);
        !disable_transparency
    }

    fn validate(&self, new_value: u8) -> u8 {
        if new_value < Self::MIN {
            log::warn!(
                "Window background opacity should not be smaller than {}",
                Self::MIN
            );
            Self::MIN
        } else if new_value > Self::MAX {
            log::warn!(
                "Window background opacity should not be bigger than {}",
                Self::MAX
            );
            Self::MAX
        } else {
            new_value
        }
    }
}
