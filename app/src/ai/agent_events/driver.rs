use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures::StreamExt;
use futures::future::Either;
use instant::Instant;
use warp_errors::{AnyhowErrorExt as _, report_error};
use warpui::r#async::Timer;

use crate::server::retry_strategies::{is_auth_error, is_transient_http_error};
use crate::server::server_api::ServerApi;
use crate::server::server_api::ai::AgentRunEvent;
use crate::server::server_api::presigned_upload::HttpStatusError;

pub(crate) const DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS: &[u64] = &[1, 2, 5, 10];
pub(crate) const DEFAULT_PERMANENT_ERROR_BACKOFF_STEPS: &[u64] = &[30];
pub(crate) const DEFAULT_AGENT_EVENT_PROACTIVE_RECONNECT: Duration = Duration::from_secs(14 * 60);
pub(crate) const DEFAULT_AGENT_EVENT_FAILURES_BEFORE_ERROR_LOG: usize = 5;

/// Consecutive authentication failures (HTTP 401/403) after which a bounded
/// listener gives up instead of reconnecting. A small threshold (rather than 1)
/// tolerates a one-off token blip while still bailing quickly once credentials
/// are permanently invalid.
// Only consumed by the (native-only) cloud-agent message bridge via
// `bounded_run_ids`, so it is dead code on the wasm target.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) const DEFAULT_AUTH_ERROR_GIVE_UP_FAILURES: usize = 3;

/// Total wall-clock time a bounded listener keeps retrying, measured from the
/// first failure since the last successful open/event, before it gives up. Set
/// larger than the proactive reconnect window so healthy streams are never
/// affected; it only bounds sustained failure.
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) const DEFAULT_AGENT_EVENT_MAX_RETRY_DURATION: Duration = Duration::from_secs(30 * 60);

/// Selects which server-side filter shape an [`AgentEventSource`] should use
/// when opening a stream.
///
/// `RunIds` maps to the `?run_ids[]=` query parameter on the SSE endpoint
/// and is used by child-only per-conversation streams and the dormant
/// Claude wake listener. `AncestorRunId` maps to the `?ancestor_run_id=`
/// shape: with `include_self=false` it streams events for every direct
/// child of the supplied parent run (the shared-session viewer's pill bar),
/// and with `include_self=true` it additionally streams the parent run's
/// own events so an owner-side orchestrator can receive child lifecycle
/// events plus its own inbox on one ordered stream.
#[derive(Clone, Debug)]
pub(crate) enum AgentEventFilter {
    /// One stream per multiplexed set of run IDs. Matches today's
    /// `?run_ids[]=` endpoint.
    RunIds(Vec<String>),
    /// Stream events for every direct child of the supplied parent run, and
    /// (when `include_self` is true) the parent run itself. Matches the
    /// `?ancestor_run_id=` endpoint.
    AncestorRunId {
        ancestor_run_id: String,
        include_self: bool,
    },
}

impl AgentEventFilter {
    /// Returns a short debug label used in driver log lines so we don't have
    /// to format the full `Vec<String>` payload on every retry.
    pub(crate) fn log_label(&self) -> String {
        match self {
            AgentEventFilter::RunIds(ids) => format!("run_ids={ids:?}"),
            AgentEventFilter::AncestorRunId {
                ancestor_run_id,
                include_self,
            } => format!("ancestor_run_id={ancestor_run_id} include_self={include_self}"),
        }
    }
}

