use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use warp_core::features::FeatureFlag;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::{CloudAmbientAgentEnvironment, CloudEnvironmentCatalog, environment_matches_scope};
use crate::ai::cloud_agent_settings::{CloudAgentSettings, CloudSelectorPreference};
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::cloud_object::CloudObjectLookup as _;
use crate::server::ids::{ServerId, SyncId};
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::{AIClient, FactorySelectorOption};
use crate::server::team_scope::RequestTeamScope;
use crate::workspaces::user_workspaces::TeamScope;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloudSelectorChoice {
    Environment(SyncId),
    Factory {
        uid: String,
        environment_uid: SyncId,
        foreman_agent_uid: String,
    },
}

#[cfg(feature = "local_fs")]
pub(crate) async fn revalidate_factory_choice(
    ai_client: Arc<dyn AIClient>,
    scope: RequestTeamScope,
    choice: &CloudSelectorChoice,
) -> Result<bool> {
    let CloudSelectorChoice::Factory { uid, .. } = choice else {
        return Ok(true);
    };
    let team_uid = scope
        .team_uid()
        .ok_or_else(|| anyhow!("Factory launch requires a team"))?;
    let snapshot = fetch_snapshot(ai_client, scope, team_uid).await?;
    Ok(snapshot
        .factory(uid)
        .is_some_and(|row| &row.choice == choice))
}

impl CloudSelectorChoice {
    pub fn environment_id(&self) -> SyncId {
        match self {
            Self::Environment(id) => *id,
            Self::Factory {
                environment_uid, ..
            } => *environment_uid,
        }
    }

