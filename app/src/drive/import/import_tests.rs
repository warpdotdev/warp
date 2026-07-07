use std::env::current_dir;

use warp_core::ui::appearance::Appearance;
use warpui::App;

use super::expand_dirs;
use crate::cloud_object::model::persistence::CloudModel;
use crate::network::NetworkStatus;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::sync_queue::SyncQueue;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

#[test]
fn test_expand_directories() {
    App::test((), |mut app| async move {
        app.update(crate::settings::init_and_register_user_preferences);

        let global_resource_handles = GlobalResourceHandles::mock(&mut app);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));
        app.add_singleton_model(CloudModel::mock);
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(SyncQueue::mock);
        app.add_singleton_model(TeamTesterStatus::mock);
        app.add_singleton_model(UpdateManager::mock);

        let directory = current_dir()
            .expect("current directory should exist")
            .parent()
            .expect("parent directory should exist")
            .to_path_buf()
            .join("crates")
            .join("integration");

        // Open a folder and verify we could expand it into the correct folder tree structure.
        assert_eq!(warpui::r#async::block_on(expand_dirs([directory].into_iter().collect())).debug_print(), "(integration(tests(INTEGRATION_TESTING, data(test, test_launch_config, test_theme, test_theme_with_name, test_workflow))))");
    });
}
