use std::sync::{LazyLock, Once, mpsc};
use std::thread::JoinHandle;

use itertools::Itertools as _;
use parking_lot::Mutex;
use windows::Win32::System::Power::{self, SetThreadExecutionState};

/// The global backing state for the sleep prevention logic.
static STATE: LazyLock<State> = LazyLock::new(State::new);

/// Ensures that we only log message send failures once.
static SEND_FAILURE: Once = Once::new();

enum StateUpdate {
    AddTask { task_id: u64, reason: &'static str },
    RemoveTask { task_id: u64 },
}

/// The underlying state for the sleep prevention logic.
struct State {
    inner: Mutex<StateInner>,
}

impl State {
    /// Constructs a new state object.
    fn new() -> Self {
        let (update_tx, update_rx) = mpsc::channel::<StateUpdate>();

        let join_handle = std::thread::Builder::new()
            .name("prevent_sleep".to_string())
            .spawn(move || {
                Self::thread_main(update_rx);
            })
            .expect("should not fail to spawn thread");

        State {
            inner: Mutex::new(StateInner {
                update_tx,
                join_handle: Some(join_handle),
                next_task_id: 0,
            }),
        }
    }

    /// The main function of the thread that handles changes to the set of sleep-preventing
    /// tasks and updates the system state accordingly.
    fn thread_main(update_rx: mpsc::Receiver<StateUpdate>) {
        let mut active_tasks: Vec<(u64, &'static str)> = Default::default();

        while let Ok(task) = update_rx.recv() {
            match task {
                StateUpdate::AddTask { task_id, reason } => {
                    let was_empty = active_tasks.is_empty();
                    active_tasks.push((task_id, reason));

                    // If this is the first task, prevent sleep.
                    if was_empty {
                        unsafe {
                            SetThreadExecutionState(
                                Power::ES_CONTINUOUS
                                    | Power::ES_AWAYMODE_REQUIRED
                                    | Power::ES_SYSTEM_REQUIRED,
                            );
                        }
                    }

                    Self::log_active_tasks(&active_tasks);
                }
                StateUpdate::RemoveTask { task_id } => {
                    // Remove the task with this ID.
                    active_tasks.retain(|(id, _)| *id != task_id);

                    if active_tasks.is_empty() {
                        // Allow sleep again.
                        unsafe {
                            SetThreadExecutionState(Power::ES_CONTINUOUS);
                        }
                        log::info!("No longer preventing sleep");
                    } else {
                        // Log remaining active reasons.
                        Self::log_active_tasks(&active_tasks);
                    }
                }
            }
        }

        // The channel was closed, so allow sleep and terminate the thread.
        unsafe {
            SetThreadExecutionState(Power::ES_CONTINUOUS);
        }
        log::warn!("Sleep-prevention thread terminating...");
    }

    fn log_active_tasks(active_tasks: &[(u64, &'static str)]) {
        let reasons = active_tasks.iter().map(|(_, reason)| reason).collect_vec();
        log::info!("Preventing sleep with reasons: {reasons:?}");
    }

    fn new_guard(&self, reason: &'static str) -> Guard {
        let (task_id, update_tx) = {
            let mut inner = self.inner.lock();

            let task_id = inner.next_task_id;
            inner.next_task_id += 1;

            let update_tx = inner.update_tx.clone();

            (task_id, update_tx)
        };

        if let Err(err) = update_tx.send(StateUpdate::AddTask { task_id, reason }) {
            SEND_FAILURE.call_once(|| {
                log::warn!("Failed to send AddTask to sleep-prevention thread: {err}");
            });
        }

        Guard { task_id, update_tx }
    }
}

/// The internal state of for the sleep prevention logic.
struct StateInner {
    update_tx: mpsc::Sender<StateUpdate>,
    join_handle: Option<JoinHandle<()>>,
    next_task_id: u64,
}

impl Drop for StateInner {
    fn drop(&mut self) {
        // Close the channel to signal the thread to exit.  We replace the
        // sender with a new one, then drop the original sender.
        let old_sender = std::mem::replace(&mut self.update_tx, mpsc::channel().0);
        std::mem::drop(old_sender);

        // Wait for the thread to finish.
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

/// A guard that prevents system sleep while it continues to exist.
pub struct Guard {
    task_id: u64,
    update_tx: mpsc::Sender<StateUpdate>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Err(err) = self.update_tx.send(StateUpdate::RemoveTask {
            task_id: self.task_id,
        }) {
            SEND_FAILURE.call_once(|| {
                log::warn!("Failed to send RemoveTask to sleep-prevention thread: {err}");
            });
        }
    }
}

/// Returns a guard that prevents system sleep while it remains in scope.
pub fn prevent_sleep(reason: &'static str) -> Guard {
    STATE.new_guard(reason)
}
