use super::LSPServerType;
use crate::config::LanguageId;

#[test]
fn dart_analysis_server_configuration() {
    let server = LSPServerType::DartAnalysisServer;

    assert_eq!(server.binary_name(), "dart");
    assert_eq!(server.languages(), vec![LanguageId::Dart]);
    assert_eq!(server.language_name(), "Dart");

    #[cfg(not(target_arch = "wasm32"))]
    assert_eq!(server.args(), vec!["language-server"]);
}
