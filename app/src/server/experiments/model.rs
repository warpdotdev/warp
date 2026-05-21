//! The model for maintaining global experiment state.

use std::collections::HashSet;

use super::ServerExperiment;
use crate::{persistence::ModelEvent, report_if_error, GlobalResourceHandlesProvider};
use anyhow::Context;
use warpui::{Entity, ModelContext, SingletonEntity};

#[cfg(test)]
pub use tests::TestModel;

/// A global model for maintaining server-side experiment state.
pub struct ServerExperiments {
    /// The latest-known set of server-side enabled experiments.
    latest: HashSet<ServerExperiment>,
}

impl ServerExperiments {
    /// Creates a new [`ServerExperiments`] model and seeds it with
    /// the provided `cached` experiment state.
    pub fn new_from_cache(cached: Vec<ServerExperiment>, ctx: &mut ModelContext<Self>) -> Self {
        let mut model = Self {
            latest: HashSet::new(),
        };
        model.apply_latest_state(cached, ctx);
        model
    }

    /// Updates the model with the latest server-side state.
    ///
    /// Assumes the set of provided [`ServerExperiment`]s are unambiguous;
    /// that is, there are not two arms enabled for the same experiment group.
    pub fn apply_latest_state(
        &mut self,
        incoming: Vec<ServerExperiment>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Dedup the set of experiments.
        let incoming = HashSet::from_iter(incoming);

        // For every experiment that the client isn't already part of,
        // perform the necessary logic to add them.
        for experiment in incoming.difference(&self.latest) {
            experiment.on_added_to(ctx);
        }

        self.cache_latest_state(incoming, ctx);
        ctx.emit(Event::ExperimentsUpdated);
    }

    /// Returns true iff the `experiment` is enabled.
    pub fn is_experiment_enabled(&self, experiment: &ServerExperiment) -> bool {
        self.latest.contains(experiment)
    }

    /// Saves the latest experiment state in-memory and to the local cache.
    fn cache_latest_state(
        &mut self,
        latest: HashSet<ServerExperiment>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.latest = latest;

        if let Some(model_event_sender) = GlobalResourceHandlesProvider::as_ref(ctx)
            .get()
            .model_event_sender
            .as_ref()
        {
            let event = ModelEvent::SaveExperiments {
                experiments: self.latest.iter().copied().collect(),
            };
            report_if_error!(model_event_sender
                .send(event)
                .context("Unable to save experiments to sqlite"));
        }
    }
}

pub enum Event {
    ExperimentsUpdated,
}

impl Entity for ServerExperiments {
    type Event = Event;
}

impl SingletonEntity for ServerExperiments {}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
