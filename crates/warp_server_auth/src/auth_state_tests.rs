use super::*;

fn auth_state_with_principal(user_id: &str, principal_type: PrincipalType) -> AuthState {
    let auth_state = AuthState::new_for_test();
    let mut user = User::test();
    user.local_id = UserUid::new(user_id);
    user.principal_type = principal_type;
    auth_state.set_user(Some(user));
    auth_state
}

#[test]
fn telemetry_user_id_prefixes_service_account_uid() {
    let auth_state = auth_state_with_principal(
        "01994db3-4b84-7dd7-88ba-052af4edbbd9",
        PrincipalType::ServiceAccount,
    );
    assert_eq!(
        auth_state.user_id(),
        Some(UserUid::new("01994db3-4b84-7dd7-88ba-052af4edbbd9"))
    );

    assert_eq!(
        auth_state.telemetry_user_id().as_deref(),
        Some("serviceAccount:01994db3-4b84-7dd7-88ba-052af4edbbd9")
    );
}

#[test]
fn telemetry_user_id_preserves_prefixed_service_account_uid() {
    let auth_state = auth_state_with_principal(
        "serviceAccount:01994db3-4b84-7dd7-88ba-052af4edbbd9",
        PrincipalType::ServiceAccount,
    );

    assert_eq!(
        auth_state.telemetry_user_id().as_deref(),
        Some("serviceAccount:01994db3-4b84-7dd7-88ba-052af4edbbd9")
    );
}

#[test]
fn telemetry_user_id_preserves_user_uid() {
    let auth_state = auth_state_with_principal("firebase-user-id", PrincipalType::User);

    assert_eq!(
        auth_state.telemetry_user_id().as_deref(),
        Some("firebase-user-id")
    );
}
