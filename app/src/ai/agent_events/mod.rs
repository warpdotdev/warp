//! Shared agent-event stream utilities used by orchestration consumers and
//! third-party harness bridges.

mod driver;
mod message_hydrator;

#[cfg(test)]
pub(crate) use driver::{
    agent_event_backoff, agent_event_failures_exceeded_threshold, AgentEventDriverState,
    AgentEventSource, AgentEventSourceItem, DEFAULT_AGENT_EVENT_FAILURES_BEFORE_ERROR_LOG,
    DEFAULT_AGENT_EVENT_RECONNECT_BACKOFF_STEPS, DEFAULT_PERMANENT_ERROR_BACKOFF_STEPS,
};
pub(crate) use driver::{
    run_agent_event_driver, AgentEventConsumer, AgentEventConsumerControlFlow,
    AgentEventDriverConfig, AgentMessageEventMetadata, ServerApiAgentEventSource,
};
pub(crate) use message_hydrator::MessageHydrator;

#[cfg(test)]
mod driver_tests;
#[cfg(test)]
mod message_hydrator_tests;
