//! Responsive horizontal tabs composed by a retained [`TuiView`].
//!
//! Width-dependent composition uses [`TuiSizeConstraintSwitch`] to select
//! between rows built from generic flex, text, container, and hoverable
//! elements.
//! The view retains stable mouse handles; callers retain semantic selection,
//! focus, and page-anchor state.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

use unicode_segmentation::UnicodeSegmentation;
use warpui_core::elements::MouseStateHandle;
use warpui_core::elements::tui::{
    Color, Modifier, TuiConstrainedBox, TuiContainer, TuiElement, TuiFlex, TuiHoverable,
    TuiParentElement, TuiSizeConstraintCondition, TuiSizeConstraintSwitch, TuiStyle, TuiText,
    text_width,
};
use warpui_core::{AppContext, Entity, TuiView, TypedActionView, ViewContext};
const DIVIDER: &str = "|";
const DIVIDER_PADDING_LEFT: u16 = 1;
const DIVIDER_PADDING_RIGHT: u16 = 2;
const ELLIPSIS: &str = "...";
/// Gap between breadcrumb tabs and between the last breadcrumb and the main tab.
const BREADCRUMB_GAP_COLUMNS: u16 = 1;

/// Stable tab data rendered by [`TuiTabBarView`].
#[derive(Clone, PartialEq)]
pub struct TuiTab {
    pub key: String,
    pub label: String,
    leading: Option<TuiTabAdornment>,
    trailing: Option<TuiTabAdornment>,
    /// Per-tab label cap that overrides the config-wide
    /// `maximum_label_columns` when present.
    max_label_columns: Option<u16>,
    /// Whether the tab can be selected. A non-selectable tab still renders
    /// (e.g. a sessionless drill-down anchor framing its level) but is
    /// skipped by keyboard navigation and ignores clicks.
    selectable: bool,
}

/// Styled text rendered before or after a tab label.
#[derive(Clone, PartialEq)]
struct TuiTabAdornment {
    text: String,
    style: TuiStyle,
}

impl TuiTab {
    /// Creates a tab with stable identity and no adornments.
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            leading: None,
            trailing: None,
            max_label_columns: None,
            selectable: true,
        }
    }

    /// Marks whether the tab can be selected via navigation or clicks.
    pub fn with_selectable(mut self, selectable: bool) -> Self {
        self.selectable = selectable;
        self
    }

    /// Adds styled text rendered immediately before the tab label.
    pub fn with_leading_text(mut self, text: impl Into<String>, style: TuiStyle) -> Self {
        self.leading = Some(TuiTabAdornment {
            text: text.into(),
            style,
        });
        self
    }

    /// Adds styled text rendered immediately after the tab label (e.g. a
    /// subtree rollup badge). The trailing text shares the tab's click target.
    pub fn with_trailing_text(mut self, text: impl Into<String>, style: TuiStyle) -> Self {
        self.trailing = Some(TuiTabAdornment {
            text: text.into(),
            style,
        });
        self
    }

    /// Caps this tab's label independently of the config-wide maximum.
    pub fn with_max_label_columns(mut self, columns: u16) -> Self {
        self.max_label_columns = Some(columns);
        self
    }

    /// Replaces the label, e.g. when a narrow variant collapses a tab to its
    /// leading adornment alone.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Drops the trailing adornment.
    pub fn without_trailing(mut self) -> Self {
        self.trailing = None;
        self
    }
}

/// Caller-supplied styles for the tab bar and its semantic states.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TuiTabBarStyles {
    pub background: Option<Color>,
    pub leading: TuiStyle,
    pub chrome: TuiStyle,
    pub tab: TuiStyle,
    pub selected_focused: TuiStyle,
    pub selected_unfocused: TuiStyle,
}

