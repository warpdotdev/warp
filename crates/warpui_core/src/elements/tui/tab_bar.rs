//! Retained horizontal TUI tab bar with an optional fixed main tab and
//! width-aware paging for secondary tabs.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::rc::Rc;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::{
    TuiConstraint, TuiElement, TuiEvent, TuiEventContext, TuiFlex, TuiHoverable, TuiLayoutContext,
    TuiPaintContext, TuiPaintSurface, TuiPresentationContext, TuiScreenPoint, TuiScreenPosition,
    TuiSize, TuiStyle, TuiText,
};
use crate::elements::MouseStateHandle;
use crate::AppContext;

const ELLIPSIS: &str = "...";

/// Styled text used for fixed tab-bar chrome or a tab's leading glyph.
#[derive(Clone, Debug, Default)]
pub struct TuiTabBarText {
    pub text: String,
    pub style: TuiStyle,
}

impl TuiTabBarText {
    /// Creates styled tab-bar text.
    pub fn new(text: impl Into<String>, style: TuiStyle) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

/// One stable tab supplied by a tab-bar owner.
#[derive(Clone, Debug)]
pub struct TuiTab<K> {
    pub key: K,
    pub label: String,
    pub leading: Option<TuiTabBarText>,
}

impl<K> TuiTab<K> {
    /// Creates a tab without a leading glyph.
    pub fn new(key: K, label: impl Into<String>) -> Self {
        Self {
            key,
            label: label.into(),
            leading: None,
        }
    }

    /// Adds styled leading text to the tab.
    pub fn with_leading(mut self, leading: TuiTabBarText) -> Self {
        self.leading = Some(leading);
        self
    }
}

/// Caller-supplied styles for a tab-bar row.
#[derive(Clone, Copy, Debug, Default)]
pub struct TuiTabBarStyles {
    pub bar: TuiStyle,
    pub tab: TuiStyle,
    pub selected_focused: TuiStyle,
    pub selected_unfocused: TuiStyle,
}

/// Semantic events emitted by pointer interaction with the tab bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TuiTabBarEvent<K> {
    SelectTab(K),
    PageChanged(K),
}

/// Direction for width-aware tab navigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuiTabBarNavigationDirection {
    Previous,
    Next,
}

type EventHandler<K> = Rc<dyn for<'a> Fn(TuiTabBarEvent<K>, &mut TuiEventContext<'a>, &AppContext)>;

/// Per-render input for [`TuiTabBar`].
pub struct TuiTabBarConfig<K> {
    pub leading: Option<TuiTabBarText>,
    pub main_tab: Option<TuiTab<K>>,
    pub divider: Option<TuiTabBarText>,
    pub tabs: Vec<TuiTab<K>>,
    pub selected_key: Option<K>,
    pub focused: bool,
    pub page_anchor: Option<K>,
    pub reveal_selected: bool,
    pub maximum_label_columns: Option<u16>,
    pub tab_padding_columns: u16,
    pub secondary_gap_columns: u16,
    pub previous_overflow: TuiTabBarText,
    pub next_overflow: TuiTabBarText,
    pub styles: TuiTabBarStyles,
    on_event: EventHandler<K>,
}

impl<K> TuiTabBarConfig<K> {
    /// Creates a tab-bar configuration with neutral chrome defaults.
    pub fn new(
        tabs: Vec<TuiTab<K>>,
        on_event: impl for<'a> Fn(TuiTabBarEvent<K>, &mut TuiEventContext<'a>, &AppContext) + 'static,
    ) -> Self {
        Self {
            leading: None,
            main_tab: None,
            divider: None,
            tabs,
            selected_key: None,
            focused: false,
            page_anchor: None,
            reveal_selected: false,
            maximum_label_columns: None,
            tab_padding_columns: 1,
            secondary_gap_columns: 1,
            previous_overflow: TuiTabBarText::new("←", TuiStyle::default()),
            next_overflow: TuiTabBarText::new("→", TuiStyle::default()),
            styles: TuiTabBarStyles::default(),
            on_event: Rc::new(on_event),
        }
    }
}

#[derive(Clone)]
struct RenderedTab<K> {
    tab: TuiTab<K>,
    label: String,
    width: u16,
}

