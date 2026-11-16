use super::ModelDropped;
use crate::{App, Entity};

#[test]
fn test_model_spawner() {
    #[derive(Default)]
    struct Model {
        count: usize,
    }

    impl Entity for Model {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let handle = app.add_model(|_| Model::default());

        let task = handle.update(&mut app, |_model, ctx| {
            let spawner = ctx.spawner();

            // Background::spawn requires a 'static future, so this shows that we can move the
            // ModelSpawner without borrowing anything from the model or ModelContext.
            ctx.background_executor().spawn(async move {
                let result = spawner
                    .spawn(move |me, _ctx| {
                        me.count += 1;
                        me.count
                    })
                    .await
                    .expect("Spawn failed");
                assert_eq!(result, 1);

                let result = spawner
                    .spawn(move |me, _ctx| {
                        me.count += 1;
                        me.count
                    })
                    .await
                    .expect("Spawn failed");
                assert_eq!(result, 2);
            })
        });

        task.await.expect("should not fail to join with task");

        handle.read(&app, |model, _| {
            assert_eq!(model.count, 2);
        });
    })
}

#[test]
fn test_model_spawner_dropped_model() {
    #[derive(Default)]
    struct Model {
        count: usize,
    }

    impl Entity for Model {
        type Event = ();
    }

    App::test((), |mut app| async move {
        let handle = app.add_model(|_| Model::default());

        let spawner = handle.update(&mut app, |_model, ctx| ctx.spawner());

        // Explicitly drop the model handle and allow the app to flush effects, removing the task subscriber.
        app.update(|_| drop(handle));

        let result = spawner
            .spawn(|me, _ctx| {
                me.count += 1;
                me.count
            })
            .await;

        assert_eq!(result, Err(ModelDropped));
    })
}
