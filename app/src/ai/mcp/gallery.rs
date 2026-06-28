use std::collections::HashMap;

use uuid::Uuid;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::mcp::templatable::{
    GalleryData, JsonTemplate, TemplatableMCPServer, TemplateVariable,
};
use crate::server::cloud_objects::update_manager::{UpdateManager, UpdateManagerEvent};

#[derive(Clone, Debug)]
pub struct GalleryMCPServer {
    uuid: Uuid,
    title: String,
    description: String,
    #[allow(dead_code)]
    version: i32,
    #[allow(dead_code)]
    instructions_in_markdown: Option<String>,
    json_template: JsonTemplate,
}

impl GalleryMCPServer {
    pub fn new(
        uuid: Uuid,
        title: String,
        description: String,
        version: i32,
        instructions_in_markdown: Option<String>,
        json_template: JsonTemplate,
    ) -> Self {
        Self {
            uuid,
            title,
            description,
            version,
            instructions_in_markdown,
            json_template,
        }
    }

    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    pub fn title(&self) -> String {
        self.title.clone()
    }

    pub fn description(&self) -> String {
        self.description.clone()
    }

    pub fn version(&self) -> i32 {
        self.version
    }

    pub fn json_template(&self) -> &JsonTemplate {
        &self.json_template
    }

    pub fn instructions_in_markdown(&self) -> Option<&String> {
        self.instructions_in_markdown.as_ref()
    }
}

impl TryFrom<GalleryMCPServer> for TemplatableMCPServer {
    type Error = String;

    fn try_from(gallery_server: GalleryMCPServer) -> Result<Self, Self::Error> {
        let GalleryMCPServer {
            uuid: gallery_uuid,
            title,
            description,
            version: gallery_version,
            instructions_in_markdown: _,
            json_template,
        } = gallery_server;

        Ok(TemplatableMCPServer {
            uuid: Uuid::new_v4(),
            name: title,
            description: Some(description),
            template: json_template,
            version: chrono::Local::now().timestamp(),
            gallery_data: Some(GalleryData {
                gallery_item_id: gallery_uuid,
                version: gallery_version,
            }),
        })
    }
}

pub struct MCPGalleryManager {
    gallery_items: HashMap<Uuid, GalleryMCPServer>,
    templatable_mcp_servers: HashMap<Uuid, TemplatableMCPServer>,
}

impl MCPGalleryManager {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let gallery_manager = Self {
            gallery_items: Default::default(),
            templatable_mcp_servers: Default::default(),
        };

        // Subscribe to UpdateManager events to receive MCP gallery updates
        let update_manager = UpdateManager::handle(ctx);
        ctx.subscribe_to_model(&update_manager, |me, event, ctx| {
            if let UpdateManagerEvent::MCPGalleryUpdated { templates } = event {
                me.update_gallery_items(templates.clone(), ctx);
            }
        });

        gallery_manager
    }

    pub fn get_gallery(&self) -> Vec<GalleryMCPServer> {
        self.gallery_items.values().cloned().collect()
    }

    pub fn get_gallery_item(&self, gallery_uuid: Uuid) -> Option<&GalleryMCPServer> {
        self.gallery_items.get(&gallery_uuid)
    }

    pub fn get_templatable_mcp_server(&self, gallery_uuid: Uuid) -> Option<&TemplatableMCPServer> {
        self.templatable_mcp_servers.get(&gallery_uuid)
    }

    /// Update gallery items from the server response
    pub fn update_gallery_items(
        &mut self,
        templates: Vec<warp_graphql::mcp_gallery_template::MCPGalleryTemplate>,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut gallery_items = HashMap::new();
        let mut templatable_mcp_servers = HashMap::new();

        for gallery_template in templates {
            let Ok(uuid) = Uuid::parse_str(&gallery_template.gallery_item_id) else {
                log::debug!(
                    "Failed to parse uuid for gallery item {}",
                    gallery_template.gallery_item_id
                );
                continue;
            };

            let json_template = JsonTemplate {
                json: gallery_template.json_template.json,
                variables: gallery_template
                    .json_template
                    .variables
                    .into_iter()
                    .map(|v| TemplateVariable {
                        key: v.key,
                        allowed_values: v.allowed_values,
                    })
                    .collect(),
            };

            let gallery_item = GalleryMCPServer::new(
                uuid,
                gallery_template.title,
                gallery_template.description,
                gallery_template.version,
                gallery_template.instructions_in_markdown,
                json_template,
            );

            let Ok(templatable_mcp_server): Result<TemplatableMCPServer, _> =
                gallery_item.clone().try_into()
            else {
                log::debug!("Failed to parse template for gallery item {}", uuid);
                continue;
            };

            gallery_items.insert(uuid, gallery_item);
            templatable_mcp_servers.insert(uuid, templatable_mcp_server);
        }

        self.gallery_items = gallery_items;
        self.templatable_mcp_servers = templatable_mcp_servers;

        ctx.emit(MCPGalleryManagerEvent::ItemsRefreshed);
    }
}

pub enum MCPGalleryManagerEvent {
    ItemsRefreshed,
}

impl Entity for MCPGalleryManager {
    type Event = MCPGalleryManagerEvent;
}

impl SingletonEntity for MCPGalleryManager {}
