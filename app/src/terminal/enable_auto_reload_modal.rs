use std::sync::Arc;

use enclose::enclose;
use itertools::Itertools as _;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use warp_core::ui::appearance::Appearance;
use warp_graphql::billing::AddonCreditsOption;
use warpui::elements::{
    Border, ChildView, Container, CrossAxisAlignment, Empty, Flex, HighlightedHyperlink,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Text,
};
use warpui::fonts::Weight;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent as _, UiComponentStyles};
use warpui::{AppContext, Element, Entity, SingletonEntity as _, View, ViewContext, ViewHandle};

use crate::features::FeatureFlag;
use crate::menu::MenuItemFields;
use crate::modal::{Modal, ModalEvent, MODAL_PADDING, MODAL_WIDTH};
use crate::pricing::{PricingInfoModel, PricingInfoModelEvent};
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::{AutoReloadModalAction, TelemetryEvent};
use crate::settings_view::create_discount_badge;
use crate::ui_components::blended_colors;
use crate::view_components::{Dropdown, ToastFlavor};
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};

const DENOMINATION_DROPDOWN_WIDTH: f32 = MODAL_WIDTH - 2. * MODAL_PADDING;

#[derive(Default)]
struct MouseStates {
    enable_button: MouseStateHandle,
    cancel_button: MouseStateHandle,
}

/// The body content of the enable auto-reload modal
pub struct EnableAutoReloadModalBody {
    mouse_states: MouseStates,
    denomination_dropdown: ViewHandle<Dropdown<Action>>,
    addon_credits_options: Vec<AddonCreditsOption>,
    selected_denomination_index: usize,
    update_workspace_settings_loading: bool,
}

/// The main modal that wraps the body
pub struct EnableAutoReloadModal {
    modal: ViewHandle<Modal<EnableAutoReloadModalBody>>,
}

/// Called when user clicks the 'x' OR cancel button
fn send_auto_reload_dismissed_telemetry<V: View>(ctx: &mut ViewContext<V>) {
    send_telemetry_from_ctx!(
        TelemetryEvent::AutoReloadModalClosed {
            action: AutoReloadModalAction::Dismissed,
            selected_credits: None,
            banner_toggle_flag_enabled: FeatureFlag::BuildPlanAutoReloadBannerToggle.is_enabled(),
            post_purchase_modal_flag_enabled: FeatureFlag::BuildPlanAutoReloadPostPurchaseModal
                .is_enabled(),
        },
        ctx
    );
}

impl EnableAutoReloadModalBody {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(
            &PricingInfoModel::handle(ctx),
            |me, _, event, ctx| match event {
                PricingInfoModelEvent::PricingInfoUpdated => {
                    me.update_addon_credits_options(ctx);
                    ctx.notify();
                }
            },
        );

        ctx.subscribe_to_model(
            &UserWorkspaces::handle(ctx),
            |me, _handle, event, ctx| {
                match event {
                    UserWorkspacesEvent::UpdateWorkspaceSettingsSuccess => {
                        if me.update_workspace_settings_loading {
                            me.update_workspace_settings_loading = false;

                            // Emit telemetry for successful auto-reload enablement
                            let selected_credits = me
                                .addon_credits_options
                                .get(me.selected_denomination_index)
                                .map(|option| option.credits);
                            send_telemetry_from_ctx!(
                                TelemetryEvent::AutoReloadModalClosed {
                                    action: AutoReloadModalAction::EnabledAutoReload,
                                    selected_credits,
                                    banner_toggle_flag_enabled:
                                        FeatureFlag::BuildPlanAutoReloadBannerToggle.is_enabled(),
                                    post_purchase_modal_flag_enabled:
                                        FeatureFlag::BuildPlanAutoReloadPostPurchaseModal.is_enabled(),
                                },
                                ctx
                            );

                            ctx.emit(EnableAutoReloadModalBodyEvent::ShowToast {
                                message: "Auto-reload settings updated".to_string(),
                                flavor: ToastFlavor::Success,
                            });
                            ctx.emit(EnableAutoReloadModalBodyEvent::Close);
                        }
                    }
                    UserWorkspacesEvent::UpdateWorkspaceSettingsRejected(_err) => {
                        if me.update_workspace_settings_loading {
                            me.update_workspace_settings_loading = false;
                            ctx.emit(EnableAutoReloadModalBodyEvent::ShowToast {
                                message: "Failed to enable auto-reload. Please try updating your settings in Billing & usage.".to_string(),
                                flavor: ToastFlavor::Error,
                            });
                            ctx.notify();
                        }
                    }
                    _ => {}
                }
            },
        );

