/// UI (compile-fail) tests for the `MODEL_HANDLE_IN_SUBSCRIPTION` lint.
///
/// Each `.rs` file in `tests/ui/` is compiled with the lint loaded.
/// Files with `//~ ERROR` annotations expect the lint to fire on that line.
/// Files without annotations are expected to compile cleanly (no false positives).
#[test]
fn compile_fail() {
    dylint_testing::ui_test(
        env!("CARGO_PKG_NAME"),
        &std::path::Path::new("tests/ui"),
    );
}
