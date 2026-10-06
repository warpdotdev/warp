use serde_json::json;

use super::*;

fn batch() -> serde_json::Value {
    json!({
        "working_dir": std::env::current_dir().unwrap(),
        "repositories": [{
            "source": {"code_forge": "GITHUB", "owner": "warpdotdev", "repo": "warp"},
            "checkout_name": "warp",
            "head": null,
            "fetch_branch_only": false
        }]
    })
}

#[test]
fn batch_rejects_malformed_and_untrusted_payloads() {
    assert!(CheckoutBatch::parse(b"{").is_err());
    for (pointer, value) in [
        ("/repositories/0/source/code_forge", json!("UNKNOWN")),
        ("/repositories/0/source/owner", json!("../owner")),
        ("/repositories/0/source/repo", json!("repo?token=secret")),
        ("/repositories/0/checkout_name", json!("../outside")),
        ("/repositories/0/checkout_name", json!("C:\\outside")),
        ("/repositories/0/checkout_name", json!("CON.txt")),
        (
            "/repositories/0/head",
            json!({"type": "BRANCH", "value": "--upload-pack=bad"}),
        ),
        (
            "/repositories/0/head",
            json!({"type": "COMMIT_SHA", "value": "not-a-sha"}),
        ),
        (
            "/repositories/0/head",
            json!({"type": "BRANCH", "value": "main", "url": "untrusted"}),
        ),
        ("/repositories/0/fetch_branch_only", json!(true)),
    ] {
        let mut batch = batch();
        *batch.pointer_mut(pointer).unwrap() = value;
        assert!(
            CheckoutBatch::parse(&serde_json::to_vec(&batch).unwrap()).is_err(),
            "{pointer}"
        );
    }
    let mut batch = batch();
    batch["repositories"][0]["url"] = json!("https://user:password@example.com/repo");
    assert!(CheckoutBatch::parse(&serde_json::to_vec(&batch).unwrap()).is_err());
}

#[test]
fn duplicate_targets_are_rejected_before_any_checkout() {
    let mut batch = batch();
    let mut second = batch["repositories"][0].clone();
    second["checkout_name"] = json!("WARP");
    batch["repositories"].as_array_mut().unwrap().push(second);
    assert_eq!(
        CheckoutBatch::parse(&serde_json::to_vec(&batch).unwrap())
            .unwrap_err()
            .to_string(),
        "duplicate checkout target"
    );
}

#[test]
fn nested_gitlab_and_azure_identities_and_tag_pins_are_accepted() {
    for (forge, owner, repo) in [
        ("GITLAB", "platform/backend", "api"),
        ("AZURE_DEVOPS", "organization/My Project", "My Repo"),
    ] {
        let mut batch = batch();
        batch["repositories"][0]["source"] =
            json!({"code_forge": forge, "owner": owner, "repo": repo});
        batch["repositories"][0]["head"] = json!({"type": "BRANCH", "value": "refs/tags/v1"});
        CheckoutBatch::parse(&serde_json::to_vec(&batch).unwrap()).unwrap();
    }
}
