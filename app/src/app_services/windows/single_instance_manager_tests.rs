use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use warpui::r#async::block_on;
use warpui::r#async::executor::Background;

use super::super::service_impl::connect_to_sole_running_instance;
use super::{InstanceRole, bind_uri_listener, claim_instance, mutex_exists, try_acquire_mutex};

/// Names unique to this process and call, so a test never collides with another test or with a
/// Warp instance running on the same machine.
fn unique_object_names() -> (String, String) {
    static NEXT_ID: AtomicU32 = AtomicU32::new(0);
    let id = format!(
        "{}_{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    );
    (
        format!("Local\\WarpTest{id}_SingleInstance"),
        format!("WarpTest{id}_URI_CHANNEL"),
    )
}

/// Connects the way a redirected launch does, so the probe exercises the real retry path rather
/// than a simplified stand-in.
fn listener_is_reachable(pipe_name: &str) -> bool {
    let executor = Arc::new(Background::new(1, |_| "uri-pipe-probe".to_owned()));
    block_on(connect_to_sole_running_instance(pipe_name, executor)).is_ok()
}

/// A launch told that another instance owns the claim must be able to reach that instance.
#[test]
fn a_secondary_claim_has_a_listener_to_reach() {
    let (mutex_name, pipe_name) = unique_object_names();

    let sole = claim_instance(&mutex_name, &pipe_name).expect("first claim should succeed");
    assert!(
        matches!(sole, InstanceRole::Sole(_)),
        "the first claim on unused names should take the role"
    );

    let secondary = claim_instance(&mutex_name, &pipe_name).expect("second claim should succeed");
    assert!(
        matches!(secondary, InstanceRole::Secondary),
        "a second claim should defer to the instance holding the role"
    );
    assert!(
        listener_is_reachable(&pipe_name),
        "being told another instance owns the claim must imply that instance is reachable"
    );

    drop(sole);
}

/// The interleaving that decides whether the ordering is safe: a claimant that has taken the mutex
/// but has not yet bound its listener must still be reported as an existing instance. Concluding
/// "no instance" here is what starts a duplicate, and it is why the mutex is acquired before the
/// pipe is bound rather than after.
#[test]
fn a_claim_caught_before_its_listener_binds_still_reports_an_existing_instance() {
    let (mutex_name, pipe_name) = unique_object_names();

    // Stop the first claimant exactly between its two steps by taking the mutex without binding.
    let mid_claim = try_acquire_mutex(&mutex_name)
        .expect("acquiring the mutex should not fail")
        .expect("the mutex should be unheld");

    let concurrent = claim_instance(&mutex_name, &pipe_name)
        .expect("claiming while another process is mid-claim should not fail");
    assert!(
        matches!(concurrent, InstanceRole::Secondary),
        "a launch arriving mid-claim must defer to the claimant, not start its own instance"
    );

    // Completing the first claimant's second step makes the listener reachable, which is what the
    // deferring launch's connect budget is there to wait for.
    let listener = bind_uri_listener(&pipe_name).expect("binding should succeed");
    assert!(listener_is_reachable(&pipe_name));

    drop(listener);
    drop(mid_claim);
}

/// Teardown is the mirror of the claim, and gets the ordering the other way round: the listener
/// has to be gone before the role is offered back. Releasing the mutex first lets the next launch
/// take a role it then cannot bind, stranding it with neither the claim nor a listener while the
/// old pipe is still closing - and a launch after that claims cleanly, leaving two full instances.
#[test]
fn teardown_does_not_strand_the_next_launch() {
    let (mutex_name, pipe_name) = unique_object_names();

    let sole = claim_instance(&mutex_name, &pipe_name).expect("first claim should succeed");
    assert!(matches!(sole, InstanceRole::Sole(_)));
    drop(sole);

    let next =
        claim_instance(&mutex_name, &pipe_name).expect("claim after teardown should succeed");
    assert!(
        matches!(next, InstanceRole::Sole(_)),
        "a launch after teardown must be able to take the role outright, not land undiscoverable"
    );
    assert!(
        listener_is_reachable(&pipe_name),
        "the launch that took the role must be reachable"
    );

    drop(next);
}

/// A process that cannot listen must not leave a claim behind, or the next launch would defer to
/// something it can never reach.
#[test]
fn a_process_that_cannot_listen_leaves_no_claim() {
    let (blocking_mutex_name, pipe_name) = unique_object_names();
    let (mutex_name, _) = unique_object_names();

    // Hold the pipe under a different mutex so the next claim takes its mutex and then fails to
    // bind, which is the shape of a process whose listener could not be created.
    let blocking = claim_instance(&blocking_mutex_name, &pipe_name).expect("claim should succeed");
    assert!(matches!(blocking, InstanceRole::Sole(_)));

    let undiscoverable = claim_instance(&mutex_name, &pipe_name)
        .expect("claiming without a listener should not fail");
    assert!(
        matches!(undiscoverable, InstanceRole::Undiscoverable),
        "failing to bind should not leave this process holding the role"
    );
    assert!(
        !mutex_exists(&mutex_name),
        "a process that cannot listen must release the mutex it took"
    );

    drop(blocking);
}
