use async_broadcast::{InactiveReceiver, Sender as BroadcastSender};
use futures::channel::mpsc::{self, Receiver as MpscReceiver, Sender as MpscSender};
use futures::channel::oneshot::{self, Receiver, Sender};
use futures::future::{AbortHandle, Abortable};
use futures::StreamExt;
use instant::Instant;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use warpui::r#async::executor::Background;

use anyhow::Result;
use warpui::{r#async::Timer, Entity, RetryOption, SingletonEntity};

const DEFAULT_BUFFER_SIZE: usize = 1024;
const DEFAULT_SYNC_RETRY_STRATEGY: RetryOption = RetryOption::exponential(
    Duration::from_millis(500), /* initial interval */
    2.0,                        /* exponential factor */
    3,                          /* max retry count */
)
.with_jitter(0.2 /* max_jitter_percentage */);

/// An opaque identifier for a task in the sync queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TaskId(u64);

impl TaskId {
    /// Constructs a new globally-unique task ID.
    #[allow(clippy::new_without_default)]
    fn new() -> TaskId {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let raw = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        TaskId(raw)
    }
}

/// Trait for errors that can be classified as transient.
pub trait IsTransientError {
    fn is_transient(&self) -> bool;
}

/// Trait for any task that can be enqueued in the sync queue.
pub trait SyncQueueTaskTrait: Send + 'static {
    /// Error type for the task (if it fails). It needs to derive IsTransientError
    /// to decide the retry logic.
    type Error: std::error::Error + Send + Sync + IsTransientError + 'static;

    /// Result type for the task (if it succeeds).
    type Result: Send + Sync;

    /// The future should return a result of Self::Result or Self::Error. Note that
    /// we can only implement Send on non-wasm platforms.
    #[cfg(not(target_arch = "wasm32"))]
    type Fut: Future<Output = Result<Self::Result, Self::Error>> + Send;
    #[cfg(target_arch = "wasm32")]
    type Fut: Future<Output = Result<Self::Result, Self::Error>>;

    /// Implementation for running the task.
    fn run(&mut self) -> Self::Fut;
}

/// The operational mode of a [`SyncQueue`], specified at construction time.
enum SyncQueueMode<T: SyncQueueTaskTrait> {
    /// Each caller receives results via a per-task oneshot channel
    /// (returned by [`SyncQueue::enqueue_with_result`]).
    PerTask,
    /// Results are broadcast to all subscribers via an
    /// [`async_broadcast`] channel (obtained from [`SyncQueue::subscribe`]).
    Streaming {
        /// Keeps the broadcast channel alive even when no active receivers exist.
        /// Without this, dropping the initial receiver from `async_broadcast::broadcast()`
        /// would permanently close the channel. New subscribers are created via
        /// [`InactiveReceiver::activate_cloned`].
        _keepalive: InactiveReceiver<BroadcastResult<T>>,
    },
}

/// The result type broadcast in streaming mode.
pub type BroadcastResult<T> =
    Result<Arc<<T as SyncQueueTaskTrait>::Result>, Arc<<T as SyncQueueTaskTrait>::Error>>;

/// Broadcast receiver for streaming mode results.
pub type BroadcastReceiver<T> = async_broadcast::Receiver<BroadcastResult<T>>;

/// A queued task, with metadata and retry options.
struct QueuedTask<T: SyncQueueTaskTrait> {
    task: T,
    retry_options: RetryOption,
    result_sender: Option<Sender<Result<T::Result, T::Error>>>,
    /// Context of the task used in logging / telemetry.
    context: String,
}

/// Configuration for rate limiting in the sync queue.
#[derive(Clone)]
struct RateLimitConfig {
    max_requests_per_minute: u32,
    tokens: Arc<Mutex<f64>>,
    last_refill: Arc<Mutex<Instant>>,
}

