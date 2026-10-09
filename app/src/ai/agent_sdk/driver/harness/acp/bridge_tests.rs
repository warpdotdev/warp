use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

use futures::executor::block_on;
use futures::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use instant::Instant;
use warp_util::path::ShellFamily;

use super::{
    BridgeEndpoint, BridgeListener, BridgeReader, BridgeStream, BridgeWriter, MAX_TOKEN_LINE,
    TOKEN_BYTES, bridge_command, connect_with_token, read_launch, read_line, render_bridge_command,
};

#[test]
fn bridge_command_names_the_executable_and_the_launch_file_escaped_for_the_shell() {
    let command = render_bridge_command(
        Path::new("/Applications/Warp Preview.app/Contents/MacOS/preview"),
        Path::new("/tmp/oz-acp x/launch.json"),
        ShellFamily::Posix,
    );
    assert_eq!(
        command,
        "/Applications/Warp\\ Preview.app/Contents/MacOS/preview acp-bridge --launch-file \
         /tmp/oz-acp\\ x/launch.json"
    );

    let command = render_bridge_command(
        Path::new(r"C:\Program Files\Warp\warp.exe"),
        Path::new(r"C:\Temp\oz-acp-x\launch.json"),
        ShellFamily::PowerShell,
    );
    assert_eq!(
        command,
        r"& C:\Program` Files\Warp\warp.exe acp-bridge --launch-file C:\Temp\oz-acp-x\launch.json"
    );
}

#[test]
fn bridge_command_uses_the_current_executable() {
    let command = bridge_command(Path::new("/tmp/launch.json"), ShellFamily::Posix).unwrap();
    let executable = std::env::current_exe().unwrap().display().to_string();
    assert!(
        command.starts_with(&ShellFamily::Posix.shell_escape(&executable).to_string()),
        "{command}"
    );
}

#[test]
fn launch_file_carries_the_agent_command_and_a_fresh_token() {
    let listener =
        BridgeListener::bind("npx", &["-y".into(), "@zed-industries/codex-acp".into()]).unwrap();
    let launch = read_launch(listener.launch_file()).unwrap();
    assert_eq!(launch.agent, ["npx", "-y", "@zed-industries/codex-acp"]);
    assert_eq!(launch.endpoint, listener.endpoint);
    assert_eq!(launch.token.len(), 2 * TOKEN_BYTES);
    assert!(launch.token.len() <= MAX_TOKEN_LINE);
    assert!(launch.token.bytes().all(|b| b.is_ascii_hexdigit()));

    let other = BridgeListener::bind("cat", &[]).unwrap();
    assert_ne!(
        read_launch(other.launch_file()).unwrap().token,
        launch.token
    );
}

#[cfg(unix)]
#[test]
fn unix_hosts_listen_on_a_socket_file_in_the_private_directory() {
    let listener = BridgeListener::bind("cat", &[]).unwrap();
    let BridgeEndpoint::Unix(path) = &listener.endpoint else {
        panic!(
            "expected a Unix socket endpoint, got {:?}",
            listener.endpoint
        );
    };
    assert_eq!(path.parent(), listener.launch_file().parent());
    assert!(path.exists());
}

#[cfg(not(unix))]
#[test]
fn non_unix_hosts_listen_on_loopback_tcp() {
    let listener = BridgeListener::bind("cat", &[]).unwrap();
    let BridgeEndpoint::Tcp(address) = &listener.endpoint else {
        panic!("expected a TCP endpoint, got {:?}", listener.endpoint);
    };
    assert!(address.ip().is_loopback());
}

/// Does what the bridge subcommand does before handing off to the agent.
fn connect(listener: &BridgeListener) -> BridgeStream {
    let launch = read_launch(listener.launch_file()).unwrap();
    connect_with_token(&launch.endpoint, &launch.token).unwrap()
}

