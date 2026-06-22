use warpui::App;

use super::{NetworkLogItem, NetworkLogModel, NETWORK_LOGGING_MAX_ITEMS};

#[test]
fn empty_snapshot_is_empty_string() {
    App::test((), |app| async move {
        let model = app.add_singleton_model(|_| NetworkLogModel::default());
        model.read(&app, |model, _| {
            assert_eq!(model.snapshot_text(), "");
            assert_eq!(model.len(), 0);
        });
    });
}

#[test]
fn snapshot_joins_items_with_newlines() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| NetworkLogModel::default());
        model.update(&mut app, |model, ctx| {
            model.push(NetworkLogItem::from_string("first"), ctx);
            model.push(NetworkLogItem::from_string("second"), ctx);
            model.push(NetworkLogItem::from_string("third"), ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(model.snapshot_text(), "first\nsecond\nthird");
            assert_eq!(model.len(), 3);
        });
    });
}

#[test]
fn push_beyond_capacity_drops_oldest() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| NetworkLogModel::default());
        // Push exactly the capacity; the snapshot should contain all items
        // and the count should equal the capacity.
        model.update(&mut app, |model, ctx| {
            for i in 0..NETWORK_LOGGING_MAX_ITEMS {
                model.push(NetworkLogItem::from_string(format!("item-{i}")), ctx);
            }
        });
        model.read(&app, |model, _| {
            assert_eq!(model.len(), NETWORK_LOGGING_MAX_ITEMS);
            // Oldest is still present when we're exactly at capacity.
            assert!(model.snapshot_text().starts_with("item-0\n"));
        });

        // Push one more: the oldest item should be evicted so the store stays
        // at capacity, and the snapshot should start at item-1 now.
        model.update(&mut app, |model, ctx| {
            model.push(NetworkLogItem::from_string("overflow"), ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(model.len(), NETWORK_LOGGING_MAX_ITEMS);
            assert!(!model.snapshot_text().contains("item-0\n"));
            assert!(model.snapshot_text().starts_with("item-1\n"));
            assert!(model.snapshot_text().ends_with("\noverflow"));
        });

        // Pushing many additional items keeps the store at capacity.
        model.update(&mut app, |model, ctx| {
            for i in 0..10 {
                model.push(NetworkLogItem::from_string(format!("extra-{i}")), ctx);
            }
        });
        model.read(&app, |model, _| {
            assert_eq!(model.len(), NETWORK_LOGGING_MAX_ITEMS);
        });
    });
}