impl RateLimitConfig {
    fn new(max_requests_per_minute: u32) -> Self {
        Self {
            max_requests_per_minute,
            tokens: Arc::new(Mutex::new(max_requests_per_minute as f64)),
            last_refill: Arc::new(Mutex::new(Instant::now())),
        }
    }

    async fn wait_for_token(&self) {
        loop {
            {
                let now = Instant::now();
                let mut tokens = self.tokens.lock().unwrap_or_else(|e| e.into_inner());
                let mut last_refill = self.last_refill.lock().unwrap_or_else(|e| e.into_inner());

                // Calculate tokens to add based on time elapsed
                let elapsed = now.duration_since(*last_refill);
                let tokens_to_add =
                    (elapsed.as_secs_f64() / 60.0) * self.max_requests_per_minute as f64;

                // Refill tokens (capped at max_rpm)
                *tokens = (*tokens + tokens_to_add).min(self.max_requests_per_minute as f64);
                *last_refill = now;

                // Try to consume a token
                if *tokens >= 1.0 {
                    *tokens -= 1.0;
                    return; // Token consumed, can proceed
                }
            }

            // No tokens available, wait a bit before checking again
            // Wait time is calculated to ensure we don't busy-wait
            let wait_time = Duration::from_millis(60_000 / self.max_requests_per_minute as u64);
            Timer::after(wait_time).await;
        }
    }
}

/// The global sync queue singleton, generic over the task type.
pub struct SyncQueue<T: SyncQueueTaskTrait> {
    sender: Arc<MpscSender<TaskId>>,
    task_map: Arc<Mutex<HashMap<TaskId, QueuedTask<T>>>>,
    /// Abort handle for the currently executing task. Set by the background
    /// processor before running a task and cleared after it completes.
    active_task_handle: Arc<Mutex<Option<AbortHandle>>>,
    mode: SyncQueueMode<T>,
}

impl<T: SyncQueueTaskTrait> Clone for SyncQueueMode<T> {
    fn clone(&self) -> Self {
        match self {
            SyncQueueMode::PerTask => SyncQueueMode::PerTask,
            SyncQueueMode::Streaming { _keepalive } => SyncQueueMode::Streaming {
                _keepalive: _keepalive.clone(),
            },
        }
    }
}

impl<T: SyncQueueTaskTrait> Clone for SyncQueue<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            task_map: self.task_map.clone(),
            active_task_handle: self.active_task_handle.clone(),
            mode: self.mode.clone(),
        }
    }
}

impl<T: SyncQueueTaskTrait> SyncQueue<T> {
    pub fn new(executor: &Arc<Background>) -> Self {
        Self::new_with_rate_limit(executor, None)
    }

    pub fn new_with_rate_limit(executor: &Arc<Background>, max_rpm: Option<u32>) -> Self {
        Self::new_inner(executor, max_rpm, SyncQueueMode::PerTask, None)
    }

    pub fn new_streaming(executor: &Arc<Background>) -> Self {
        Self::new_streaming_with_rate_limit(executor, None)
    }

    pub fn new_streaming_with_rate_limit(executor: &Arc<Background>, max_rpm: Option<u32>) -> Self {
        let (broadcast_tx, broadcast_rx) = async_broadcast::broadcast(DEFAULT_BUFFER_SIZE);
        let keepalive = broadcast_rx.deactivate();
        Self::new_inner(
            executor,
            max_rpm,
            SyncQueueMode::Streaming {
                _keepalive: keepalive,
            },
            Some(broadcast_tx),
        )
    }

    fn new_inner(
        executor: &Arc<Background>,
        max_rpm: Option<u32>,
        mode: SyncQueueMode<T>,
        broadcast_sender: Option<BroadcastSender<BroadcastResult<T>>>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(DEFAULT_BUFFER_SIZE);
        let rate_limit_config = max_rpm.map(RateLimitConfig::new);
        let task_map: Arc<Mutex<HashMap<TaskId, QueuedTask<T>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let active_task_handle: Arc<Mutex<Option<AbortHandle>>> = Arc::new(Mutex::new(None));

        executor.spawn(Self::process_queue(
            receiver,
            rate_limit_config,
            task_map.clone(),
            active_task_handle.clone(),
            broadcast_sender,
        ));

        Self {
            sender: Arc::new(sender),
            task_map,
            active_task_handle,
            mode,
        }
    }

