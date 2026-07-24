use libtest_mimic::{Arguments, Trial};

fn main() {
    let mut args = Arguments::from_args();
    // This test must run on the process main thread to service the macOS run loop.
    args.test_threads = Some(1);

    #[cfg(target_os = "macos")]
    let tests = vec![Trial::test("services_main_dispatch_queue", || {
        macos::services_main_dispatch_queue().map_err(|error| format!("{error:#}").into())
    })];

    #[cfg(not(target_os = "macos"))]
    let tests = Vec::<Trial>::new();

    libtest_mimic::run(&args, tests).exit();
}

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    use anyhow::anyhow;
    use dispatch2::run_on_main;
    use instant::Instant;
    use warpui::r#async::Timer;
    use warpui::platform::TerminationMode;
    use warpui::platform::app::{AppBuilder, AppCallbacks};

    pub(super) fn services_main_dispatch_queue() -> anyhow::Result<()> {
        objc2::MainThreadMarker::new()
            .expect("the custom test harness must run on the process main thread");
        let process_main_thread = thread::current().id();
        let dispatched_on_main = Arc::new(AtomicBool::new(false));
        let worker_returned = Arc::new(AtomicBool::new(false));

        AppBuilder::new_headless(AppCallbacks::default(), Box::new(()), None).run({
            let dispatched_on_main = dispatched_on_main.clone();
            let worker_returned = worker_returned.clone();
            move |ctx| {
                thread::spawn({
                    let dispatched_on_main = dispatched_on_main.clone();
                    let worker_returned = worker_returned.clone();
                    move || {
                        run_on_main(|_| {
                            dispatched_on_main.store(
                                thread::current().id() == process_main_thread,
                                Ordering::SeqCst,
                            );
                        });
                        worker_returned.store(true, Ordering::SeqCst);
                    }
                });

                let weak_app = ctx.weak_app();
                ctx.foreground_executor()
                    .spawn(async move {
                        let deadline = Instant::now() + Duration::from_secs(5);
                        while Instant::now() < deadline
                            && !(dispatched_on_main.load(Ordering::SeqCst)
                                && worker_returned.load(Ordering::SeqCst))
                        {
                            Timer::after(Duration::from_millis(10)).await;
                        }

                        let test_result = if dispatched_on_main.load(Ordering::SeqCst)
                            && worker_returned.load(Ordering::SeqCst)
                        {
                            Ok(())
                        } else {
                            Err(anyhow!(
                                "headless Warp did not service the GCD main queue before timeout"
                            ))
                        };
                        if let Some(mut app) = weak_app.upgrade() {
                            app.update(|ctx| {
                                ctx.terminate_app(
                                    TerminationMode::ForceTerminate,
                                    Some(test_result),
                                );
                            });
                        }
                    })
                    .detach();
            }
        })
    }
}
