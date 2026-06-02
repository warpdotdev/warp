#[cfg(target_family = "wasm")]
pub mod wasm;
#[cfg(windows)]
pub mod windows;

pub fn init() {
    #[cfg(target_family = "wasm")]
    wasm::init();
}
