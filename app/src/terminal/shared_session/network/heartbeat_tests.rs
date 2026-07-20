use std::time::Duration;

use warpui::App;
use warpui::r#async::Timer;

use super::{Event, Heartbeat};

#[test]
#[ignore = "Flakes in CI"]
fn test_periodic_ping() {
    App::test((), |mut app| async move {
        let heartbeat =
            app.add_model(|_| Heartbeat::default().with_ping_frequency(Duration::from_millis(100)));
        let (tx, rx) = async_channel::unbounded();
        let tx_clone = tx.clone();
        app.update(|ctx| {
            ctx.subscribe_to_model(&heartbeat, move |_, event, _| {
                if matches!(event, Event::Ping) {
                    tx_clone.try_send(()).expect("can send over channel");
                }
            });
        });

        // Start the heartbeat.
        heartbeat.update(&mut app, |heartbeat, ctx| heartbeat.start(ctx));

        // After 50ms, there shouldn't have been any pings.
        Timer::after(Duration::from_millis(50)).await;
        assert_eq!(rx.len(), 0);

        // After 150ms, there should have been 1 ping.
        Timer::after(Duration::from_millis(100)).await;
        assert_eq!(rx.len(), 1);

        // After 175ms, there still should have only been 1 ping.
        Timer::after(Duration::from_millis(25)).await;
        assert_eq!(rx.len(), 1);

        // After 250ms, there should have been 2 pings in total.
        Timer::after(Duration::from_millis(75)).await;
        assert_eq!(rx.len(), 2);
    })
}

#[test]
#[ignore = "Flakes in CI"]
fn test_idle_timeout() {
    App::test((), |mut app| async move {
        let heartbeat =
            app.add_model(|_| Heartbeat::default().with_idle_timeout(Duration::from_millis(100)));
        let (tx, rx) = async_channel::unbounded();
        app.update(|ctx| {
            ctx.subscribe_to_model(&heartbeat, move |_, event, _| {
                if matches!(event, Event::Idle) {
                    tx.try_send(()).expect("can send over channel");
                }
            });
        });

        // Start the heartbeat.
        heartbeat.update(&mut app, |heartbeat, ctx| heartbeat.start(ctx));

        // The idle timeout should not have expired yet.
        Timer::after(Duration::from_millis(50)).await;
        assert_eq!(rx.len(), 0);

        // Reset the idle timeout.
        heartbeat.update(
            &mut app,
            |heartbeat, ctx: &mut warpui::ModelContext<Heartbeat>| {
                heartbeat.reset_idle_timeout(ctx)
            },
        );

        // If the idle timeout was reset properly, then there should not be a idle event yet
        // even though one full idle timeout has elapsed since `start`.
        Timer::after(Duration::from_millis(75)).await;
        assert_eq!(rx.len(), 0);

        // One full idle timeout has elapsed since the last reset, so
        // we should have received an idle event.
        Timer::after(Duration::from_millis(75)).await;
        assert_eq!(rx.len(), 1);
    })
}
