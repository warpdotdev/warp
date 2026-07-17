//! Responsive horizontal tabs composed by a retained [`TuiView`].
//!
//! Width-dependent composition uses [`TuiSizeConstraintSwitch`] to select
//! between rows built from generic flex, text, container, and hoverable
//! elements.
//! The view retains stable mouse handles; callers retain semantic selection,
//! focus, and page-anchor state.

use std::collections::{HashMap, HashSet};

use warpui_core::elements::tui::{
    text_width, Modifier, TuiConstrainedBox, TuiContainer, TuiElement, TuiFlex, TuiHoverable,
    TuiParentElement, TuiSizeConstraintCondition, TuiSizeConstraintSwitch, TuiStyle, TuiText,
};
use warpui_core::elements::MouseStateHandle;
use warpui_core::{AppContext, Entity, TuiView, TypedActionView, ViewContext};
const DIVIDER: &str = "|";
const DIVIDER_PADDING_LEFT: u16 = 1;
const DIVIDER_PADDING_RIGHT: u16 = 2;
const ELLIPSIS_COLUMNS: u16 = 3;

/// Stable tab data rendered by [`TuiTabBarView`].
#[derive(Clone)]
pub struct TuiTab {
    pub key: String,
    pub label: String,
    leading: Option<TuiTabLeading>,
}

#[derive(Clone)]
struct TuiTabLeading {
    text: String,
    style: TuiStyle,
}

