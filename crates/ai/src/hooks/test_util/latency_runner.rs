use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::FutureExt;
use futures::future::BoxFuture;
use warpui_core::r#async::Timer;

use crate::hooks::model::HookBindingId;
use crate::hooks::runner::{HookInvocation, HookProcessOutcome, HookRunner, ProcessExit};

/// How often a running invocation looks at its cancel flag.
const CANCEL_POLL: Duration = Duration::from_millis(5);

/// A runner that never spawns anything but takes a scripted amount of time per binding, for
/// tests about concurrency, ordering, and cancellation. Bindings without a script finish at once
/// with exit code 0 and no output. A running invocation ends with `Cancelled` as soon as it sees
/// its cancel flag.
#[derive(Default)]
pub struct LatencyRunner {
    scripts: Mutex<HashMap<HookBindingId, (Duration, HookProcessOutcome)>>,
    started: Mutex<Vec<HookBindingId>>,
    /// Shared with the futures, which outlive any borrow of the runner.
    finished: Arc<Mutex<Vec<(HookBindingId, ProcessExit)>>>,
}

impl LatencyRunner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn script(&self, binding: HookBindingId, latency: Duration, outcome: HookProcessOutcome) {
        self.scripts
            .lock()
            .expect("runner lock")
            .insert(binding, (latency, outcome));
    }

    /// Bindings in the order their invocations started.
    pub fn started(&self) -> Vec<HookBindingId> {
        self.started.lock().expect("runner lock").clone()
    }

    /// Bindings in the order their invocations finished, with how each ended.
    pub fn finished(&self) -> Vec<(HookBindingId, ProcessExit)> {
        self.finished.lock().expect("runner lock").clone()
    }
}

impl HookRunner for LatencyRunner {
    fn run(&self, invocation: HookInvocation) -> BoxFuture<'static, HookProcessOutcome> {
        let (latency, outcome) = self
            .scripts
            .lock()
            .expect("runner lock")
            .get(&invocation.binding)
            .cloned()
            .unwrap_or_else(|| {
                (
                    Duration::ZERO,
                    HookProcessOutcome {
                        exit: ProcessExit::Code(0),
                        stdout: Vec::new(),
                        stdout_truncated: false,
                        stderr_tail: String::new(),
                        duration: Duration::ZERO,
                    },
                )
            });
        self.started
            .lock()
            .expect("runner lock")
            .push(invocation.binding.clone());
        let finished = self.finished.clone();
        let binding = invocation.binding;
        let cancel = invocation.cancel;
        async move {
            let started = Instant::now();
            let outcome = loop {
                if cancel.load(Ordering::Acquire) {
                    break HookProcessOutcome {
                        exit: ProcessExit::Cancelled,
                        stdout: Vec::new(),
                        stdout_truncated: false,
                        stderr_tail: String::new(),
                        duration: started.elapsed(),
                    };
                }
                if started.elapsed() >= latency {
                    break HookProcessOutcome {
                        duration: started.elapsed(),
                        ..outcome
                    };
                }
                Timer::after(CANCEL_POLL).await;
            };
            finished
                .lock()
                .expect("runner lock")
                .push((binding, outcome.exit.clone()));
            outcome
        }
        .boxed()
    }
}