/// Configuration for the shared agent-event stream driver.
#[derive(Clone, Debug)]
pub(crate) struct AgentEventDriverConfig {
    /// Wire-level filter selecting which run IDs the stream serves. Either a
    /// concrete multiplexed list of run IDs or an ancestor-scoped child set;
    /// see [`AgentEventFilter`].
    pub filter: AgentEventFilter,
    /// Last fully handled event sequence. Events at or below this cursor are
    /// ignored on reconnect so the consumer only sees new work.
    pub since_sequence: i64,
    /// Exponential-ish reconnect delays, in seconds, used after transient
    /// stream failures (5xx, timeouts, connection resets).
    pub reconnect_backoff_steps: &'static [u64],
    /// Reconnect delays for permanent HTTP errors (4xx other than 408/429).
    /// Typically much slower than transient backoff to reduce log spam while
    /// still allowing recovery if the error was spurious.
    pub permanent_error_backoff_steps: &'static [u64],
    /// Optional deadline for proactively recycling an otherwise healthy stream
    /// before upstream infrastructure times it out (for example, before Cloud
    /// Run's 20-minute streaming timeout).
    pub proactive_reconnect_after: Option<Duration>,
    /// Failure count at which actionable reconnect failures are reported at Error level.
    /// This only affects log severity; retry behavior stays the same.
    pub failures_before_error_log: usize,
    /// Consecutive authentication failures (HTTP 401/403) after which the driver
    /// stops and returns an error instead of reconnecting. `None` keeps retrying
    /// through auth errors forever (the default for local, interactive
    /// listeners whose credentials refresh in the background).
    pub auth_error_give_up_failures: Option<usize>,
    /// Total wall-clock time the driver keeps retrying after failures begin
    /// (measured from the first failure since the last successful open/event)
    /// before it stops and returns an error. `None` retries without a time
    /// limit.
    pub max_retry_duration: Option<Duration>,
}

impl AgentEventDriverConfig {
    /// Build the production reconnecting configuration used by long-lived
    /// orchestration and harness listeners, parameterised on the wire filter.
    pub(crate) fn retry_forever(filter: AgentEventFilter, since_sequence: i64) -> Self {
        Self {
            filter,
            since_sequence,
            reconnect_backoff_steps: DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS,
            permanent_error_backoff_steps: DEFAULT_PERMANENT_ERROR_BACKOFF_STEPS,
            proactive_reconnect_after: Some(DEFAULT_AGENT_EVENT_PROACTIVE_RECONNECT),
            failures_before_error_log: DEFAULT_AGENT_EVENT_FAILURES_BEFORE_ERROR_LOG,
            auth_error_give_up_failures: None,
            max_retry_duration: None,
        }
    }

    /// Convenience: build a `retry_forever` config from a concrete list of
    /// run IDs. Lets existing call sites keep their current ergonomics.
    pub(crate) fn retry_forever_run_ids(run_ids: Vec<String>, since_sequence: i64) -> Self {
        Self::retry_forever(AgentEventFilter::RunIds(run_ids), since_sequence)
    }

    /// Build a reconnecting config for a cloud-agent listener that must NOT run
    /// forever. Unlike [`retry_forever`], this stops after sustained
    /// authentication failures or a bounded total retry window, so a listener
    /// whose task has ended (and whose credentials are now permanently
    /// rejected) shuts down instead of reconnecting indefinitely.
    // Only called by the (native-only) cloud-agent message bridge, so it is dead
    // code on the wasm target.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub(crate) fn bounded_run_ids(run_ids: Vec<String>, since_sequence: i64) -> Self {
        Self {
            auth_error_give_up_failures: Some(DEFAULT_AUTH_ERROR_GIVE_UP_FAILURES),
            max_retry_duration: Some(DEFAULT_AGENT_EVENT_MAX_RETRY_DURATION),
            ..Self::retry_forever(AgentEventFilter::RunIds(run_ids), since_sequence)
        }
    }
}

/// Tells the shared driver whether to continue or stop after a handled event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentEventConsumerControlFlow {
    Continue,
    #[cfg_attr(not(test), allow(dead_code))]
    Stop,
}