        let denomination_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(DENOMINATION_DROPDOWN_WIDTH);
            dropdown.set_menu_width(DENOMINATION_DROPDOWN_WIDTH, ctx);
            dropdown
        });

        let mut me = Self {
            mouse_states: Default::default(),
            denomination_dropdown,
            addon_credits_options: Default::default(),
            selected_denomination_index: 0,
            update_workspace_settings_loading: false,
        };
        me.update_addon_credits_options(ctx);
        me
    }

    fn update_addon_credits_options(&mut self, ctx: &mut ViewContext<Self>) {
        self.addon_credits_options = PricingInfoModel::as_ref(ctx)
            .addon_credits_options()
            .map(|opts| opts.to_vec())
            .unwrap_or_default();

        let base_rate = self
            .addon_credits_options
            .first()
            .map_or(0., |option| option.rate());
        let items = self
            .addon_credits_options
            .iter()
            .enumerate()
            .map(|(index, option)| {
                let primary_text = format!(
                    "${:.0} / {} credits",
                    option.price_usd_cents as f32 / 100.,
                    option.credits
                );
                let discount_percent = if base_rate > 0.0 {
                    let actual_rate = option.rate();
                    ((base_rate - actual_rate) / base_rate * 100.0).round() as u32
                } else {
                    0
                };
                if discount_percent > 0 {
                    MenuItemFields::new_with_custom_label(
                        Arc::new(enclose!((primary_text) move |is_selected, is_hovered, appearance, _| {
                            let text_color = appearance.theme().main_text_color(
                                if is_selected || is_hovered {
                                    appearance.theme().accent()
                                } else {
                                    appearance.theme().surface_1()
                                }
                            );
                            let main_text = Text::new_inline(
                                primary_text.clone(),
                                appearance.ui_font_family(),
                                appearance.ui_font_size(),
                            )
                            .with_color(text_color.into())
                            .finish();

                            let discount_badge = create_discount_badge(discount_percent, appearance);

                            Flex::row()
                                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                                .with_main_axis_size(MainAxisSize::Max)
                                .with_child(main_text)
                                .with_child(discount_badge)
                                .finish()
                        })),
                        Some(primary_text),
                    )
                    .with_on_select_action(Action::SelectDenomination(index).into())
                    .into_item()
                } else {
                    MenuItemFields::new(primary_text.clone())
                        .with_on_select_action(Action::SelectDenomination(index).into())
                        .into_item()
                }
            })
            .collect_vec();
        self.denomination_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_rich_items(items, ctx);
            dropdown.set_selected_by_index(self.selected_denomination_index, ctx);
        });
    }

    fn render_content(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let explanation_fragments = vec![
            FormattedTextFragment::plain_text("When enabled, "),
            FormattedTextFragment::bold("auto-reload"),
            FormattedTextFragment::plain_text(
                " will automatically purchase your selected package when you run out. ",
            ),
            FormattedTextFragment::hyperlink(
                "Learn more",
                "https://docs.warp.dev/support-and-community/plans-and-billing/add-on-credits#id-2.-enable-auto-reload",
            ),
        ];
        let explanation_text = warpui::elements::FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(explanation_fragments)]),
            appearance.ui_font_size(),
            appearance.ui_font_family(),
            appearance.ui_font_family(),
            blended_colors::text_sub(theme, theme.surface_1()),
            HighlightedHyperlink::default(),
        )
        .with_hyperlink_font_color(theme.accent().into_solid())
        .register_default_click_handlers_with_action_support(|hyperlink_lens, _event, ctx| {
            match hyperlink_lens {
                warpui::elements::HyperlinkLens::Url(url) => {
                    ctx.open_url(url);
                }
                warpui::elements::HyperlinkLens::Action(_action_ref) => {}
            }
        })
        .finish();

        let denomination_dropdown =
            Container::new(ChildView::new(&self.denomination_dropdown).finish())
                .with_vertical_padding(16.)
                .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(explanation_text)
            .with_child(denomination_dropdown)
            .finish()
    }

    fn render_buttons(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let cancel_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.mouse_states.cancel_button.clone(),
            )
            .with_style(UiComponentStyles {
                font_weight: Some(Weight::Semibold),
                padding: Some(Coords {
                    top: 6.,
                    bottom: 6.,
                    left: 12.,
                    right: 12.,
                }),
                ..Default::default()
            })
            .with_text_label(crate::i18n::tr_static(app, "Cancel").to_string())
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(Action::Cancel);
            })
            .finish();

        let button_text = if self.update_workspace_settings_loading {
            "Saving...".to_string()
        } else {
            "Enable".to_string()
        };

        let mut enable_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                self.mouse_states.enable_button.clone(),
            )
            .with_style(UiComponentStyles {
                font_weight: Some(Weight::Semibold),
                padding: Some(Coords {
                    top: 6.,
                    bottom: 6.,
                    left: 12.,
                    right: 12.,
                }),
                ..Default::default()
            })
            .with_text_label(button_text)
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(Action::Enable);
            });

        if self.update_workspace_settings_loading {
            enable_button = enable_button.disable();
        }

        Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(8.)
            .with_child(cancel_button)
            .with_child(enable_button.finish())
            .finish()
    }
}

