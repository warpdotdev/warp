use std::sync::{Arc, Mutex};

use bounded_vec_deque::BoundedVecDeque;
use instant::Instant;
use lazy_static::lazy_static;

lazy_static! {
    static ref TRACES: Arc<Mutex<Traces>> = Arc::new(Mutex::new(Traces::new()));
}

const MAX_BUFFER_SIZE: usize = 1024;

struct Traces {
    // Bounded in case we are tracing a lot of events and we don't want to blow
    // up memory
    events: BoundedVecDeque<TraceEvent>,
    end_after_next: Option<&'static str>,
}

impl Traces {
    fn new() -> Self {
        Self {
            events: BoundedVecDeque::new(MAX_BUFFER_SIZE),
            end_after_next: None,
        }
    }
}

struct TraceEvent {
    timestamp: Instant,
    name: &'static str,
}

#[macro_export]
macro_rules! start_trace {
    ($name:expr) => {
        #[cfg(feature = "traces")]
        $crate::traces::start_trace($name)
    };
}

#[macro_export]
macro_rules! record_trace_event {
    ($name:expr) => {
        #[cfg(feature = "traces")]
        $crate::traces::record_event($name)
    };
}

#[macro_export]
macro_rules! end_trace_after_next {
    ($name:expr) => {
        #[cfg(feature = "traces")]
        $crate::traces::end_trace_after_next($name)
    };
}

#[macro_export]
macro_rules! end_trace {
    () => {
        #[cfg(feature = "traces")]
        $crate::traces::end_trace()
    };
}

pub fn start_trace(event_name: &'static str) {
    TRACES.lock().unwrap().events.clear();
    record_event(event_name);
}

pub fn record_event(name: &'static str) {
    let should_end = {
        // Separate block to let the mutex go out of scope before calling end_trace
        let mut traces = TRACES.lock().unwrap();
        traces.events.push_back(TraceEvent {
            name,
            timestamp: Instant::now(),
        });

        traces.end_after_next == Some(name)
    };
    if should_end {
        end_trace();
    }
}

pub fn end_trace_after_next(event_name: &'static str) {
    TRACES.lock().unwrap().end_after_next = Some(event_name);
}

pub fn end_trace() {
    let mut traces = TRACES.lock().unwrap();
    let start = traces.events.pop_front().expect("no empty traces");
    for event in traces.events.iter() {
        println!(
            "[{:.4} ms] {} ",
            (event.timestamp.duration_since(start.timestamp).as_micros() as f32 / 1000.),
            event.name,
        );
    }
    traces.end_after_next = None;
    traces.events.clear();
}
