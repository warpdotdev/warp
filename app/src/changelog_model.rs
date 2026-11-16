use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use channel_versions::{Changelog, MarkdownSection};
use itertools::Itertools;
use markdown_parser::{FormattedText, parse_markdown};
use warpui::assets::asset_cache::{AssetCache, AssetSource};
use warpui::image_cache::ImageType;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::autoupdate::{self};
use crate::channel::{Channel, ChannelState};
use crate::features::{FeatureFlag, PREVIEW_FLAGS};
use crate::server::server_api::ServerApi;

pub struct ChangelogModel {
    pub changelog: ChangelogState,
    pub parsed_changelog: HashMap<String, FormattedText>,
    pub oz_updates: Vec<FormattedText>,
    pub server_api: Arc<ServerApi>,
    pub image: Option<AssetSource>,
}

impl ChangelogModel {
    pub fn new(server_api: Arc<ServerApi>) -> Self {
        Self {
            changelog: ChangelogState::None,
            parsed_changelog: HashMap::new(),
            oz_updates: Vec::new(),
            server_api,
            image: None,
        }
    }

    pub fn check_for_changelog(
        &mut self,
        request_type: ChangelogRequestType,
        ctx: &mut ModelContext<Self>,
    ) {
        match &self.changelog {
            ChangelogState::Some(changelog) => {
                // Don't refetch the changelog if we already have it
                ctx.notify();
                ctx.emit(Event::ChangelogRequestComplete {
                    request_type,
                    changelog: changelog.clone(),
                });
            }
            ChangelogState::Pending => {
                // There is already a request pending, so no-op while we wait for the response
            }
            ChangelogState::None => {
                self.changelog = ChangelogState::Pending;
                let server_api = self.server_api.clone();
                let _ = ctx.spawn(
                    async move {
                        (
                            request_type,
                            autoupdate::get_current_changelog(server_api).await,
                        )
                    },
                    Self::handle_changelog_check,
                );
            }
        }
    }

    fn handle_changelog_check(
        &mut self,
        (request_type, changelog): (
            ChangelogRequestType,
            Result<Option<Changelog>, anyhow::Error>,
        ),
        ctx: &mut ModelContext<Self>,
    ) {
        match changelog {
            Ok(Some(changelog)) => {
                if FeatureFlag::OzChangelogUpdates.is_enabled() {
                    self.oz_updates = changelog
                        .oz_updates
                        .iter()
                        .filter_map(|update_markdown| parse_markdown(update_markdown).ok())
                        .collect();
                }
                self.changelog = ChangelogState::Some(changelog.clone());
                self.maybe_add_changelog_sections();
                self.parse_changelog_markdown();
                ctx.notify();
                ctx.emit(Event::ChangelogRequestComplete {
                    request_type,
                    changelog,
                });
                // If the image URL is empty, we just log info and don't try to fetch any image
                self.fetch_changelog_image(ctx);
            }
            Ok(None) => {
                self.changelog = ChangelogState::None;
                log::info!("No changelog found for current version and channel");
                ctx.emit(Event::ChangelogRequestFailed { request_type });
            }
            Err(e) => {
                self.changelog = ChangelogState::None;
                log::warn!("Error checking for changelog {e:?}");
                ctx.emit(Event::ChangelogRequestFailed { request_type });
            }
        }
    }

    fn fetch_changelog_image(&mut self, ctx: &mut ModelContext<Self>) {
        let ChangelogState::Some(changelog) = &self.changelog else {
            return;
        };

        let Some(image_url) = changelog.image_url.as_ref() else {
            return;
        };

        let source = asset_cache::url_source(image_url);

        // By starting the fetch eagerly, we try to reduce the amount of time the changelog needs to render prior to
        // the image being available.
        AssetCache::as_ref(ctx).load_asset::<ImageType>(source.clone());

        self.image = Some(source);
    }

    /// Modifies the set of sections in the changelog, if necessary.
    fn maybe_add_changelog_sections(&mut self) {
        let markdown_sections = match &mut self.changelog {
            ChangelogState::Some(changelog) => &mut changelog.markdown_sections,
            _ => return,
        };

        // For WarpPreview, add a section at the beginning describing
        // preview-exclusive flags.
        if ChannelState::channel() == Channel::Preview && !PREVIEW_FLAGS.is_empty() {
            let preview_flags_vec: Vec<String> = PREVIEW_FLAGS
                .iter()
                .filter_map(|flag| flag.flag_description().map(ToOwned::to_owned))
                .collect();

            let mut preview_flags_string = preview_flags_vec
                .iter()
                .map(|flag| format!("* ***Preview-exclusive***: {flag}"))
                .join("\n");
            preview_flags_string.push('\n');

            // Insert preview-exclusive features into the "New features" section (markdown_sections[0])
            if markdown_sections.is_empty() {
                markdown_sections.push(MarkdownSection {
                    title: ChangelogHeader::NewFeatures.to_string(),
                    markdown: preview_flags_string,
                });
            } else {
                markdown_sections[0]
                    .markdown
                    .insert_str(0, preview_flags_string.as_str());
            }
        }

        // If there are no sections at all, add a section clarifying that there
        // are no meaningful updates this release.
        if markdown_sections.is_empty() {
            markdown_sections.push(MarkdownSection {
                title: ChangelogHeader::NewFeatures.to_string(),
                markdown: "* No notable changes this release\n".to_owned(),
            });
            if ChannelState::channel() == Channel::Dev {
                markdown_sections[0].markdown.push_str("* *Don't forget to put changelog information in your PR description, if applicable!*\n");
            }
        } else if markdown_sections
            .iter()
            .all(|section| section.markdown.is_empty())
        {
            // Add this to the "New features" section (markdown_sections[0])
            "* No notable changes this release\n".clone_into(&mut markdown_sections[0].markdown);
            if ChannelState::channel() == Channel::Dev {
                markdown_sections[0].markdown.push_str("* *Don't forget to put changelog information in your PR description, if applicable!*\n");
            }
        }
    }

    fn parse_changelog_markdown(&mut self) {
        if let ChangelogState::Some(changelog) = &self.changelog {
            for markdown_section in &changelog.markdown_sections {
                if !markdown_section.markdown.is_empty()
                    && let Ok(parsed_markdown) = parse_markdown(markdown_section.markdown.as_str())
                {
                    self.parsed_changelog
                        .insert(markdown_section.title.clone(), parsed_markdown);
                }
            }
        }
    }

    pub fn is_check_pending(&self) -> bool {
        matches!(self.changelog, ChangelogState::Pending)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChangelogHeader {
    NewFeatures,
    Improvements,
    BugFixes,
}

impl fmt::Display for ChangelogHeader {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ChangelogHeader::NewFeatures => write!(f, "New features"),
            ChangelogHeader::Improvements => write!(f, "Improvements"),
            ChangelogHeader::BugFixes => write!(f, "Bug fixes"),
        }
    }
}

#[derive(Debug)]
pub enum Event {
    ChangelogRequestComplete {
        request_type: ChangelogRequestType,
        changelog: Changelog,
    },
    ChangelogRequestFailed {
        request_type: ChangelogRequestType,
    },
    ImageRequestComplete,
}

#[derive(Debug)]
pub enum ChangelogRequestType {
    WindowLaunch,
    UserAction,
}

pub enum ChangelogState {
    None,
    Pending,
    Some(Changelog),
}

impl Entity for ChangelogModel {
    type Event = Event;
}

impl SingletonEntity for ChangelogModel {}
