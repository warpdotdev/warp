use warp_core::context_flag::ContextFlag;
use warp_core::features::FeatureFlag;
use warpui::ViewContext;

use super::{
    ContentItem, ContentSectionData, FeatureItem, FeatureSection, FeatureSectionData,
    ResourceCenterMainView, Section, Tip, TipAction, TipHint,
};

pub fn sections(ctx: &mut ViewContext<ResourceCenterMainView>) -> Vec<Section> {
    let mut sections = vec![Section::Changelog()];

    if FeatureFlag::AvatarInTabBar.is_enabled() {
        return sections;
    }

    let get_started = FeatureSectionData {
        section_name: FeatureSection::GettingStarted,
        items: vec![
            FeatureItem::new(
                "resource_center.feature.create_first_block.title",
                "resource_center.feature.create_first_block.description",
                Tip::Hint(TipHint::CreateBlock),
                ctx,
            ),
            FeatureItem::new(
                "resource_center.feature.navigate_blocks.title",
                "resource_center.feature.navigate_blocks.description",
                Tip::Hint(TipHint::BlockSelect),
                ctx,
            ),
            FeatureItem::new(
                "resource_center.feature.block_action.title",
                "resource_center.feature.block_action.description",
                Tip::Hint(TipHint::BlockAction),
                ctx,
            ),
            FeatureItem::new(
                "resource_center.feature.command_palette.title",
                "resource_center.feature.command_palette.description",
                Tip::Action(TipAction::CommandPalette),
                ctx,
            ),
            FeatureItem::new(
                "resource_center.feature.theme_picker.title",
                "resource_center.feature.theme_picker.description",
                Tip::Action(TipAction::ThemePicker),
                ctx,
            ),
        ],
    };
    sections.push(Section::Feature(get_started));

    let maximize_warp = FeatureSectionData {
        section_name: FeatureSection::MaximizeWarp,
        items: maximize_warp_items(ctx),
    };
    sections.push(Section::Feature(maximize_warp));

    let advanced_setup = ContentSectionData {
        section_name: FeatureSection::AdvancedSetup,
        items: vec![
            ContentItem {
                title: "resource_center.content.custom_prompt.title",
                description: "resource_center.content.custom_prompt.description",
                url: "https://docs.warp.dev/terminal/appearance/prompt",
                button_label: "resource_center.content.view_documentation",
            },
            ContentItem {
                title: "resource_center.content.ide_integration.title",
                description: "resource_center.content.ide_integration.description",
                url: "https://docs.warp.dev/terminal/integrations-and-plugins",
                button_label: "resource_center.content.view_documentation",
            },
            ContentItem {
                title: "resource_center.content.how_warp_uses_warp.title",
                description: "resource_center.content.how_warp_uses_warp.description",
                url: "https://www.warp.dev/blog/how-warp-uses-warp",
                button_label: "resource_center.content.read_article",
            },
        ],
    };
    sections.push(Section::Content(advanced_setup));

    sections
}

fn maximize_warp_items(ctx: &mut ViewContext<ResourceCenterMainView>) -> Vec<FeatureItem> {
    let mut maximize_warp_items = vec![];

    maximize_warp_items.push(FeatureItem::new(
        "resource_center.feature.command_search.title",
        "resource_center.feature.command_search.description",
        Tip::Action(TipAction::CommandSearch),
        ctx,
    ));

    maximize_warp_items.push(FeatureItem::new(
        "resource_center.feature.ai_command_search.title",
        "resource_center.feature.ai_command_search.description",
        Tip::Action(TipAction::AiCommandSearch),
        ctx,
    ));

    if ContextFlag::CreateNewSession.is_enabled() {
        maximize_warp_items.push(FeatureItem::new(
            "resource_center.feature.split_panes.title",
            "resource_center.feature.split_panes.description",
            Tip::Action(TipAction::SplitPane),
            ctx,
        ));
    }

    if ContextFlag::LaunchConfigurations.is_enabled() {
        maximize_warp_items.push(FeatureItem::new(
            "resource_center.feature.launch_configuration.title",
            "resource_center.feature.launch_configuration.description",
            Tip::Action(TipAction::SaveNewLaunchConfig),
            ctx,
        ));
    }

    maximize_warp_items
}