    pub fn preference(&self) -> CloudSelectorPreference {
        match self {
            Self::Environment(id) => CloudSelectorPreference::Environment(*id),
            Self::Factory { uid, .. } => CloudSelectorPreference::Factory(uid.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactorySelectorRow {
    pub choice: CloudSelectorChoice,
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct FactorySelectorSnapshot {
    factories: Vec<FactorySelectorRow>,
    managed_environment_ids: HashSet<SyncId>,
}

impl FactorySelectorSnapshot {
    pub fn factories(&self) -> &[FactorySelectorRow] {
        &self.factories
    }

    pub fn is_managed(&self, id: SyncId) -> bool {
        self.managed_environment_ids.contains(&id)
    }

    pub fn factory(&self, uid: &str) -> Option<&FactorySelectorRow> {
        self.factories.iter().find(|row| {
            matches!(&row.choice, CloudSelectorChoice::Factory { uid: row_uid, .. } if row_uid == uid)
        })
    }
}

#[derive(Clone, Debug)]
pub enum FactorySelectorState {
    Loading,
    Failed,
    Ready(FactorySelectorSnapshot),
}

#[derive(Clone, Copy, Debug)]
pub struct FactorySelectorChanged;

pub struct FactorySelectorCatalog {
    team_uid: Option<ServerId>,
    initialized: bool,
    generation: u64,
    state: FactorySelectorState,
}

impl FactorySelectorCatalog {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&AuthManager::handle(ctx), |catalog, _, event, ctx| {
            if matches!(
                event,
                AuthManagerEvent::AuthComplete
                    | AuthManagerEvent::AuthFailed(_)
                    | AuthManagerEvent::SkippedLogin
                    | AuthManagerEvent::NeedsReauth
            ) {
                catalog.invalidate(ctx);
            }
        });
        Self {
            team_uid: None,
            initialized: false,
            generation: 0,
            state: FactorySelectorState::Loading,
        }
    }

    pub fn state_for(&self, scope: &(impl TeamScope + ?Sized)) -> Option<&FactorySelectorState> {
        (self.initialized && self.team_uid == scope.team_uid()).then_some(&self.state)
    }

    #[cfg(feature = "local_fs")]
    pub fn state_for_team_uid(&self, team_uid: Option<ServerId>) -> Option<&FactorySelectorState> {
        (self.initialized && self.team_uid == team_uid).then_some(&self.state)
    }

    pub fn refresh(&mut self, scope: &impl TeamScope, ctx: &mut ModelContext<Self>) {
        if !FeatureFlag::CloudModeFactorySelector.is_enabled() {
            return;
        }
        self.team_uid = scope.team_uid();
        self.initialized = true;
        self.generation += 1;
        let generation = self.generation;
        let team_uid = self.team_uid;
        self.state = FactorySelectorState::Loading;
        ctx.emit(FactorySelectorChanged);

        let Some(team_uid) = team_uid else {
            self.state = FactorySelectorState::Ready(FactorySelectorSnapshot::default());
            ctx.emit(FactorySelectorChanged);
            return;
        };
        if !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            self.state = FactorySelectorState::Ready(FactorySelectorSnapshot::default());
            ctx.emit(FactorySelectorChanged);
            return;
        }
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();
        let request_scope = RequestTeamScope::from_scope(scope);
        ctx.spawn(
            async move { fetch_snapshot(ai_client, request_scope, team_uid).await },
            move |catalog, result, ctx| {
                if catalog.generation != generation || catalog.team_uid != Some(team_uid) {
                    return;
                }
                catalog.state = match result {
                    Ok(snapshot) => FactorySelectorState::Ready(snapshot),
                    Err(error) => {
                        log::warn!("Factory selector unavailable: {error}");
                        FactorySelectorState::Failed
                    }
                };
                ctx.emit(FactorySelectorChanged);
            },
        );
    }

    pub fn ensure_loaded(&mut self, scope: &impl TeamScope, ctx: &mut ModelContext<Self>) {
        if !self.initialized || self.team_uid != scope.team_uid() {
            self.refresh(scope, ctx);
        }
    }

    fn invalidate(&mut self, ctx: &mut ModelContext<Self>) {
        self.generation += 1;
        self.team_uid = None;
        self.initialized = false;
        self.state = FactorySelectorState::Loading;
        ctx.emit(FactorySelectorChanged);
    }
}

async fn fetch_snapshot(
    ai_client: Arc<dyn AIClient>,
    scope: RequestTeamScope,
    team_uid: ServerId,
) -> Result<FactorySelectorSnapshot> {
    let mut snapshot = FactorySelectorSnapshot::default();
    let mut cursor = None;
    let mut seen_cursors = HashSet::new();
    let mut seen_factories = HashSet::new();
    let mut managed_uids: Option<HashSet<String>> = None;
    loop {
        let page = ai_client
            .get_factory_selector_options(scope, cursor.clone())
            .await?;
        let page_managed = page
            .managed_environment_uids
            .into_iter()
            .collect::<HashSet<_>>();
        if managed_uids
            .as_ref()
            .is_some_and(|uids| uids != &page_managed)
        {
            return Err(anyhow!(
                "Factory managed environments changed during pagination"
            ));
        }
        managed_uids = Some(page_managed);
        for factory in page.factories {
            if factory.team_uid != team_uid.uid()
                || factory.uid.is_empty()
                || factory.foreman_agent_uid.is_empty()
            {
                return Err(anyhow!("Invalid Factory selector response"));
            }
            if seen_factories.insert(factory.uid.clone()) {
                snapshot.factories.push(factory_row(factory)?);
            }
        }
        if !page.page_info.has_next_page {
            if page.page_info.next_cursor.is_some() {
                return Err(anyhow!("Unexpected final Factory page cursor"));
            }
            break;
        }
        let next = page
            .page_info
            .next_cursor
            .filter(|next| !next.is_empty() && seen_cursors.insert(next.clone()))
            .ok_or_else(|| anyhow!("Missing or repeated Factory page cursor"))?;
        cursor = Some(next);
    }
    snapshot.managed_environment_ids = managed_uids
        .unwrap_or_default()
        .into_iter()
        .map(|uid| ServerId::try_from(uid.as_str()).map(SyncId::ServerId))
        .collect::<Result<_, _>>()?;
    Ok(snapshot)
}

fn factory_row(factory: FactorySelectorOption) -> Result<FactorySelectorRow> {
    Ok(FactorySelectorRow {
        choice: CloudSelectorChoice::Factory {
            uid: factory.uid,
            environment_uid: SyncId::ServerId(ServerId::try_from(
                factory.default_environment_uid.as_str(),
            )?),
            foreman_agent_uid: factory.foreman_agent_uid,
        },
        name: factory.name,
        alias: factory.alias,
    })
}

impl FactorySelectorCatalog {
    pub fn visible_environments(
        &self,
        scope: &impl TeamScope,
        ctx: &warpui::AppContext,
    ) -> Vec<super::catalog::CloudEnvironment> {
        let snapshot = match self.state_for(scope) {
            Some(FactorySelectorState::Ready(snapshot)) => snapshot,
            _ => return vec![],
        };
        CloudEnvironmentCatalog::as_ref(ctx)
            .environments()
            .iter()
            .filter(|environment| !snapshot.is_managed(environment.id))
            .filter(|environment| {
                CloudAmbientAgentEnvironment::get_by_id(&environment.id, ctx)
                    .is_some_and(|environment| environment_matches_scope(environment, scope, true))
            })
            .cloned()
            .collect()
    }

    pub fn preferred_choice(
        &self,
        scope: &impl TeamScope,
        ctx: &warpui::AppContext,
    ) -> Option<CloudSelectorChoice> {
        let visible = self.visible_environments(scope, ctx);
        let snapshot = match self.state_for(scope) {
            Some(FactorySelectorState::Ready(snapshot)) => snapshot,
            _ => return None,
        };
        let settings = CloudAgentSettings::as_ref(ctx);
        match settings.cloud_selector_preference(scope) {
            Some(CloudSelectorPreference::Factory(uid)) => {
                if let Some(row) = snapshot.factory(&uid) {
                    return Some(row.choice.clone());
                }
            }
            Some(CloudSelectorPreference::Environment(id)) => {
                if visible.iter().any(|env| env.id == id) {
                    return Some(CloudSelectorChoice::Environment(id));
                }
            }
            None => {
                if let Some(id) = CloudEnvironmentCatalog::as_ref(ctx)
                    .default_environment_id(ctx)
                    .filter(|id| visible.iter().any(|env| env.id == *id))
                {
                    return Some(CloudSelectorChoice::Environment(id));
                }
            }
        }
        visible
            .first()
            .map(|env| CloudSelectorChoice::Environment(env.id))
    }
}

impl Entity for FactorySelectorCatalog {
    type Event = FactorySelectorChanged;
}

impl SingletonEntity for FactorySelectorCatalog {}

#[cfg(test)]
#[path = "factory_selector_tests.rs"]
mod tests;
