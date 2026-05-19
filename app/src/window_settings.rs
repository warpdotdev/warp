use settings::{
    macros::define_settings_group, RespectUserSyncSetting, SupportedPlatforms, SyncToCloud,
};
use warpui::{AppContext, WindowId};

define_settings_group!(WindowSettings, settings: [
    background_blur_radius: BackgroundBlurRadius {
        type: u8,
        default: 1,
        supported_platforms: SupportedPlatforms::MAC,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        storage_key: "OverrideBlur",
        toml_path: "appearance.window.override_blur",
        description: "The blur radius applied to the window background.",
    },
    background_blur_texture: BackgroundBlurTexture {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::WINDOWS,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        storage_key: "OverrideBlurTexture",
        toml_path: "appearance.window.override_blur_texture",
        description: "Whether to apply a blur texture to the window background.",
    }
    background_opacity: BackgroundOpacity {
        type: u8,
        default: 100,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        storage_key: "OverrideOpacity",
        toml_path: "appearance.window.override_opacity",
        description: "The opacity of the window background, from 1 to 100 percent.",
    },
    background_image_overlay_opacity: BackgroundImageOverlayOpacity {
        type: u8,
        default: 40,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        storage_key: "BackgroundImageOverlayOpacity",
        toml_path: "appearance.window.background_image_overlay_opacity",
        description: "The opacity of the overlay on top of the background image, from 0 to 100 percent. Higher values make the overlay darker, improving text readability.",
    },
    open_windows_at_custom_size: OpenWindowsAtCustomSize {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "appearance.window.open_windows_at_custom_size",
        description: "Whether to open new windows at a custom size instead of the default.",
    },
    new_windows_num_columns: NewWindowsNumColumns {
        type: u16,
        default: 80,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "appearance.window.new_windows_num_columns",
        description: "The number of columns for new windows when using a custom size.",
    },
    new_windows_num_rows: NewWindowsNumRows {
        type: u16,
        default: 40,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "appearance.window.new_windows_num_rows",
        description: "The number of rows for new windows when using a custom size.",
    },
    left_panel_visibility_across_tabs: LeftPanelVisibilityAcrossTabs {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "appearance.window.left_panel_visibility_across_tabs",
        description: "Whether the left panel visibility is shared across all tabs.",
    },
    zoom_level: ZoomLevel {
        type: u16,
        default: 100,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "appearance.window.zoom_level",
        description: "The zoom level for the window, as a percentage.",
    },
]);

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

impl BackgroundImageOverlayOpacity {
    pub const MIN: u8 = 0;
    pub const MAX: u8 = 100;

    fn validate(&self, new_value: u8) -> u8 {
        if new_value > Self::MAX {
            log::warn!(
                "Background image overlay opacity should not be bigger than {}",
                Self::MAX
            );
            Self::MAX
        } else {
            new_value
        }
    }
}