/// Caller-owned semantic state and presentation options for a tab bar.
#[derive(Clone, PartialEq)]
pub struct TuiTabBarConfig {
    pub leading: Option<String>,
    /// Selectable ancestor chips rendered between the leading text and the
    /// main tab. Part of the fixed (non-paginating) prefix.
    pub breadcrumb_tabs: Vec<TuiTab>,
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
    /// Width-bounded presentation variants applied below their exclusive
    /// width bounds, narrowest bound winning. Each variant must contain the
    /// same tab keys as the base config; a variant's own `narrow_variants`
    /// are ignored.
    pub narrow_variants: Vec<TuiTabBarNarrowVariant>,
}

/// One width-bounded presentation alternative for a [`TuiTabBarConfig`].
#[derive(Clone, PartialEq)]
pub struct TuiTabBarNarrowVariant {
    /// The variant applies when the available width is strictly below this.
    pub max_width_exclusive: u16,
    pub config: TuiTabBarConfig,
}

impl TuiTabBarConfig {
    /// Creates a neutral configuration for the supplied secondary tabs.
    pub fn new(tabs: Vec<TuiTab>) -> Self {
        Self {
            leading: None,
            breadcrumb_tabs: Vec::new(),
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
            narrow_variants: Vec::new(),
        }
    }

    /// All rendered tabs in row order: breadcrumbs, main tab, then secondary.
    fn row_tabs(&self) -> impl Iterator<Item = &TuiTab> {
        self.breadcrumb_tabs
            .iter()
            .chain(self.main_tab.iter())
            .chain(self.tabs.iter())
    }
}

/// Caller-owned responsive paging intent for a tab bar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TuiTabBarPagingState<K> {
    explicit_anchor: Option<K>,
}
impl<K> Default for TuiTabBarPagingState<K> {
    fn default() -> Self {
        Self {
            explicit_anchor: None,
        }
    }
}

impl<K> TuiTabBarPagingState<K> {
    /// Preserves the page beginning at `anchor` instead of revealing selection.
    ///
    /// Automatic reveal resumes when the owner drops the level's paging state
    /// entirely (see `TuiOrchestrationModel::focus_conversation_session`).
    pub(crate) fn set_explicit_anchor(&mut self, anchor: K) {
        self.explicit_anchor = Some(anchor);
    }
}

impl<K: Clone> TuiTabBarPagingState<K> {
    /// Resolves paging intent against the owner's current ordered keys.
    pub(crate) fn resolve(
        &self,
        default_anchor: Option<K>,
        explicit_anchor_is_valid: impl FnOnce(&K) -> bool,
    ) -> TuiTabBarResolvedPage<K> {
        let explicit_anchor = self
            .explicit_anchor
            .as_ref()
            .and_then(|anchor| explicit_anchor_is_valid(anchor).then_some(anchor));
        TuiTabBarResolvedPage {
            page_anchor: explicit_anchor.cloned().or(default_anchor),
            reveal_selected: explicit_anchor.is_none(),
        }
    }
}

/// Effective paging inputs resolved from [`TuiTabBarPagingState`].
pub(crate) struct TuiTabBarResolvedPage<K> {
    pub(crate) page_anchor: Option<K>,
    pub(crate) reveal_selected: bool,
}
/// Invalid caller-supplied tab-bar configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TuiTabBarConfigError {
    /// A stable key was assigned to more than one main or secondary tab.
    DuplicateKey(String),
    /// The label cap cannot show the complete label or one grapheme plus `...`.
    LabelWidthTooSmall {
        key: String,
        configured: u16,
        required: u16,
    },
}

impl fmt::Display for TuiTabBarConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateKey(key) => write!(formatter, "duplicate TUI tab-bar key `{key}`"),
            Self::LabelWidthTooSmall {
                key,
                configured,
                required,
            } => write!(
                formatter,
                "TUI tab-bar label width {configured} for tab `{key}` leaves no room for visible \
                 content; at least {required} columns are required"
            ),
        }
    }
}

impl Error for TuiTabBarConfigError {}

/// Semantic interactions emitted by [`TuiTabBarView`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TuiTabBarEvent {
    SelectTab(String),
    PageChanged(String),
}

/// Direction used to resolve an adjacent semantic tab.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiTabBarNavigationDirection {
    Previous,
    Next,
}

