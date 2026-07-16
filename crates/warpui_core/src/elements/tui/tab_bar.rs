//! Retained horizontal TUI tab bar with an optional fixed main tab and
//! width-aware paging for secondary tabs.
//!
//! # Ownership
//!
//! The caller owns semantic application state: the ordered tabs, selected key,
//! focus state, and current page anchor. [`TuiTabBar`] owns only UI state that
//! must survive element-tree reconstruction: mouse handles and the navigation
//! data produced by the latest completed layout.
//!
//! # Per-frame flow
//!
//! 1. The caller passes [`TuiTabBarConfig`] and an event callback to
//!    [`TuiTabBar::render`].
//! 2. The returned [`TabBarElement`] waits for `layout` to provide the actual
//!    row width. It builds each caller-supplied leading element once, measures
//!    it, and keeps that same instance for the rendered row.
//! 3. [`tab_bar_layout`] performs the pure width calculation. It chooses the
//!    visible secondary page, truncates labels, and computes previous/next page
//!    anchors.
//! 4. [`TabBarElement::build_row`] assembles the fixed visual order: caller
//!    label, main tab, divider, previous control, visible tabs, and next
//!    control.
//! 5. `after_layout` publishes only [`SettledNavigation`] back to the retained
//!    component. Callers can then request previous/next keyboard targets
//!    without gaining access to private widths or visible ranges.
//!
//! Pointer handlers emit [`TuiTabBarEvent`] values only. They never mutate the
//! caller's selection, focus, page anchor, or tab collection.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use ratatui::style::Modifier;

use super::{
    text_width, truncate_with_ellipsis, TuiConstraint, TuiContainer, TuiElement, TuiEvent,
    TuiEventContext, TuiFlex, TuiHoverable, TuiLayoutContext, TuiPaintContext, TuiPaintSurface,
    TuiPresentationContext, TuiScreenPoint, TuiScreenPosition, TuiSize, TuiStyle, TuiText,
};
use crate::elements::MouseStateHandle;
use crate::AppContext;

// -----------------------------------------------------------------------------
// Public API
// -----------------------------------------------------------------------------

/// One stable tab supplied by a tab-bar owner.
#[derive(Clone)]
pub struct TuiTab {
    /// Stable identity used for retained mouse state, callbacks, and paging.
    pub key: String,
    /// Human-readable text displayed after any leading element.
    pub label: String,
    /// Rebuilds caller-owned visual content for each TUI layout pass.
    leading_element: Option<Rc<dyn Fn() -> Box<dyn TuiElement>>>,
}

impl TuiTab {
    /// Creates a tab without leading content.
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            leading_element: None,
        }
    }

    /// Adds arbitrary caller-rendered content before the tab label.
    ///
    /// The component invokes this factory once per layout pass. It measures the
    /// returned element and moves that same instance into the visible tab.
    pub fn with_leading_element(
        mut self,
        build_element: impl Fn() -> Box<dyn TuiElement> + 'static,
    ) -> Self {
        self.leading_element = Some(Rc::new(build_element));
        self
    }
}

/// Caller-supplied styles for a tab-bar row.
#[derive(Clone, Copy, Debug, Default)]
pub struct TuiTabBarStyles {
    /// Background style applied across the component's full assigned row.
    pub bar: TuiStyle,
    /// Style for the caller-provided product label before the tabs.
    pub leading: TuiStyle,
    /// Style for the fixed divider and previous/next arrows.
    pub chrome: TuiStyle,
    /// Style for an unselected tab.
    pub tab: TuiStyle,
    /// Style for the selected tab while the bar owns keyboard focus.
    pub selected_focused: TuiStyle,
    /// Style for the selected tab while another surface owns keyboard focus.
    pub selected_unfocused: TuiStyle,
}

/// Semantic events emitted by pointer interaction with the tab bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TuiTabBarEvent {
    /// The user clicked a visible tab.
    SelectTab(String),
    /// The user clicked an overflow arrow targeting another page anchor.
    PageChanged(String),
}

