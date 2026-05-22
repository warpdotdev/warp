use std::fs;

use criterion::{Criterion, criterion_group, criterion_main};
use rand::SeedableRng;
use rand::rngs::StdRng;
use warp_editor::content::buffer::Buffer;
use warp_editor::content::selection_model::BufferSelectionModel;
use warp_editor::content::text::IndentBehavior;
use warpui::{App, ModelHandle};

const EDIT_SAMPLE_SIZE: usize = 10;
const MAX_EDIT_REPLACEMENT_LENGTH: usize = 20;
const MAX_INSERTED_TEXT_LENGTH: usize = 10;
const MAX_STYLE_TEXT_LENGTH: usize = 20;

fn mock_buffer(
    text: String,
    app: &mut App,
) -> (ModelHandle<Buffer>, ModelHandle<BufferSelectionModel>) {
    let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
    let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

    buffer.update(app, |buffer, ctx| {
        *buffer = Buffer::from_plain_text(
            &text,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            selection.clone(),
            ctx,
        );
    });
    (buffer, selection)
}

fn initiate_buffer(text: String) {
    App::test((), |mut app| async move {
        mock_buffer(text, &mut app);
    });
}

fn read_text(text: String) {
    App::test((), |mut app| async move {
        let (buffer, _) = mock_buffer(text, &mut app);
        buffer.read(&app, |buffer, _| buffer.text().into_string());
    });
}

fn edit_text(text: String) {
    App::test((), |mut app| async move {
        let (buffer, selection_model) = mock_buffer(text, &mut app);
        buffer.read(&app, |buffer, _| buffer.text().into_string());

        let mut seed = StdRng::seed_from_u64(50);
        buffer.update(&mut app, |buffer, ctx| {
            buffer.random_edit(
                EDIT_SAMPLE_SIZE,
                &mut seed,
                MAX_EDIT_REPLACEMENT_LENGTH,
                MAX_INSERTED_TEXT_LENGTH,
                selection_model,
                ctx,
            );
        });
    });
}

fn style_text(text: String) {
    // Disabled: random_style requires ModelHandle and ModelContext which are complex for benchmarks
    // let mut seed = StdRng::seed_from_u64(50);
    // buffer.random_style(EDIT_SAMPLE_SIZE, &mut seed, MAX_STYLE_TEXT_LENGTH, selection_model, ctx);
    App::test((), |mut app| async move {
        let (buffer, selection_model) = mock_buffer(text, &mut app);
        buffer.read(&app, |buffer, _| buffer.text().into_string());

        let mut seed = StdRng::seed_from_u64(50);
        buffer.update(&mut app, |buffer, ctx| {
            buffer.random_style(
                EDIT_SAMPLE_SIZE,
                &mut seed,
                MAX_STYLE_TEXT_LENGTH,
                selection_model,
                ctx,
            );
        });
    });
}

fn criterion_benchmark(c: &mut Criterion) {
    let text = fs::read_to_string("test_data/test_rust_file.rs").expect("Should work");
    c.bench_function("initiate_buffer", |b| {
        b.iter(|| initiate_buffer(text.clone()))
    });
    c.bench_function("read_text", |b| b.iter(|| read_text(text.clone())));
    c.bench_function("edit_text", |b| b.iter(|| edit_text(text.clone())));
    c.bench_function("style_text", |b| b.iter(|| style_text(text.clone())));
}

criterion_group!(
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = criterion_benchmark
);
criterion_main!(benches);
