use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::anyhow;
use chrono::{DateTime, Utc};
use clap::Parser as _;
use futures::channel::oneshot;
use uuid::Uuid;
use warp_cli::mcp::McpCredentialFormat;
use warp_core::features::FeatureFlag;
use warp_graphql::mutations::create_managed_mcp_client_config::{
    CreateManagedMcpClientConfigOutput, ManagedMcpTransportKind,
};
use warp_graphql::response_context::ResponseContext;

use super::*;
use crate::ai::agent_sdk::driver::managed_mcp_refresh::ManagedMcpCredential;
use crate::server::server_api::managed_mcp::MockManagedMcpClient;

const SERVER: &str = "Linear";
const WARP_ID: &str = "linear";
const HOUR: Duration = Duration::from_secs(60 * 60);

fn start_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000, 0).expect("valid timestamp")
}

fn lease() -> ManagedMcpLease {
    ManagedMcpLease {
        installation_id: Uuid::from_u128(1),
        server_name: SERVER.to_owned(),
        warp_id: WARP_ID.to_owned(),
        header_name: "Authorization".to_owned(),
        credential: ManagedMcpCredential {
            header_value: "Bearer first".to_owned(),
            expires_at: start_time() + 3 * HOUR,
        },
    }
}

fn minted(token: &str) -> anyhow::Result<CreateManagedMcpClientConfigOutput> {
    Ok(CreateManagedMcpClientConfigOutput {
        transport_kind: ManagedMcpTransportKind::Url,
        mcp_config_json: String::new(),
        proxy_url: Some("https://proxy.test/mcp".to_owned()),
        proxy_token: Some(token.to_owned()),
        authorization_header_name: Some("Authorization".to_owned()),
        authorization_header_value: Some(format!("Bearer {token}")),
        expires_at: Some((start_time() + 6 * HOUR).into()),
        response_context: ResponseContext {
            server_version: None,
        },
    })
}

/// A broker for [`SERVER`] answering helpers on its own thread until dropped.
struct Served {
    endpoint_file: PathBuf,
    mints: Arc<AtomicUsize>,
    now: Arc<Mutex<DateTime<Utc>>>,
    authenticated: Arc<AtomicBool>,
    _stop: oneshot::Sender<()>,
}

impl Served {
    /// Starts a broker whose client answers each mint with the next of `results`. Asking for
    /// more mints than it was given fails the helper's request.
    fn start(
        results: impl IntoIterator<Item = anyhow::Result<CreateManagedMcpClientConfigOutput>>,
    ) -> Self {
        let (broker, _helpers) = McpCredentialBroker::bind(
            HashMap::from([(SERVER.to_owned(), lease())]),
            Path::new("/usr/local/bin/oz"),
            start_time(),
        )
        .expect("bind broker");
        let endpoint_file = broker._dir.path().join(ENDPOINT_FILE_NAME);

        let results = Mutex::new(results.into_iter().collect::<VecDeque<_>>());
        let mints = Arc::new(AtomicUsize::new(0));
        let counted = mints.clone();
        let mut client = MockManagedMcpClient::new();
        client
            .expect_create_managed_mcp_client_config()
            .returning(move |uid| {
                assert_eq!(uid, WARP_ID);
                counted.fetch_add(1, Ordering::SeqCst);
                results
                    .lock()
                    .expect("mint results")
                    .pop_front()
                    .unwrap_or_else(|| Err(anyhow!("no further mint was expected")))
            });

        let now = Arc::new(Mutex::new(start_time()));
        let authenticated = Arc::new(AtomicBool::new(true));
        let (stop, stopped) = oneshot::channel::<()>();
        let clock = now.clone();
        let is_authenticated = authenticated.clone();
        std::thread::spawn(move || {
            futures::executor::block_on(async move {
                let serve = broker
                    .serve(
                        Arc::new(client),
                        Arc::new(move || *clock.lock().expect("clock")),
                        move || {
                            let authenticated = is_authenticated.load(Ordering::SeqCst);
                            async move { authenticated }
                        },
                    )
                    .fuse();
                let stopped = stopped.fuse();
                futures::pin_mut!(serve, stopped);
                futures::select! {
                    _ = serve => {}
                    _ = stopped => {}
                }
            });
        });
        Self {
            endpoint_file,
            mints,
            now,
            authenticated,
            _stop: stop,
        }
    }

