use warpui::{Entity, ModelContext, SingletonEntity};

use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::AuthStateProvider;
use crate::network::{NetworkStatus, NetworkStatusEvent, NetworkStatusKind};
use crate::report_error;
use crate::server::server_api::ai::ConnectedSelfHostedWorker;
use crate::server::server_api::ServerApiProvider;
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};
pub const WARP_WORKER_HOST: &str = "warp";

pub enum ConnectedSelfHostedWorkersEvent {
    Changed,
}

pub struct ConnectedSelfHostedWorkersModel {
    workers: Vec<ConnectedSelfHostedWorker>,
}

impl ConnectedSelfHostedWorkersModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&NetworkStatus::handle(ctx), |me, _, event, ctx| {
            if let NetworkStatusEvent::NetworkStatusChanged {
                new_status: NetworkStatusKind::Online,
            } = event
            {
                me.refresh(ctx);
            }
        });

        ctx.subscribe_to_model(&AuthManager::handle(ctx), |me, _, event, ctx| match event {
            AuthManagerEvent::AuthComplete => {
                me.refresh(ctx);
            }
            AuthManagerEvent::AuthFailed(_)
            | AuthManagerEvent::SkippedLogin
            | AuthManagerEvent::NeedsReauth => {
                me.clear_workers(ctx);
            }
            AuthManagerEvent::CreateAnonymousUserFailed
            | AuthManagerEvent::AttemptedLoginGatedFeature { .. }
            | AuthManagerEvent::LoginOverrideDetected(_)
            | AuthManagerEvent::MintCustomTokenFailed(_)
            | AuthManagerEvent::ReceivedDeviceAuthorizationCode { .. } => {}
        });

        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _, event, ctx| {
            if let UserWorkspacesEvent::TeamsChanged = event {
                me.refresh(ctx);
            }
        });

        let mut me = Self {
            workers: Vec::new(),
        };
        me.refresh(ctx);
        me
    }

    pub fn worker_hosts_excluding(&self, excluded: Option<&str>) -> Vec<String> {
        let mut hosts: Vec<String> = self
            .workers
            .iter()
            .map(|worker| worker.worker_host.clone())
            .filter(|host| !host.trim().is_empty())
            .filter(|host| !host.eq_ignore_ascii_case(WARP_WORKER_HOST))
            .filter(|host| match excluded {
                Some(excluded) => !host.eq_ignore_ascii_case(excluded),
                None => true,
            })
            .collect();
        hosts.sort();
        hosts.dedup();
        hosts
    }

    pub fn refresh(&mut self, ctx: &mut ModelContext<Self>) {
        if !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            self.clear_workers(ctx);
            return;
        }

        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        ctx.spawn(
            async move { ai_client.list_connected_self_hosted_workers().await },
            |me, result, ctx| match result {
                Ok(response) => {
                    let mut workers = response.workers;
                    workers.sort_by(|left, right| left.worker_host.cmp(&right.worker_host));
                    if workers != me.workers {
                        me.workers = workers;
                        ctx.emit(ConnectedSelfHostedWorkersEvent::Changed);
                    }
                }
                Err(e) => {
                    report_error!(e.context("Failed to fetch connected self-hosted workers"));
                }
            },
        );
    }

    fn clear_workers(&mut self, ctx: &mut ModelContext<Self>) {
        if self.clear_worker_cache() {
            ctx.emit(ConnectedSelfHostedWorkersEvent::Changed);
        }
    }

    fn clear_worker_cache(&mut self) -> bool {
        if self.workers.is_empty() {
            return false;
        }
        self.workers.clear();
        true
    }
}

impl Entity for ConnectedSelfHostedWorkersModel {
    type Event = ConnectedSelfHostedWorkersEvent;
}

impl SingletonEntity for ConnectedSelfHostedWorkersModel {}

#[cfg(test)]
#[path = "connected_self_hosted_workers_tests.rs"]
mod tests;
