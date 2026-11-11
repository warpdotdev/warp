use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::channel::{mpsc, oneshot};
use futures::executor::LocalPool;
use futures::task::LocalSpawnExt as _;
use http::StatusCode;
use mcp::refreshable_headers::{RefreshableHeaders, RejectionHandler};
use uuid::Uuid;
use warp_graphql::mutations::create_managed_mcp_client_config::{
    CreateManagedMcpClientConfigOutput, ManagedMcpTransportKind,
};
use warp_graphql::response_context::ResponseContext;
use warp_server_client::graphql_helpers::AUTH_REJECTED_MESSAGE;

use super::*;
use crate::server::graphql::GraphQLError;
use crate::server::server_api::managed_mcp::{
    ManagedMcpClientConfigRejected, MockManagedMcpClient,
};

const WARP_ID: &str = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";
const EXPIRED_CHALLENGE: &str =
    r#"Bearer error="invalid_token", error_description="proxy_token_expired""#;
const STALE_CHALLENGE: &str =
    r#"Bearer error="invalid_token", error_description="proxy_token_stale""#;
const HOUR: Duration = Duration::from_secs(60 * 60);
const MINUTE: Duration = Duration::from_secs(60);

struct ClockState {
    now: DateTime<Utc>,
    sleeps: Vec<(DateTime<Utc>, oneshot::Sender<()>)>,
}

/// A wall clock tests move by hand, whose sleeps end only when the clock is advanced past them.
#[derive(Clone)]
struct TestClock(Arc<Mutex<ClockState>>);

impl TestClock {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(ClockState {
            now: DateTime::from_timestamp(1_800_000_000, 0).expect("valid timestamp"),
            sleeps: Vec::new(),
        })))
    }

    fn now(&self) -> DateTime<Utc> {
        self.0.lock().expect("clock").now
    }

    fn refresh_clock(&self) -> RefreshClock {
        let now = self.clone();
        let sleep = self.clone();
        RefreshClock {
            now: Arc::new(move || now.now()),
            sleep: Arc::new(move |duration| {
                let (wake, woken) = oneshot::channel();
                let mut state = sleep.0.lock().expect("clock");
                let deadline = state.now + duration;
                state.sleeps.push((deadline, wake));
                async move {
                    let _ = woken.await;
                }
                .boxed()
            }),
            jitter: Arc::new(|| 0.0),
        }
    }

    /// Moves the clock forward and ends the sleeps that have come due.
    fn advance(&self, duration: Duration) {
        let mut state = self.0.lock().expect("clock");
        state.now += duration;
        let now = state.now;
        let sleeps = std::mem::take(&mut state.sleeps);
        for (deadline, wake) in sleeps {
            if deadline <= now {
                let _ = wake.send(());
            } else {
                state.sleeps.push((deadline, wake));
            }
        }
    }

    /// Moves the clock forward without ending any sleep, as a suspended machine does: its
    /// timers resume where they stopped.
    fn suspend(&self, duration: Duration) {
        self.0.lock().expect("clock").now += duration;
    }
}

fn lease(expires_at: DateTime<Utc>) -> ManagedMcpLease {
    ManagedMcpLease {
        installation_id: Uuid::from_u128(1),
        server_name: "Linear".to_owned(),
        warp_id: WARP_ID.to_owned(),
        header_name: "Authorization".to_owned(),
        credential: ManagedMcpCredential {
            header_value: "Bearer first".to_owned(),
            expires_at,
        },
    }
}

fn url_config(token: &str, expires_at: DateTime<Utc>) -> CreateManagedMcpClientConfigOutput {
    CreateManagedMcpClientConfigOutput {
        transport_kind: ManagedMcpTransportKind::Url,
        mcp_config_json: String::new(),
        proxy_url: Some("https://proxy.test/mcp".to_owned()),
        proxy_token: Some(token.to_owned()),
        authorization_header_name: Some("Authorization".to_owned()),
        authorization_header_value: Some(format!("Bearer {token}")),
        expires_at: Some(expires_at.into()),
        response_context: ResponseContext {
            server_version: None,
        },
    }
}

fn transient_error() -> anyhow::Error {
    GraphQLError::HttpError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        body: String::new(),
    }
    .into()
}

type MintResult = anyhow::Result<CreateManagedMcpClientConfigOutput>;

