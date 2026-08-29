pub use warp_terminal::available_shells::*;

#[cfg(feature = "local_tty")]
pub fn register(app: &mut impl warpui::AddSingletonModel) {
    #[cfg(windows)]
    app.add_singleton_model(|_| warp_terminal::wsl::WslInfo::new());
    app.add_singleton_model(AvailableShells::new);
}
