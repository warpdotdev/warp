use warpui::{text_layout::TextStyle, App};

use crate::{
    appearance::Appearance,
    terminal::{
        input::{
            decorations::InputBackgroundJobOptions,
            tests::{
                add_window_with_bootstrapped_terminal, initialize_app,
                simulate_directory_for_completion,
            },
        },
        model::session::SessionInfo,
    },
    themes::theme::AnsiColorIdentifier,
};
use warp_completer::completer::SuggestionTypeName;

#[test]
fn test_decorations_with_multibyte_chars() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal_colors_normal = app
            .get_singleton_model_handle::<Appearance>()
            .read(&app, |a, _| a.theme().terminal_colors().normal.to_owned());
        let error_underline_color = AnsiColorIdentifier::Red
            .to_ansi_color(&terminal_colors_normal)
            .into();
        let command_color = AnsiColorIdentifier::from(SuggestionTypeName::Command)
            .to_ansi_color(&terminal_colors_normal)
            .into();

        let session_info = SessionInfo::new_for_test();
        let session_id = session_info.session_id;

        let terminal =
            add_window_with_bootstrapped_terminal(&mut app, None, Some(session_info)).await;
        let input = terminal.read(&app, |view, _| view.input().clone());
        let editor = input.read(&app, |input, _| input.editor.clone());

        let session_id = terminal.update(&mut app, |terminal_view, ctx| {
            terminal_view
                .sessions_model()
                .update(ctx, |sessions, _ctx| {
                    // Wait until external commands have been loaded.
                    let session = sessions.get(session_id).expect("session should exist");
                    warpui::r#async::block_on(session.load_external_commands());
                });
            session_id
        });

        simulate_directory_for_completion(session_id, &terminal, &mut app, "/usr/bin");

        input
            .update(&mut app, |input, ctx| {
                input.clear_buffer_and_reset_undo_stack(ctx);
                input.user_insert("echo 'multibyte: שלום עולם'\nechoo hello world", ctx);

                // Trigger input decoration computation instead of waiting for the
                // debounced stream to trigger.
                input.run_input_background_jobs(
                    InputBackgroundJobOptions::default().with_command_decoration(),
                    ctx,
                );

                // Take the future handle from the input to ensure that it doesn't
                // get cancelled by the debounced decoration update stream.
                let future_handle = input
                    .decorations_future_handle
                    .take()
                    .expect("should have spawned decoration task");
                ctx.await_spawned_future(future_handle.future_id())
            })
            .await;

        let text_style_runs = editor.read(&app, |editor, ctx| {
            editor
                .text_style_runs(ctx)
                .map(|run| (run.text().to_owned(), run.text_style()))
                .collect::<Vec<_>>()
        });

        let expected = &[
            (
                "echo".to_string(),
                TextStyle::new().with_syntax_color(command_color),
            ),
            (" 'multibyte: שלום עולם'\n".to_string(), Default::default()),
            (
                "echoo".to_string(),
                TextStyle::new().with_error_underline_color(error_underline_color),
            ),
            (" hello world".to_string(), Default::default()),
        ];
        assert_eq!(
            text_style_runs, expected,
            "---- Expected ----\n{expected:#?}\n---- Actual ----\n{text_style_runs:#?}\n"
        );
    });
}
