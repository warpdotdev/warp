use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;

use crate::hooks::model::HookBindingId;
use crate::hooks::runner::{HookInvocation, HookProcessOutcome, HookRunner, ProcessExit};

/// Lets a test hold every scripted invocation open until it decides to let them finish.
pub struct RunnerHold {
    sender: Mutex<Option<async_channel::Sender<()>>>,
}

impl RunnerHold {
    /// Lets every waiting and future invocation complete.
    pub fn release(&self) {
        self.sender.lock().expect("hold lock").take();
    }
}

/// A runner that never spawns anything. Each binding id can be given an outcome; bindings without
/// one get the default outcome (exit code 0, no output).
pub struct ScriptedRunner {
    outcomes: Mutex<HashMap<HookBindingId, HookProcessOutcome>>,
    default_outcome: Mutex<HookProcessOutcome>,
    invocations: Mutex<Vec<HookInvocation>>,
    hold: Mutex<Option<async_channel::Receiver<()>>>,
}

impl Default for ScriptedRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptedRunner {
    pub fn new() -> Self {
        Self {
            outcomes: Mutex::new(HashMap::new()),
            default_outcome: Mutex::new(Self::exit_with(0, "")),
            invocations: Mutex::new(Vec::new()),
            hold: Mutex::new(None),
        }
    }

    /// Makes every invocation from now on wait until the returned hold is released. The
    /// invocation is still recorded as soon as it starts.
    pub fn hold(&self) -> Arc<RunnerHold> {
        let (sender, receiver) = async_channel::bounded::<()>(1);
        *self.hold.lock().expect("runner lock") = Some(receiver);
        Arc::new(RunnerHold {
            sender: Mutex::new(Some(sender)),
        })
    }

    pub fn exit_with(code: i32, stdout: &str) -> HookProcessOutcome {
        HookProcessOutcome {
            exit: ProcessExit::Code(code),
            stdout: stdout.as_bytes().to_vec(),
            stderr_tail: String::new(),
            duration: Duration::from_millis(1),
        }
    }

    /// An outcome the recording adapter decodes as a deny with `reason`.
    pub fn deny_with(reason: &str) -> HookProcessOutcome {
        Self::exit_with(2, reason)
    }

    pub fn script(&self, binding: HookBindingId, outcome: HookProcessOutcome) {
        self.outcomes
            .lock()
            .expect("runner lock")
            .insert(binding, outcome);
    }

    pub fn set_default_outcome(&self, outcome: HookProcessOutcome) {
        *self.default_outcome.lock().expect("runner lock") = outcome;
    }

    pub fn invocations(&self) -> Vec<HookInvocation> {
        self.invocations.lock().expect("runner lock").clone()
    }

    pub fn invocation_count(&self) -> usize {
        self.invocations.lock().expect("runner lock").len()
    }
}

impl HookRunner for ScriptedRunner {
    fn run(&self, invocation: HookInvocation) -> BoxFuture<'static, HookProcessOutcome> {
        let outcome = self
            .outcomes
            .lock()
            .expect("runner lock")
            .get(&invocation.binding)
            .cloned()
            .unwrap_or_else(|| self.default_outcome.lock().expect("runner lock").clone());
        self.invocations
            .lock()
            .expect("runner lock")
            .push(invocation);
        let hold = self.hold.lock().expect("runner lock").clone();
        match hold {
            // The sender is only ever dropped, never used, so `recv` returns once the hold is
            // released.
            Some(receiver) => async move {
                let _ = receiver.recv().await;
                outcome
            }
            .boxed(),
            None => futures::future::ready(outcome).boxed(),
        }
    }
}