/// Direction for width-aware tab navigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuiTabBarNavigationDirection {
    /// Resolve the tab before the current selection.
    Previous,
    /// Resolve the tab after the current selection.
    Next,
}
/// Callback shape shared by every clickable tab and overflow control.
/// Stored behind `Rc` so each child handler observes the same callback.
type EventHandler = dyn for<'a> Fn(TuiTabBarEvent, &mut TuiEventContext<'a>, &AppContext);

/// Per-render input for [`TuiTabBar`].
pub struct TuiTabBarConfig {
    /// Optional product label rendered before the main tab.
    pub leading: Option<String>,
    /// Optional fixed tab that never participates in secondary paging.
    pub main_tab: Option<TuiTab>,
    /// Ordered secondary tabs packed into the remaining row width.
    pub tabs: Vec<TuiTab>,
    /// Key whose tab receives the selected style.
    pub selected_key: Option<String>,
    /// Whether the selected tab uses the focused or unfocused selected style.
    pub focused: bool,
    /// Secondary key from which the requested page begins.
    pub page_anchor: Option<String>,
    /// Whether layout should replace an off-page anchor with the selected tab's page.
    pub reveal_selected: bool,
    /// Maximum display-cell width of each label, including its ellipsis.
    pub maximum_label_columns: Option<u16>,
    /// Blank display cells placed on each side of every tab.
    pub tab_padding_columns: u16,
    /// Blank display cells separating secondary tabs and overflow arrows.
    pub secondary_gap_columns: u16,
    /// Caller-supplied semantic styles for the complete row.
    pub styles: TuiTabBarStyles,
}

impl TuiTabBarConfig {
    /// Creates a tab-bar configuration with neutral chrome defaults.
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

// -----------------------------------------------------------------------------
// Private layout model
// -----------------------------------------------------------------------------

/// Render-ready description of one tab after label truncation.
///
/// `source_index` reconnects a visible secondary tab to the leading element
/// that was built and measured before the pure layout calculation.
#[derive(Clone)]
struct RenderedTab {
    /// Stable callback and mouse-state identity.
    key: String,
    /// Index into the original secondary-tab and leading-element vectors.
    source_index: usize,
    /// Label after configured and width-derived truncation.
    label: String,
    /// Total display cells occupied by padding, leading content, gap, and label.
    width: u16,
}

/// One deterministic secondary page packed from `start`.
#[derive(Clone)]
struct PageLayout {
    /// Inclusive index of the first secondary tab considered for this page.
    start: usize,
    /// Exclusive index after the last tab that actually fit.
    end: usize,
    /// Visible tabs in paint order.
    tabs: Vec<RenderedTab>,
    /// Strictly later index that begins the next page, when one exists.
    next_start: Option<usize>,
}

/// Navigation-only data retained after the render-specific layout is discarded.
#[derive(Clone)]
struct SettledNavigation {
    /// Complete navigation order: optional main tab followed by all secondaries.
    order: Vec<String>,
    /// Explicit main-tab identity; never inferred from `order.first()`.
    main_tab_key: Option<String>,
    /// Selection used by the layout that produced this state.
    selected_key: Option<String>,
    /// Secondary keys that were actually painted.
    visible_secondary_keys: Vec<String>,
}

/// Complete output of the pure width calculation for one frame.
#[derive(Clone)]
struct TabBarLayout {
    /// Fixed main tab after label truncation.
    main_tab: Option<RenderedTab>,
    /// Anchor emitted by the previous overflow control.
    previous_anchor: Option<String>,
    /// Visible secondary tabs in paint order.
    tabs: Vec<RenderedTab>,
    /// Anchor emitted by the next overflow control.
    next_anchor: Option<String>,
    /// Lightweight subset published to the retained component after layout.
    navigation: SettledNavigation,
}
// -----------------------------------------------------------------------------
// Retained component state
// -----------------------------------------------------------------------------

/// UI-only state that survives per-frame element reconstruction.
/// Application selection and page state deliberately do not live here.
#[derive(Default)]
struct TuiTabBarState {
    /// Stable pointer state for every currently supplied tab key.
    mouse_states: HashMap<String, MouseStateHandle>,
    /// Pointer state for the previous overflow arrow.
    previous_overflow_mouse_state: MouseStateHandle,
    /// Pointer state for the next overflow arrow.
    next_overflow_mouse_state: MouseStateHandle,
    /// Navigation data from the latest completed layout.
    settled_navigation: Option<SettledNavigation>,
}

/// Retained owner for mouse state and settled keyboard navigation.
///
/// Views keep one instance of this type and call [`Self::render`] each frame.
/// The returned element owns all width-dependent rendering for that frame.
#[derive(Default)]
pub struct TuiTabBar {
    /// Shared bridge between the retained owner and the per-frame element.
    state: Rc<RefCell<TuiTabBarState>>,
}

impl TuiTabBar {
    /// Creates an empty retained tab-bar component.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a per-frame element from caller-owned semantic state.
    ///
    /// This reconciles the retained mouse-state map with the supplied keys,
    /// invalidates stale navigation, and moves the config into a
    /// [`TabBarElement`] that will resolve paging during layout.
    pub fn render<F>(&self, config: TuiTabBarConfig, on_event: F) -> Box<dyn TuiElement>
    where
        F: for<'a> Fn(TuiTabBarEvent, &mut TuiEventContext<'a>, &AppContext) + 'static,
    {
        let live_keys: HashSet<_> = config
            .main_tab
            .iter()
            .chain(config.tabs.iter())
            .map(|tab| tab.key.clone())
            .collect();
        {
            let mut state = self.state.borrow_mut();
            state.mouse_states.retain(|key, _| live_keys.contains(key));
            for key in live_keys {
                state.mouse_states.entry(key).or_default();
            }
            state.settled_navigation = None;
        }
        TabBarElement {
            state: self.state.clone(),
            config,
            on_event: Rc::new(on_event),
            row: None,
            layout: None,
            size: None,
            origin: None,
        }
        .finish()
    }

