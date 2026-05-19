use super::{NetworkLogItem, NetworkLogModel, NETWORK_LOGGING_MAX_ITEMS};
use warpui::App;

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
fn snapshot_joins_items_with_blank_lines() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| NetworkLogModel::default());
        model.update(&mut app, |model, ctx| {
            model.push(NetworkLogItem::from_string("first"), ctx);
            model.push(NetworkLogItem::from_string("second"), ctx);
            model.push(NetworkLogItem::from_string("third"), ctx);
        });
        model.read(&app, |model, _| {
            let snapshot = model.snapshot();
            assert_eq!(snapshot.display_text, "first\n\nsecond\n\nthird");
            assert_eq!(snapshot.plain_text, "first\n\nsecond\n\nthird");
            assert_eq!(model.snapshot_text(), "first\n\nsecond\n\nthird");
            assert_eq!(model.len(), 3);
        });
    });
}

#[test]
fn push_beyond_capacity_drops_oldest() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| NetworkLogModel::default());
        model.update(&mut app, |model, ctx| {
            for i in 0..NETWORK_LOGGING_MAX_ITEMS {
                model.push(NetworkLogItem::from_string(format!("item-{i}")), ctx);
            }
        });
        model.read(&app, |model, _| {
            assert_eq!(model.len(), NETWORK_LOGGING_MAX_ITEMS);
            assert!(model.snapshot_text().starts_with("item-0\n\n"));
        });

        model.update(&mut app, |model, ctx| {
            model.push(NetworkLogItem::from_string("overflow"), ctx);
        });
        model.read(&app, |model, _| {
            assert_eq!(model.len(), NETWORK_LOGGING_MAX_ITEMS);
            assert!(!model.snapshot_text().contains("item-0\n\n"));
            assert!(model.snapshot_text().starts_with("item-1\n\n"));
            assert!(model.snapshot_text().ends_with("\n\noverflow"));
        });

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

#[test]
fn snapshot_preserves_plain_text_for_copying() {
    App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_| NetworkLogModel::default());
        model.update(&mut app, |model, ctx| {
            model.push(NetworkLogItem::from_string("line one\nline two"), ctx);
        });
        model.read(&app, |model, _| {
            let snapshot = model.snapshot();
            assert_eq!(snapshot.display_text, "line one\nline two");
            assert_eq!(snapshot.plain_text, "line one\nline two");
        });
    });
}
