/// Sends a telemetry event to Rudderstack immediately instead of adding it to the event queue that is
/// periodically flushed. This is useful under certain conditions where we want to ensure an event
/// is immediately sent to Rudderstack even if the user quits before the queue is flushed.
#[macro_export]
macro_rules! send_telemetry_sync_from_ctx {
    ($event:expr_2021, $ctx:expr_2021) => {
        #[allow(unused_imports)]
        use warp_core::telemetry::TelemetryEvent as _;
        let event = $event;
        if event.enablement_state().is_enabled() {
            let server_api =
                <$crate::server::server_api::ServerApiProvider as warpui::SingletonEntity>::handle(
                    $ctx,
                )
                .as_ref($ctx)
                .get();
            let privacy_settings_snapshot =
                <$crate::settings::PrivacySettings as warpui::SingletonEntity>::handle($ctx)
                    .as_ref($ctx)
                    .get_snapshot($ctx);
            let _ = $ctx.spawn(
                async move {
                    if let Err(error) = server_api
                        .send_telemetry_event(event, privacy_settings_snapshot)
                        .await
                    {
                        log::warn!("Error occurred with sending telemetry event: {}", error);
                    }
                },
                |_, _, _| {},
            );
        }
    };
}

/// Sends a telemetry event to Rudderstack immediately. This is the same as [`send_telemetry_sync_from_ctx`],
/// but can be used when the caller only has access to an [`App`] and not a
/// `ViewContext`.
#[macro_export]
macro_rules! send_telemetry_sync_from_app_ctx {
    ($event:expr_2021, $app_ctx:expr_2021) => {
        #[allow(unused_imports)]
        use warp_core::telemetry::TelemetryEvent as _;
        if $event.enablement_state().is_enabled() {
            let server_api =
                <$crate::server::server_api::ServerApiProvider as warpui::SingletonEntity>::handle(
                    $app_ctx,
                )
                .as_ref($app_ctx)
                .get();
            let privacy_settings_snapshot =
                <$crate::settings::PrivacySettings as warpui::SingletonEntity>::handle($app_ctx)
                    .as_ref($app_ctx)
                    .get_snapshot($app_ctx);
            $app_ctx
                .background_executor()
                .spawn(async move {
                    if let Err(error) = server_api
                        .send_telemetry_event($event, privacy_settings_snapshot)
                        .await
                    {
                        log::warn!("Error occurred with sending telemetry event: {error}");
                    }
                })
                .detach();
        }
    };
}

/// Sends a telemetry `track` event Rudderstack asynchronously. This is the same as the
/// [`send_telemetry_from_ctx`], except can be called any time you have an Arc<Background>.
/// This should only be called when invoking one of the other macros isn't possible; for example,
/// when you are already on a background thread and thus can't access any app context.
#[macro_export]
macro_rules! send_telemetry_on_executor {
    ($auth_state: expr_2021, $event:expr_2021, $executor:expr_2021) => {
        #[allow(unused_imports)]
        use warp_core::telemetry::TelemetryEvent as _;
        let event = $event;
        if event.enablement_state().is_enabled() {
            let user_id = $auth_state.user_id().map(|uid| uid.as_string());
            let anonymous_id = $auth_state.anonymous_id();
            warpui::record_telemetry_on_executor!(
                user_id,
                anonymous_id,
                event.name().into(),
                event.payload(),
                event.contains_ugc(),
                $executor
            );
        }
    };
}
