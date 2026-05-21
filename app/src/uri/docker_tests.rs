use warpui::App;

use crate::{
    auth::{auth_manager::AuthManager, AuthStateProvider},
    server::{
        server_api::ServerApiProvider, telemetry::context_provider::AppTelemetryContextProvider,
    },
};

use super::*;

#[test]
// Tests behavior based on which query parameters are required.
fn test_open_docker_container() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
        app.add_singleton_model(AuthManager::new_for_test);

        let base_url = Url::parse("warplocal://action/docker/open_subshell")
            .expect("base url should be successfully parsed");

        let container_id = (
            "container_id",
            "85aa47c9ef3fbd338cb3cfe45c99eaed0c57f374c3c47bf3da3e44fd6b5c3399",
        );
        let shell_path = ("shell", "/bin/bash");
        let user = ("user", "root");

        let mut container_and_shell = base_url.to_owned();
        container_and_shell
            .query_pairs_mut()
            .append_pair(container_id.0, container_id.1)
            .append_pair(shell_path.0, shell_path.1);
        let mut container_shell_user = base_url.to_owned();
        container_shell_user
            .query_pairs_mut()
            .append_pair(container_id.0, container_id.1)
            .append_pair(shell_path.0, shell_path.1)
            .append_pair(user.0, user.1);
        let mut missing_container_id = base_url.to_owned();
        missing_container_id
            .query_pairs_mut()
            .append_pair(shell_path.0, shell_path.1);
        let mut missing_shell = base_url.to_owned();
        missing_shell
            .query_pairs_mut()
            .append_pair(container_id.0, container_id.1);

        app.update(|app_ctx| {
            assert!(open_docker_container(&container_shell_user, app_ctx).is_ok());
            assert!(open_docker_container(&container_and_shell, app_ctx).is_ok());
            assert!(open_docker_container(&missing_shell, app_ctx).is_err());
            assert!(open_docker_container(&missing_container_id, app_ctx).is_err());
        });
    });
}