    fn advance(&self, duration: Duration) {
        *self.now.lock().expect("clock") += duration;
    }

    /// Asks as the helper does, and returns the header value it would print.
    fn ask(&self) -> String {
        let response = request_credential(&self.endpoint_file, SERVER).expect("credential");
        render_credential(response, SERVER, McpCredentialFormat::Value).expect("a credential")
    }

    fn mints(&self) -> usize {
        self.mints.load(Ordering::SeqCst)
    }
}

#[test]
fn helper_command_quotes_everything_the_shell_would_split_or_expand() {
    let command = render_helper_command(
        Path::new("/Applications/Warp Dev.app/Contents/Resources/bin/oz"),
        Path::new("/tmp/oz-mcp-abc/endpoint.json"),
        "~GitHub MCP",
    );
    assert_eq!(
        command,
        "'/Applications/Warp Dev.app/Contents/Resources/bin/oz' mcp-credential \
         --endpoint-file /tmp/oz-mcp-abc/endpoint.json --server '~GitHub MCP'"
    );

    let helper = McpCredentialHelper::for_test("Authorization", &command);
    assert_eq!(helper.headers_command(), command);
    assert_eq!(helper.value_command(), format!("{command} --format value"));
}

/// A harness runs the command through a shell, so what the broker renders has to split into an
/// invocation this executable's own argument parser accepts.
#[test]
fn helper_command_is_parsed_as_the_credential_worker() {
    let endpoint_file = Path::new("/tmp/oz mcp/endpoint.json");
    let helper = McpCredentialHelper::for_test(
        "Authorization",
        &render_helper_command(Path::new("/usr/local/bin/oz"), endpoint_file, "GitHub MCP"),
    );

    for (command, expected_format) in [
        (
            helper.headers_command().to_owned(),
            McpCredentialFormat::Headers,
        ),
        (helper.value_command(), McpCredentialFormat::Value),
    ] {
        let argv = shell_words::split(&command).expect("a splittable command");
        let args = warp_cli::Args::try_parse_from(argv).expect("an accepted command line");
        let Some(warp_cli::Command::Worker(warp_cli::WorkerCommand::McpCredential {
            endpoint_file: parsed_endpoint_file,
            server,
            format,
        })) = args.command()
        else {
            panic!("expected the credential worker, got {:?}", args.command());
        };
        assert_eq!(parsed_endpoint_file, endpoint_file);
        assert_eq!(server, "GitHub MCP");
        assert_eq!(*format, expected_format);
    }
}

#[test]
fn credential_is_printed_as_a_header_object_or_a_bare_value() {
    let response = || CredentialResponse::Credential {
        header_name: "Authorization".to_owned(),
        header_value: "Bearer token".to_owned(),
    };

    let headers = render_credential(response(), SERVER, McpCredentialFormat::Headers).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&headers).unwrap(),
        serde_json::json!({ "Authorization": "Bearer token" })
    );
    assert_eq!(
        render_credential(response(), SERVER, McpCredentialFormat::Value).unwrap(),
        "Bearer token"
    );

    let error = render_credential(
        CredentialResponse::UnknownServer,
        SERVER,
        McpCredentialFormat::Headers,
    )
    .expect_err("an unknown server has no credential to print");
    assert!(error.to_string().contains(SERVER), "{error}");
}

/// The servers were resolved moments before the harness starts, so its first questions are
/// answered with the credential already held.
#[test]
fn helper_gets_the_resolved_credential_without_a_mint() {
    let served = Served::start([]);

    assert_eq!(served.ask(), "Bearer first");
    assert_eq!(served.mints(), 0);
}

#[test]
fn helper_gets_a_new_credential_once_the_current_one_is_no_longer_fresh() {
    let served = Served::start([minted("second")]);

    served.advance(MINT_INTERVAL);
    assert_eq!(served.ask(), "Bearer second");
    assert_eq!(served.mints(), 1);

    // A harness that was rejected asks again at once; it gets the credential just minted.
    assert_eq!(served.ask(), "Bearer second");
    assert_eq!(served.mints(), 1);
}