#[derive(Clone)]
struct PageLayout<K> {
    start: usize,
    end: usize,
    tabs: Vec<RenderedTab<K>>,
    previous_anchor: Option<K>,
    next_anchor: Option<K>,
}

#[derive(Clone)]
enum LayoutPiece<K> {
    Text(TuiTabBarText),
    Gap(u16),
    Tab(RenderedTab<K>),
    Overflow {
        text: TuiTabBarText,
        anchor: K,
        is_previous: bool,
    },
}

#[derive(Clone)]
struct TabBarLayout<K> {
    pieces: Vec<LayoutPiece<K>>,
    order: Vec<K>,
    selected_key: Option<K>,
    visible_secondary_keys: Vec<K>,
}

struct TuiTabBarState<K> {
    mouse_states: HashMap<K, MouseStateHandle>,
    previous_overflow_mouse_state: MouseStateHandle,
    next_overflow_mouse_state: MouseStateHandle,
    settled_layout: Option<TabBarLayout<K>>,
}

impl<K> Default for TuiTabBarState<K> {
    fn default() -> Self {
        Self {
            mouse_states: HashMap::new(),
            previous_overflow_mouse_state: MouseStateHandle::default(),
            next_overflow_mouse_state: MouseStateHandle::default(),
            settled_layout: None,
        }
    }
}

/// Retained TUI tab-bar component. Width-derived layout stays private; owners
/// receive only semantic tab keys and page anchors.
pub struct TuiTabBar<K> {
    state: Rc<RefCell<TuiTabBarState<K>>>,
}

impl<K> Default for TuiTabBar<K> {
    fn default() -> Self {
        Self {
            state: Rc::new(RefCell::new(TuiTabBarState::default())),
        }
    }
}

impl<K> TuiTabBar<K>
where
    K: Clone + Eq + Hash + 'static,
{
    /// Creates an empty retained tab-bar component.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds the per-frame tab-bar element from caller-owned semantic state.
    pub fn render(&self, config: TuiTabBarConfig<K>) -> Box<dyn TuiElement> {
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
            state.settled_layout = None;
        }
        TabBarElement {
            state: self.state.clone(),
            config,
            row: None,
            layout: None,
            size: None,
            origin: None,
        }
        .finish()
    }

    /// Resolves navigation from the component's private settled layout.
    pub fn navigation_target(&self, direction: TuiTabBarNavigationDirection) -> Option<K> {
        let state = self.state.borrow();
        let layout = state.settled_layout.as_ref()?;
        if layout.order.is_empty() {
            return None;
        }
        let selected_index = layout
            .selected_key
            .as_ref()
            .and_then(|selected| layout.order.iter().position(|key| key == selected));
        let selected_is_visible = layout.selected_key.as_ref().is_some_and(|selected| {
            layout.visible_secondary_keys.contains(selected)
                || layout.order.first() == Some(selected)
        });
        if selected_is_visible {
            let selected_index = selected_index?;
            let target_index = match direction {
                TuiTabBarNavigationDirection::Previous => selected_index
                    .checked_sub(1)
                    .unwrap_or(layout.order.len() - 1),
                TuiTabBarNavigationDirection::Next => (selected_index + 1) % layout.order.len(),
            };
            return layout.order.get(target_index).cloned();
        }
        match direction {
            TuiTabBarNavigationDirection::Previous => layout.visible_secondary_keys.last().cloned(),
            TuiTabBarNavigationDirection::Next => layout.visible_secondary_keys.first().cloned(),
        }
    }
}

struct TabBarElement<K> {
    state: Rc<RefCell<TuiTabBarState<K>>>,
    config: TuiTabBarConfig<K>,
    row: Option<TuiFlex>,
    layout: Option<TabBarLayout<K>>,
    size: Option<TuiSize>,
    origin: Option<TuiScreenPoint>,
}

