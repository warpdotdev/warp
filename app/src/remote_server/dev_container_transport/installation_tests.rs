use super::expand_home_path;

#[test]
fn expand_home_path_rewrites_tilde_prefix() {
    assert_eq!(
        expand_home_path("/home/vscode", "~/.warp/remote-server"),
        "/home/vscode/.warp/remote-server"
    );
}

#[test]
fn expand_home_path_leaves_absolute_paths() {
    assert_eq!(
        expand_home_path("/home/vscode", "/tmp/oz.tar.gz"),
        "/tmp/oz.tar.gz"
    );
}