/// A client that answers each mint with the next of `results`, counting the mints it was asked
/// for. Asking for more than it was given fails the test.
fn client_answering(
    results: impl IntoIterator<Item = MintResult>,
) -> (MockManagedMcpClient, Arc<AtomicUsize>) {
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
                .expect("no further mint was expected")
        });
    (client, mints)
}

/// A client that never answers a mint, counting the mints it was asked for.
struct UnansweringClient {
    mints: Arc<AtomicUsize>,
}

#[async_trait]
impl ManagedMcpClient for UnansweringClient {
    async fn create_managed_mcp_client_config(&self, _uid: String) -> MintResult {
        self.mints.fetch_add(1, Ordering::SeqCst);
        future::pending().await
    }
}

/// A refresh loop for one lease, driven by a [`TestClock`].
struct Run {
    pool: LocalPool,
    clock: TestClock,
    headers: RefreshableHeaders,
    reject: RejectionHandler,
    authenticated: Rc<Cell<bool>>,
}

impl Run {
    fn start(
        clock: &TestClock,
        client: impl ManagedMcpClient,
        lease: ManagedMcpLease,
        must_outlive: Option<DateTime<Utc>>,
    ) -> Self {
        Self::start_authenticated(clock, client, lease, must_outlive, true)
    }

    fn start_authenticated(
        clock: &TestClock,
        client: impl ManagedMcpClient,
        lease: ManagedMcpLease,
        must_outlive: Option<DateTime<Utc>>,
        authenticated: bool,
    ) -> Self {
        let (sender, requests) = mpsc::unbounded();
        let running = Arc::new(AtomicBool::new(false));
        let reject = rejection_handler(lease.installation_id, sender, running.clone());
        let headers = RefreshableHeaders::new(
            HashMap::from([(
                lease.header_name.clone(),
                lease.credential.header_value.clone(),
            )]),
            Some(reject.clone()),
        )
        .expect("valid headers");
        let refresh = ManagedMcpRefresh {
            leases: vec![(lease, headers.clone())],
            requests,
            running,
        };
        let authenticated = Rc::new(Cell::new(authenticated));
        let is_authenticated = {
            let authenticated = authenticated.clone();
            move || {
                let authenticated = authenticated.get();
                async move { authenticated }
            }
        };
        let pool = LocalPool::new();
        pool.spawner()
            .spawn_local(refresh.run(
                Arc::new(client),
                clock.refresh_clock(),
                must_outlive,
                is_authenticated,
            ))
            .expect("spawn refresh loop");
        let mut run = Self {
            pool,
            clock: clock.clone(),
            headers,
            reject,
            authenticated,
        };
        run.pool.run_until_stalled();
        run
    }

    /// Lets `duration` pass the way an awake machine experiences it, with the loop running
    /// whenever one of its sleeps ends.
    fn advance(&mut self, duration: Duration) {
        let mut remaining = duration;
        while !remaining.is_zero() {
            let step = remaining.min(MAX_SLEEP);
            self.clock.advance(step);
            self.pool.run_until_stalled();
            remaining -= step;
        }
    }

    /// Tells the loop a server rejected the credential with `challenge`, and returns a flag
    /// that is set once the rejected request may be sent again.
    fn reject(&mut self, challenge: &str) -> Rc<Cell<bool>> {
        let answered = Rc::new(Cell::new(false));
        let handled = (self.reject)(challenge.to_owned());
        let flag = answered.clone();
        self.pool
            .spawner()
            .spawn_local(async move {
                handled.await;
                flag.set(true);
            })
            .expect("spawn rejection");
        self.pool.run_until_stalled();
        answered
    }

    fn authorization(&self) -> String {
        self.headers.current()["Authorization"].clone()
    }
}

#[test]
fn refresh_is_due_a_lead_time_before_expiry_and_no_earlier_than_allowed() {
    let now = TestClock::new().now();
    let expires_at = now + 3 * HOUR;
    let never = DateTime::<Utc>::MIN_UTC;

    assert_eq!(
        next_refresh_at(expires_at, REFRESH_LEAD_TIME, never),
        expires_at - REFRESH_LEAD_TIME
    );
    assert!(next_refresh_at(now + MINUTE, REFRESH_LEAD_TIME, never) <= now);
    assert!(next_refresh_at(now - HOUR, REFRESH_LEAD_TIME, never) <= now);
    assert_eq!(
        next_refresh_at(now + MINUTE, REFRESH_LEAD_TIME, now + 5 * MINUTE),
        now + 5 * MINUTE
    );
}