    /// Returns a new broadcast receiver for task results.
    ///
    /// # Panics
    /// Panics if this is a per-task queue.
    pub fn subscribe(&self) -> BroadcastReceiver<T> {
        match &self.mode {
            SyncQueueMode::Streaming { _keepalive } => _keepalive.activate_cloned(),
            SyncQueueMode::PerTask => panic!("subscribe() called on a per-task queue"),
        }
    }

    /// Enqueues a task without returning a per-task result receiver.
    /// Results are delivered through the broadcast channel.
    ///
    /// # Panics
    /// Panics if this is a per-task queue.
    pub fn enqueue(&self, task: T, retry_options: Option<RetryOption>, context: impl Into<String>) {
        assert!(
            matches!(self.mode, SyncQueueMode::Streaming { .. }),
            "enqueue() called on a per-task queue"
        );

        let task_id = TaskId::new();
        let queued_task = QueuedTask {
            task,
            retry_options: retry_options.unwrap_or(DEFAULT_SYNC_RETRY_STRATEGY),
            context: context.into(),
            result_sender: None,
        };

        self.task_map.lock().unwrap_or_else(|e| e.into_inner()).insert(task_id, queued_task);

        if let Err(e) = self.sender.as_ref().clone().try_send(task_id) {
            log::warn!("Failed to enqueue task because of receiver error {e}");
            self.task_map.lock().unwrap_or_else(|e| e.into_inner()).remove(&task_id);
        }
    }

    /// Enqueues a task and returns a oneshot receiver for that task's result.
    ///
    /// # Panics
    /// Panics if this is a streaming queue.
    pub async fn enqueue_with_result(
        &self,
        task: T,
        retry_options: Option<RetryOption>,
        context: impl Into<String>,
    ) -> Receiver<Result<T::Result, T::Error>> {
        assert!(
            matches!(self.mode, SyncQueueMode::PerTask),
            "enqueue_with_result() called on a streaming queue"
        );

        let (tx, rx) = oneshot::channel();
        let task_id = TaskId::new();
        let queued_task = QueuedTask {
            task,
            retry_options: retry_options.unwrap_or(DEFAULT_SYNC_RETRY_STRATEGY),
            context: context.into(),
            result_sender: Some(tx),
        };

        self.task_map.lock().unwrap_or_else(|e| e.into_inner()).insert(task_id, queued_task);

        // Ignore send error if no receiver (e.g., queue processor dropped)
        if let Err(e) = self.sender.as_ref().clone().try_send(task_id) {
            log::warn!("Failed to enqueue task because of receiver error {e}");
            // Clean up the task from the map since it will never be processed.
            self.task_map.lock().unwrap_or_else(|e| e.into_inner()).remove(&task_id);
        }
        rx
    }

    /// Check if there is a queued task (not currently executing) that matches
    /// the given comparison function.
    pub fn has_queued_task(&self, comparison: impl Fn(&T) -> bool) -> bool {
        self.task_map
            .lock()
            .unwrap()
            .values()
            .any(|queued_task| comparison(&queued_task.task))
    }