/// Edge used to resolve a tab from the secondary collection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiTabBarSecondaryEdge {
    First,
    Last,
}

/// Internal actions dispatched by tab and overflow hit targets.
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
    /// Creates an empty tab bar without fallible caller-supplied configuration.
    pub fn empty() -> Self {
        Self::from_valid_config(TuiTabBarConfig::new(Vec::new()), HashSet::new())
    }

    /// Creates a retained view and initializes mouse state for every unique tab key.
    ///
    /// Returns an error before constructing the view when keys are duplicated
    /// or a configured label cap cannot preserve visible label content.
    pub fn new(config: TuiTabBarConfig) -> Result<Self, TuiTabBarConfigError> {
        let live_keys = validated_live_keys(&config)?;
        Ok(Self::from_valid_config(config, live_keys))
    }

    fn from_valid_config(config: TuiTabBarConfig, live_keys: HashSet<String>) -> Self {
        let mouse_states = live_keys
            .into_iter()
            .map(|key| (key, MouseStateHandle::default()))
            .collect();
        Self {
            config,
            mouse_states,
            previous_overflow_mouse_state: MouseStateHandle::default(),
            next_overflow_mouse_state: MouseStateHandle::default(),
        }
    }

    /// Replaces caller-owned semantic inputs while preserving mouse state for live keys.
    ///
    /// Invalid configuration returns an error without replacing the current
    /// configuration or notifying the view.
    pub fn set_config(
        &mut self,
        config: TuiTabBarConfig,
        ctx: &mut ViewContext<Self>,
    ) -> Result<(), TuiTabBarConfigError> {
        if self.config == config {
            return Ok(());
        }
        let live_keys = validated_live_keys(&config)?;
        self.config = config;
        self.reconcile_mouse_states(live_keys);
        ctx.notify();
        Ok(())
    }

    /// Reuses mouse handles for live keys and drops handles for removed tabs.
    fn reconcile_mouse_states(&mut self, live_keys: HashSet<String>) {
        self.mouse_states.retain(|key, _| live_keys.contains(key));
        for key in live_keys {
            self.mouse_states.entry(key).or_default();
        }
    }
    /// Whether the current configuration contains any main or secondary tabs.
    pub(crate) fn has_tabs(&self) -> bool {
        self.config.main_tab.is_some() || !self.config.tabs.is_empty()
    }

    /// Resolves the adjacent tab in rendered row order — breadcrumbs, main
    /// tab, then the secondary tabs — wrapping at either end and skipping
    /// non-selectable tabs so navigation can never dead-end on them.
    pub fn navigation_target(&self, direction: TuiTabBarNavigationDirection) -> Option<String> {
        let order = self.config.row_tabs().collect::<Vec<_>>();
        let selected = self.config.selected_key.as_ref()?;
        let selected_index = order.iter().position(|tab| &tab.key == selected)?;
        let mut target_index = selected_index;
        for _ in 0..order.len() {
            target_index = match direction {
                TuiTabBarNavigationDirection::Previous => {
                    target_index.checked_sub(1).unwrap_or(order.len() - 1)
                }
                TuiTabBarNavigationDirection::Next => (target_index + 1) % order.len(),
            };
            let target = order[target_index];
            if target.selectable {
                return Some(target.key.clone());
            }
        }
        None
    }

    /// Returns the stable key at one edge of the secondary-tab collection.
    pub fn secondary_edge_target(&self, edge: TuiTabBarSecondaryEdge) -> Option<String> {
        match edge {
            TuiTabBarSecondaryEdge::First => self.config.tabs.first(),
            TuiTabBarSecondaryEdge::Last => self.config.tabs.last(),
        }
        .map(|tab| tab.key.clone())
    }

    /// Returns the currently selected tab's stable key, if any.
    pub(crate) fn selected_key(&self) -> Option<&str> {
        self.config.selected_key.as_deref()
    }

    /// Returns the stable key of the orchestration tree root: the first
    /// breadcrumb while the bar is drilled below the root, otherwise the
    /// main tab.
    ///
    /// Callers (e.g. the terminal-session Escape binding) use this to return
    /// focus to the root agent without duplicating tab-bar configuration.
    pub(crate) fn tree_root_key(&self) -> Option<String> {
        self.config
            .breadcrumb_tabs
            .first()
            .or(self.config.main_tab.as_ref())
            .map(|tab| tab.key.clone())
    }
}

