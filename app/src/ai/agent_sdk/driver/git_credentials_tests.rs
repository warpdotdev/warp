use super::*;

#[test]
fn write_gh_hosts_yml_uses_gh_cli_filename() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let gh_config_dir = temp_dir.path().join(".config").join("gh");

    write_gh_hosts_yml(
        &[GitCredential {
            token: "token".to_string(),
            username: Some("octocat".to_string()),
            email: Some("octocat@example.com".to_string()),
            host: "github.com".to_string(),
        }],
        temp_dir.path(),
    )?;

    let hosts_path = gh_config_dir.join(GH_HOSTS_FILENAME);
    assert!(hosts_path.exists());
    assert!(!gh_config_dir
        .join(format!("{GH_HOSTS_FILENAME}.tmp"))
        .exists());

    let hosts = std::fs::read_to_string(hosts_path)?;
    assert!(hosts.contains("github.com:"));
    assert!(hosts.contains("    oauth_token: token"));
    assert!(hosts.contains("    git_protocol: https"));
    assert!(hosts.contains("    user: octocat"));

    Ok(())
}
