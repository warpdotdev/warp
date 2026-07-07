use std::sync::{Arc, Mutex};

use async_broadcast::Receiver;
use futures_lite::StreamExt;
use instant::{Duration, Instant};
use parking_lot::FairMutex;
use warpui::r#async::executor::Background;

use crate::auth::auth_state::AuthState;
use crate::send_telemetry_on_executor;
use crate::server::telemetry::TelemetryEvent;
use crate::terminal::TerminalModel;

/// We want to measure throughput as bytes / sec.
const PTY_THROUGHPUT_TIME_INTERVAL: Duration = Duration::from_secs(1);

/// We don't want to actually emit a metric every second because that
/// will be wasteful. So we aggregate over 10 minute periods.
const PTY_THROUGHPUT_METRIC_INTERVAL: Duration = Duration::from_secs(10);

/// Records the max PTY bytes/sec throughput observed over every 10 minute period.
pub fn record_pty_throughput(
    mut pty_reads_rx: Receiver<Arc<Vec<u8>>>,
    model: Arc<FairMutex<TerminalModel>>,
    auth_state: Arc<AuthState>,
    executor: Arc<Background>,
) {
    let num_bytes_read_in_last_second = Arc::new(Mutex::new(0));
    let max_throughput_so_far = Arc::new(Mutex::new(0));
    let last_emitted_event_time = Arc::new(Mutex::new(Instant::now()));

    // We use an inactive receiver to keep track of when the channel is closed (in the task
    // that loops on a 1s interval).
    let inactive = pty_reads_rx.clone().deactivate();

    // Keep track of the number of bytes read.
    let num_bytes_read_in_last_second_clone = num_bytes_read_in_last_second.clone();
    executor
        .spawn(async move {
            while let Ok(bytes) = pty_reads_rx.recv().await {
                let model = model.lock();
                // Don't care about pre-bootstrap or in-band-command bytes for this calculation.
                if !model.is_receiving_in_band_command_output()
                    && model.is_active_block_bootstrapped()
                {
                    *num_bytes_read_in_last_second_clone.lock().unwrap() += bytes.len();
                }
            }
        })
        .detach();

    // Every second, update the max throughput and check if it's time to emit an event.
    let executor_clone = executor.clone();
    executor
        .spawn(async move {
            while async_io::Timer::interval(PTY_THROUGHPUT_TIME_INTERVAL)
                .next()
                .await
                .is_some()
            {
                // If the PTY reads are done (i.e. the session is over), let's end this task.
                if inactive.is_closed() {
                    break;
                }

                let mut num_bytes_read_in_last_second =
                    num_bytes_read_in_last_second.lock().unwrap();
                let mut max_throughput = max_throughput_so_far.lock().unwrap();
                let mut last_emitted_event_time = last_emitted_event_time.lock().unwrap();

                *max_throughput = std::cmp::max(*max_throughput, *num_bytes_read_in_last_second);
                *num_bytes_read_in_last_second = 0;

                // If the max throughput was non-zero and it's been 10 minutes,
                // let's emit an event.
                if Instant::now().duration_since(*last_emitted_event_time)
                    >= PTY_THROUGHPUT_METRIC_INTERVAL
                {
                    if *max_throughput > 0 {
                        send_telemetry_on_executor!(
                            auth_state,
                            TelemetryEvent::PtyThroughput {
                                max_bytes_per_second: *max_throughput,
                            },
                            executor_clone
                        );
                    }
                    *max_throughput = 0;
                    *last_emitted_event_time = Instant::now();
                }
            }
        })
        .detach();
}
