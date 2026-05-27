use warpui::App;

use super::*;
use crate::terminal::model::session::ExecuteCommandOptions;
use crate::terminal::shell::ShellType;

async fn execute_test_command<F>(
    executor: Arc<TmuxCommandExecutor>,
    command: &'static str,
    assert_result_fn: F,
) where
    F: FnOnce(Result<CommandOutput>) + Send + 'static,
{
    let shell = Shell::new(ShellType::Zsh, None, None, Default::default(), None);
    let test_command_result = executor
        .execute_command(
            command,
            &shell,
            /*current_directory_path=*/ None,
            /*environment_variables=*/ None,
            ExecuteCommandOptions::default(),
        )
        .await;

    assert_result_fn(test_command_result);
}

/// Returns a closure that asserts the given `Result<CommandOutput>` is `Ok(..)` and contains
/// `CommandOutput` with the given `expected_output` and `expected_success` values.
fn assert_command_output_result_fn(
    expected_output: &'static str,
    expected_success: bool,
) -> impl FnOnce(Result<CommandOutput>) {
    move |result| {
        assert!(result.is_ok());

        let output = result.unwrap();
        assert_eq!(output.success(), expected_success);
        assert_eq!(output.output(), expected_output.as_bytes());
    }
}

#[test]
fn test_emits_successful_command_output() {
    App::test((), |_app| async move {
        let (executor_command_tx, _) = async_channel::unbounded();
        let executor = Arc::new(TmuxCommandExecutor::new(executor_command_tx));

        let task_executor = async_executor::LocalExecutor::new();

        let execute_command_future = task_executor.spawn(execute_test_command(
            executor.clone(),
            "echo foo",
            assert_command_output_result_fn("foo", true),
        ));
        let handle_command_output_future = task_executor.spawn(async move {
            let test_command_id = executor
                .in_flight_commands
                .lock()
                .keys()
                .next()
                .expect("Executor should be running test command.")
                .to_string();
            executor.handle_executed_command_event(ExecutedExecutorCommandEvent {
                command_id: test_command_id,
                exit_code: 0,
                output: "foo".as_bytes().to_vec(),
            });
        });

        task_executor
            .run(async move {
                execute_command_future.await;
                handle_command_output_future.await;
            })
            .await;
    });
}