/// A command that echoes stdin to stdout line by line and exits on EOF. The Windows variant
/// flushes after every line (`findstr`/`more` buffer until EOF when stdout is a pipe) and emits
/// CRLF, so callers compare trimmed lines.
fn echo_command() -> (&'static str, Vec<String>) {
    if cfg!(windows) {
        (
            "powershell",
            vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                "while (($line = [Console]::In.ReadLine()) -ne $null) { \
                 [Console]::Out.WriteLine($line); [Console]::Out.Flush() }"
                    .into(),
            ],
        )
    } else {
        ("cat", vec![])
    }
}

/// A command that never reads stdin and runs well past any test's patience.
fn ignores_eof_command() -> (&'static str, Vec<String>) {
    if cfg!(windows) {
        ("ping", vec!["-n".into(), "60".into(), "127.0.0.1".into()])
    } else {
        ("sh", vec!["-c".into(), "exec sleep 60".into()])
    }
}

/// A command that exits immediately without touching stdin.
fn exits_immediately_command() -> (&'static str, Vec<String>) {
    if cfg!(windows) {
        ("cmd", vec!["/c".into(), "exit".into(), "0".into()])
    } else {
        ("true", vec![])
    }
}

/// Drives one echo round trip and an EOF-triggered shutdown through the accepted halves.
fn assert_echo_then_eof(reader: BridgeReader, mut writer: BridgeWriter) {
    let mut reader = BufReader::new(reader);
    block_on(async {
        writer.write_all(b"{\"jsonrpc\":\"2.0\"}\n").await.unwrap();
        writer.flush().await.unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert_eq!(line.trim_end(), "{\"jsonrpc\":\"2.0\"}");

        // Closing our writer shuts down the socket's write side, which is EOF on the agent's
        // stdin; merely dropping the half would leave the agent waiting.
        writer.close().await.unwrap();
        let mut rest = String::new();
        reader.read_line(&mut rest).await.unwrap();
        assert!(rest.is_empty(), "agent should close stdout after EOF");
    });
}

/// Runs the bridge-side connect on another thread while the driver accepts on this one, since
/// the connect blocks on the driver's verdict.
fn connect_while_accepting(
    listener: &BridgeListener,
    token: &str,
) -> (
    anyhow::Result<BridgeStream>,
    anyhow::Result<(BridgeReader, BridgeWriter)>,
) {
    let endpoint = read_launch(listener.launch_file()).unwrap().endpoint;
    std::thread::scope(|scope| {
        let connected = scope.spawn(|| connect_with_token(&endpoint, token));
        let accepted = block_on(listener.accept(Duration::from_millis(500)));
        (connected.join().unwrap(), accepted)
    })
}

#[cfg(unix)]
#[test]
fn exec_style_agent_talks_to_the_driver_over_the_socket() {
    let listener = BridgeListener::bind("cat", &[]).unwrap();
    let (connected, accepted) = connect_while_accepting(&listener, &listener.token);
    // Stand in for the exec'd agent: spawn `cat` with the socket as its stdio.
    let mut agent = super::exec_agent_command(connected.unwrap(), "cat", &[])
        .unwrap()
        .spawn()
        .unwrap();

    let (reader, writer) = accepted.unwrap();
    assert_echo_then_eof(reader, writer);
    assert!(agent.wait().unwrap().success());
}

#[test]
fn relayed_agent_talks_to_the_driver_over_the_socket() {
    let (program, args) = echo_command();
    let listener = BridgeListener::bind(program, &args).unwrap();
    let (connected, accepted) = connect_while_accepting(&listener, &listener.token);
    let relay = std::thread::spawn(move || {
        super::relay_agent(connected.unwrap(), program, &args, Duration::from_secs(10))
    });

    let (reader, writer) = accepted.unwrap();
    assert_echo_then_eof(reader, writer);
    assert!(relay.join().unwrap().unwrap().success());
}

