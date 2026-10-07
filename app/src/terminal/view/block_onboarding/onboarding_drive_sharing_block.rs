use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    Border, Container, Flex, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::ui_components::button::{ButtonVariant, TextAndIcon, TextAndIconAlignment};
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, Entity, SingletonEntity, View, ViewContext};

use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
use crate::drive::CloudObjectTypeAndId;
use crate::terminal::view::telemetry::SharingDialogSource;
use crate::ui_components::icons::Icon;
use crate::workspace::WorkspaceAction;

/// A rich onboarding block that prompts the user to share a newly-created personal Warp Drive
/// object.
pub struct OnboardingDriveSharingBlock {
    object_id: CloudObjectTypeAndId,
    share_button: MouseStateHandle,
}

impl OnboardingDriveSharingBlock {
    pub fn new(object_id: CloudObjectTypeAndId, ctx: &mut ViewContext<Self>) -> Self {
        // Re-render if the object in the block is renamed.
        ctx.subscribe_to_model(&CloudModel::handle(ctx), |me, _, event, ctx| {
            if let CloudModelEvent::ObjectUpdated { type_and_id, .. } = event
                && &me.object_id == type_and_id
            {
                ctx.notify();
            }
        });

        Self {
            object_id,
            share_button: Default::default(),
        }
    }
}

impl Entity for OnboardingDriveSharingBlock {
    type Event = ();
}

const TITLE_TEXT: &str = "Sharing in Warp Drive";
const BODY_TEXT: &[&str] = &[
    "You can now share drive objects, in Warp or on the web, with anyone - Warp user or not. Click Share in the Warp Drive menu or the pane header to share via link or email.",
    "You’ll be able to modify the access permissions any time.",
];

const BLOCK_PADDING: f32 = 16.;
const BUTTON_WIDTH: f32 = 100.;
const BUTTON_HEIGHT: f32 = 32.;
const BUTTON_FONT_SIZE: f32 = 14.;

impl View for OnboardingDriveSharingBlock {
    fn ui_name() -> &'static str {
        "OnboardingDriveSharingBlock"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let font_family = appearance.monospace_font_family();
        let font_size = appearance.monospace_font_size();

        let header = Container::new(
            Text::new(TITLE_TEXT, font_family, font_size)
                .with_color(appearance.theme().accent().into_solid())
                .with_style(Properties::default().weight(Weight::Bold))
                .finish(),
        )
        .with_padding_bottom(BLOCK_PADDING)
        .finish();

        let mut content = Flex::column().with_child(header);

        for paragraph in BODY_TEXT.iter() {
            content.add_child(
                appearance
                    .ui_builder()
                    .paragraph(*paragraph)
                    .with_style(UiComponentStyles {
                        font_family_id: Some(font_family),
                        font_size: Some(font_size),
                        ..Default::default()
                    })
                    .build()
                    .with_padding_bottom(BLOCK_PADDING)
                    .finish(),
            );
        }

        let button_label = match CloudModel::as_ref(app).get_by_uid(&self.object_id.uid()) {
            Some(object) => format!("Share {}", object.display_name()),
            None => format!("Share this {}", self.object_id.object_type()),
        };
        let object_id = self.object_id;
        let button = appearance
            .ui_builder()
            .button(ButtonVariant::Accent, self.share_button.clone())
            .with_style(UiComponentStyles {
                width: Some(BUTTON_WIDTH),
                height: Some(BUTTON_HEIGHT),
                font_size: Some(BUTTON_FONT_SIZE),
                font_weight: Some(Weight::Medium),
                ..Default::default()
            })
            .with_text_and_icon_label(
                TextAndIcon::new(
                    TextAndIconAlignment::IconFirst,
                    button_label,
                    Icon::Share.to_warpui_icon(appearance.theme().background()),
                    MainAxisSize::Min,
                    MainAxisAlignment::SpaceEvenly,
                    vec2f(BUTTON_FONT_SIZE, BUTTON_FONT_SIZE),
                )
                .with_inner_padding(10.),
            )
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(WorkspaceAction::OpenObjectSharingSettings {
                    object_id,
                    source: SharingDialogSource::OnboardingBlock,
                });
            })
            .finish();
        content.add_child(button);

        Container::new(content.finish())
            .with_uniform_padding(BLOCK_PADDING)
            .with_border(Border::top(1.).with_border_fill(appearance.theme().outline()))
            .finish()
    }
}
