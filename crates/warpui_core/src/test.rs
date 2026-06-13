use ctor::ctor;

// Initialize the logger before running tests.
#[ctor]
fn init() {
    simplelog::SimpleLogger::init(simplelog::LevelFilter::Info, simplelog::Config::default())
        .unwrap()
}

/// Produces a minimal render output. Shared by core test files.
pub(crate) fn empty_render_output() -> crate::RenderOutput {
    use crate::elements::{Element, Empty};
    Empty::new().finish()
}