#[test]
fn concurrent_helpers_share_one_mint() {
    let served = Arc::new(Served::start([minted("second")]));
    served.advance(MINT_INTERVAL);

    let helpers: Vec<_> = (0..4)
        .map(|_| {
            let served = served.clone();
            std::thread::spawn(move || served.ask())
        })
        .collect();
    for helper in helpers {
        assert_eq!(helper.join().expect("helper thread"), "Bearer second");
    }
    assert_eq!(served.mints(), 1);
}

#[test]
fn failed_mint_answers_with_the_credential_already_held() {
    let served = Served::start([Err(anyhow!("server unavailable")), minted("second")]);

    served.advance(MINT_INTERVAL);
    assert_eq!(served.ask(), "Bearer first");
    assert_eq!(served.mints(), 1);

    // The failure is not retried until the interval has passed again.
    assert_eq!(served.ask(), "Bearer first");
    assert_eq!(served.mints(), 1);

    served.advance(MINT_INTERVAL);
    assert_eq!(served.ask(), "Bearer second");
    assert_eq!(served.mints(), 2);
}

#[test]
fn nothing_is_minted_while_the_process_is_not_authenticated() {
    let served = Served::start([minted("second")]);
    served.authenticated.store(false, Ordering::SeqCst);

    served.advance(MINT_INTERVAL);
    assert_eq!(served.ask(), "Bearer first");
    assert_eq!(served.mints(), 0);

    served.authenticated.store(true, Ordering::SeqCst);
    served.advance(MINT_INTERVAL);
    assert_eq!(served.ask(), "Bearer second");
    assert_eq!(served.mints(), 1);
}

#[test]
fn server_the_broker_does_not_hold_is_reported_as_unknown() {
    let served = Served::start([]);

    let response = request_credential(&served.endpoint_file, "Unknown").expect("an answer");

    assert!(matches!(response, CredentialResponse::UnknownServer));
}

#[test]
fn client_with_the_wrong_token_is_turned_away() {
    let served = Served::start([]);
    let contents = std::fs::read_to_string(&served.endpoint_file).expect("endpoint file");
    let BrokerEndpoint { endpoint, .. } = serde_json::from_str(&contents).expect("endpoint");
    let impostor_file = tempfile::NamedTempFile::new().expect("temp file");
    std::fs::write(
        impostor_file.path(),
        serde_json::to_vec(&BrokerEndpoint {
            endpoint,
            token: "not-the-token".to_owned(),
        })
        .expect("serialize endpoint"),
    )
    .expect("write endpoint");

    let error = request_credential(impostor_file.path(), SERVER)
        .err()
        .expect("the broker should reject the token");

    assert!(error.to_string().contains("rejected the token"), "{error}");
    assert_eq!(served.ask(), "Bearer first");
}

#[cfg(unix)]
#[test]
fn broker_is_bound_only_for_expiring_credentials_with_the_helper_enabled() {
    let resolved_with_lease = || ResolvedMCPServers {
        leases: HashMap::from([(SERVER.to_owned(), lease())]),
        ..Default::default()
    };

    {
        let _flag = FeatureFlag::ManagedMcpHarnessCredentialHelper.override_enabled(false);
        let mut resolved = resolved_with_lease();
        assert!(McpCredentialBroker::bind_for(&mut resolved).is_none());
        assert!(resolved.credential_helpers.is_empty());
    }

    let _flag = FeatureFlag::ManagedMcpHarnessCredentialHelper.override_enabled(true);
    let mut without_lease = ResolvedMCPServers::default();
    assert!(McpCredentialBroker::bind_for(&mut without_lease).is_none());

    let mut resolved = resolved_with_lease();
    let broker = McpCredentialBroker::bind_for(&mut resolved);
    assert!(broker.is_some());
    assert_eq!(
        resolved.credential_helpers[SERVER].header_name,
        "Authorization"
    );
    assert!(
        resolved.credential_helpers[SERVER]
            .headers_command()
            .ends_with(&format!("--server {SERVER}"))
    );
}