/// High-level connection state updates emitted by the shared driver.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AgentEventDriverState {
    Connected,
    RetryScheduled {
        /// Number of consecutive failed reconnect cycles since the last
        /// successful stream open or event delivery.
        consecutive_failures: usize,
        /// Delay before the next reconnect attempt.
        backoff: Duration,
        /// Whether the retry happened before the stream ever connected.
        is_initial_connect: bool,
    },
    /// A healthy stream was intentionally recycled after
    /// `proactive_reconnect_after`.
    ProactiveReconnect,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentMessageEventMetadata {
    pub sequence: i64,
    pub message_id: String,
    pub occurred_at: String,
}

impl AgentMessageEventMetadata {
    pub(crate) fn from_event(event: &AgentRunEvent) -> Option<Self> {
        if event.event_type != "new_message" {
            return None;
        }

        Some(Self {
            sequence: event.sequence,
            message_id: event.ref_id.clone()?,
            occurred_at: event.occurred_at.clone(),
        })
    }
}

/// Parsed items emitted by an [`AgentEventSource`].
pub(crate) enum AgentEventSourceItem {
    Open,
    Event(AgentRunEvent),
}

cfg_if::cfg_if! {
    if #[cfg(target_family = "wasm")] {
        type AgentEventSourceStream =
            futures::stream::LocalBoxStream<'static, Result<AgentEventSourceItem>>;
    } else {
        type AgentEventSourceStream =
            futures::stream::BoxStream<'static, Result<AgentEventSourceItem>>;
    }
}

/// Opens a stream of parsed agent events for the supplied filter.
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
pub(crate) trait AgentEventSource: Send + Sync {
    async fn open_stream(
        &self,
        filter: &AgentEventFilter,
        since_sequence: i64,
    ) -> Result<AgentEventSourceStream>;
}

/// [`AgentEventSource`] backed by [`ServerApi::stream_agent_events`].
pub(crate) struct ServerApiAgentEventSource {
    server_api: Arc<ServerApi>,
}

impl ServerApiAgentEventSource {
    pub(crate) fn new(server_api: Arc<ServerApi>) -> Self {
        Self { server_api }
    }
}

#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
impl AgentEventSource for ServerApiAgentEventSource {
    async fn open_stream(
        &self,
        filter: &AgentEventFilter,
        since_sequence: i64,
    ) -> Result<AgentEventSourceStream> {
        let stream = match filter {
            AgentEventFilter::RunIds(run_ids) => {
                self.server_api
                    .stream_agent_events(run_ids, since_sequence)
                    .await?
            }
            AgentEventFilter::AncestorRunId {
                ancestor_run_id,
                include_self,
            } => {
                self.server_api
                    .stream_agent_events_for_ancestor(
                        ancestor_run_id,
                        *include_self,
                        since_sequence,
                    )
                    .await?
            }
        };

        let stream = stream.filter_map(|event_result| async move {
            match event_result {
                Ok(reqwest_eventsource::Event::Open) => Some(Ok(AgentEventSourceItem::Open)),
                Ok(reqwest_eventsource::Event::Message(message)) => {
                    match serde_json::from_str::<AgentRunEvent>(&message.data) {
                        Ok(event) => Some(Ok(AgentEventSourceItem::Event(event))),
                        Err(err) => {
                            log::warn!("Skipping malformed agent event from SSE stream: {err}");
                            None
                        }
                    }
                }
                Err(err) => {
                    let anyhow_err = match err {
                        reqwest_eventsource::Error::InvalidStatusCode(status_code, response) => {
                            let body = response
                                .text()
                                .await
                                .unwrap_or_else(|err| format!("(no response body: {err:#})"));
                            let status_err = HttpStatusError {
                                status: status_code.as_u16(),
                                body: body.clone(),
                            };
                            anyhow::Error::new(status_err).context(format!(
                                "SSE stream error: invalid status code {status_code}: {body}"
                            ))
                        }
                        #[cfg(not(target_family = "wasm"))]
                        reqwest_eventsource::Error::Transport(err) => {
                            anyhow::Error::new(err).context("SSE stream error")
                        }
                        err => anyhow!("SSE stream error: {err:?}"),
                    };
                    Some(Err(anyhow_err))
                }
            }
        });

        cfg_if::cfg_if! {
            if #[cfg(target_family = "wasm")] {
                Ok(stream.boxed_local())
            } else {
                Ok(stream.boxed())
            }
        }
    }
}

