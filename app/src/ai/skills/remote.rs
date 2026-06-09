use remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use warp_core::features::FeatureFlag;
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::{AppContext, ModelContext, SingletonEntity};

use super::bundled::BundledSkill;
use super::SkillManager;

pub(crate) fn wire_remote_bundled_skills(ctx: &mut AppContext) {
    SkillManager::handle(ctx).update(ctx, |manager, ctx| {
        manager.subscribe_to_remote_bundled_skills(ctx);
    });
}

impl SkillManager {
    fn subscribe_to_remote_bundled_skills(&mut self, ctx: &mut ModelContext<Self>) {
        let remote_server_manager = RemoteServerManager::handle(ctx);
        ctx.subscribe_to_model(&remote_server_manager, |me, event, ctx| match event {
            RemoteServerManagerEvent::HostConnected { host_id } => {
                me.bootstrap_remote_bundled_skill(host_id.clone(), ctx);
            }
            RemoteServerManagerEvent::HostDisconnected { host_id } => {
                me.remove_remote_bundled_skill(host_id);
            }
            RemoteServerManagerEvent::SessionConnecting { .. }
            | RemoteServerManagerEvent::SessionConnected { .. }
            | RemoteServerManagerEvent::SessionConnectionFailed { .. }
            | RemoteServerManagerEvent::SessionDisconnected { .. }
            | RemoteServerManagerEvent::SessionReconnected { .. }
            | RemoteServerManagerEvent::SessionDeregistered { .. }
            | RemoteServerManagerEvent::NavigatedToDirectory { .. }
            | RemoteServerManagerEvent::RepoMetadataSnapshot { .. }
            | RemoteServerManagerEvent::RepoMetadataUpdated { .. }
            | RemoteServerManagerEvent::RepoMetadataDirectoryLoaded { .. }
            | RemoteServerManagerEvent::CodebaseIndexStatusesSnapshot { .. }
            | RemoteServerManagerEvent::CodebaseIndexStatusUpdated { .. }
            | RemoteServerManagerEvent::BufferUpdated { .. }
            | RemoteServerManagerEvent::BufferConflictDetected { .. }
            | RemoteServerManagerEvent::DiffStateSnapshotReceived { .. }
            | RemoteServerManagerEvent::DiffStateMetadataUpdateReceived { .. }
            | RemoteServerManagerEvent::DiffStateFileDeltaReceived { .. }
            | RemoteServerManagerEvent::GetBranchesResponse { .. }
            | RemoteServerManagerEvent::SetupStateChanged { .. }
            | RemoteServerManagerEvent::BinaryCheckComplete { .. }
            | RemoteServerManagerEvent::BinaryInstallComplete { .. }
            | RemoteServerManagerEvent::ClientRequestFailed { .. }
            | RemoteServerManagerEvent::CodebaseIndexMutationFailed { .. }
            | RemoteServerManagerEvent::ServerMessageDecodingError { .. } => {}
        });

        let connected_host_ids = remote_server_manager
            .as_ref(ctx)
            .connected_host_ids()
            .cloned()
            .collect::<Vec<_>>();
        for host_id in connected_host_ids {
            self.bootstrap_remote_bundled_skill(host_id, ctx);
        }
    }

    fn bootstrap_remote_bundled_skill(&mut self, host_id: HostId, ctx: &mut ModelContext<Self>) {
        if !FeatureFlag::BundledSkills.is_enabled() {
            return;
        }
        let Some(advertised_dir) = RemoteServerManager::as_ref(ctx)
            .bundled_resources_dir_for_host(&host_id)
            .map(str::to_owned)
        else {
            log::warn!("Remote host {host_id} did not advertise a bundled resources directory");
            return;
        };
        let Ok(resources_dir) = StandardizedPath::try_new(&advertised_dir) else {
            log::warn!(
                "Remote host {host_id} advertised an invalid bundled resources directory: {advertised_dir}"
            );
            return;
        };
        let resources_dir =
            LocalOrRemotePath::Remote(RemotePath::new(host_id.clone(), resources_dir));

        ctx.spawn(
            BundledSkill::detect_for_resources_dir(resources_dir),
            move |me, bundled_skill, ctx| {
                // Stale-completion guard: the advertised directory is cleared on
                // final disconnect and may change across reconnects (e.g. after
                // a daemon upgrade). Only install a catalog rendered against the
                // host's current resources.
                if RemoteServerManager::as_ref(ctx).bundled_resources_dir_for_host(&host_id)
                    != Some(advertised_dir.as_str())
                {
                    return;
                }
                me.set_remote_bundled_skill(host_id, bundled_skill);
            },
        );
    }
}