#[derive(Clone, Debug)]
pub enum EnableAutoReloadModalBodyEvent {
    Close,
    ShowToast {
        message: String,
        flavor: ToastFlavor,
    },
}

impl Entity for EnableAutoReloadModalBody {
    type Event = EnableAutoReloadModalBodyEvent;
}

impl View for EnableAutoReloadModalBody {
    fn ui_name() -> &'static str {
        "EnableAutoReloadModalBody"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let content = Container::new(self.render_content(appearance))
            .with_horizontal_padding(MODAL_PADDING)
            .with_margin_top(0.) // let the header padding handle the top margin
            .finish();

        let separator = Container::new(Empty::new().finish())
            .with_border(Border::bottom(1.).with_border_fill(appearance.theme().outline()))
            .finish();

        let buttons = Container::new(self.render_buttons(appearance, app))
            .with_horizontal_padding(MODAL_PADDING)
            .with_vertical_padding(12.)
            .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(content)
            .with_child(separator)
            .with_child(buttons)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    SelectDenomination(usize),
    Cancel,
    Enable,
}

impl warpui::TypedActionView for EnableAutoReloadModalBody {
    type Action = Action;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            Action::SelectDenomination(index) => {
                self.selected_denomination_index = *index;
                ctx.notify();
            }
            Action::Cancel => {
                send_auto_reload_dismissed_telemetry(ctx);
                ctx.emit(EnableAutoReloadModalBodyEvent::Close);
            }
            Action::Enable => {
                let workspaces = UserWorkspaces::as_ref(ctx);
                let Some(team_uid) = workspaces.current_team_uid() else {
                    ctx.emit(EnableAutoReloadModalBodyEvent::ShowToast {
                        message: "Oops, something went wrong; your team's data could not be found."
                            .to_string(),
                        flavor: ToastFlavor::Error,
                    });
                    return;
                };

                // Set loading state before making the API call
                self.update_workspace_settings_loading = true;
                ctx.notify();

                UserWorkspaces::handle(ctx).update(ctx, move |user_workspaces, ctx| {
                    user_workspaces.update_addon_credits_settings(
                        team_uid,
                        Some(true),
                        // TODO: consider allowing user to set max monthly spend too in this modal
                        None,
                        Some(self.addon_credits_options[self.selected_denomination_index].credits),
                        ctx,
                    );
                });
            }
        }
    }
}

// Main modal wrapper
impl EnableAutoReloadModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let body = ctx.add_typed_action_view(EnableAutoReloadModalBody::new);

        let modal = ctx.add_typed_action_view(|ctx| {
            Modal::new(Some("Enable auto reload?".to_string()), body.clone(), ctx).with_body_style(
                UiComponentStyles {
                    // Padding of 0 here since we add a horizontal bar that needs to span the full width in the body
                    // So we handle padding in the body itself
                    padding: Some(Coords::uniform(0.)),
                    ..Default::default()
                },
            )
        });

        ctx.subscribe_to_view(&modal, |_, _, event, ctx| match event {
            ModalEvent::Close => {
                // "x" clicked
                send_auto_reload_dismissed_telemetry(ctx);
                ctx.emit(EnableAutoReloadModalEvent::Close);
            }
        });

        ctx.subscribe_to_view(&body, |_, _, event, ctx| match event {
            EnableAutoReloadModalBodyEvent::Close => {
                ctx.emit(EnableAutoReloadModalEvent::Close);
            }
            EnableAutoReloadModalBodyEvent::ShowToast { message, flavor } => {
                ctx.emit(EnableAutoReloadModalEvent::ShowToast {
                    message: message.clone(),
                    flavor: *flavor,
                });
            }
        });

        Self { modal }
    }

    pub fn set_selected_denomination_by_credits(
        &mut self,
        credits: i32,
        ctx: &mut ViewContext<Self>,
    ) {
        self.modal.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, ctx| {
                if let Some((index, _)) = body
                    .addon_credits_options
                    .iter()
                    .enumerate()
                    .find(|(_, option)| option.credits == credits)
                {
                    body.selected_denomination_index = index;
                    // Update the dropdown to reflect the new selection
                    body.denomination_dropdown.update(ctx, |dropdown, ctx| {
                        dropdown.set_selected_by_index(index, ctx);
                    });
                    ctx.notify();
                }
            });
        });
    }
}

#[derive(Clone, Debug)]
pub enum EnableAutoReloadModalEvent {
    Close,
    ShowToast {
        message: String,
        flavor: ToastFlavor,
    },
}

impl Entity for EnableAutoReloadModal {
    type Event = EnableAutoReloadModalEvent;
}

impl warpui::TypedActionView for EnableAutoReloadModal {
    type Action = ();
}

impl View for EnableAutoReloadModal {
    fn ui_name() -> &'static str {
        "EnableAutoReloadModal"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.modal).finish()
    }
}
