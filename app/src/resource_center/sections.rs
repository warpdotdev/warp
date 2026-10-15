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
                t!("help_ui.create_your_first_block"),
                t!("help_ui.run_a_command_to_see_your_command_and_output"),
                Tip::Hint(TipHint::CreateBlock),
                ctx,
            ),
            FeatureItem::new(
                t!("help_ui.navigate_blocks"),
                t!("help_ui.click_to_select_a_block_and_navigate_with_arrow"),
                Tip::Hint(TipHint::BlockSelect),
                ctx,
            ),
            FeatureItem::new(
                t!("help_ui.take_an_action_on_block"),
                t!("help_ui.right_click_on_a_block_to_copy_paste_share"),
                Tip::Hint(TipHint::BlockAction),
                ctx,
            ),
            FeatureItem::new(
                t!("help_ui.open_command_palette"),
                t!("help_ui.access_all_of_warp_via_the_keyboard"),
                Tip::Action(TipAction::CommandPalette),
                ctx,
            ),
            FeatureItem::new(
                t!("help_ui.set_your_theme"),
                t!("help_ui.make_warp_your_own_by_choosing_a_theme"),
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
                title: t!("help_ui.use_your_custom_prompt"),
                description: t!("help_ui.set_up_warp_to_honor_your_ps1_setting"),
                url: "https://docs.warp.dev/terminal/appearance/prompt",
                button_label: t!("help_ui.view_documentation"),
            },
            ContentItem {
                title: t!("help_ui.integrate_warp_with_your_ide"),
                description: t!("help_ui.configure_warp_to_launch_from_your_most_used_development"),
                url: "https://docs.warp.dev/terminal/integrations-and-plugins",
                button_label: t!("help_ui.view_documentation"),
            },
            ContentItem {
                title: t!("help_ui.how_warp_uses_warp"),
                description: t!("help_ui.learn_how_warp_s_engineering_team_uses_their_favorite"),
                url: "https://www.warp.dev/blog/how-warp-uses-warp",
                button_label: t!("help_ui.read_article"),
            },
        ],
    };
    sections.push(Section::Content(advanced_setup));

    sections
}

fn maximize_warp_items(ctx: &mut ViewContext<ResourceCenterMainView>) -> Vec<FeatureItem> {
    let mut maximize_warp_items = vec![];

    maximize_warp_items.push(FeatureItem::new(
        t!("terminal_context_menu.command_search"),
        t!("help_ui.find_and_run_previously_executed_commands_workflows_and_more"),
        Tip::Action(TipAction::CommandSearch),
        ctx,
    ));

    maximize_warp_items.push(FeatureItem::new(
        t!("terminal_context_menu.ai_command_search"),
        t!("settings_ui.generate_shell_commands_with_natural_language"),
        Tip::Action(TipAction::AiCommandSearch),
        ctx,
    ));

    if ContextFlag::CreateNewSession.is_enabled() {
        maximize_warp_items.push(FeatureItem::new(
            t!("help_ui.split_panes"),
            t!("settings_ui.split_tabs_into_multiple_panes_to_make_your_ideal"),
            Tip::Action(TipAction::SplitPane),
            ctx,
        ));
    }

    if ContextFlag::LaunchConfigurations.is_enabled() {
        maximize_warp_items.push(FeatureItem::new(
            t!("help_ui.launch_configuration"),
            t!("help_ui.save_your_current_configuration_of_windows_tabs_and_panes"),
            Tip::Action(TipAction::SaveNewLaunchConfig),
            ctx,
        ));
    }

    maximize_warp_items
}
