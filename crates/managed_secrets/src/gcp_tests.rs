use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use serde_json::json;

use super::{
    GcpCredentials, GcpFederationConfig, PrepareGcpCredentialsError, generate_gcp_credential_config,
};

#[test]
fn basic_config_shape() {
    let config = GcpFederationConfig {
        project_number: "123456789".to_string(),
        pool_id: "my-pool".to_string(),
        provider_id: "my-provider".to_string(),
        service_account_email: None,
        token_lifetime: None,
    };

    let result = generate_gcp_credential_config(
        "task-42",
        &config,
        Path::new("/usr/bin/oz"),
        Path::new("/tmp/token_cache"),
    )
    .unwrap();

    assert_eq!(
        result,
        json!({
            "type": "external_account",
            "audience": "//iam.googleapis.com/projects/123456789/locations/global/workloadIdentityPools/my-pool/providers/my-provider",
            "subject_token_type": "urn:ietf:params:oauth:token-type:id_token",
            "token_url": "https://sts.googleapis.com/v1/token",
            "credential_source": {
                "executable": {
                    "command": "/usr/bin/oz federate issue-gcp-token --run-id task-42",
                    "timeout_millis": 30000,
                    "output_file": "/tmp/token_cache"
                }
            }
        })
    );
}

#[test]
fn rejects_binary_path_with_spaces() {
    let config = GcpFederationConfig {
        project_number: "123".to_string(),
        pool_id: "pool".to_string(),
        provider_id: "prov".to_string(),
        service_account_email: None,
        token_lifetime: None,
    };

    let result = generate_gcp_credential_config(
        "task-1",
        &config,
        Path::new("/path with spaces/oz"),
        Path::new("/tmp/out"),
    );
    assert!(matches!(
        result,
        Err(PrepareGcpCredentialsError::InvalidBinaryPath { .. })
    ));
}

#[test]
fn rejects_task_id_with_spaces() {
    let config = GcpFederationConfig {
        project_number: "123".to_string(),
        pool_id: "pool".to_string(),
        provider_id: "prov".to_string(),
        service_account_email: None,
        token_lifetime: None,
    };

    let result = generate_gcp_credential_config(
        "task with spaces",
        &config,
        Path::new("/bin/oz"),
        Path::new("/tmp/out"),
    );
    assert!(matches!(
        result,
        Err(PrepareGcpCredentialsError::InvalidTaskId { .. })
    ));
}

#[test]
fn service_account_impersonation() {
    let config = GcpFederationConfig {
        project_number: "111".to_string(),
        pool_id: "pool".to_string(),
        provider_id: "prov".to_string(),
        service_account_email: Some("sa@project.iam.gserviceaccount.com".to_string()),
        token_lifetime: Some(Duration::from_secs(1800)),
    };

    let result =
        generate_gcp_credential_config("t-1", &config, Path::new("/bin/oz"), Path::new("/tmp/out"))
            .unwrap();

    assert_eq!(
        result,
        json!({
            "type": "external_account",
            "audience": "//iam.googleapis.com/projects/111/locations/global/workloadIdentityPools/pool/providers/prov",
            "subject_token_type": "urn:ietf:params:oauth:token-type:id_token",
            "token_url": "https://sts.googleapis.com/v1/token",
            "credential_source": {
                "executable": {
                    "command": "/bin/oz federate issue-gcp-token --run-id t-1 --duration 1800s",
                    "timeout_millis": 30000,
                    "output_file": "/tmp/out"
                }
            },
            "service_account_impersonation_url": "https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/sa@project.iam.gserviceaccount.com:generateAccessToken",
            "service_account_impersonation": {
                "token_lifetime_seconds": 1800
            }
        })
    );
}

#[test]
fn config_file_path_matches_application_credentials_env_var() {
    let config = GcpFederationConfig {
        project_number: "123456789".to_string(),
        pool_id: "my-pool".to_string(),
        provider_id: "my-provider".to_string(),
        service_account_email: None,
        token_lifetime: None,
    };

    let credentials = GcpCredentials::federated("run-77", &config).unwrap();

    // The accessor used by the GCP cloud provider to point `gcloud --cred-file`
    // at the credential config must resolve to the same file that GCP SDKs
    // discover via `GOOGLE_APPLICATION_CREDENTIALS`.
    let env_vars = credentials.env_vars();
    assert_eq!(
        env_vars.get(&OsString::from("GOOGLE_APPLICATION_CREDENTIALS")),
        Some(&OsString::from(credentials.config_file_path())),
    );
    assert!(credentials.config_file_path().exists());
}

#[test]
fn no_duration_flag_when_lifetime_absent() {
    let config = GcpFederationConfig {
        project_number: "333".to_string(),
        pool_id: "pool".to_string(),
        provider_id: "prov".to_string(),
        service_account_email: None,
        token_lifetime: None,
    };

    let result =
        generate_gcp_credential_config("t-3", &config, Path::new("/bin/oz"), Path::new("/tmp/out"))
            .unwrap();
    assert_eq!(
        result,
        json!({
            "type": "external_account",
            "audience": "//iam.googleapis.com/projects/333/locations/global/workloadIdentityPools/pool/providers/prov",
            "subject_token_type": "urn:ietf:params:oauth:token-type:id_token",
            "token_url": "https://sts.googleapis.com/v1/token",
            "credential_source": {
                "executable": {
                    "command": "/bin/oz federate issue-gcp-token --run-id t-3",
                    "timeout_millis": 30000,
                    "output_file": "/tmp/out"
                }
            }
        })
    );
}