/// Validates tab identities and label widths across the base configuration
/// and every narrow variant, returning the union of live keys.
fn validated_live_keys(config: &TuiTabBarConfig) -> Result<HashSet<String>, TuiTabBarConfigError> {
    let mut live_keys = HashSet::new();
    for config in
        std::iter::once(config).chain(config.narrow_variants.iter().map(|variant| &variant.config))
    {
        let mut config_keys = HashSet::new();
        for tab in config.row_tabs() {
            if !config_keys.insert(tab.key.clone()) {
                return Err(TuiTabBarConfigError::DuplicateKey(tab.key.clone()));
            }
            let required = minimum_visible_label_width(tab);
            let configured = configured_label_width(tab, config);
            if configured < required {
                return Err(TuiTabBarConfigError::LabelWidthTooSmall {
                    key: tab.key.clone(),
                    configured,
                    required,
                });
            }
        }
        live_keys.extend(config_keys);
    }
    Ok(live_keys)
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

/// Builds precomposed page alternatives — across every narrow variant's
/// width segment and the base configuration — and selects between them
/// during layout.
fn render_tab_bar(
    config: &TuiTabBarConfig,
    mouse_states: &HashMap<String, MouseStateHandle>,
    previous_overflow_mouse_state: &MouseStateHandle,
    next_overflow_mouse_state: &MouseStateHandle,
) -> Box<dyn TuiElement> {
    let mut variants = config.narrow_variants.iter().collect::<Vec<_>>();
    variants.sort_by_key(|variant| variant.max_width_exclusive);

    let mut conditional_children = Vec::new();
    let mut segment_start = 0u16;
    for variant in variants {
        if variant.max_width_exclusive <= segment_start {
            continue;
        }
        let (transitions, final_page) = page_transitions_in_range(
            &variant.config,
            segment_start,
            Some(variant.max_width_exclusive),
        );
        for (width, page) in transitions {
            conditional_children.push((
                TuiSizeConstraintCondition::WidthLessThan(width),
                render_page(
                    &variant.config,
                    page,
                    mouse_states,
                    previous_overflow_mouse_state,
                    next_overflow_mouse_state,
                ),
            ));
        }
        conditional_children.push((
            TuiSizeConstraintCondition::WidthLessThan(variant.max_width_exclusive),
            render_page(
                &variant.config,
                final_page,
                mouse_states,
                previous_overflow_mouse_state,
                next_overflow_mouse_state,
            ),
        ));
        segment_start = variant.max_width_exclusive;
    }

    let (transitions, default_page) = page_transitions_in_range(config, segment_start, None);
    for (width, page) in transitions {
        conditional_children.push((
            TuiSizeConstraintCondition::WidthLessThan(width),
            render_page(
                config,
                page,
                mouse_states,
                previous_overflow_mouse_state,
                next_overflow_mouse_state,
            ),
        ));
    }
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

    if !config.breadcrumb_tabs.is_empty() || config.main_tab.is_some() {
        let mut prefix = TuiFlex::row().with_spacing(BREADCRUMB_GAP_COLUMNS);
        for breadcrumb in &config.breadcrumb_tabs {
            prefix.add_child(render_tab(config, breadcrumb, false, mouse_states));
        }
        if let Some(main_tab) = &config.main_tab {
            prefix.add_child(render_tab(config, main_tab, false, mouse_states));
        }
        row.add_child(prefix.finish());
        if config.main_tab.is_some() && !config.tabs.is_empty() {
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
    let content = match config.styles.background {
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

    let content_count = tab_content_count(tab);
    let mut content = TuiFlex::row().with_spacing(u16::from(content_count > 1));
    if let Some(leading) = &tab.leading {
        content.add_child(
            TuiText::new(leading.text.clone())
                .with_style(leading.style)
                .truncate()
                .finish(),
        );
    }

    if !tab.label.is_empty() {
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
    }

    if let Some(trailing) = &tab.trailing {
        content.add_child(
            TuiText::new(trailing.text.clone())
                .with_style(trailing.style)
                .truncate()
                .finish(),
        );
    }

    let mut container =
        TuiContainer::new(content.finish()).with_padding_x(config.tab_padding_columns);
    if let Some(background) = tab_style.bg {
        container = container.with_background(background);
    }

    let hoverable = TuiHoverable::new(state, container.finish());
    if !tab.selectable {
        return hoverable.finish();
    }
    let key = tab.key.clone();
    hoverable
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

/// Returns the page transitions of `config` within a width interval:
/// pairs of (exclusive upper width, page shown below it) plus the page shown
/// from the last transition up to `upper_exclusive` (or unbounded when
/// `None`). The interval starts at `lower_inclusive`.
fn page_transitions_in_range(
    config: &TuiTabBarConfig,
    lower_inclusive: u16,
    upper_exclusive: Option<u16>,
) -> (Vec<(u16, PageVariant)>, PageVariant) {
    let mut boundaries = (0..config.tabs.len())
        .flat_map(|start| {
            (1..=config.tabs.len().saturating_sub(start))
                .map(move |count| minimum_row_width(config, start, count))
        })
        .filter(|boundary| {
            *boundary > lower_inclusive && upper_exclusive.is_none_or(|upper| *boundary < upper)
        })
        .collect::<Vec<_>>();
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut page = page_variant_at_width(config, lower_inclusive);
    let mut transitions = Vec::new();
    for boundary in boundaries {
        let next_page = page_variant_at_width(config, boundary);
        if next_page == page {
            continue;
        }
        transitions.push((boundary, page));
        page = next_page;
    }
    (transitions, page)
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
    for breadcrumb in &config.breadcrumb_tabs {
        width = width
            .saturating_add(natural_tab_width(breadcrumb, config))
            .saturating_add(BREADCRUMB_GAP_COLUMNS);
    }
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
    tab.max_label_columns
        .or(config.maximum_label_columns)
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
    minimum_visible_label_width(tab).min(configured_label_width(tab, config))
}

/// Minimum label width that shows the full label or one grapheme plus ellipsis.
fn minimum_visible_label_width(tab: &TuiTab) -> u16 {
    let label_width = text_width(&tab.label);
    let ellipsis_width = text_width(ELLIPSIS);
    if label_width <= ellipsis_width {
        return label_width;
    }
    let first_grapheme_width = UnicodeSegmentation::graphemes(tab.label.as_str(), true)
        .map(text_width)
        .find(|width| *width > 0)
        .unwrap_or(1);
    label_width.min(ellipsis_width.saturating_add(first_grapheme_width))
}

/// Counts a tab's rendered content pieces: leading text, label, trailing text.
fn tab_content_count(tab: &TuiTab) -> usize {
    usize::from(tab.leading.is_some())
        + usize::from(!tab.label.is_empty())
        + usize::from(tab.trailing.is_some())
}

/// Counts non-label cells: horizontal padding, adornments, and separators.
fn tab_fixed_columns(tab: &TuiTab, padding_columns: u16) -> u16 {
    let adornment_columns = tab
        .leading
        .iter()
        .chain(tab.trailing.iter())
        .map(|adornment| text_width(&adornment.text))
        .fold(0u16, u16::saturating_add);
    let content_count = tab_content_count(tab);
    padding_columns
        .saturating_mul(2)
        .saturating_add(adornment_columns)
        .saturating_add(u16::try_from(content_count.saturating_sub(1)).unwrap_or(u16::MAX))
}

#[cfg(test)]
#[path = "tab_bar_tests.rs"]
mod tests;