    /// Resolves a keyboard-navigation target from the latest completed layout.
    ///
    /// Visible selections navigate through the complete order and wrap.
    /// Off-page selections enter the visible page from its nearest edge.
    /// Returns `None` before the first layout or when no visible target exists.
    pub fn navigation_target(&self, direction: TuiTabBarNavigationDirection) -> Option<String> {
        let state = self.state.borrow();
        let navigation = state.settled_navigation.as_ref()?;
        if navigation.order.is_empty() {
            return None;
        }
        let selected_index = navigation
            .selected_key
            .as_ref()
            .and_then(|selected| navigation.order.iter().position(|key| key == selected));
        let selected_is_visible = navigation.selected_key.as_ref().is_some_and(|selected| {
            navigation.visible_secondary_keys.contains(selected)
                || navigation.main_tab_key.as_ref() == Some(selected)
        });
        if selected_is_visible {
            let selected_index = selected_index?;
            let target_index = match direction {
                TuiTabBarNavigationDirection::Previous => selected_index
                    .checked_sub(1)
                    .unwrap_or(navigation.order.len() - 1),
                TuiTabBarNavigationDirection::Next => (selected_index + 1) % navigation.order.len(),
            };
            return navigation.order.get(target_index).cloned();
        }
        match direction {
            TuiTabBarNavigationDirection::Previous => {
                navigation.visible_secondary_keys.last().cloned()
            }
            TuiTabBarNavigationDirection::Next => {
                navigation.visible_secondary_keys.first().cloned()
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Per-frame element construction
// -----------------------------------------------------------------------------

/// Per-frame element that turns a config plus an assigned width into a row.
struct TabBarElement {
    /// Retained state used for mouse handles and settled navigation publication.
    state: Rc<RefCell<TuiTabBarState>>,
    /// Caller-owned semantic inputs for this frame.
    config: TuiTabBarConfig,
    /// Semantic event callback shared by all clickable children.
    on_event: Rc<EventHandler>,
    /// Composed row built during layout and delegated to afterward.
    row: Option<TuiFlex>,
    /// Width-derived result kept until `after_layout` publishes navigation.
    layout: Option<TabBarLayout>,
    /// Full assigned component size from the latest layout.
    size: Option<TuiSize>,
    /// Screen-space origin from the latest paint.
    origin: Option<TuiScreenPoint>,
}

impl TabBarElement {
    /// Resolves a tab's caller-supplied style from selection and focus.
    fn tab_style(&self, key: &str) -> TuiStyle {
        if self.config.selected_key.as_deref() != Some(key) {
            self.config.styles.tab
        } else if self.config.focused {
            self.config.styles.selected_focused
        } else {
            self.config.styles.selected_unfocused
        }
    }

    /// Builds one clickable tab from a settled label and measured leading child.
    ///
    /// Text receives the tab's selected/focused style, hover bolds only the
    /// label, and an outer container fills selected background behind arbitrary
    /// caller-supplied leading content.
    fn render_tab(
        &self,
        tab: &RenderedTab,
        leading_element: Option<Box<dyn TuiElement>>,
    ) -> Box<dyn TuiElement> {
        let padding = " ".repeat(usize::from(self.config.tab_padding_columns));
        let state = self
            .state
            .borrow()
            .mouse_states
            .get(&tab.key)
            .cloned()
            .expect("tab mouse state was initialized before layout");
        let is_hovered = state.lock().unwrap().is_hovered();
        let tab_style = self.tab_style(&tab.key);
        let label_style = if is_hovered {
            tab_style.add_modifier(Modifier::BOLD)
        } else {
            tab_style
        };
        let mut row = TuiFlex::row().child(
            TuiText::new(padding.clone())
                .with_style(tab_style)
                .truncate()
                .finish(),
        );
        if let Some(leading_element) = leading_element {
            row = row.child(leading_element);
            if !tab.label.is_empty() {
                row = row.child(TuiText::new(" ").with_style(tab_style).truncate().finish());
            }
        }
        row = row
            .child(
                TuiText::new(tab.label.clone())
                    .with_style(label_style)
                    .truncate()
                    .finish(),
            )
            .child(
                TuiText::new(padding)
                    .with_style(tab_style)
                    .truncate()
                    .finish(),
            );
        let key = tab.key.clone();
        let on_event = self.on_event.clone();
        let row = row.finish();
        let content = if let Some(background) = tab_style.bg {
            TuiContainer::new(row).with_background(background).finish()
        } else {
            row
        };
        TuiHoverable::new(state, content)
            .on_click(move |event_ctx, app| {
                on_event(TuiTabBarEvent::SelectTab(key.clone()), event_ctx, app);
            })
            .finish()
    }

    /// Builds and measures a tab's leading child exactly once for this layout.
    fn build_leading_element(
        tab: &TuiTab,
        size: TuiSize,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> (Option<Box<dyn TuiElement>>, u16) {
        let Some(build_leading_element) = &tab.leading_element else {
            return (None, 0);
        };
        let mut element = build_leading_element();
        let width = element.layout(TuiConstraint::loose(size), ctx, app).width;
        (Some(element), width)
    }

    /// Builds a clickable, hover-bold overflow arrow for a settled page anchor.
    fn render_overflow(
        &self,
        text: &'static str,
        anchor: String,
        is_previous: bool,
    ) -> Box<dyn TuiElement> {
        let state = {
            let state = self.state.borrow();
            if is_previous {
                state.previous_overflow_mouse_state.clone()
            } else {
                state.next_overflow_mouse_state.clone()
            }
        };
        let style = if state.lock().unwrap().is_hovered() {
            self.config.styles.chrome.add_modifier(Modifier::BOLD)
        } else {
            self.config.styles.chrome
        };
        let on_event = self.on_event.clone();
        TuiHoverable::new(
            state,
            TuiText::new(text).with_style(style).truncate().finish(),
        )
        .on_click(move |event_ctx, app| {
            on_event(TuiTabBarEvent::PageChanged(anchor.clone()), event_ctx, app);
        })
        .finish()
    }

    /// Renders the configured spacing between pageable row items.
    fn render_gap(&self) -> Box<dyn TuiElement> {
        TuiText::new(" ".repeat(usize::from(self.config.secondary_gap_columns)))
            .with_style(self.config.styles.bar)
            .truncate()
            .finish()
    }

    /// Renders the caller-provided product label at the start of the row.
    fn render_leading(&self, leading: &str) -> Box<dyn TuiElement> {
        TuiText::new(leading)
            .with_style(self.config.styles.leading)
            .truncate()
            .finish()
    }

    /// Renders the fixed divider between the main tab and pageable tabs.
    fn render_divider(&self) -> Box<dyn TuiElement> {
        TuiText::new(" |  ")
            .with_style(self.config.styles.chrome)
            .truncate()
            .finish()
    }

    /// Assembles the fixed visual order from named layout fields.
    ///
    /// Leading elements are moved from their measurement slots into visible
    /// tabs, ensuring the measured instance is the instance that gets painted.
    fn build_row(
        &self,
        layout: &TabBarLayout,
        main_leading_element: Option<Box<dyn TuiElement>>,
        secondary_leading_elements: &mut [Option<Box<dyn TuiElement>>],
    ) -> TuiFlex {
        let mut row = TuiFlex::row();
        if let Some(leading) = &self.config.leading {
            row = row.child(self.render_leading(leading));
        }
        if let Some(main_tab) = &layout.main_tab {
            row = row.child(self.render_tab(main_tab, main_leading_element));
            if !self.config.tabs.is_empty() {
                row = row.child(self.render_divider());
            }
        }
        if let Some(previous_anchor) = &layout.previous_anchor {
            row = row.child(self.render_overflow("←", previous_anchor.clone(), true));
            if !layout.tabs.is_empty() || layout.next_anchor.is_some() {
                row = row.child(self.render_gap());
            }
        }
        for (index, tab) in layout.tabs.iter().enumerate() {
            if index > 0 {
                row = row.child(self.render_gap());
            }
            let leading_element = secondary_leading_elements
                .get_mut(tab.source_index)
                .and_then(Option::take);
            row = row.child(self.render_tab(tab, leading_element));
        }
        if let Some(next_anchor) = &layout.next_anchor {
            if !layout.tabs.is_empty() {
                row = row.child(self.render_gap());
            }
            row = row.child(self.render_overflow("→", next_anchor.clone(), false));
        }
        row
    }
}

impl TuiElement for TabBarElement {
    /// Measures leading children, computes the visible page, and builds the row.
    ///
    /// All child construction stays local to this pass. The measured leading
    /// element instances are moved into the row before the row is laid out.
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let size = TuiSize::new(
            constraint.constrain_width(constraint.max.width),
            constraint.constrain_height(u16::from(constraint.max.height > 0)),
        );
        let (main_leading_element, main_leading_columns) = self
            .config
            .main_tab
            .as_ref()
            .map(|tab| Self::build_leading_element(tab, size, ctx, app))
            .unwrap_or((None, 0));
        let (mut secondary_leading_elements, secondary_leading_columns): (Vec<_>, Vec<_>) = self
            .config
            .tabs
            .iter()
            .map(|tab| Self::build_leading_element(tab, size, ctx, app))
            .unzip();
        let layout = tab_bar_layout(
            &self.config,
            main_leading_columns,
            &secondary_leading_columns,
            size.width,
        );
        let mut row = self.build_row(
            &layout,
            main_leading_element,
            &mut secondary_leading_elements,
        );
        row.layout(TuiConstraint::loose(size), ctx, app);
        self.row = Some(row);
        self.layout = Some(layout);
        self.size = Some(size);
        size
    }

    /// Publishes navigation only after the complete row layout has settled.
    fn after_layout(&mut self, ctx: &mut TuiLayoutContext, app: &AppContext) {
        if let Some(row) = &mut self.row {
            row.after_layout(ctx, app);
        }
        self.state.borrow_mut().settled_navigation =
            self.layout.as_ref().map(|layout| layout.navigation.clone());
    }

    /// Paints the bar background, then delegates content paint to the row.
    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.origin = Some(ctx.scene_point(origin));
        let Some(size) = self.size else {
            return;
        };
        surface.set_style(origin, size, self.config.styles.bar);
        if let Some(row) = &mut self.row {
            row.render(origin, surface, ctx);
        }
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.origin
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        if let Some(row) = &mut self.row {
            row.present(ctx);
        }
    }

    /// Delegates pointer interaction to the row's tab and overflow hoverables.
    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        app: &AppContext,
    ) -> bool {
        self.row
            .as_mut()
            .is_some_and(|row| row.dispatch_event(event, event_ctx, app))
    }
}

// -----------------------------------------------------------------------------
// Pure width and paging calculations
// -----------------------------------------------------------------------------

/// Computes the render-ready row and its retained navigation subset.
///
/// Fixed content is measured first. The remaining columns are handed to the
/// secondary-page packer. Selected-tab reveal and previous-page resolution
/// both use the same deterministic page sequence.
fn tab_bar_layout(
    config: &TuiTabBarConfig,
    main_leading_columns: u16,
    secondary_leading_columns: &[u16],
    available_columns: u16,
) -> TabBarLayout {
    let main_tab = config
        .main_tab
        .as_ref()
        .map(|tab| layout_tab(tab, 0, main_leading_columns, config, None));
    let mut fixed_columns = config
        .leading
        .as_deref()
        .map(text_width)
        .unwrap_or_default();
    if let Some(main_tab) = &main_tab {
        fixed_columns = fixed_columns.saturating_add(main_tab.width);
        if !config.tabs.is_empty() {
            fixed_columns = fixed_columns.saturating_add(text_width(" |  "));
        }
    }

    let secondary_columns = available_columns.saturating_sub(fixed_columns);
    let requested_start = config
        .page_anchor
        .as_ref()
        .and_then(|anchor| config.tabs.iter().position(|tab| &tab.key == anchor))
        .unwrap_or_default();
    let pages = deterministic_pages(config, secondary_leading_columns, secondary_columns);
    let mut page = page_layout(
        config,
        secondary_leading_columns,
        requested_start,
        secondary_columns,
    );
    if config.reveal_selected {
        if let Some(selected_index) = config
            .selected_key
            .as_ref()
            .and_then(|selected| config.tabs.iter().position(|tab| &tab.key == selected))
        {
            if selected_index < page.start || selected_index >= page.end {
                if let Some(selected_page) = pages.iter().find(|candidate| {
                    (selected_index >= candidate.start && selected_index < candidate.end)
                        || (candidate.tabs.is_empty() && candidate.start == selected_index)
                }) {
                    page = selected_page.clone();
                }
            }
        }
    }

    let previous_anchor = pages
        .iter()
        .take_while(|candidate| candidate.start < page.start)
        .last()
        .and_then(|candidate| config.tabs.get(candidate.start))
        .map(|tab| tab.key.clone());
    let next_anchor = page
        .next_start
        .and_then(|start| config.tabs.get(start))
        .map(|tab| tab.key.clone());
    let visible_secondary_keys = page.tabs.iter().map(|tab| tab.key.clone()).collect();
    let order = config
        .main_tab
        .iter()
        .chain(config.tabs.iter())
        .map(|tab| tab.key.clone())
        .collect();
    TabBarLayout {
        main_tab,
        previous_anchor,
        tabs: page.tabs,
        next_anchor,
        navigation: SettledNavigation {
            order,
            main_tab_key: config.main_tab.as_ref().map(|tab| tab.key.clone()),
            selected_key: config.selected_key.clone(),
            visible_secondary_keys,
        },
    }
}

/// Enumerates every secondary page by following strictly advancing starts.
fn deterministic_pages(
    config: &TuiTabBarConfig,
    leading_columns: &[u16],
    available_columns: u16,
) -> Vec<PageLayout> {
    if config.tabs.is_empty() {
        return Vec::new();
    }
    let mut pages = Vec::new();
    let mut start = 0;
    loop {
        let page = page_layout(config, leading_columns, start, available_columns);
        let next_start = page.next_start;
        pages.push(page);
        let Some(next_start) = next_start else {
            break;
        };
        start = next_start;
    }
    pages
}

/// Packs one secondary page into the columns left after fixed chrome.
///
/// A full tab is preferred. If it does not fit, the final visible tab may use
/// the remaining label budget. If even its fixed content plus one label cell
/// cannot fit, the page advances without rendering that tab.
fn page_layout(
    config: &TuiTabBarConfig,
    leading_columns: &[u16],
    requested_start: usize,
    available_columns: u16,
) -> PageLayout {
    let start = requested_start.min(config.tabs.len().saturating_sub(1));
    let previous_columns = if start > 0 {
        text_width("←").saturating_add(config.secondary_gap_columns)
    } else {
        0
    };
    let mut remaining = available_columns.saturating_sub(previous_columns);
    let next_columns = text_width("→").saturating_add(config.secondary_gap_columns);
    let mut rendered_tabs = Vec::new();
    let mut end = start;

    for (index, tab) in config.tabs.iter().enumerate().skip(start) {
        let gap = if rendered_tabs.is_empty() {
            0
        } else {
            config.secondary_gap_columns
        };
        let reserve_next = if index + 1 < config.tabs.len() {
            next_columns
        } else {
            0
        };
        let available_for_tab = remaining.saturating_sub(gap).saturating_sub(reserve_next);
        let leading_columns = leading_columns.get(index).copied().unwrap_or_default();
        let rendered = layout_tab(tab, index, leading_columns, config, None);
        if rendered.width <= available_for_tab {
            remaining = remaining.saturating_sub(gap.saturating_add(rendered.width));
            rendered_tabs.push(rendered);
            end = index + 1;
            continue;
        }
        let fixed = tab_fixed_columns(tab, leading_columns, config.tab_padding_columns);
        let minimum = fixed.saturating_add(u16::from(!tab.label.is_empty()));
        if available_for_tab >= minimum {
            rendered_tabs.push(layout_tab(
                tab,
                index,
                leading_columns,
                config,
                Some(available_for_tab),
            ));
            end = index + 1;
        }
        break;
    }

    let candidate = if end > start {
        end
    } else {
        start.saturating_add(1)
    };
    PageLayout {
        start,
        end,
        tabs: rendered_tabs,
        next_start: (candidate < config.tabs.len()).then_some(candidate),
    }
}

/// Produces one render-ready tab description within a total column budget.
fn layout_tab(
    tab: &TuiTab,
    source_index: usize,
    leading_columns: u16,
    config: &TuiTabBarConfig,
    total_columns: Option<u16>,
) -> RenderedTab {
    let fixed_columns = tab_fixed_columns(tab, leading_columns, config.tab_padding_columns);
    let configured_label_columns = config
        .maximum_label_columns
        .unwrap_or_else(|| text_width(&tab.label));
    let label_columns = total_columns
        .map(|columns| columns.saturating_sub(fixed_columns))
        .unwrap_or(configured_label_columns)
        .min(configured_label_columns);
    let label = truncate_with_ellipsis(&tab.label, usize::from(label_columns));
    RenderedTab {
        key: tab.key.clone(),
        source_index,
        width: fixed_columns.saturating_add(text_width(&label)),
        label,
    }
}

/// Returns columns occupied before label text is added.
fn tab_fixed_columns(tab: &TuiTab, leading_columns: u16, padding_columns: u16) -> u16 {
    padding_columns
        .saturating_mul(2)
        .saturating_add(leading_columns)
        .saturating_add(u16::from(
            tab.leading_element.is_some() && !tab.label.is_empty(),
        ))
}

#[cfg(test)]
#[path = "tab_bar_tests.rs"]
mod tests;
