use warp::tui_export::HandoffPrepareError;

use super::TuiHandoffModel;

#[test]
fn missing_token_after_eager_cancellation_restores_only_trimmed_argument() {
    let argument = "  keep this prompt  ".to_owned();
    let (replacement, _) = TuiHandoffModel::preparation_failure(
        HandoffPrepareError::MissingServerConversationToken,
        true,
        Some(&argument),
    )
    .into_parts();
    assert_eq!(replacement.as_deref(), Some("keep this prompt"));

    let (replacement, _) = TuiHandoffModel::preparation_failure(
        HandoffPrepareError::MissingServerConversationToken,
        true,
        None,
    )
    .into_parts();
    assert_eq!(replacement.as_deref(), Some(""));

    let (replacement, _) = TuiHandoffModel::preparation_failure(
        HandoffPrepareError::LongRunningCommand,
        true,
        Some(&argument),
    )
    .into_parts();
    assert!(
        replacement.is_none(),
        "pre-cancellation guard failures keep the full slash command draft"
    );

    let (replacement, _) = TuiHandoffModel::preparation_failure(
        HandoffPrepareError::MissingServerConversationToken,
        false,
        Some(&argument),
    )
    .into_parts();
    assert!(
        replacement.is_none(),
        "idle missing-token failures did not eagerly cancel the source"
    );
}
