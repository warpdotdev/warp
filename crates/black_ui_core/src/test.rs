use ctor::ctor;

// Initialize the logger before running tests.
#[ctor]
fn init() {
    simplelog::SimpleLogger::init(simplelog::LevelFilter::Info, simplelog::Config::default())
        .unwrap()
}