impl<K> TabBarElement<K>
where
    K: Clone + Eq + Hash + 'static,
{
    /// Resolves a tab's caller-supplied style from selection and focus.
    fn tab_style(&self, key: &K) -> TuiStyle {
        if self.config.selected_key.as_ref() != Some(key) {
            self.config.styles.tab
        } else if self.config.focused {
            self.config.styles.selected_focused
        } else {
            self.config.styles.selected_unfocused
        }
    }

    /// Builds one clickable tab from its settled label and retained mouse state.
    fn tab_element(&self, tab: &RenderedTab<K>) -> Box<dyn TuiElement> {
        let padding = " ".repeat(usize::from(self.config.tab_padding_columns));
        let mut spans = vec![(padding.clone(), TuiStyle::default())];
        if let Some(leading) = &tab.tab.leading {
            spans.push((leading.text.clone(), leading.style));
            if !tab.label.is_empty() {
                spans.push((" ".to_string(), TuiStyle::default()));
            }
        }
        spans.push((tab.label.clone(), TuiStyle::default()));
        spans.push((padding, TuiStyle::default()));
        let state = self
            .state
            .borrow()
            .mouse_states
            .get(&tab.tab.key)
            .cloned()
            .expect("tab mouse state was initialized before layout");
        let key = tab.tab.key.clone();
        let on_event = self.config.on_event.clone();
        TuiHoverable::new(
            state,
            TuiText::from_spans(spans)
                .with_style(self.tab_style(&tab.tab.key))
                .truncate()
                .finish(),
        )
        .on_click(move |event_ctx, app| {
            on_event(TuiTabBarEvent::SelectTab(key.clone()), event_ctx, app);
        })
        .finish()
    }

    /// Builds a clickable overflow control for a settled page anchor.
    fn overflow_element(
        &self,
        text: &TuiTabBarText,
        anchor: K,
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
        let on_event = self.config.on_event.clone();
        TuiHoverable::new(
            state,
            TuiText::new(text.text.clone())
                .with_style(text.style)
                .truncate()
                .finish(),
        )
        .on_click(move |event_ctx, app| {
            on_event(TuiTabBarEvent::PageChanged(anchor.clone()), event_ctx, app);
        })
        .finish()
    }

    /// Converts settled layout pieces into the row delegated to for paint and events.
    fn build_row(&self, layout: &TabBarLayout<K>) -> TuiFlex {
        let mut row = TuiFlex::row();
        for piece in &layout.pieces {
            let element = match piece {
                LayoutPiece::Text(text) => TuiText::new(text.text.clone())
                    .with_style(text.style)
                    .truncate()
                    .finish(),
                LayoutPiece::Gap(columns) => TuiText::new(" ".repeat(usize::from(*columns)))
                    .with_style(self.config.styles.bar)
                    .truncate()
                    .finish(),
                LayoutPiece::Tab(tab) => self.tab_element(tab),
                LayoutPiece::Overflow {
                    text,
                    anchor,
                    is_previous,
                } => self.overflow_element(text, anchor.clone(), *is_previous),
            };
            row = row.child(element);
        }
        row
    }
}

impl<K> TuiElement for TabBarElement<K>
where
    K: Clone + Eq + Hash + 'static,
{
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
        let layout = tab_bar_layout(&self.config, size.width);
        let mut row = self.build_row(&layout);
        row.layout(TuiConstraint::loose(size), ctx, app);
        self.row = Some(row);
        self.layout = Some(layout);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut TuiLayoutContext, app: &AppContext) {
        if let Some(row) = &mut self.row {
            row.after_layout(ctx, app);
        }
        self.state.borrow_mut().settled_layout = self.layout.clone();
    }

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

