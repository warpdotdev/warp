use std::ops::Not;

use warpui::{clipboard::ClipboardContent, AppContext};

use settings::{
    macros::define_settings_group, RespectUserSyncSetting, Setting, SupportedPlatforms, SyncToCloud,
};

define_settings_group!(SelectionSettings, settings: [
    copy_on_select: CopyOnSelect {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "terminal.copy_on_select",
        description: "Whether text is automatically copied to the clipboard when selected.",
    },
    linux_selection_clipboard: LinuxSelectionClipboard {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::LINUX,
        sync_to_cloud: SyncToCloud::PerPlatform(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "system.linux_selection_clipboard",
        description: "Whether the Linux primary selection clipboard is used.",
    },
    middle_click_paste_enabled: MiddleClickPasteEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::OR(
            SupportedPlatforms::WINDOWS.into(),
            SupportedPlatforms::MAC.into()
        ),
        sync_to_cloud: SyncToCloud::PerPlatform(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "terminal.input.middle_click_paste_enabled",
        description: "Whether middle-click pastes from the clipboard.",
    }
]);

impl SelectionSettings {
    pub fn copy_on_select_enabled(&self) -> bool {
        *self.copy_on_select.value()
    }

    /// Returns whether honoring the Linux primary selection clipboard is enabled. On non-linux
    /// platforms this always returns false.
    pub fn linux_selection_clipboard_enabled(&self) -> bool {
        *self.linux_selection_clipboard.value()
            && self
                .linux_selection_clipboard
                .is_supported_on_current_platform()
    }

    /// Writes the selection content to the user's clipboard if `copy_on_select` is enabled.
    pub fn maybe_copy_on_select(&self, clipboard_content: ClipboardContent, ctx: &mut AppContext) {
        self.maybe_write_to_linux_selection_clipboard(|_| clipboard_content.clone(), ctx);
        if self.copy_on_select_enabled() && !clipboard_content.plain_text.is_empty() {
            ctx.clipboard().write(clipboard_content);
        }
    }

    /// Writes the selected content to the user's primary selection clipboard. On non-Linux
    /// platforms this is a noop.
    pub fn maybe_write_to_linux_selection_clipboard(
        &self,
        clipboard_contents_fn: impl FnOnce(&mut AppContext) -> ClipboardContent,
        ctx: &mut AppContext,
    ) {
        if self.linux_selection_clipboard_enabled() {
            let clipboard_content = clipboard_contents_fn(ctx);
            if !clipboard_content.plain_text.is_empty() {
                ctx.clipboard()
                    .write_to_primary_clipboard(clipboard_content);
            }
        }
    }

    fn maybe_read_from_linux_selection_clipboard(
        &self,
        ctx: &mut AppContext,
    ) -> Option<ClipboardContent> {
        self.linux_selection_clipboard_enabled()
            .then(|| ctx.clipboard().read_from_primary_clipboard())
    }

    /// Implements the correct middle-click paste behavior for the current platform.
    ///
    /// Linux has the "primary clipboard" to which it maps the middle mouse button. Other platforms
    /// lack this separate clipboard, and so we map middle-click to the normal clipboard on those
    /// platforms.
    pub fn read_for_middle_click_paste(&self, ctx: &mut AppContext) -> Option<ClipboardContent> {
        if cfg!(any(target_os = "linux", target_os = "freebsd")) {
            return self.maybe_read_from_linux_selection_clipboard(ctx);
        }
        (self
            .middle_click_paste_enabled
            .is_supported_on_current_platform()
            && *self.middle_click_paste_enabled.value())
        .then(|| ctx.clipboard().read())
        .filter(|content| content.is_empty().not())
    }
}