/// Consumes events produced by [`run_agent_event_driver`].
///
/// Errors from [`on_event`](Self::on_event) are fatal because the event could not
/// be safely processed. [`persist_cursor`](Self::persist_cursor) and
/// [`on_driver_state`](Self::on_driver_state) are treated as best-effort hooks:
/// their errors are logged and the driver continues.
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
pub(crate) trait AgentEventConsumer: Send {
    async fn on_event(&mut self, event: AgentRunEvent) -> Result<AgentEventConsumerControlFlow>;

    async fn persist_cursor(&mut self, _sequence: i64) -> Result<()> {
        Ok(())
    }

    async fn on_driver_state(&mut self, _state: AgentEventDriverState) -> Result<()> {
        Ok(())
    }
}

/// Runs a reconnecting agent-event stream until the consumer stops it or a
/// fatal event-processing error occurs.
pub(crate) async fn run_agent_event_driver<S, C>(
    source: S,
    config: AgentEventDriverConfig,
    consumer: &mut C,
) -> Result<()>
where
    S: AgentEventSource,
    C: AgentEventConsumer,
{
    let mut since_sequence = config.since_sequence;
    let mut failures = 0usize;
    // Consecutive authentication failures (HTTP 401/403), tracked separately from
    // `failures` so the auth give-up threshold only counts an uninterrupted run
    // of auth failures. Any non-auth failure or success resets it, so a mix like
    // 500, 500, 401 does not trip a "3 consecutive auth failures" policy.
    let mut consecutive_auth_failures = 0usize;
    let mut has_connected_once = false;
    // Start of the current run of consecutive failures. Reset to `None` after any
    // successful open/event so the `max_retry_duration` window only measures
    // sustained failure, not total stream lifetime.
    let mut retry_window_started_at: Option<Instant> = None;

    loop {
        // `open_stream` is lazy for the SSE-backed source: the TCP
        // connect happens when the stream is first polled, not when
        // this returns Ok. Wait for the `AgentEventSourceItem::Open`
        // event below before declaring connectivity, so a server
        // outage doesn't reset `failures` between every retry.
        let mut stream = match source.open_stream(&config.filter, since_sequence).await {
            Ok(stream) => stream,
            Err(err) => {
                failures += 1;
                handle_http_error(
                    &config,
                    consumer,
                    err,
                    failures,
                    &mut consecutive_auth_failures,
                    &mut retry_window_started_at,
                    !has_connected_once,
                )
                .await?;
                continue;
            }
        };

        let proactive_reconnect_deadline = config
            .proactive_reconnect_after
            .map(|duration| Instant::now() + duration);

        loop {
            let next_item = if let Some(deadline) = proactive_reconnect_deadline {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    NextDriverItem::ProactiveReconnect
                } else {
                    let next_stream_item = stream.next();
                    let reconnect_timer = Timer::after(remaining);
                    futures::pin_mut!(next_stream_item);
                    futures::pin_mut!(reconnect_timer);
                    match futures::future::select(next_stream_item, reconnect_timer).await {
                        Either::Left((stream_item, _)) => NextDriverItem::StreamItem(stream_item),
                        Either::Right(_) => NextDriverItem::ProactiveReconnect,
                    }
                }
            } else {
                NextDriverItem::StreamItem(stream.next().await)
            };

            match next_item {
                NextDriverItem::ProactiveReconnect => {
                    notify_driver_state(consumer, AgentEventDriverState::ProactiveReconnect).await;
                    break;
                }
                NextDriverItem::StreamItem(Some(Ok(AgentEventSourceItem::Open))) => {
                    failures = 0;
                    consecutive_auth_failures = 0;
                    retry_window_started_at = None;
                    has_connected_once = true;
                    notify_driver_state(consumer, AgentEventDriverState::Connected).await;
                    log::info!(
                        "Agent event stream opened for {}",
                        config.filter.log_label()
                    );
                }
                NextDriverItem::StreamItem(Some(Ok(AgentEventSourceItem::Event(event)))) => {
                    failures = 0;
                    consecutive_auth_failures = 0;
                    retry_window_started_at = None;
                    if event.sequence <= since_sequence {
                        continue;
                    }

                    let event_sequence = event.sequence;
                    let control_flow = consumer.on_event(event).await?;
                    since_sequence = event_sequence;

                    if let Err(err) = consumer.persist_cursor(since_sequence).await {
                        log::warn!(
                            "Ignoring agent event cursor persistence failure at sequence {since_sequence}: {err:#}"
                        );
                    }

                    if matches!(control_flow, AgentEventConsumerControlFlow::Stop) {
                        return Ok(());
                    }
                }
                NextDriverItem::StreamItem(Some(Err(err))) => {
                    failures += 1;
                    handle_http_error(
                        &config,
                        consumer,
                        err,
                        failures,
                        &mut consecutive_auth_failures,
                        &mut retry_window_started_at,
                        false,
                    )
                    .await?;
                    break;
                }
                // Clean stream closure (server-side close, not an HTTP
                // error) — always use the transient backoff schedule since
                // there is no HTTP status to classify.
                NextDriverItem::StreamItem(None) => {
                    failures += 1;
                    // A clean server-side close carries no HTTP status, so it is
                    // never treated as an auth failure; reset the auth streak and
                    // let only the time-based backstop trigger a give-up here.
                    consecutive_auth_failures = 0;
                    if let Some(reason) = agent_event_give_up_reason(
                        &config,
                        consecutive_auth_failures,
                        &mut retry_window_started_at,
                    ) {
                        return Err(anyhow!(
                            "Agent event driver {reason} for {} after stream closed",
                            config.filter.log_label()
                        ));
                    }
                    let backoff = agent_event_backoff(failures, config.reconnect_backoff_steps);
                    log::warn!(
                        "Agent event stream closed for {}, reconnecting in {backoff:?}",
                        config.filter.log_label()
                    );
                    notify_driver_state(
                        consumer,
                        AgentEventDriverState::RetryScheduled {
                            consecutive_failures: failures,
                            backoff,
                            is_initial_connect: false,
                        },
                    )
                    .await;
                    Timer::after(backoff).await;
                    break;
                }
            }
        }
    }
}

