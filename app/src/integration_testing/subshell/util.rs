/// The command used to proxy ssh requests through GCP's Identity-Aware Proxy.
const PROXY_COMMAND: &str = "gcloud compute start-iap-tunnel ubuntu-14-04 25784 --listen-on-stdin --project=warp-ssh-integration-testing --zone=us-east4-a";
/// The command used to proxy remote-server ssh requests through GCP's Identity-Aware Proxy.
const REMOTE_SERVER_PROXY_COMMAND: &str = "gcloud compute start-iap-tunnel ssh-remote-server-testing 22 --listen-on-stdin --project=warp-ssh-integration-testing --zone=us-east4-b";

/// Produces a user/host pair for testing a given remote shell.
pub fn user_host(shell: &str) -> String {
    format!("{shell}@ubuntu-14-04")
}
/// Produces a user/host pair for remote-server tests.
pub fn remote_server_user_host(shell: &str) -> String {
    format!("{shell}@ssh-remote-server-testing")
}

/// Produces the full ssh command to run to ssh into a given remote shell.
pub fn ssh_command(shell: &str, should_use_ssh_wrapper: bool) -> String {
    [
        if should_use_ssh_wrapper {
            "ssh"
        } else {
            "command ssh"
        },
        &user_host(shell),
        "-p 25784",
        &format!("-o ProxyCommand=\"{PROXY_COMMAND}\""),
        "-o StrictHostKeyChecking=no",
        "-o UserKnownHostsFile=/dev/null",
    ]
    .join(" ")
}

/// Produces the full ssh command to connect to the dedicated remote-server test host.
pub fn remote_server_ssh_command(shell: &str, should_use_ssh_wrapper: bool) -> String {
    [
        if should_use_ssh_wrapper {
            "ssh"
        } else {
            "command ssh"
        },
        &remote_server_user_host(shell),
        "-p 22",
        &format!("-o ProxyCommand=\"{REMOTE_SERVER_PROXY_COMMAND}\""),
        "-o PreferredAuthentications=password",
        "-o PubkeyAuthentication=no",
        "-o StrictHostKeyChecking=no",
        "-o UserKnownHostsFile=/dev/null",
    ]
    .join(" ")
}