impl TuiTab {
    /// Creates a tab with stable identity and no leading text.
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            leading: None,
        }
    }

    /// Adds styled text rendered immediately before the tab label.
    pub fn with_leading_text(mut self, text: impl Into<String>, style: TuiStyle) -> Self {
        self.leading = Some(TuiTabLeading {
            text: text.into(),
            style,
        });
        self
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TuiTabBarStyles {
    pub bar: TuiStyle,
    pub leading: TuiStyle,
    pub chrome: TuiStyle,
    pub tab: TuiStyle,
    pub selected_focused: TuiStyle,
    pub selected_unfocused: TuiStyle,
}

#[derive(Clone)]
pub struct TuiTabBarConfig {
    pub leading: Option<String>,
    pub main_tab: Option<TuiTab>,
    pub tabs: Vec<TuiTab>,
    pub selected_key: Option<String>,
    pub focused: bool,
    pub page_anchor: Option<String>,
    pub reveal_selected: bool,
    pub maximum_label_columns: Option<u16>,
    pub tab_padding_columns: u16,
    pub secondary_gap_columns: u16,
    pub styles: TuiTabBarStyles,
}

impl TuiTabBarConfig {
    /// Creates a neutral configuration for the supplied secondary tabs.
    pub fn new(tabs: Vec<TuiTab>) -> Self {
        Self {
            leading: None,
            main_tab: None,
            tabs,
            selected_key: None,
            focused: false,
            page_anchor: None,
            reveal_selected: false,
            maximum_label_columns: None,
            tab_padding_columns: 1,
            secondary_gap_columns: 1,
            styles: TuiTabBarStyles::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TuiTabBarEvent {
    SelectTab(String),
    PageChanged(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiTabBarNavigationDirection {
    Previous,
    Next,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiTabBarSecondaryEdge {
    First,
    Last,
}

#[derive(Clone, Debug)]
#[doc(hidden)]
pub enum TuiTabBarAction {
    SelectTab(String),
    PageChanged(String),
}

/// Retained responsive tab-bar view.
pub struct TuiTabBarView {
    config: TuiTabBarConfig,
    mouse_states: HashMap<String, MouseStateHandle>,
    previous_overflow_mouse_state: MouseStateHandle,
    next_overflow_mouse_state: MouseStateHandle,
}

impl TuiTabBarView {
    /// Creates a retained view and initializes mouse state for every tab key.
    pub fn new(config: TuiTabBarConfig) -> Self {
        let mut view = Self {
            config,
            mouse_states: HashMap::new(),
            previous_overflow_mouse_state: MouseStateHandle::default(),
            next_overflow_mouse_state: MouseStateHandle::default(),
        };
        view.reconcile_mouse_states();
        view
    }

    /// Replaces caller-owned semantic inputs while preserving mouse state for live keys.
    pub fn set_config(&mut self, config: TuiTabBarConfig, ctx: &mut ViewContext<Self>) {
        self.config = config;
        self.reconcile_mouse_states();
        ctx.notify();
    }

    /// Reuses mouse handles for live keys and drops handles for removed tabs.
    fn reconcile_mouse_states(&mut self) {
        let live_keys = self
            .config
            .main_tab
            .iter()
            .chain(self.config.tabs.iter())
            .map(|tab| tab.key.clone())
            .collect::<HashSet<_>>();
        self.mouse_states.retain(|key, _| live_keys.contains(key));
        for key in live_keys {
            self.mouse_states.entry(key).or_default();
        }
    }

    /// Resolves the adjacent tab in semantic order, wrapping at either end.
    pub fn navigation_target(&self, direction: TuiTabBarNavigationDirection) -> Option<String> {
        let order = self
            .config
            .main_tab
            .iter()
            .chain(self.config.tabs.iter())
            .map(|tab| &tab.key)
            .collect::<Vec<_>>();
        let selected = self.config.selected_key.as_ref()?;
        let selected_index = order.iter().position(|key| *key == selected)?;
        let target_index = match direction {
            TuiTabBarNavigationDirection::Previous => {
                selected_index.checked_sub(1).unwrap_or(order.len() - 1)
            }
            TuiTabBarNavigationDirection::Next => (selected_index + 1) % order.len(),
        };
        order.get(target_index).map(|key| (*key).clone())
    }

    /// Returns the stable key at one edge of the secondary-tab collection.
    pub fn secondary_edge_target(&self, edge: TuiTabBarSecondaryEdge) -> Option<String> {
        match edge {
            TuiTabBarSecondaryEdge::First => self.config.tabs.first(),
            TuiTabBarSecondaryEdge::Last => self.config.tabs.last(),
        }
        .map(|tab| tab.key.clone())
    }
}

impl Entity for TuiTabBarView {
    type Event = TuiTabBarEvent;
}

impl TuiView for TuiTabBarView {
    fn ui_name() -> &'static str {
        "TuiTabBarView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn TuiElement> {
        render_tab_bar(
            &self.config,
            &self.mouse_states,
            &self.previous_overflow_mouse_state,
            &self.next_overflow_mouse_state,
        )
    }
}

impl TypedActionView for TuiTabBarView {
    type Action = TuiTabBarAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TuiTabBarAction::SelectTab(key) => {
                ctx.emit(TuiTabBarEvent::SelectTab(key.clone()));
            }
            TuiTabBarAction::PageChanged(anchor) => {
                ctx.emit(TuiTabBarEvent::PageChanged(anchor.clone()));
            }
        }
    }
}

/// Builds precomposed page alternatives and selects between them during layout.
fn render_tab_bar(
    config: &TuiTabBarConfig,
    mouse_states: &HashMap<String, MouseStateHandle>,
    previous_overflow_mouse_state: &MouseStateHandle,
    next_overflow_mouse_state: &MouseStateHandle,
) -> Box<dyn TuiElement> {
    let (default_page, transitions) = page_variant_transitions(config);
    let conditional_children = transitions
        .into_iter()
        .map(|(width, page)| {
            (
                TuiSizeConstraintCondition::WidthLessThan(width),
                render_page(
                    config,
                    page,
                    mouse_states,
                    previous_overflow_mouse_state,
                    next_overflow_mouse_state,
                ),
            )
        })
        .collect::<Vec<_>>();
    TuiSizeConstraintSwitch::new(
        render_page(
            config,
            default_page,
            mouse_states,
            previous_overflow_mouse_state,
            next_overflow_mouse_state,
        ),
        conditional_children,
    )
    .finish()
}

/// Composes one page alternative from generic TUI elements.
fn render_page(
    config: &TuiTabBarConfig,
    page: PageVariant,
    mouse_states: &HashMap<String, MouseStateHandle>,
    previous_overflow_mouse_state: &MouseStateHandle,
    next_overflow_mouse_state: &MouseStateHandle,
) -> Box<dyn TuiElement> {
    let PageVariant {
        start,
        visible_count,
        previous_start,
        next_start,
    } = page;
    let mut row = TuiFlex::row();
    if let Some(leading) = &config.leading {
        row.add_child(
            TuiText::new(leading)
                .with_style(config.styles.leading)
                .truncate()
                .finish(),
        );
    }

    if let Some(main_tab) = &config.main_tab {
        row.add_child(render_tab(config, main_tab, false, mouse_states));
        if !config.tabs.is_empty() {
            row.add_child(render_divider(config.styles.chrome));
        }
    }
    let visible_end = start.saturating_add(visible_count).min(config.tabs.len());
    let mut secondary = TuiFlex::row().with_spacing(config.secondary_gap_columns);
    if let Some(previous_start) = previous_start {
        secondary.add_child(render_overflow(
            "←",
            config.tabs[previous_start].key.clone(),
            previous_overflow_mouse_state.clone(),
            config.styles.chrome,
        ));
    }
    for (visible_index, tab) in config.tabs[start..visible_end].iter().enumerate() {
        let is_last_visible = visible_index + 1 == visible_count;
        let tab = render_tab(config, tab, is_last_visible, mouse_states);
        if is_last_visible {
            secondary = secondary.flex_child(
                TuiConstrainedBox::new(tab)
                    .with_max_cols(natural_tab_width(
                        &config.tabs[start + visible_index],
                        config,
                    ))
                    .finish(),
            );
        } else {
            secondary.add_child(tab);
        }
    }
    if let Some(next_start) = next_start {
        secondary.add_child(render_overflow(
            "→",
            config.tabs[next_start].key.clone(),
            next_overflow_mouse_state.clone(),
            config.styles.chrome,
        ));
    }
    row = row.flex_child(secondary.finish());

    let content = row.finish();
    let content = match config.styles.bar.bg {
        Some(background) => TuiContainer::new(content)
            .with_background(background)
            .finish(),
        None => content,
    };
    TuiFlex::row().flex_child(content).finish()
}

/// Renders one styled, hoverable tab that dispatches selection by stable key.
fn render_tab(
    config: &TuiTabBarConfig,
    tab: &TuiTab,
    flexible_label: bool,
    mouse_states: &HashMap<String, MouseStateHandle>,
) -> Box<dyn TuiElement> {
    let state = mouse_states
        .get(&tab.key)
        .cloned()
        .expect("tab mouse state is reconciled before render");

    let tab_style = if config.selected_key.as_deref() != Some(&tab.key) {
        config.styles.tab
    } else if config.focused {
        config.styles.selected_focused
    } else {
        config.styles.selected_unfocused
    };

    let label_style = if state.lock().unwrap().is_hovered() {
        tab_style.add_modifier(Modifier::BOLD)
    } else {
        tab_style
    };

    let leading_and_label_are_present = tab.leading.is_some() && !tab.label.is_empty();
    let mut content = TuiFlex::row().with_spacing(u16::from(leading_and_label_are_present));

    if let Some(leading) = &tab.leading {
        content.add_child(
            TuiText::new(leading.text.clone())
                .with_style(leading.style)
                .truncate()
                .finish(),
        );
    }

    let label = TuiConstrainedBox::new(
        TuiText::new(tab.label.clone())
            .with_style(label_style)
            .truncate_with_ellipsis()
            .finish(),
    )
    .with_max_cols(configured_label_width(tab, config))
    .finish();
    if flexible_label {
        content = content.flex_child(label);
    } else {
        content.add_child(label);
    }

    let mut container =
        TuiContainer::new(content.finish()).with_padding_x(config.tab_padding_columns);
    if let Some(background) = tab_style.bg {
        container = container.with_background(background);
    }
    let key = tab.key.clone();

    TuiHoverable::new(state, container.finish())
        .on_click(move |event_ctx, _| {
            event_ctx.dispatch_typed_action(TuiTabBarAction::SelectTab(key.clone()));
        })
        .finish()
}

/// Renders a hoverable paging arrow that dispatches its destination anchor.
fn render_overflow(
    text: &'static str,
    anchor: String,
    state: MouseStateHandle,
    style: TuiStyle,
) -> Box<dyn TuiElement> {
    let style = if state.lock().unwrap().is_hovered() {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    };
    TuiHoverable::new(
        state,
        TuiText::new(text).with_style(style).truncate().finish(),
    )
    .on_click(move |event_ctx, _| {
        event_ctx.dispatch_typed_action(TuiTabBarAction::PageChanged(anchor.clone()));
    })
    .finish()
}

/// Renders the fixed divider with layout padding rather than embedded spaces.
fn render_divider(style: TuiStyle) -> Box<dyn TuiElement> {
    TuiContainer::new(TuiText::new(DIVIDER).with_style(style).truncate().finish())
        .with_padding_left(DIVIDER_PADDING_LEFT)
        .with_padding_right(DIVIDER_PADDING_RIGHT)
        .finish()
}

/// One width-specific secondary-page structure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PageVariant {
    start: usize,
    visible_count: usize,
    previous_start: Option<usize>,
    next_start: Option<usize>,
}

/// Resolves the caller's requested page anchor.
fn requested_page_start(config: &TuiTabBarConfig) -> usize {
    if config.tabs.is_empty() {
        return 0;
    }
    config
        .page_anchor
        .as_ref()
        .and_then(|anchor| config.tabs.iter().position(|tab| &tab.key == anchor))
        .unwrap_or_default()
}

/// Resolves the secondary selected index, excluding an optional main tab.
fn selected_secondary_index(config: &TuiTabBarConfig) -> Option<usize> {
    config
        .selected_key
        .as_ref()
        .and_then(|selected| config.tabs.iter().position(|tab| &tab.key == selected))
}

/// Chooses a page at one concrete width.
///
/// The caller's anchor remains stable while the selected tab fits on that
/// page. Selected reveal re-anchors only when the selected secondary tab is
/// actually off-page at this width.
fn page_variant_at_width(config: &TuiTabBarConfig, width: u16) -> PageVariant {
    let requested_start = requested_page_start(config);
    let requested_page = page_from_start(config, requested_start, width);
    let selected_index = selected_secondary_index(config);
    let selected_is_visible = selected_index.is_some_and(|selected| {
        selected >= requested_start
            && selected < requested_start.saturating_add(requested_page.visible_count)
    });
    let deterministic_pages = deterministic_pages_at_width(config, width);
    let mut page = if config.reveal_selected && !selected_is_visible {
        selected_index
            .and_then(|selected| {
                deterministic_pages.iter().find(|page| {
                    (page.visible_count > 0
                        && selected >= page.start
                        && selected < page.start.saturating_add(page.visible_count))
                        || (page.visible_count == 0 && selected == page.start)
                })
            })
            .copied()
            .unwrap_or(requested_page)
    } else {
        requested_page
    };
    page.previous_start = deterministic_pages
        .iter()
        .take_while(|candidate| candidate.start < page.start)
        .last()
        .map(|candidate| candidate.start);
    page
}

/// Returns the widest page and each narrower width-specific alternative.
fn page_variant_transitions(config: &TuiTabBarConfig) -> (PageVariant, Vec<(u16, PageVariant)>) {
    let mut boundaries = (0..config.tabs.len())
        .flat_map(|start| {
            (1..=config.tabs.len().saturating_sub(start))
                .map(move |count| minimum_row_width(config, start, count))
        })
        .collect::<Vec<_>>();
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut page = page_variant_at_width(config, 0);
    let mut transitions = Vec::new();
    for boundary in boundaries {
        let next_page = page_variant_at_width(config, boundary);
        if next_page == page {
            continue;
        }
        if boundary > 0 {
            transitions.push((boundary, page));
        }
        page = next_page;
    }
    (page, transitions)
}

/// Packs one page beginning at `start`, without resolving its previous page.
fn page_from_start(config: &TuiTabBarConfig, start: usize, width: u16) -> PageVariant {
    let visible_count = visible_count_at_width(config, start, width);
    let candidate = if visible_count > 0 {
        start.saturating_add(visible_count)
    } else {
        start.saturating_add(1)
    };
    PageVariant {
        start,
        visible_count,
        previous_start: None,
        next_start: (candidate < config.tabs.len()).then_some(candidate),
    }
}

/// Deterministic non-overlapping page sequence for one concrete width.
fn deterministic_pages_at_width(config: &TuiTabBarConfig, width: u16) -> Vec<PageVariant> {
    let mut pages = Vec::new();
    let mut start = 0;
    let mut previous_start = None;
    while start < config.tabs.len() {
        let mut page = page_from_start(config, start, width);
        page.previous_start = previous_start;
        let next_start = page.next_start;
        pages.push(page);
        let Some(next_start) = next_start else {
            break;
        };
        previous_start = Some(start);
        start = next_start;
    }
    pages
}

/// Largest number of tabs whose minimum row width fits in `width`.
fn visible_count_at_width(config: &TuiTabBarConfig, start: usize, width: u16) -> usize {
    (1..=config.tabs.len().saturating_sub(start))
        .filter(|count| minimum_row_width(config, start, *count) <= width)
        .max()
        .unwrap_or_default()
}

/// Computes the minimum total row width that can display `visible_count` tabs.
///
/// All but the final visible tab reserve their natural widths. The final tab
/// reserves only its fixed chrome plus one label cell; generic text ellipsis
/// consumes any additional width assigned by flex.
fn minimum_row_width(config: &TuiTabBarConfig, start: usize, visible_count: usize) -> u16 {
    let visible_end = start.saturating_add(visible_count).min(config.tabs.len());
    let has_previous = start > 0;
    let has_next = visible_end < config.tabs.len();
    let mut width = fixed_prefix_width(config);
    if has_previous {
        width = width.saturating_add(text_width("←"));
        if visible_count > 0 || has_next {
            width = width.saturating_add(config.secondary_gap_columns);
        }
    }
    for (index, tab) in config.tabs[start..visible_end].iter().enumerate() {
        if index > 0 {
            width = width.saturating_add(config.secondary_gap_columns);
        }
        width = width.saturating_add(if index + 1 == visible_count {
            minimum_tab_width(tab, config)
        } else {
            natural_tab_width(tab, config)
        });
    }
    if has_next {
        if visible_count > 0 {
            width = width.saturating_add(config.secondary_gap_columns);
        }
        width = width.saturating_add(text_width("→"));
    }
    width
}

/// Width occupied before pageable tabs and overflow controls are added.
fn fixed_prefix_width(config: &TuiTabBarConfig) -> u16 {
    let mut width = config
        .leading
        .as_deref()
        .map(text_width)
        .unwrap_or_default();
    if let Some(main_tab) = &config.main_tab {
        width = width.saturating_add(natural_tab_width(main_tab, config));
        if !config.tabs.is_empty() {
            width = width
                .saturating_add(text_width(DIVIDER))
                .saturating_add(DIVIDER_PADDING_LEFT)
                .saturating_add(DIVIDER_PADDING_RIGHT);
        }
    }
    width
}

/// Maximum display-cell width assigned to a tab label.
fn configured_label_width(tab: &TuiTab, config: &TuiTabBarConfig) -> u16 {
    config
        .maximum_label_columns
        .unwrap_or_else(|| text_width(&tab.label))
        .min(text_width(&tab.label))
}

/// Natural width of a tab after applying its configured label cap.
fn natural_tab_width(tab: &TuiTab, config: &TuiTabBarConfig) -> u16 {
    tab_fixed_columns(tab, config.tab_padding_columns)
        .saturating_add(configured_label_width(tab, config))
}

/// Minimum width that preserves fixed tab content and visible label content.
fn minimum_tab_width(tab: &TuiTab, config: &TuiTabBarConfig) -> u16 {
    tab_fixed_columns(tab, config.tab_padding_columns)
        .saturating_add(minimum_label_width(tab, config))
}

/// Minimum label width that shows content rather than only ellipsis dots.
fn minimum_label_width(tab: &TuiTab, config: &TuiTabBarConfig) -> u16 {
    let configured_width = configured_label_width(tab, config);
    if configured_width <= ELLIPSIS_COLUMNS {
        return configured_width;
    }
    let first_glyph_width = tab
        .label
        .chars()
        .map(|character| {
            let mut buffer = [0; 4];
            text_width(character.encode_utf8(&mut buffer))
        })
        .find(|width| *width > 0)
        .unwrap_or(1);
    configured_width.min(ELLIPSIS_COLUMNS.saturating_add(first_glyph_width))
}

/// Counts non-label cells: horizontal padding, leading text, and its separator.
fn tab_fixed_columns(tab: &TuiTab, padding_columns: u16) -> u16 {
    let leading_columns = tab
        .leading
        .as_ref()
        .map(|leading| text_width(&leading.text))
        .unwrap_or_default();
    padding_columns
        .saturating_mul(2)
        .saturating_add(leading_columns)
        .saturating_add(u16::from(tab.leading.is_some() && !tab.label.is_empty()))
}

#[cfg(test)]
#[path = "tab_bar_tests.rs"]
mod tests;