#[test]
fn retry_interval_doubles_up_to_the_cap() {
    assert_eq!(retry_interval(1, 0.0), MIN_ATTEMPT_INTERVAL);
    assert_eq!(retry_interval(2, 0.0), 2 * MIN_ATTEMPT_INTERVAL);
    assert_eq!(retry_interval(3, 0.0), 4 * MIN_ATTEMPT_INTERVAL);
    assert_eq!(retry_interval(4, 0.0), MAX_RETRY_INTERVAL);
    assert_eq!(retry_interval(u32::MAX, 0.0), MAX_RETRY_INTERVAL);
    assert_eq!(
        retry_interval(1, 0.5),
        MIN_ATTEMPT_INTERVAL + MIN_ATTEMPT_INTERVAL.mul_f32(RETRY_JITTER * 0.5)
    );
}

#[test]
fn only_expired_and_stale_proxy_rejections_ask_for_a_refresh() {
    assert!(is_refreshable_rejection(EXPIRED_CHALLENGE));
    assert!(is_refreshable_rejection(STALE_CHALLENGE));
    assert!(!is_refreshable_rejection(r#"Bearer error="invalid_token""#));
    assert!(!is_refreshable_rejection(
        r#"Bearer resource_metadata="https://example.com/.well-known/oauth-protected-resource""#
    ));
}

#[test]
fn only_url_configs_with_every_credential_field_are_refreshable() {
    let expires_at = TestClock::new().now() + 3 * HOUR;

    let (header_name, credential) =
        credential_from_client_config(&url_config("token", expires_at)).expect("credential");
    assert_eq!(header_name, "Authorization");
    assert_eq!(credential.header_value, "Bearer token");
    assert_eq!(credential.expires_at, expires_at);

    let command = CreateManagedMcpClientConfigOutput {
        transport_kind: ManagedMcpTransportKind::Command,
        ..url_config("token", expires_at)
    };
    assert!(credential_from_client_config(&command).is_none());

    let without_expiry = CreateManagedMcpClientConfigOutput {
        expires_at: None,
        ..url_config("token", expires_at)
    };
    assert!(credential_from_client_config(&without_expiry).is_none());

    let without_header = CreateManagedMcpClientConfigOutput {
        authorization_header_value: None,
        ..url_config("token", expires_at)
    };
    assert!(credential_from_client_config(&without_header).is_none());
}

#[test]
fn request_errors_are_classified_by_what_retrying_can_fix() {
    assert!(matches!(
        MintError::from_request_error(transient_error()),
        MintError::Transient(_)
    ));
    assert!(matches!(
        MintError::from_request_error(anyhow!(AUTH_REJECTED_MESSAGE)),
        MintError::Unauthenticated(_)
    ));
    assert!(matches!(
        MintError::from_request_error(
            GraphQLError::HttpError {
                status: StatusCode::UNAUTHORIZED,
                body: String::new(),
            }
            .into()
        ),
        MintError::Unauthenticated(_)
    ));
    assert!(matches!(
        MintError::from_request_error(
            ManagedMcpClientConfigRejected("integration is disconnected".to_owned()).into()
        ),
        MintError::Rejected(_)
    ));
    assert!(matches!(
        MintError::from_request_error(
            GraphQLError::HttpError {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                body: String::new(),
            }
            .into()
        ),
        MintError::Rejected(_)
    ));
    // A failure with no status behind it never got the server's verdict.
    assert!(matches!(
        MintError::from_request_error(anyhow!("failed to refresh the ID token")),
        MintError::Transient(_)
    ));
}

#[test]
fn debug_output_omits_the_credential() {
    let lease = lease(TestClock::new().now());
    assert!(!format!("{lease:?}").contains("Bearer first"));
}

#[test]
fn credential_is_replaced_shortly_before_it_expires() {
    let clock = TestClock::new();
    let expires_at = clock.now() + 3 * HOUR;
    let (client, mints) = client_answering([Ok(url_config("second", expires_at + 3 * HOUR))]);
    let mut run = Run::start(&clock, client, lease(expires_at), None);

    run.advance(3 * HOUR - REFRESH_LEAD_TIME - MINUTE);
    assert_eq!(mints.load(Ordering::SeqCst), 0);
    assert_eq!(run.authorization(), "Bearer first");

    run.advance(MINUTE);
    assert_eq!(mints.load(Ordering::SeqCst), 1);
    assert_eq!(run.authorization(), "Bearer second");

    // The replacement is good for another three hours, so nothing more is minted for now.
    run.advance(2 * HOUR);
    assert_eq!(mints.load(Ordering::SeqCst), 1);
}

#[test]
fn credential_already_close_to_expiry_is_replaced_at_once() {
    let clock = TestClock::new();
    let (client, mints) = client_answering([Ok(url_config("second", clock.now() + 3 * HOUR))]);
    let run = Run::start(&clock, client, lease(clock.now() + 5 * MINUTE), None);

    assert_eq!(mints.load(Ordering::SeqCst), 1);
    assert_eq!(run.authorization(), "Bearer second");
}

#[test]
fn credential_that_expired_during_a_suspend_is_replaced_on_resume() {
    let clock = TestClock::new();
    let expires_at = clock.now() + 3 * HOUR;
    let (client, mints) = client_answering([Ok(url_config("second", expires_at + 6 * HOUR))]);
    let mut run = Run::start(&clock, client, lease(expires_at), None);

    clock.suspend(4 * HOUR);
    run.pool.run_until_stalled();
    assert_eq!(mints.load(Ordering::SeqCst), 0);

    // The sleep that was interrupted by the suspend finishes within its own length.
    run.advance(MAX_SLEEP);
    assert_eq!(mints.load(Ordering::SeqCst), 1);
    assert_eq!(run.authorization(), "Bearer second");
}

#[test]
fn transient_failures_back_off_and_keep_the_current_credential() {
    let clock = TestClock::new();
    let (client, mints) = client_answering([
        Err(transient_error()),
        Err(transient_error()),
        Ok(url_config("second", clock.now() + 3 * HOUR)),
    ]);
    let mut run = Run::start(&clock, client, lease(clock.now() + 10 * MINUTE), None);
    assert_eq!(mints.load(Ordering::SeqCst), 1);
    assert_eq!(run.authorization(), "Bearer first");

    run.advance(MIN_ATTEMPT_INTERVAL - Duration::from_secs(1));
    assert_eq!(mints.load(Ordering::SeqCst), 1);
    run.advance(Duration::from_secs(1));
    assert_eq!(mints.load(Ordering::SeqCst), 2);
    assert_eq!(run.authorization(), "Bearer first");

    run.advance(2 * MIN_ATTEMPT_INTERVAL - Duration::from_secs(1));
    assert_eq!(mints.load(Ordering::SeqCst), 2);
    run.advance(Duration::from_secs(1));
    assert_eq!(mints.load(Ordering::SeqCst), 3);
    assert_eq!(run.authorization(), "Bearer second");
}

#[test]
fn nothing_is_minted_while_the_process_is_not_authenticated() {
    let clock = TestClock::new();
    let (client, mints) = client_answering([Ok(url_config("second", clock.now() + 3 * HOUR))]);
    let mut run = Run::start_authenticated(
        &clock,
        client,
        lease(clock.now() + 10 * MINUTE),
        None,
        false,
    );
    assert_eq!(mints.load(Ordering::SeqCst), 0);

    run.authenticated.set(true);
    run.advance(MAX_RETRY_INTERVAL - Duration::from_secs(1));
    assert_eq!(mints.load(Ordering::SeqCst), 0);
    run.advance(Duration::from_secs(1));
    assert_eq!(mints.load(Ordering::SeqCst), 1);
    assert_eq!(run.authorization(), "Bearer second");
}

#[test]
fn rejected_process_credentials_are_retried_only_after_a_long_wait() {
    let clock = TestClock::new();
    let (client, mints) = client_answering([
        Err(anyhow!(AUTH_REJECTED_MESSAGE)),
        Ok(url_config("second", clock.now() + 3 * HOUR)),
    ]);
    let mut run = Run::start(&clock, client, lease(clock.now() + 10 * MINUTE), None);
    assert_eq!(mints.load(Ordering::SeqCst), 1);

    run.advance(MAX_RETRY_INTERVAL - Duration::from_secs(1));
    assert_eq!(mints.load(Ordering::SeqCst), 1);
    assert_eq!(run.authorization(), "Bearer first");
    run.advance(Duration::from_secs(1));
    assert_eq!(mints.load(Ordering::SeqCst), 2);
    assert_eq!(run.authorization(), "Bearer second");
}

/// A run whose credential is anchored to its own deadline gets the same expiry back from every
/// mint, so asking again ahead of that expiry would never stop.
#[test]
fn credential_the_server_will_not_extend_is_applied_but_not_requested_again() {
    let clock = TestClock::new();
    let expires_at = clock.now() + 10 * MINUTE;
    let (client, mints) = client_answering([Ok(url_config("second", expires_at))]);
    let mut run = Run::start(&clock, client, lease(expires_at), None);
    assert_eq!(mints.load(Ordering::SeqCst), 1);
    assert_eq!(run.authorization(), "Bearer second");

    run.advance(HOUR);
    assert_eq!(mints.load(Ordering::SeqCst), 1);
}

/// Two mints for the same deadline can differ by a fraction of the lifetime, so an expiry that
/// is merely later does not show the server extending the credential.
#[test]
fn credential_that_is_due_as_soon_as_it_arrives_is_not_requested_again() {
    let clock = TestClock::new();
    let expires_at = clock.now() + 10 * MINUTE;
    let (client, mints) = client_answering([Ok(url_config(
        "second",
        expires_at + Duration::from_secs(1),
    ))]);
    let mut run = Run::start(&clock, client, lease(expires_at), None);
    assert_eq!(mints.load(Ordering::SeqCst), 1);
    assert_eq!(run.authorization(), "Bearer second");

    run.advance(HOUR);
    assert_eq!(mints.load(Ordering::SeqCst), 1);
}

/// With the local clock ahead of the server's, every credential looks expired when it arrives.
#[test]
fn credentials_that_look_expired_on_arrival_are_replaced_only_when_rejected() {
    let clock = TestClock::new();
    let (client, mints) = client_answering([
        Ok(url_config("second", clock.now() - 9 * MINUTE)),
        Ok(url_config("third", clock.now() - 4 * MINUTE)),
    ]);
    let mut run = Run::start(&clock, client, lease(clock.now() - 10 * MINUTE), None);
    assert_eq!(mints.load(Ordering::SeqCst), 1);
    assert_eq!(run.authorization(), "Bearer second");

    run.advance(5 * MINUTE);
    assert_eq!(mints.load(Ordering::SeqCst), 1);

    let answered = run.reject(EXPIRED_CHALLENGE);
    assert!(answered.get());
    assert_eq!(mints.load(Ordering::SeqCst), 2);
    assert_eq!(run.authorization(), "Bearer third");
}

#[test]
fn credential_that_outlives_the_run_is_not_replaced_ahead_of_expiry() {
    let clock = TestClock::new();
    let (client, mints) = client_answering([]);
    let mut run = Run::start(
        &clock,
        client,
        lease(clock.now() + 10 * MINUTE),
        Some(clock.now() + 5 * MINUTE),
    );

    run.advance(HOUR);
    assert_eq!(mints.load(Ordering::SeqCst), 0);
    assert_eq!(run.authorization(), "Bearer first");
}

#[test]
fn rejected_credential_is_replaced_on_request() {
    let clock = TestClock::new();
    let expires_at = clock.now() + 3 * HOUR;
    let (client, mints) = client_answering([Ok(url_config("second", expires_at + HOUR))]);
    let mut run = Run::start(&clock, client, lease(expires_at), None);

    let answered = run.reject(STALE_CHALLENGE);

    assert!(answered.get());
    assert_eq!(mints.load(Ordering::SeqCst), 1);
    assert_eq!(run.authorization(), "Bearer second");
}

/// A credential that outlives the run is never replaced ahead of time, but a server that
/// rejects it still gets a new one.
#[test]
fn rejected_credential_is_replaced_even_when_it_outlives_the_run() {
    let clock = TestClock::new();
    let expires_at = clock.now() + 3 * HOUR;
    let (client, mints) = client_answering([Ok(url_config("second", expires_at))]);
    let mut run = Run::start(&clock, client, lease(expires_at), Some(clock.now() + HOUR));

    let answered = run.reject(EXPIRED_CHALLENGE);

    assert!(answered.get());
    assert_eq!(mints.load(Ordering::SeqCst), 1);
    assert_eq!(run.authorization(), "Bearer second");
}

#[test]
fn rejections_close_together_share_one_mint() {
    let clock = TestClock::new();
    let expires_at = clock.now() + 3 * HOUR;
    let (client, mints) = client_answering([Ok(url_config("second", expires_at + HOUR))]);
    let mut run = Run::start(&clock, client, lease(expires_at), None);

    let first = run.reject(EXPIRED_CHALLENGE);
    let second = run.reject(EXPIRED_CHALLENGE);

    assert!(first.get());
    assert!(second.get());
    assert_eq!(mints.load(Ordering::SeqCst), 1);
    assert_eq!(run.authorization(), "Bearer second");
}

/// The mint that follows a resume can fail before the network is back, which says nothing about
/// the one after it.
#[test]
fn mint_that_fails_before_reaching_the_server_is_tried_again_soon() {
    let clock = TestClock::new();
    let (client, mints) = client_answering([
        Err(anyhow!("network is unreachable")),
        Ok(url_config("second", clock.now() + 3 * HOUR)),
    ]);
    let mut run = Run::start(&clock, client, lease(clock.now() + 3 * HOUR), None);

    let first = run.reject(EXPIRED_CHALLENGE);
    assert!(first.get());
    assert_eq!(mints.load(Ordering::SeqCst), 1);
    assert_eq!(run.authorization(), "Bearer first");

    run.advance(MIN_ATTEMPT_INTERVAL);
    let second = run.reject(EXPIRED_CHALLENGE);
    assert!(second.get());
    assert_eq!(mints.load(Ordering::SeqCst), 2);
    assert_eq!(run.authorization(), "Bearer second");
}

#[test]
fn mint_that_is_never_answered_is_given_up_on() {
    let clock = TestClock::new();
    let mints = Arc::new(AtomicUsize::new(0));
    let client = UnansweringClient {
        mints: mints.clone(),
    };
    let mut run = Run::start(&clock, client, lease(clock.now() + 3 * HOUR), None);

    let first = run.reject(EXPIRED_CHALLENGE);
    assert_eq!(mints.load(Ordering::SeqCst), 1);
    assert!(!first.get());

    run.advance(MINT_TIMEOUT);
    assert!(first.get());
    assert_eq!(run.authorization(), "Bearer first");

    // The loop is serving requests again, and holds off the next mint like after any failure.
    let second = run.reject(EXPIRED_CHALLENGE);
    assert!(second.get());
    assert_eq!(mints.load(Ordering::SeqCst), 1);

    run.advance(MIN_ATTEMPT_INTERVAL);
    run.reject(EXPIRED_CHALLENGE);
    assert_eq!(mints.load(Ordering::SeqCst), 2);
}

/// Servers connect before the loop starts, when a request for a refresh would go unanswered.
#[test]
fn rejection_before_the_loop_runs_is_not_held_back() {
    let (sender, mut requests) = mpsc::unbounded();
    let reject = rejection_handler(Uuid::from_u128(1), sender, Arc::new(AtomicBool::new(false)));
    let mut pool = LocalPool::new();
    let answered = Rc::new(Cell::new(false));
    let handled = reject(EXPIRED_CHALLENGE.to_owned());
    let flag = answered.clone();
    pool.spawner()
        .spawn_local(async move {
            handled.await;
            flag.set(true);
        })
        .expect("spawn rejection");
    pool.run_until_stalled();
    assert!(answered.get());

    drop(reject);
    assert!(pool.run_until(requests.next()).is_none());
}

#[test]
fn rejection_that_a_new_credential_cannot_fix_is_not_acted_on() {
    let clock = TestClock::new();
    let (client, mints) = client_answering([]);
    let mut run = Run::start(&clock, client, lease(clock.now() + 3 * HOUR), None);

    let answered = run.reject(r#"Bearer error="invalid_token""#);

    assert!(answered.get());
    assert_eq!(mints.load(Ordering::SeqCst), 0);
    assert_eq!(run.authorization(), "Bearer first");
}

#[test]
fn leases_with_unusable_headers_are_left_out() {
    let expires_at = TestClock::new().now() + 3 * HOUR;
    let unusable = ManagedMcpLease {
        installation_id: Uuid::from_u128(2),
        header_name: "Not A Header".to_owned(),
        ..lease(expires_at)
    };

    let refresh =
        ManagedMcpRefresh::new([lease(expires_at), unusable.clone()]).expect("one usable lease");
    let headers = refresh.headers();
    assert_eq!(headers.len(), 1);
    assert_eq!(
        headers[&Uuid::from_u128(1)].current(),
        HashMap::from([("Authorization".to_owned(), "Bearer first".to_owned())])
    );

    assert!(ManagedMcpRefresh::new([unusable]).is_none());
    assert!(ManagedMcpRefresh::new([]).is_none());
}
