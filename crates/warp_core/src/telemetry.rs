use std::fmt;
use std::marker::PhantomData;

// Re-export for macro use.
#[doc(hidden)]
#[cfg(not(target_family = "wasm"))]
pub use inventory::submit;
use serde_json::Value;
use strum::IntoEnumIterator;
use warpui_core::{AppContext, Entity, SingletonEntity};

use crate::channel::{Channel, ChannelState};
use crate::features::FeatureFlag;

/// Core trait defining telemetry event behavior.
///
/// This trait encapsulates the basic functionality required for any telemetry event
/// in the Warp ecosystem. It enables events to be defined in any crate while maintaining
/// consistent telemetry reporting behavior.
pub trait TelemetryEvent: RegisteredTelemetryEvent {
    /// Returns the name of the telemetry event.
    ///
    /// The name should be a stable identifier that uniquely identifies this type of event.
    /// It is used for analytics tracking and should remain consistent over time.
    ///
    /// Returns a borrowed string to avoid allocations for static event names.
    fn name(&self) -> &'static str;

    /// Returns optional structured data associated with this event.
    ///
    /// The payload allows events to include additional context or metadata beyond
    /// just the event name. This is useful for including dynamic data about the
    /// event occurrence.
    ///
    /// Returns None if the event has no additional data to report.
    fn payload(&self) -> Option<Value>;

    /// Returns a human-readable description of what this event represents.
    ///
    /// The description should clearly explain the significance of the event to help
    /// with analytics and monitoring. This is used both for documentation and
    /// telemetry dashboards.
    fn description(&self) -> &'static str;

    /// Determines if an event is enabled in the current build. This only works when all
    /// feature flags are set appropriately, so this should be used when running
    /// the bundled app.
    fn enablement_state(&self) -> EnablementState;

    /// Returns whether this event contains user-generated content (UGC).
    ///
    /// Events containing UGC may need special handling for privacy and data
    /// retention reasons. This flag helps route the event to the appropriate
    /// analytics destination.
    fn contains_ugc(&self) -> bool;

    /// Returns an iterator over the descriptors for all telemetry events of this type.
    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>>;
}

#[macro_export]
macro_rules! register_telemetry_event {
    ($event:ty) => {
        impl $crate::telemetry::RegisteredTelemetryEvent for $event {}

        #[cfg(not(target_family = "wasm"))]
        $crate::telemetry::submit! {
            $crate::telemetry::TelemetryEventRegistration::<$event>::adapt()
        }
    };
}

/// Marker trait for known telemetry events. We rely on this to print an exhaustive telemetry
/// table in Warp's documentation.
///
/// DO NOT implement this trait directly - use the [`register_telemetry_event!`] macro instead.
pub trait RegisteredTelemetryEvent {}

/// An abstract description of a telemetry event we may emit. Every [`TelemetryEvent`] has a
/// corresponding [`TelemetryEventDesc`].
pub trait TelemetryEventDesc: fmt::Debug {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn enablement_state(&self) -> EnablementState;
}

/// A type-erased version of [`TelemetryEventRegistration`]. This is only used by the
/// [`register_telemetry_event!`] macro implementation.
#[doc(hidden)]
pub trait AnyTelemetryEventRegistration: Sync {
    /// Returns an iterator over the descriptors for all telemetry events in this [`TelemetryEvent`] implementation.
    fn events(&self) -> Box<dyn Iterator<Item = Box<dyn TelemetryEventDesc>>>;
}

/// Adapter for statically registering all [`TelemetryEvent`] implementations.
#[doc(hidden)]
pub struct TelemetryEventRegistration<T: TelemetryEvent + 'static> {
    /// Marker that `TelemetryEventRegistration` references `T`, but doesn't own a `T` value.
    /// See https://doc.rust-lang.org/nomicon/phantom-data.html
    _marker: PhantomData<fn(T) -> T>,
}

impl<T: TelemetryEvent + 'static> TelemetryEventRegistration<T> {
    pub const fn adapt() -> &'static dyn AnyTelemetryEventRegistration {
        &Self {
            _marker: PhantomData,
        }
    }
}

impl<T: TelemetryEvent + 'static> AnyTelemetryEventRegistration for TelemetryEventRegistration<T> {
    fn events(&self) -> Box<dyn Iterator<Item = Box<dyn TelemetryEventDesc>>> {
        Box::new(T::event_descs())
    }
}

/// Returns an iterator over all discriminants of `T` as [`TelemetryEventDesc`]s.
///
/// Telemetry events that use [`strum`] may use this to implement [`TelemetryEvent::event_descs`].
pub fn enum_events<T>() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>>
where
    T: strum::IntoDiscriminant,
    T::Discriminant: strum::IntoEnumIterator + TelemetryEventDesc + 'static,
{
    T::Discriminant::iter()
        .map(|discriminant| Box::new(discriminant) as Box<dyn TelemetryEventDesc>)
}