enum NextDriverItem {
    StreamItem(Option<Result<AgentEventSourceItem>>),
    ProactiveReconnect,
}

/// Update the consecutive auth-failure streak for `err` and return a give-up
/// reason if the driver should stop retrying. Bumps `consecutive_auth_failures`
/// on an HTTP 401/403 error and resets it otherwise, then defers to
/// [`agent_event_give_up_reason`].
fn classify_failure_and_give_up_reason(
    config: &AgentEventDriverConfig,
    err: &anyhow::Error,
    consecutive_auth_failures: &mut usize,
    retry_window_started_at: &mut Option<Instant>,
) -> Option<String> {
    if is_auth_error(err) {
        *consecutive_auth_failures += 1;
    } else {
        *consecutive_auth_failures = 0;
    }
    agent_event_give_up_reason(config, *consecutive_auth_failures, retry_window_started_at)
}

/// Handles an HTTP error from either `open_stream` or a stream item: checks the
/// give-up policy, selects a backoff schedule, logs, notifies the consumer of the
/// pending retry, and waits out the backoff delay. Returns `Err` if the driver
/// should stop — the error is ready to propagate directly — or `Ok(())` once the
/// retry delay has elapsed.
async fn handle_http_error<C: AgentEventConsumer>(
    config: &AgentEventDriverConfig,
    consumer: &mut C,
    err: anyhow::Error,
    failures: usize,
    consecutive_auth_failures: &mut usize,
    retry_window_started_at: &mut Option<Instant>,
    is_initial_connect: bool,
) -> Result<()> {
    if let Some(reason) = classify_failure_and_give_up_reason(
        config,
        &err,
        consecutive_auth_failures,
        retry_window_started_at,
    ) {
        return Err(err.context(format!(
            "Agent event driver {reason} for {}",
            config.filter.log_label()
        )));
    }
    let backoff_steps = if is_transient_http_error(&err) {
        config.reconnect_backoff_steps
    } else {
        config.permanent_error_backoff_steps
    };
    let backoff = agent_event_backoff(failures, backoff_steps);
    log_stream_failure(
        &config.filter,
        failures,
        backoff,
        &err,
        config.failures_before_error_log,
    );
    notify_driver_state(
        consumer,
        AgentEventDriverState::RetryScheduled {
            consecutive_failures: failures,
            backoff,
            is_initial_connect,
        },
    )
    .await;
    Timer::after(backoff).await;
    Ok(())
}

