use codex_app_server::{Account, AccountStatus, AccountType};

use super::connected_account_message;

#[test]
fn formats_chatgpt_account_status_without_credentials() {
    let message = connected_account_message(&AccountStatus {
        account: Some(Account {
            account_type: AccountType::ChatGpt,
            email: Some("coder@example.com".to_owned()),
            plan_type: Some("pro".to_owned()),
        }),
        requires_openai_auth: true,
    });

    assert_eq!(
        message,
        "Signed in to Codex with ChatGPT as coder@example.com (pro)."
    );
}
