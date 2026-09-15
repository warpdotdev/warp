use std::future::pending;

use super::*;
use crate::r#async::block_on;

/// Runs a single scheduled task on `foreground`'s test executor, and returns
/// whether a task was actually found to run.
fn try_tick(foreground: &Foreground) -> bool {
    match foreground {
        Foreground::Test { executor, .. } => executor.try_tick(),
        Foreground::Platform { .. } => unreachable!("expected the test executor"),
    }
}

#[test]
fn spawn_is_attributed_to_the_caller_not_the_executor() {
    let foreground = Foreground::test();
    assert_eq!(foreground.task_census_snapshot(10).total_live_tasks, 0);

    let task = foreground.spawn(pending::<()>());

    let snapshot = foreground.task_census_snapshot(10);
    assert_eq!(snapshot.total_live_tasks, 1);
    assert_eq!(snapshot.top_spawn_sites.len(), 1);
    let site = &snapshot.top_spawn_sites[0];
    // The recorded location should be this test file (the call above), not
    // `executor.rs` -- otherwise every spawn site in the app would collapse
    // into one bucket, defeating the point of the census.
    assert!(
        site.location.contains("executor_tests.rs"),
        "expected the spawning test file in the location, got {}",
        site.location
    );
    assert_eq!(site.live_tasks, 1);
    assert_eq!(site.total_spawned, 1);

    task.detach();
}

#[test]
fn distinct_call_sites_are_tracked_and_ranked_separately() {
    // Mirrors the incident this census is meant to catch: many tasks piling
    // up from one call site alongside a few from an unrelated one should
    // show up as a clear, separately-attributed outlier.
    fn spawn_from_hot_site(foreground: &Foreground) -> ForegroundTask {
        foreground.spawn(pending::<()>())
    }
    fn spawn_from_other_site(foreground: &Foreground) -> ForegroundTask {
        foreground.spawn(pending::<()>())
    }

    let foreground = Foreground::test();
    let hot_tasks: Vec<_> = (0..5).map(|_| spawn_from_hot_site(&foreground)).collect();
    let other_task = spawn_from_other_site(&foreground);

    let snapshot = foreground.task_census_snapshot(10);
    assert_eq!(snapshot.total_live_tasks, 6);
    assert_eq!(snapshot.top_spawn_sites.len(), 2);
    assert_eq!(snapshot.top_spawn_sites[0].live_tasks, 5);
    assert_eq!(snapshot.top_spawn_sites[0].total_spawned, 5);
    assert_eq!(snapshot.top_spawn_sites[1].live_tasks, 1);
    assert_ne!(
        snapshot.top_spawn_sites[0].location, snapshot.top_spawn_sites[1].location,
        "the two call sites must be attributed separately"
    );

    for task in hot_tasks {
        task.detach();
    }
    other_task.detach();
}

#[test]
fn snapshot_respects_the_requested_limit() {
    let foreground = Foreground::test();
    let tasks: Vec<_> = (0..3).map(|_| foreground.spawn(pending::<()>())).collect();

    let snapshot = foreground.task_census_snapshot(1);
    assert_eq!(snapshot.total_live_tasks, 3);
    assert_eq!(snapshot.top_spawn_sites.len(), 1);
    assert_eq!(snapshot.top_spawn_sites[0].live_tasks, 3);

    for task in tasks {
        task.detach();
    }
}

#[test]
fn completing_a_task_decrements_the_live_count() {
    let foreground = Foreground::test();
    let task = foreground.spawn(async {});
    assert_eq!(foreground.task_census_snapshot(10).total_live_tasks, 1);

    block_on(foreground.run(task));

    let snapshot = foreground.task_census_snapshot(10);
    assert_eq!(snapshot.total_live_tasks, 0);
    assert!(snapshot.top_spawn_sites.is_empty());
}

#[test]
fn aborting_a_task_decrements_the_live_count() {
    let foreground = Foreground::test();
    let (task, abort_handle) = foreground.spawn_abortable(pending::<()>());
    assert_eq!(foreground.task_census_snapshot(10).total_live_tasks, 1);

    abort_handle.abort();
    block_on(foreground.run(async {
        let _ = task.await;
    }));

    assert_eq!(foreground.task_census_snapshot(10).total_live_tasks, 0);
}

#[test]
fn dropping_the_task_handle_eventually_decrements_the_live_count() {
    let foreground = Foreground::test();
    let task = foreground.spawn(pending::<()>());
    assert_eq!(foreground.task_census_snapshot(10).total_live_tasks, 1);

    // Dropping a `Task` handle cancels it by rescheduling its `Runnable` one
    // last time; the future (and so our census guard) is only actually
    // dropped once that rescheduled run happens.
    drop(task);
    assert!(
        try_tick(&foreground),
        "expected cancellation to reschedule a runnable"
    );

    assert_eq!(foreground.task_census_snapshot(10).total_live_tasks, 0);
}