#[test]
fn relay_kills_an_agent_that_ignores_eof_after_the_grace_period() {
    let (program, args) = ignores_eof_command();
    let listener = BridgeListener::bind(program, &args).unwrap();
    let (connected, accepted) = connect_while_accepting(&listener, &listener.token);
    let grace = Duration::from_millis(200);
    let relay =
        std::thread::spawn(move || super::relay_agent(connected.unwrap(), program, &args, grace));

    let (reader, mut writer) = accepted.unwrap();
    let closed_at = Instant::now();
    block_on(writer.close()).unwrap();
    let status = relay.join().unwrap().unwrap();
    assert!(!status.success(), "agent should have been killed: {status}");
    assert!(closed_at.elapsed() < Duration::from_secs(10));
    drop(reader);
}

#[test]
fn relay_finishes_when_the_agent_exits_before_the_driver_closes_the_socket() {
    let (program, args) = exits_immediately_command();
    let listener = BridgeListener::bind(program, &args).unwrap();
    let (connected, accepted) = connect_while_accepting(&listener, &listener.token);
    let relay = std::thread::spawn(move || {
        super::relay_agent(connected.unwrap(), program, &args, Duration::from_secs(10))
    });

    // The driver keeps both halves open; the relay must still return once the agent is gone.
    let (reader, writer) = accepted.unwrap();
    let status = relay.join().unwrap().unwrap();
    assert!(status.success(), "{status}");
    drop((reader, writer));
}

#[test]
fn impostors_are_told_they_are_rejected_and_the_genuine_bridge_is_accepted() {
    let listener = BridgeListener::bind("cat", &[]).unwrap();
    let endpoint = read_launch(listener.launch_file()).unwrap().endpoint;
    let (impostor_verdict, genuine, accepted) = std::thread::scope(|scope| {
        let impostor = scope.spawn(|| {
            let mut impostor = BridgeStream::connect(&endpoint).unwrap();
            impostor.write_all(b"not-the-token\n").unwrap();
            read_line(&mut impostor, 16).unwrap()
        });
        let genuine = scope.spawn(|| {
            // Let the impostor get in first so the driver has to move past it.
            std::thread::sleep(Duration::from_millis(50));
            connect(&listener)
        });
        let accepted = block_on(listener.accept(Duration::from_secs(10)));
        (
            impostor.join().unwrap(),
            genuine.join().unwrap(),
            accepted.unwrap(),
        )
    });
    assert_eq!(impostor_verdict, "rejected");

    // Prove we are talking to the genuine peer: it sees what we write.
    let (reader, mut writer) = accepted;
    block_on(async {
        writer.write_all(b"ping\n").await.unwrap();
        writer.flush().await.unwrap();
    });
    let mut genuine = genuine;
    assert_eq!(read_line(&mut genuine, 16).unwrap(), "ping");
    drop(reader);
}

#[test]
fn bridge_fails_clearly_when_its_token_is_rejected() {
    let listener = BridgeListener::bind("cat", &[]).unwrap();
    let (connected, accepted) = connect_while_accepting(&listener, "not-the-token");
    let Err(error) = connected else {
        panic!("the bridge should not have been accepted");
    };
    assert!(
        error.to_string().contains("rejected the bridge token"),
        "{error:#}"
    );
    // The driver keeps waiting for the real bridge, which never came.
    assert!(accepted.is_err());
}

#[test]
fn accept_times_out_when_no_agent_connects() {
    let listener = BridgeListener::bind("cat", &[]).unwrap();
    let Err(error) = block_on(listener.accept(Duration::from_millis(100))) else {
        panic!("accept should time out without a client");
    };
    assert!(error.to_string().contains("did not connect"), "{error}");
}

#[test]
fn bridge_rejects_a_launch_file_without_an_agent_command() {
    let dir = tempfile::tempdir().unwrap();
    let launch_file = dir.path().join("launch.json");
    std::fs::write(
        &launch_file,
        r#"{"endpoint":{"Tcp":"127.0.0.1:1"},"token":"t","agent":[]}"#,
    )
    .unwrap();
    let error = super::run_bridge(&launch_file).unwrap_err();
    assert!(error.to_string().contains("no agent command"), "{error}");
}