/// Decide whether a bounded driver should stop retrying after a failure.
///
/// Returns `Some(reason)` (a short human-readable explanation for logs) when the
/// driver should give up, or `None` to keep retrying. `consecutive_auth_failures`
/// is the length of the current uninterrupted run of HTTP 401/403 failures (the
/// caller resets it on any non-auth failure or success), and
/// `retry_window_started_at` is seeded on the first call of a failure run so the
/// `max_retry_duration` window measures sustained failure.
fn agent_event_give_up_reason(
    config: &AgentEventDriverConfig,
    consecutive_auth_failures: usize,
    retry_window_started_at: &mut Option<Instant>,
) -> Option<String> {
    let now = Instant::now();
    let window_start = *retry_window_started_at.get_or_insert(now);

    if let Some(threshold) = config.auth_error_give_up_failures
        && consecutive_auth_failures >= threshold
    {
        return Some(format!(
            "stopping after {consecutive_auth_failures} consecutive authentication failures"
        ));
    }

    if let Some(max) = config.max_retry_duration {
        let elapsed = now.saturating_duration_since(window_start);
        if elapsed >= max {
            return Some(format!(
                "stopping after retrying for {elapsed:?} (max_retry_duration={max:?})"
            ));
        }
    }

    None
}

async fn notify_driver_state<C: AgentEventConsumer>(
    consumer: &mut C,
    state: AgentEventDriverState,
) {
    if let Err(err) = consumer.on_driver_state(state.clone()).await {
        log::warn!("Ignoring agent event driver state callback error for {state:?}: {err:#}");
    }
}

fn log_stream_failure(
    filter: &AgentEventFilter,
    failures: usize,
    backoff: Duration,
    err: &anyhow::Error,
    failures_before_error_log: usize,
) {
    let label = filter.log_label();
    if agent_event_failure_should_log_error(err, failures, failures_before_error_log) {
        report_error!(
            err,
            extra: {
                "failures" => %failures,
                "label" => %label,
                "backoff" => ?backoff
            }
        );
    } else {
        log::warn!(
            "Agent event stream failed {failures} consecutive times for {label}, retrying in {backoff:?}: {err:#}"
        );
    }
}

pub(crate) fn agent_event_backoff(failures: usize, backoff_steps: &[u64]) -> Duration {
    let safe_steps = if backoff_steps.is_empty() {
        DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS
    } else {
        backoff_steps
    };
    let index = failures.saturating_sub(1).min(safe_steps.len() - 1);
    Duration::from_secs(safe_steps[index])
}

#[cfg(test)]
pub(crate) fn agent_event_failures_exceeded_threshold(failures: usize, threshold: usize) -> bool {
    failures >= threshold
}

pub(crate) fn agent_event_failure_should_log_error(
    err: &anyhow::Error,
    failures: usize,
    threshold: usize,
) -> bool {
    threshold > 0 && failures == threshold && err.is_actionable()
}