// Collect adapters for all registered telemetry events. Because `inventory::collect!` requires a
// concrete type, we use `&static dyn Trait` to erase the generics.
#[cfg(not(target_family = "wasm"))]
inventory::collect!(&'static dyn AnyTelemetryEventRegistration);

/// Returns all registered telemetry events. This is not available in WASM builds, as it relies on
/// the [`inventory`] crate, which does not fully work in our WASM configuration.
#[cfg(not(target_family = "wasm"))]
pub fn all_events() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
    inventory::iter::<&'static dyn AnyTelemetryEventRegistration>().flat_map(|meta| meta.events())
}

// Sends a telemetry `track` event to Rudderstack asynchronously. It adds events to the static
// telemetry queue that is periodically flushed to the Rudderstack API.
// This is the recommended way of recording telemetry events.
// You should almost always use this, unless the recording is time-sensitive and cannot be lost.
// To send a telemetry event synchronously, use [`send_telemetry_sync_from_ctx`].
#[macro_export]
macro_rules! send_telemetry_from_ctx {
    ($event:expr, $ctx:expr) => {
        #[allow(unused_imports)]
        use warp_core::telemetry::TelemetryEvent as _;
        let event = $event;
        if event.enablement_state().is_enabled() {
            let auth_state =
                <$crate::telemetry::TelemetryContextModel as $crate::warpui_core::SingletonEntity>::handle($ctx)
                    .as_ref($ctx);
            let user_id = auth_state.user_id($ctx);
            let anonymous_id = auth_state.anonymous_id($ctx);
            $crate::warpui_core::record_telemetry_from_ctx!(
                user_id,
                anonymous_id,
                event.name().into(),
                event.payload(),
                event.contains_ugc(),
                $ctx
            );
        }
    };
}

/// Sends telemetry `track` event to Rudderstack API asynchronously. This is the same as the
/// [`send_telemetry_from_ctx`], except it can be called in instances where you only have
/// a `AppContext` rather than a `ViewContext`/`ModelContext`.
///
/// If possible, use [`send_telemetry_from_ctx`].
#[macro_export]
macro_rules! send_telemetry_from_app_ctx {
    ($event:expr, $app_ctx:expr) => {
        let event = $event;
        if event.enablement_state().is_enabled() {
            let auth_state =
                <$crate::telemetry::TelemetryContextModel as $crate::warpui_core::SingletonEntity>::handle(
                    $app_ctx,
                )
                .as_ref($app_ctx);
            let user_id = auth_state.user_id($app_ctx.as_ref());
            let anonymous_id = auth_state.anonymous_id($app_ctx.as_ref());
            $crate::warpui_core::record_telemetry_on_executor!(
                user_id,
                anonymous_id,
                event.name().into(),
                event.payload(),
                event.contains_ugc(),
                $app_ctx.background_executor()
            );
        }
    };
}

/// Gives information about when a telemetry event is enabled.
#[derive(Debug)]
pub enum EnablementState {
    Always,
    /// The telemetry event is enabled when a particular feature flag is enabled.
    Flag(FeatureFlag),
    /// The event is enabled if the app is running in one of the contained channels.
    ChannelSpecific {
        channels: Vec<Channel>,
    },
}

impl EnablementState {
    pub fn is_enabled(&self) -> bool {
        match self {
            EnablementState::Always => true,
            EnablementState::Flag(flag) => flag.is_enabled(),
            EnablementState::ChannelSpecific { channels } => {
                let app_channel = ChannelState::channel();
                channels.contains(&app_channel)
            }
        }
    }
}

/// Trait for the context provider that allows us to send telemetry payloads.
pub trait TelemetryContextProvider {
    fn user_id(&self, ctx: &AppContext) -> Option<String>;

    fn anonymous_id(&self, ctx: &AppContext) -> String;
}

pub type TelemetryContextModel = Box<dyn TelemetryContextProvider>;

impl Entity for TelemetryContextModel {
    type Event = ();
}

impl SingletonEntity for TelemetryContextModel {}

#[cfg(any(test, feature = "test-util"))]
pub mod testing {
    use warpui_core::prelude::*;

    pub struct MockTelemetryContextProvider;

    impl MockTelemetryContextProvider {
        pub fn register(ctx: &mut AppContext) {
            ctx.add_singleton_model(|_| {
                Box::new(MockTelemetryContextProvider) as super::TelemetryContextModel
            });
        }
    }

    impl super::TelemetryContextProvider for MockTelemetryContextProvider {
        fn user_id(&self, _ctx: &AppContext) -> Option<String> {
            None
        }

        fn anonymous_id(&self, _ctx: &AppContext) -> String {
            "test_anonymous_id".to_string()
        }
    }
}