/// Settles the complete row, including fixed chrome and one secondary page.
fn tab_bar_layout<K>(config: &TuiTabBarConfig<K>, available_columns: u16) -> TabBarLayout<K>
where
    K: Clone + Eq,
{
    let mut pieces = Vec::new();
    let mut fixed_columns = 0u16;
    if let Some(leading) = &config.leading {
        fixed_columns = fixed_columns.saturating_add(text_width(&leading.text));
        pieces.push(LayoutPiece::Text(leading.clone()));
    }
    if let Some(main_tab) = &config.main_tab {
        let rendered = render_tab(main_tab, config, None);
        fixed_columns = fixed_columns.saturating_add(rendered.width);
        pieces.push(LayoutPiece::Tab(rendered));
    }
    if let Some(divider) = &config.divider {
        fixed_columns = fixed_columns.saturating_add(text_width(&divider.text));
        pieces.push(LayoutPiece::Text(divider.clone()));
    }

    let secondary_columns = available_columns.saturating_sub(fixed_columns);
    let start = config
        .page_anchor
        .as_ref()
        .and_then(|anchor| config.tabs.iter().position(|tab| &tab.key == anchor))
        .unwrap_or_default();
    let mut page = page_layout(config, start, secondary_columns);
    if config.reveal_selected {
        if let Some(selected_index) = config
            .selected_key
            .as_ref()
            .and_then(|selected| config.tabs.iter().position(|tab| &tab.key == selected))
        {
            if selected_index < page.start || selected_index >= page.end {
                let stable_anchor =
                    stable_page_anchor_for_index(config, selected_index, secondary_columns);
                page = page_layout(config, stable_anchor, secondary_columns);
            }
        }
    }
    page.previous_anchor = previous_page_anchor(config, page.start, secondary_columns)
        .and_then(|index| config.tabs.get(index).map(|tab| tab.key.clone()));
    if page.start > 0 {
        if let Some(anchor) = &page.previous_anchor {
            pieces.push(LayoutPiece::Overflow {
                text: config.previous_overflow.clone(),
                anchor: anchor.clone(),
                is_previous: true,
            });
            if !page.tabs.is_empty() || page.next_anchor.is_some() {
                pieces.push(LayoutPiece::Gap(config.secondary_gap_columns));
            }
        }
    }
    for (index, tab) in page.tabs.iter().enumerate() {
        if index > 0 {
            pieces.push(LayoutPiece::Gap(config.secondary_gap_columns));
        }
        pieces.push(LayoutPiece::Tab(tab.clone()));
    }
    if let Some(anchor) = &page.next_anchor {
        if !page.tabs.is_empty() {
            pieces.push(LayoutPiece::Gap(config.secondary_gap_columns));
        }
        pieces.push(LayoutPiece::Overflow {
            text: config.next_overflow.clone(),
            anchor: anchor.clone(),
            is_previous: false,
        });
    }

    let order = config
        .main_tab
        .iter()
        .chain(config.tabs.iter())
        .map(|tab| tab.key.clone())
        .collect();
    TabBarLayout {
        pieces,
        order,
        selected_key: config.selected_key.clone(),
        visible_secondary_keys: page.tabs.into_iter().map(|tab| tab.tab.key).collect(),
    }
}

/// Packs one secondary page into the columns left after fixed chrome.
fn page_layout<K>(
    config: &TuiTabBarConfig<K>,
    requested_start: usize,
    available_columns: u16,
) -> PageLayout<K>
where
    K: Clone,
{
    let start = requested_start.min(config.tabs.len().saturating_sub(1));
    let show_previous = start > 0;
    let previous_columns = if show_previous {
        text_width(&config.previous_overflow.text).saturating_add(config.secondary_gap_columns)
    } else {
        0
    };
    let mut remaining = available_columns.saturating_sub(previous_columns);
    let next_columns =
        text_width(&config.next_overflow.text).saturating_add(config.secondary_gap_columns);
    let mut rendered_tabs = Vec::new();
    let mut end = start;

    for (offset, tab) in config.tabs.iter().enumerate().skip(start) {
        let gap = if rendered_tabs.is_empty() {
            0
        } else {
            config.secondary_gap_columns
        };
        let has_later_tabs = offset + 1 < config.tabs.len();
        let reserve_next = if has_later_tabs { next_columns } else { 0 };
        let available_for_tab = remaining.saturating_sub(gap).saturating_sub(reserve_next);
        let rendered = render_tab(tab, config, None);
        if rendered.width <= available_for_tab {
            remaining = remaining.saturating_sub(gap.saturating_add(rendered.width));
            rendered_tabs.push(rendered);
            end = offset + 1;
            continue;
        }
        let fixed = tab_fixed_columns(tab, config.tab_padding_columns);
        let minimum = fixed.saturating_add(u16::from(!tab.label.is_empty()));
        if available_for_tab >= minimum {
            rendered_tabs.push(render_tab(tab, config, Some(available_for_tab)));
            end = offset + 1;
        }
        break;
    }

    let next_index = if end < config.tabs.len() {
        Some(if end > start {
            end
        } else {
            start.saturating_add(1).min(config.tabs.len() - 1)
        })
    } else {
        None
    };
    PageLayout {
        start,
        end,
        tabs: rendered_tabs,
        previous_anchor: None,
        next_anchor: next_index.and_then(|index| config.tabs.get(index).map(|tab| tab.key.clone())),
    }
}