    /// Cancel all pending and in-flight tasks.
    ///
    /// Pending tasks that have not yet started will have their result senders dropped,
    /// causing receivers to resolve to `Err(Canceled)`. The currently executing task
    /// (if any) is aborted via its `AbortHandle`.
    pub fn cancel_all(&self) {
        // Abort the currently executing task, if any.
        if let Some(handle) = self.active_task_handle.lock().unwrap_or_else(|e| e.into_inner()).take() {
            handle.abort();
        }

        // Drain all pending tasks from the map. Dropping the QueuedTask entries
        // drops their oneshot senders, signaling cancellation to receivers.
        self.task_map.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    async fn retry_with_backoff<Fut>(
        mut fut: impl FnMut() -> Fut,
        mut retry_options: RetryOption,
        context: &str,
    ) -> Result<T::Result, T::Error>
    where
        Fut: Future<Output = Result<T::Result, T::Error>>,
    {
        let mut attempt = 0;
        let max_attempts = retry_options.remaining_retries();
        loop {
            match fut().await {
                Ok(res) => return Ok(res),
                Err(e) => {
                    attempt += 1;
                    let is_transient = e.is_transient();
                    if !is_transient || attempt > max_attempts {
                        log::warn!(
                            "SyncQueue task failed after {attempt} attempts: {e}. Context: {context}"
                        );
                        return Err(e);
                    }
                    let delay = retry_options.duration();
                    retry_options.advance();
                    log::debug!(
                        "SyncQueue retryable error (attempt {attempt}/{max_attempts}), retrying after {delay:?}. Error: {e}. Context: {context}"
                    );
                    Timer::after(delay).await;
                }
            }
        }
    }

    /// Process tasks from the mpsc receiver. Should be called from an async context.
    async fn process_queue(
        mut receiver: MpscReceiver<TaskId>,
        rate_limit_config: Option<RateLimitConfig>,
        task_map: Arc<Mutex<HashMap<TaskId, QueuedTask<T>>>>,
        active_task_handle: Arc<Mutex<Option<AbortHandle>>>,
        broadcast_sender: Option<BroadcastSender<BroadcastResult<T>>>,
    ) {
        while let Some(task_id) = receiver.next().await {
            // Remove the task from the map. If it's missing, it was cancelled.
            let Some(mut queued_task) = task_map.lock().unwrap_or_else(|e| e.into_inner()).remove(&task_id) else {
                continue;
            };

            let retry_options = queued_task.retry_options;
            let rate_limit_config = rate_limit_config.clone();

            // Wrap the task in Abortable so cancel_all can abort it.
            // Rate limiting is inside the abortable so cancellation also
            // interrupts a task waiting for a rate-limit token.
            let (abort_handle, abort_registration) = AbortHandle::new_pair();
            *active_task_handle.lock().unwrap_or_else(|e| e.into_inner()) = Some(abort_handle);

            let abortable_result = Abortable::new(
                async {
                    if let Some(ref rate_limiter) = rate_limit_config {
                        rate_limiter.wait_for_token().await;
                    }
                    let fut = || queued_task.task.run();
                    Self::retry_with_backoff(fut, retry_options, queued_task.context.as_str()).await
                },
                abort_registration,
            )
            .await;

            // Clear the active handle now that the task has finished.
            *active_task_handle.lock().unwrap_or_else(|e| e.into_inner()) = None;

            match abortable_result {
                Ok(result) => {
                    if let Some(sender) = queued_task.result_sender {
                        // Per-task mode: deliver via oneshot.
                        let _ = sender.send(result);
                    } else if let Some(ref broadcast_tx) = broadcast_sender {
                        // Streaming mode: deliver via broadcast.
                        let broadcast_result = match result {
                            Ok(value) => Ok(Arc::new(value)),
                            Err(error) => Err(Arc::new(error)),
                        };
                        if let Err(e) = broadcast_tx.try_broadcast(broadcast_result) {
                            log::warn!("Failed to broadcast task result: {e}");
                        }
                    }
                }
                // Task was aborted by cancel_all — drop the sender to signal cancellation.
                Err(_aborted) => {}
            }
        }
        log::debug!("No more tasks in the queue. Receiver closed.");
    }
}

impl<T: SyncQueueTaskTrait> Entity for SyncQueue<T> {
    type Event = ();
}

impl<T: SyncQueueTaskTrait> SingletonEntity for SyncQueue<T> {}

#[cfg(test)]
#[path = "sync_queue_tests.rs"]
mod tests;
