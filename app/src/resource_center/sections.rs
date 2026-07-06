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
                crate::menu_label(
                    "resource_center.getting_started.create_block_title",
                    "Create your first block",
                ),
                crate::menu_label(
                    "resource_center.getting_started.create_block_description",
                    "Run a command to see your command and output grouped.",
                ),
                Tip::Hint(TipHint::CreateBlock),
                ctx,
            ),
            FeatureItem::new(
                crate::menu_label(
                    "resource_center.getting_started.navigate_blocks_title",
                    "Navigate blocks",
                ),
                crate::menu_label(
                    "resource_center.getting_started.navigate_blocks_description",
                    "Click to select a block and navigate with arrow keys.",
                ),
                Tip::Hint(TipHint::BlockSelect),
                ctx,
            ),
            FeatureItem::new(
                crate::menu_label(
                    "resource_center.getting_started.block_action_title",
                    "Take an action on block",
                ),
                crate::menu_label(
                    "resource_center.getting_started.block_action_description",
                    "Right click on a block to copy/paste, share, more.",
                ),
                Tip::Hint(TipHint::BlockAction),
                ctx,
            ),
            FeatureItem::new(
                crate::menu_label(
                    "resource_center.getting_started.command_palette_title",
                    "Open command palette",
                ),
                crate::menu_label(
                    "resource_center.getting_started.command_palette_description",
                    "Access all of Warp via the keyboard.",
                ),
                Tip::Action(TipAction::CommandPalette),
                ctx,
            ),
            FeatureItem::new(
                crate::menu_label(
                    "resource_center.getting_started.set_theme_title",
                    "Set your theme",
                ),
                crate::menu_label(
                    "resource_center.getting_started.set_theme_description",
                    "Make Warp your own by choosing a theme.",
                ),
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
                title: crate::menu_label(
                    "resource_center.advanced_setup.custom_prompt_title",
                    "Use your custom prompt",
                ),
                description: crate::menu_label(
                    "resource_center.advanced_setup.custom_prompt_description",
                    "Set up Warp to honor your PS1 setting",
                ),
                url: "https://docs.warp.dev/terminal/appearance/prompt",
                button_label: crate::menu_label(
                    "resource_center.advanced_setup.button_view_documentation",
                    "View documentation",
                ),
            },
            ContentItem {
                title: crate::menu_label(
                    "resource_center.advanced_setup.ide_integration_title",
                    "Integrate Warp with your IDE",
                ),
                description: crate::menu_label(
                    "resource_center.advanced_setup.ide_integration_description",
                    "Configure Warp to launch from your most used development tools",
                ),
                url: "https://docs.warp.dev/terminal/integrations-and-plugins",
                button_label: crate::menu_label(
                    "resource_center.advanced_setup.button_view_documentation",
                    "View documentation",
                ),
            },
            ContentItem {
                title: crate::menu_label(
                    "resource_center.advanced_setup.how_warp_uses_warp_title",
                    "How Warp uses Warp",
                ),
                description: crate::menu_label(
                    "resource_center.advanced_setup.how_warp_uses_warp_description",
                    "Learn how Warp's engineering team uses their favorite features",
                ),
                url: "https://www.warp.dev/blog/how-warp-uses-warp",
                button_label: crate::menu_label(
                    "resource_center.advanced_setup.button_read_article",
                    "Read article",
                ),
            },
        ],
    };
    sections.push(Section::Content(advanced_setup));

    sections
}

fn maximize_warp_items(ctx: &mut ViewContext<ResourceCenterMainView>) -> Vec<FeatureItem> {
    let mut maximize_warp_items = vec![];

    maximize_warp_items.push(FeatureItem::new(
        crate::menu_label("terminal.context_menu.command_search", "Command search"),
        crate::menu_label(
            "resource_center.maximize_warp.command_search_description",
            "Find and run previously executed commands, workflows, and more.",
        ),
        Tip::Action(TipAction::CommandSearch),
        ctx,
    ));

    maximize_warp_items.push(FeatureItem::new(
        crate::menu_label(
            "terminal.context_menu.ai_command_search",
            "AI command search",
        ),
        crate::menu_label(
            "resource_center.maximize_warp.ai_command_search_description",
            "Generate shell commands with natural language.",
        ),
        Tip::Action(TipAction::AiCommandSearch),
        ctx,
    ));

    if ContextFlag::CreateNewSession.is_enabled() {
        maximize_warp_items.push(FeatureItem::new(
            crate::menu_label(
                "resource_center.maximize_warp.split_panes_title",
                "Split panes",
            ),
            crate::menu_label(
                "resource_center.maximize_warp.split_panes_description",
                "Split tabs into multiple panes to make your ideal layout.",
            ),
            Tip::Action(TipAction::SplitPane),
            ctx,
        ));
    }

    if ContextFlag::LaunchConfigurations.is_enabled() {
        maximize_warp_items.push(FeatureItem::new(
            crate::menu_label(
                "resource_center.maximize_warp.launch_configuration_title",
                "Launch configuration",
            ),
            crate::menu_label(
                "resource_center.maximize_warp.launch_configuration_description",
                "Save your current configuration of windows, tabs, and panes.",
            ),
            Tip::Action(TipAction::SaveNewLaunchConfig),
            ctx,
        ));
    }

    maximize_warp_items
}