/// Finds the preceding deterministic page anchor for `current_start`.
fn previous_page_anchor<K>(
    config: &TuiTabBarConfig<K>,
    current_start: usize,
    available_columns: u16,
) -> Option<usize>
where
    K: Clone,
{
    if current_start == 0 {
        return None;
    }
    let mut anchor = 0usize;
    loop {
        let page = page_layout(config, anchor, available_columns);
        let next = if page.end > anchor {
            page.end
        } else {
            anchor.saturating_add(1)
        };
        if next >= current_start {
            return Some(anchor);
        }
        if next <= anchor || next >= config.tabs.len() {
            return Some(current_start.saturating_sub(1));
        }
        anchor = next;
    }
}

/// Finds the deterministic page anchor whose range contains `target`.
fn stable_page_anchor_for_index<K>(
    config: &TuiTabBarConfig<K>,
    target: usize,
    available_columns: u16,
) -> usize
where
    K: Clone,
{
    let mut anchor = 0usize;
    while anchor < target {
        let page = page_layout(config, anchor, available_columns);
        if target < page.end {
            return anchor;
        }
        let next = if page.end > anchor {
            page.end
        } else {
            anchor.saturating_add(1)
        };
        if next <= anchor || next >= config.tabs.len() {
            break;
        }
        anchor = next;
    }
    anchor.min(target)
}

/// Truncates one tab to its configured or width-derived total column budget.
fn render_tab<K>(
    tab: &TuiTab<K>,
    config: &TuiTabBarConfig<K>,
    total_columns: Option<u16>,
) -> RenderedTab<K>
where
    K: Clone,
{
    let fixed_columns = tab_fixed_columns(tab, config.tab_padding_columns);
    let configured_label_columns = config
        .maximum_label_columns
        .unwrap_or_else(|| text_width(&tab.label));
    let label_columns = total_columns
        .map(|columns| columns.saturating_sub(fixed_columns))
        .unwrap_or(configured_label_columns)
        .min(configured_label_columns);
    let label = truncate_with_ellipsis(&tab.label, label_columns);
    RenderedTab {
        tab: tab.clone(),
        width: fixed_columns.saturating_add(text_width(&label)),
        label,
    }
}

/// Columns occupied by tab padding and optional leading text.
fn tab_fixed_columns<K>(tab: &TuiTab<K>, padding_columns: u16) -> u16 {
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

/// Returns terminal display-cell width, saturating at `u16::MAX`.
fn text_width(text: &str) -> u16 {
    u16::try_from(UnicodeWidthStr::width(text)).unwrap_or(u16::MAX)
}

/// Truncates at grapheme boundaries and keeps as much of `...` as fits.
fn truncate_with_ellipsis(text: &str, maximum_columns: u16) -> String {
    if text_width(text) <= maximum_columns {
        return text.to_owned();
    }
    let ellipsis_columns = text_width(ELLIPSIS).min(maximum_columns);
    let prefix_columns = maximum_columns.saturating_sub(ellipsis_columns);
    let mut prefix = String::new();
    let mut prefix_width = 0u16;
    for grapheme in UnicodeSegmentation::graphemes(text, true) {
        let grapheme_width = text_width(grapheme);
        if prefix_width.saturating_add(grapheme_width) > prefix_columns {
            break;
        }
        prefix.push_str(grapheme);
        prefix_width = prefix_width.saturating_add(grapheme_width);
    }
    prefix.push_str(&".".repeat(usize::from(ellipsis_columns)));
    prefix
}

#[cfg(test)]
#[path = "tab_bar_tests.rs"]
mod tests;
