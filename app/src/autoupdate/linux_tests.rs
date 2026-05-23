use super::*;

#[test]
fn test_repo_name() {
    assert_eq!(repo_name(Channel::Dev), "blackdagger-dev");
    assert_eq!(repo_name(Channel::Stable), "blackdagger");
}
