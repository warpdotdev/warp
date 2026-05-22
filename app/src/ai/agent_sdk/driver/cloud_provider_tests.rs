use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

use super::aws::AwsCloudProvider;
use super::gcp::GcpCloudProvider;
use super::{collect_env_vars, load_providers, CloudProvider};
use crate::ai::cloud_environments::{AwsProviderConfig, GcpProviderConfig, ProvidersConfig};

#[test]
fn aws_provider_env_vars_before_setup() {
    let config = AwsProviderConfig {
        role_arn: "arn:aws:iam::123456789012:role/MyRole".to_string(),
    };
    let provider = AwsCloudProvider::new(&config, "abc-123").unwrap();
    let vars = provider.env_vars().unwrap();

    assert_eq!(
        vars.get(&OsString::from("AWS_ROLE_ARN")),
        Some(&OsString::from("arn:aws:iam::123456789012:role/MyRole"))
    );
    assert_eq!(
        vars.get(&OsString::from("AWS_ROLE_SESSION_NAME")),
        Some(&OsString::from("Oz_Run_abc-123"))
    );
    let token_file = PathBuf::from(
        vars.get(&OsString::from("AWS_WEB_IDENTITY_TOKEN_FILE"))
            .unwrap(),
    );
    assert!(token_file
        .extension()
        .is_some_and(|extension| extension == "token"));
}

#[test]
fn extract_cloud_providers_empty_when_no_providers() {
    let config = ProvidersConfig {
        gcp: None,
        aws: None,
    };
    let providers = load_providers(&config, "run-1").unwrap();
    assert!(providers.is_empty());
}

#[test]
fn extract_cloud_providers_creates_aws_provider() {
    let config = ProvidersConfig {
        gcp: None,
        aws: Some(AwsProviderConfig {
            role_arn: "arn:aws:iam::111111111111:role/TestRole".to_string(),
        }),
    };
    let providers = load_providers(&config, "run-42").unwrap();
    assert_eq!(providers.len(), 1);

    let vars = providers[0].env_vars().unwrap();
    assert_eq!(
        vars.get(&OsString::from("AWS_ROLE_ARN")),
        Some(&OsString::from("arn:aws:iam::111111111111:role/TestRole"))
    );
    assert_eq!(
        vars.get(&OsString::from("AWS_ROLE_SESSION_NAME")),
        Some(&OsString::from("Oz_Run_run-42"))
    );
}

#[test]
fn gcp_provider_env_vars() {
    let config = GcpProviderConfig {
        project_number: "123456789".to_string(),
        workload_identity_federation_pool_id: "my-pool".to_string(),
        workload_identity_federation_provider_id: "my-provider".to_string(),
        service_account_email: None,
    };
    let provider = GcpCloudProvider::new(&config, "run-99").unwrap();
    let vars = provider.env_vars().unwrap();

    assert!(vars.contains_key(&OsString::from("GOOGLE_APPLICATION_CREDENTIALS")));
    assert_eq!(
        vars.get(&OsString::from("GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES")),
        Some(&OsString::from("1"))
    );
}

#[test]
fn extract_cloud_providers_creates_gcp_provider() {
    let config = ProvidersConfig {
        gcp: Some(GcpProviderConfig {
            project_number: "111".to_string(),
            workload_identity_federation_pool_id: "pool".to_string(),
            workload_identity_federation_provider_id: "prov".to_string(),
            service_account_email: None,
        }),
        aws: None,
    };
    let providers = load_providers(&config, "run-1").unwrap();
    assert_eq!(providers.len(), 1);

    let vars = providers[0].env_vars().unwrap();
    assert!(vars.contains_key(&OsString::from("GOOGLE_APPLICATION_CREDENTIALS")));
}

#[test]
fn collect_provider_env_vars_merges_all_providers() {
    let config = ProvidersConfig {
        gcp: Some(GcpProviderConfig {
            project_number: "222".to_string(),
            workload_identity_federation_pool_id: "pool".to_string(),
            workload_identity_federation_provider_id: "prov".to_string(),
            service_account_email: None,
        }),
        aws: Some(AwsProviderConfig {
            role_arn: "arn:aws:iam::999:role/R".to_string(),
        }),
    };
    let providers = load_providers(&config, "id-7").unwrap();
    assert_eq!(providers.len(), 2);

    let mut vars = HashMap::new();
    collect_env_vars(&providers, &mut vars).unwrap();

    // AWS variables.
    assert!(vars.contains_key(&OsString::from("AWS_ROLE_ARN")));
    assert!(vars.contains_key(&OsString::from("AWS_ROLE_SESSION_NAME")));
    // GCP variables.
    assert!(vars.contains_key(&OsString::from("GOOGLE_APPLICATION_CREDENTIALS")));
}
