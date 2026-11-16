use std::sync::{Arc, Mutex};

use image::ImageEncoder;
use warpui::color::ColorU;
use warpui::elements::{
    ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container, Empty, Fill, Flex,
    MainAxisSize, ParentElement, RowBackground, ScrollStateHandle, Scrollable, ScrollableElement,
    ScrollbarWidth, SelectableArea, SelectionHandle, Table, TableColumnWidth, TableConfig,
    TableHeader, TableStateHandle, Text,
};
use warpui::fonts::FamilyId;
use warpui::keymap::FixedBinding;
use warpui::platform::CapturedFrame;
use warpui::presenter::ChildView;
use warpui::{
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
    ViewHandle, WindowId,
};

use crate::CaptureConfig;

const TOTAL_DEMOS: usize = 13;

#[derive(Debug, Clone)]
pub enum SampleAction {
    NextDemo,
    PreviousDemo,
}

pub fn init(ctx: &mut AppContext) {
    use warpui::keymap::macros::*;

    ctx.register_fixed_bindings([
        FixedBinding::new("right", SampleAction::NextDemo, id!("TableSampleView")),
        FixedBinding::new("left", SampleAction::PreviousDemo, id!("TableSampleView")),
    ]);
}

pub struct RootView {
    sub_view: ViewHandle<TableSampleView>,
}

impl RootView {
    pub fn new(ctx: &mut ViewContext<Self>, capture_config: CaptureConfig) -> Self {
        let config_clone = capture_config.clone();
        let sub_view = ctx.add_typed_action_view(move |ctx| {
            let font_family = warpui::fonts::Cache::handle(ctx)
                .update(ctx, |cache, _| cache.load_system_font("Arial").unwrap());
            ctx.focus_self();
            TableSampleView::new(font_family, config_clone.clone(), ctx)
        });
        Self { sub_view }
    }
}

impl Entity for RootView {
    type Event = ();
}

impl View for RootView {
    fn ui_name() -> &'static str {
        "RootView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.sub_view).finish()
    }
}

impl TypedActionView for RootView {
    type Action = ();
}

pub struct TableSampleView {
    font_family: FamilyId,
    current_demo: usize,
    table_states: Vec<TableStateHandle>,
    scroll_state: ScrollStateHandle,
    scroll_state_fixed_header: ScrollStateHandle,
    scroll_state_padding: ClippedScrollStateHandle,
    scroll_state_edge_cases: ClippedScrollStateHandle,
    scroll_state_varying_heights: ScrollStateHandle,
    scroll_state_fixed_header_mixed_columns: ScrollStateHandle,
    selection_handle_1: SelectionHandle,
    selection_handle_2: SelectionHandle,
    selection_handle_virtualized: SelectionHandle,
    selection_handle_fixed_header: SelectionHandle,
    selection_handle_varying_heights: SelectionHandle,
    selection_handle_fixed_header_mixed_columns: SelectionHandle,
    capture_config: CaptureConfig,
    window_id: WindowId,
    captured_count: Arc<Mutex<usize>>,
}

impl TableSampleView {
    fn new(
        font_family: FamilyId,
        capture_config: CaptureConfig,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let window_id = ctx.window_id();
        let captured_count = Arc::new(Mutex::new(0));

        // Auto-start capture sequence if enabled
        let should_auto_capture = capture_config.capture_screenshots;

        let view = Self {
            font_family,
            current_demo: 0,
            table_states: (0..TOTAL_DEMOS)
                .map(|_| TableStateHandle::new(0, |_, _| vec![]))
                .collect(),
            scroll_state: ScrollStateHandle::default(),
            scroll_state_fixed_header: ScrollStateHandle::default(),
            scroll_state_padding: ClippedScrollStateHandle::default(),
            scroll_state_edge_cases: ClippedScrollStateHandle::default(),
            scroll_state_varying_heights: ScrollStateHandle::default(),
            scroll_state_fixed_header_mixed_columns: ScrollStateHandle::default(),
            selection_handle_1: SelectionHandle::default(),
            selection_handle_2: SelectionHandle::default(),
            selection_handle_virtualized: SelectionHandle::default(),
            selection_handle_fixed_header: SelectionHandle::default(),
            selection_handle_varying_heights: SelectionHandle::default(),
            selection_handle_fixed_header_mixed_columns: SelectionHandle::default(),
            capture_config,
            window_id,
            captured_count,
        };

        // Trigger automated capture sequence driven from a background future
        if should_auto_capture {
            let spawner = ctx.spawner();
            ctx.spawn(
                async move {
                    use warpui::r#async::Timer;
                    Timer::after(std::time::Duration::from_millis(800)).await;
                    for i in 0..TOTAL_DEMOS {
                        let _ = spawner
                            .spawn(move |view, ctx| {
                                view.current_demo = i;
                                ctx.notify();
                            })
                            .await;
                        Timer::after(std::time::Duration::from_millis(300)).await;
                        let _ = spawner
                            .spawn(|view, ctx| {
                                view.capture_current_demo(ctx);
                            })
                            .await;
                        Timer::after(std::time::Duration::from_millis(250)).await;
                    }
                },
                |_, _, _| {},
            );
        }

        view
    }

    fn capture_current_demo(&mut self, ctx: &mut ViewContext<Self>) {
        let demo_names = [
            "01_column_widths",
            "02_virtualized",
            "03_virtualized_fixed_header",
            "04_selection",
            "05_selection_virtualized",
            "06_text_wrapping",
            "07_mixed_elements",
            "08_padding",
            "09_banding",
            "10_theming",
            "11_edge_cases",
            "12_varying_heights",
            "13_fixed_header_mixed_columns",
        ];

        let demo_name = demo_names[self.current_demo].to_string();
        let is_baseline = self.capture_config.capture_baseline;
        let captured_count = Arc::clone(&self.captured_count);

        if let Some(window) = ctx.windows().platform_window(self.window_id) {
            println!(
                "📸 Capturing demo {}/{}: {}",
                self.current_demo + 1,
                TOTAL_DEMOS,
                demo_name
            );
            let is_last_demo = self.current_demo >= TOTAL_DEMOS - 1;
            let auto_capture = self.capture_config.capture_screenshots;

            window
                .as_ctx()
                .request_frame_capture(Box::new(move |frame| {
                    let dir = if is_baseline {
                        "screenshots/baseline"
                    } else {
                        "screenshots/current"
                    };
                    let _ = std::fs::create_dir_all(dir);
                    let filename = format!("{}/{}.png", dir, demo_name);
                    if let Err(e) = save_frame_as_png(&frame, &filename) {
                        eprintln!("❌ Failed to save {}: {}", filename, e);
                        return;
                    }
                    println!("✅ Saved: {}", filename);
                    let mut count = captured_count.lock().unwrap();
                    *count += 1;
                    if *count >= TOTAL_DEMOS {
                        println!("\n🎉 All {} screenshots captured!", TOTAL_DEMOS);
                        std::process::exit(0);
                    }
                    // Next demo will be scheduled below using a timer on the main thread
                }));

            if auto_capture && !is_last_demo {
                ctx.spawn(
                    async {
                        warpui::r#async::Timer::after(std::time::Duration::from_millis(350)).await;
                    },
                    |_, _, ctx| {
                        ctx.dispatch_typed_action(&SampleAction::NextDemo);
                    },
                );
            }
        }
    }

    fn create_header(&self, text: &str) -> TableHeader {
        TableHeader::new(
            Text::new(text.to_string(), self.font_family, 14.0)
                .with_color(ColorU::new(30, 30, 30, 255))
                .finish(),
        )
    }

    fn create_header_with_width(&self, text: &str, width: TableColumnWidth) -> TableHeader {
        TableHeader::new(
            Text::new(text.to_string(), self.font_family, 14.0)
                .with_color(ColorU::new(30, 30, 30, 255))
                .finish(),
        )
        .with_width(width)
    }

    fn render_demo_header(&self, title: &str, description: &str) -> Box<dyn Element> {
        Flex::column()
            .with_spacing(8.0)
            .with_child(
                Text::new(
                    format!("Demo {}/{} - {}", self.current_demo + 1, TOTAL_DEMOS, title),
                    self.font_family,
                    24.0,
                )
                .with_color(ColorU::white())
                .finish(),
            )
            .with_child(
                Text::new(description.to_string(), self.font_family, 14.0)
                    .with_color(ColorU::new(180, 180, 180, 255))
                    .finish(),
            )
            .with_child(
                Text::new(
                    "Use ← → arrow keys to navigate between demos".to_string(),
                    self.font_family,
                    12.0,
                )
                .with_color(ColorU::new(120, 120, 120, 255))
                .finish(),
            )
            .finish()
    }

    fn render_demo_column_widths(&self) -> Box<dyn Element> {
        let font_family = self.font_family;
        let table = Table::new(self.table_states[0].clone(), 800.0, 500.0)
            .with_headers(vec![
                self.create_header_with_width("Fixed(100)", TableColumnWidth::Fixed(100.0)),
                self.create_header_with_width("Flex(1)", TableColumnWidth::Flex(1.0)),
                self.create_header_with_width("Flex(2)", TableColumnWidth::Flex(2.0)),
                self.create_header_with_width("Fraction(0.15)", TableColumnWidth::Fraction(0.15)),
                self.create_header_with_width("Intrinsic", TableColumnWidth::Intrinsic),
            ])
            .with_row_count(2)
            .with_row_render_fn(move |row_idx, _app| {
                let row_data: &[(&str, &str, &str, &str, &str)] = &[
                    ("100px", "1 part", "2 parts", "15%", "Auto-sized content"),
                    (
                        "Fixed",
                        "Flexible",
                        "More flexible",
                        "Percent",
                        "Fits content width",
                    ),
                ];
                let (a, b, c, d, e) = row_data[row_idx];
                vec![
                    Text::new(a.to_string(), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(b.to_string(), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(c.to_string(), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(d.to_string(), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(e.to_string(), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                ]
            })
            .with_config(TableConfig::default())
            .finish();

        Flex::column()
            .with_spacing(20.0)
            .with_child(self.render_demo_header(
                "Column Width Types",
                "Demonstrates Fixed, Flex, Fraction, and Intrinsic column widths",
            ))
            .with_child(Container::new(table).with_uniform_padding(10.0).finish())
            .finish()
    }

    fn render_demo_virtualized(&self) -> Box<dyn Element> {
        let row_count = 1000;
        let font_family = self.font_family;

        // Use on-demand row rendering - rows are created lazily during layout
        let table = Table::new(self.table_states[1].clone(), 800.0, 500.0)
            .with_headers(vec![
                self.create_header_with_width("ID", TableColumnWidth::Fixed(80.0)),
                self.create_header("Column A"),
                self.create_header("Column B"),
                self.create_header_with_width("Value", TableColumnWidth::Fixed(100.0)),
            ])
            .with_row_count(row_count)
            .with_row_render_fn(move |row_idx, _app| {
                vec![
                    Text::new(format!("Row {}", row_idx), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(format!("Data A-{}", row_idx), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(format!("Data B-{}", row_idx), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(format!("Value: {}", row_idx * 10), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                ]
            })
            .finish_scrollable();

        let scrollable = Scrollable::vertical(
            self.scroll_state.clone(),
            table,
            ScrollbarWidth::Auto,
            ColorU::new(60, 60, 60, 255).into(),
            ColorU::new(80, 80, 80, 255).into(),
            ColorU::new(100, 100, 100, 255).into(),
        );

        let selectable = SelectableArea::new(
            self.selection_handle_virtualized.clone(),
            |selection_args, _, _| {
                if let Some(text) = &selection_args.selection {
                    println!("Virtualized table selected: {:?}", text);
                }
            },
            scrollable.finish(),
        );

        Flex::column()
            .with_spacing(20.0)
            .with_child(self.render_demo_header(
                "Virtualized Table (Scrolling Header)",
                "1000 rows with scroll-based virtualization. Header scrolls with body.",
            ))
            .with_child(
                ConstrainedBox::new(
                    Container::new(selectable.finish())
                        .with_uniform_padding(10.0)
                        .finish(),
                )
                .with_height(500.0)
                .finish(),
            )
            .finish()
    }

    fn render_demo_virtualized_fixed_header(&self) -> Box<dyn Element> {
        let row_count = 1000;
        let font_family = self.font_family;

        let config = TableConfig {
            fixed_header: true,
            ..TableConfig::default()
        };

        // Use on-demand row rendering - rows are created lazily during layout
        let table = Table::new(self.table_states[2].clone(), 800.0, 500.0)
            .with_headers(vec![
                self.create_header_with_width("ID", TableColumnWidth::Fixed(80.0)),
                self.create_header("Column A"),
                self.create_header("Column B"),
                self.create_header_with_width("Value", TableColumnWidth::Fixed(100.0)),
            ])
            .with_row_count(row_count)
            .with_row_render_fn(move |row_idx, _app| {
                vec![
                    Text::new(format!("Row {}", row_idx), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(format!("Data A-{}", row_idx), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(format!("Data B-{}", row_idx), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(format!("Value: {}", row_idx * 10), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                ]
            })
            .with_config(config)
            .finish_scrollable();

        let scrollable = Scrollable::vertical(
            self.scroll_state_fixed_header.clone(),
            table,
            ScrollbarWidth::Auto,
            ColorU::new(60, 60, 60, 255).into(),
            ColorU::new(80, 80, 80, 255).into(),
            ColorU::new(100, 100, 100, 255).into(),
        );

        let selectable = SelectableArea::new(
            self.selection_handle_fixed_header.clone(),
            |selection_args, _, _| {
                if let Some(text) = &selection_args.selection {
                    println!("Fixed header table selected: {:?}", text);
                }
            },
            scrollable.finish(),
        );

        Flex::column()
            .with_spacing(20.0)
            .with_child(self.render_demo_header(
                "Virtualized Table (Fixed Header)",
                "1000 rows with scroll-based virtualization. Header stays fixed at top.",
            ))
            .with_child(
                ConstrainedBox::new(
                    Container::new(selectable.finish())
                        .with_uniform_padding(10.0)
                        .finish(),
                )
                .with_height(500.0)
                .finish(),
            )
            .finish()
    }

    fn render_demo_selection(&self) -> Box<dyn Element> {
        let font_family = self.font_family;
        let table = Table::new(self.table_states[3].clone(), 800.0, 500.0)
            .with_headers(vec![
                self.create_header("Name"),
                self.create_header("Email"),
                self.create_header("Role"),
            ])
            .with_row_count(5)
            .with_row_render_fn(move |row_idx, _app| {
                let row_data: &[(&str, &str, &str)] = &[
                    ("Alice Johnson", "alice@example.com", "Engineer"),
                    ("Bob Smith", "bob@example.com", "Designer"),
                    ("Carol Williams", "carol@example.com", "Manager"),
                    ("David Brown", "david@example.com", "Engineer"),
                    ("Eve Davis", "eve@example.com", "Designer"),
                ];
                let (name, email, role) = row_data[row_idx];
                vec![
                    Text::new(name.to_string(), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(email.to_string(), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(role.to_string(), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                ]
            })
            .with_config(TableConfig::default())
            .finish();

        let selectable = SelectableArea::new(
            self.selection_handle_1.clone(),
            |selection_args, _, _| {
                if let Some(text) = &selection_args.selection {
                    println!("Selected: {:?}", text);
                }
            },
            table,
        );

        Flex::column()
            .with_spacing(20.0)
            .with_child(self.render_demo_header(
                "Selection (Non-Virtualized)",
                "Click and drag to select text. Selection is logged to console.",
            ))
            .with_child(
                Container::new(selectable.finish())
                    .with_uniform_padding(10.0)
                    .finish(),
            )
            .finish()
    }

    fn render_demo_selection_virtualized(&self) -> Box<dyn Element> {
        let font_family = self.font_family;
        let table = Table::new(self.table_states[4].clone(), 800.0, 500.0)
            .with_headers(vec![
                self.create_header("Item"),
                self.create_header("Description"),
                self.create_header_with_width("Price", TableColumnWidth::Fixed(80.0)),
            ])
            .with_row_count(50)
            .with_row_render_fn(move |row_idx, _app| {
                vec![
                    Text::new(format!("Item {}", row_idx), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(
                        format!("Description for item {}", row_idx),
                        font_family,
                        14.0,
                    )
                    .with_color(ColorU::new(50, 50, 50, 255))
                    .finish(),
                    Text::new(
                        format!("${:.2}", (row_idx as f32) * 9.99),
                        font_family,
                        14.0,
                    )
                    .with_color(ColorU::new(50, 50, 50, 255))
                    .finish(),
                ]
            })
            .with_config(TableConfig::default())
            .finish();

        let selectable = SelectableArea::new(
            self.selection_handle_2.clone(),
            |selection_args, _, _| {
                if let Some(text) = &selection_args.selection {
                    println!("Virtualized selection: {:?}", text);
                }
            },
            table,
        );

        Flex::column()
            .with_spacing(20.0)
            .with_child(self.render_demo_header(
                "Selection (Virtualized)",
                "50 rows with visible range 5-25. Selection includes gap placeholders.",
            ))
            .with_child(
                Container::new(selectable.finish())
                    .with_uniform_padding(10.0)
                    .finish(),
            )
            .finish()
    }

    fn render_demo_text_wrapping(&self) -> Box<dyn Element> {
        let font_family = self.font_family;
        let table = Table::new(self.table_states[5].clone(), 800.0, 500.0)
            .with_headers(vec![
                self.create_header_with_width("Title", TableColumnWidth::Fixed(120.0)),
                self.create_header("Description"),
                self.create_header_with_width("Status", TableColumnWidth::Fixed(80.0)),
            ])
            .with_row_count(3)
            .with_row_render_fn(move |row_idx, _app| {
                let row_data: &[(&str, &str, &str)] = &[
                    ("Short", "A brief description", "Done"),
                    ("Medium Length", "This is a longer description that should wrap to multiple lines when the column width is constrained.", "In Progress"),
                    ("Very Long Title Here", "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.", "Pending"),
                ];
                let (title, desc, status) = row_data[row_idx];
                vec![
                    Text::new(title.to_string(), font_family, 14.0).with_color(ColorU::new(50, 50, 50, 255)).finish(),
                    Text::new(desc.to_string(), font_family, 14.0).with_color(ColorU::new(50, 50, 50, 255)).finish(),
                    Text::new(status.to_string(), font_family, 14.0).with_color(ColorU::new(50, 50, 50, 255)).finish(),
                ]
            })
            .with_config(TableConfig::default())
            .finish();

        Flex::column()
            .with_spacing(20.0)
            .with_child(self.render_demo_header(
                "Rich Cells - Text Wrapping",
                "Cells with varying text lengths. Row heights adjust to fit content.",
            ))
            .with_child(
                ConstrainedBox::new(Container::new(table).with_uniform_padding(10.0).finish())
                    .with_width(600.0)
                    .finish(),
            )
            .finish()
    }

    fn render_demo_mixed_elements(&self) -> Box<dyn Element> {
        let font_family = self.font_family;
        let table = Table::new(self.table_states[6].clone(), 800.0, 500.0)
            .with_headers(vec![
                self.create_header("Icon + Text"),
                self.create_header("Emojis"),
                self.create_header("Colored"),
            ])
            .with_row_count(4)
            .with_row_render_fn(move |row_idx, _app| {
                let row_data: &[(&str, &str, &str, ColorU)] = &[
                    (
                        "📁 Documents",
                        "🎉 🎊 🥳",
                        "Success",
                        ColorU::new(34, 139, 34, 255),
                    ),
                    (
                        "📷 Photos",
                        "❤️ 💙 💚 💛",
                        "Warning",
                        ColorU::new(255, 165, 0, 255),
                    ),
                    (
                        "🎵 Music",
                        "🌟 ⭐ 💫 ✨",
                        "Error",
                        ColorU::new(220, 20, 60, 255),
                    ),
                    (
                        "🎬 Videos",
                        "🚀 🛸 🌙 🌍",
                        "Info",
                        ColorU::new(30, 144, 255, 255),
                    ),
                ];
                let (icon_text, emojis, status, color) = row_data[row_idx];
                vec![
                    Text::new(icon_text.to_string(), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(emojis.to_string(), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(status.to_string(), font_family, 14.0)
                        .with_color(color)
                        .finish(),
                ]
            })
            .with_config(TableConfig::default())
            .finish();

        Flex::column()
            .with_spacing(20.0)
            .with_child(self.render_demo_header(
                "Rich Cells - Mixed Elements",
                "Cells with emojis, icons, and colored text",
            ))
            .with_child(Container::new(table).with_uniform_padding(10.0).finish())
            .finish()
    }

    fn render_demo_padding(&self) -> Box<dyn Element> {
        let font_family = self.font_family;
        let create_table = move |state: TableStateHandle, padding: f32, ff: FamilyId| {
            let config = TableConfig {
                cell_padding: padding,
                ..TableConfig::default()
            };
            Table::new(state, 800.0, 500.0)
                .with_headers(vec![
                    TableHeader::new(
                        Text::new("A".to_string(), ff, 14.0)
                            .with_color(ColorU::new(30, 30, 30, 255))
                            .finish(),
                    ),
                    TableHeader::new(
                        Text::new("B".to_string(), ff, 14.0)
                            .with_color(ColorU::new(30, 30, 30, 255))
                            .finish(),
                    ),
                ])
                .with_row_count(2)
                .with_row_render_fn(move |row_idx, _app| {
                    let cells = ["Cell 1", "Cell 2", "Cell 3", "Cell 4"];
                    vec![
                        Text::new(cells[row_idx * 2].to_string(), ff, 14.0)
                            .with_color(ColorU::new(50, 50, 50, 255))
                            .finish(),
                        Text::new(cells[row_idx * 2 + 1].to_string(), ff, 14.0)
                            .with_color(ColorU::new(50, 50, 50, 255))
                            .finish(),
                    ]
                })
                .with_config(config)
                .finish()
        };

        let tables_row = Flex::row()
            .with_spacing(20.0)
            .with_child(
                Flex::column()
                    .with_spacing(8.0)
                    .with_child(
                        Text::new_inline("4px padding", self.font_family, 12.0)
                            .with_color(ColorU::white())
                            .finish(),
                    )
                    .with_child(create_table(self.table_states[7].clone(), 4.0, font_family))
                    .finish(),
            )
            .with_child(
                Flex::column()
                    .with_spacing(8.0)
                    .with_child(
                        Text::new_inline("8px padding (default)", self.font_family, 12.0)
                            .with_color(ColorU::white())
                            .finish(),
                    )
                    .with_child(create_table(
                        TableStateHandle::new(0, |_, _| vec![]),
                        8.0,
                        font_family,
                    ))
                    .finish(),
            )
            .with_child(
                Flex::column()
                    .with_spacing(8.0)
                    .with_child(
                        Text::new_inline("16px padding", self.font_family, 12.0)
                            .with_color(ColorU::white())
                            .finish(),
                    )
                    .with_child(create_table(
                        TableStateHandle::new(0, |_, _| vec![]),
                        16.0,
                        font_family,
                    ))
                    .finish(),
            )
            .finish();

        let scrollable = ClippedScrollable::horizontal(
            self.scroll_state_padding.clone(),
            tables_row,
            ScrollbarWidth::Auto,
            ColorU::new(60, 60, 60, 255).into(),
            ColorU::new(80, 80, 80, 255).into(),
            ColorU::new(100, 100, 100, 255).into(),
        );

        Flex::column()
            .with_spacing(20.0)
            .with_child(self.render_demo_header(
                "Cell Padding Comparison",
                "Same table with different cell_padding values: 4px, 8px (default), 16px. Scroll horizontally if needed.",
            ))
            .with_child(scrollable.finish())
            .finish()
    }

    fn render_demo_banding(&self) -> Box<dyn Element> {
        let font_family = self.font_family;
        let config = TableConfig {
            row_background: RowBackground::striped(
                ColorU::white(),
                ColorU::new(240, 245, 250, 255),
            ),
            ..TableConfig::default()
        };

        let table = Table::new(self.table_states[8].clone(), 800.0, 500.0)
            .with_headers(vec![
                self.create_header("#"),
                self.create_header("Product"),
                self.create_header("Category"),
                self.create_header("Price"),
            ])
            .with_row_count(6)
            .with_row_render_fn(move |row_idx, _app| {
                let row_data: &[(&str, &str, &str, &str)] = &[
                    ("1", "Widget A", "Electronics", "$29.99"),
                    ("2", "Gadget B", "Electronics", "$49.99"),
                    ("3", "Tool C", "Hardware", "$19.99"),
                    ("4", "Device D", "Electronics", "$99.99"),
                    ("5", "Part E", "Hardware", "$9.99"),
                    ("6", "Component F", "Electronics", "$39.99"),
                ];
                let (num, product, category, price) = row_data[row_idx];
                vec![
                    Text::new(num.to_string(), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(product.to_string(), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(category.to_string(), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new(price.to_string(), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                ]
            })
            .with_config(config)
            .finish();

        Flex::column()
            .with_spacing(20.0)
            .with_child(self.render_demo_header(
                "Alternate Row Banding",
                "Zebra striping with alternate_row_background set",
            ))
            .with_child(Container::new(table).with_uniform_padding(10.0).finish())
            .finish()
    }

    fn render_demo_theming(&self) -> Box<dyn Element> {
        let font_family = self.font_family;
        let dark_config = TableConfig {
            border_width: 2.0,
            border_color: ColorU::new(80, 80, 80, 255),
            cell_padding: 12.0,
            header_background: ColorU::new(45, 45, 45, 255),
            row_background: RowBackground::striped(
                ColorU::new(30, 30, 30, 255),
                ColorU::new(40, 40, 40, 255),
            ),
            ..TableConfig::default()
        };

        let create_dark_header = |text: &str, ff: FamilyId| {
            TableHeader::new(
                Text::new(text.to_string(), ff, 14.0)
                    .with_color(ColorU::new(220, 220, 220, 255))
                    .finish(),
            )
        };

        let table = Table::new(self.table_states[9].clone(), 800.0, 500.0)
            .with_headers(vec![
                create_dark_header("Property", font_family),
                create_dark_header("Value", font_family),
                create_dark_header("Description", font_family),
            ])
            .with_row_count(4)
            .with_row_render_fn(move |row_idx, _app| {
                let row_data: &[(&str, &str, &str)] = &[
                    ("border_width", "2.0", "Thicker borders"),
                    ("border_color", "#505050", "Dark gray borders"),
                    ("header_background", "#2D2D2D", "Dark header"),
                    ("row_background", "#1E1E1E", "Very dark rows"),
                ];
                let (prop, val, desc) = row_data[row_idx];
                vec![
                    Text::new(prop.to_string(), font_family, 14.0)
                        .with_color(ColorU::new(200, 200, 200, 255))
                        .finish(),
                    Text::new(val.to_string(), font_family, 14.0)
                        .with_color(ColorU::new(200, 200, 200, 255))
                        .finish(),
                    Text::new(desc.to_string(), font_family, 14.0)
                        .with_color(ColorU::new(200, 200, 200, 255))
                        .finish(),
                ]
            })
            .with_config(dark_config)
            .finish();

        Flex::column()
            .with_spacing(20.0)
            .with_child(self.render_demo_header(
                "Border and Colors (Dark Theme)",
                "Custom theming with modified border, header, and row colors",
            ))
            .with_child(Container::new(table).with_uniform_padding(10.0).finish())
            .finish()
    }

    fn render_demo_edge_cases(&self) -> Box<dyn Element> {
        let font_family = self.font_family;

        let empty_table = Table::new(TableStateHandle::new(0, |_, _| vec![]), 800.0, 500.0)
            .with_headers(vec![
                self.create_header("A"),
                self.create_header("B"),
                self.create_header("C"),
            ])
            .finish();

        let single_row = Table::new(
            TableStateHandle::new(1, move |_, _| {
                vec![
                    Text::new("Single".to_string(), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                    Text::new("Entry".to_string(), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                ]
            }),
            800.0,
            500.0,
        )
        .with_headers(vec![self.create_header("Only"), self.create_header("Row")])
        .finish();

        let single_column = Table::new(
            TableStateHandle::new(3, move |row_idx, _| {
                vec![
                    Text::new(format!("Row {}", row_idx + 1), font_family, 14.0)
                        .with_color(ColorU::new(50, 50, 50, 255))
                        .finish(),
                ]
            }),
            800.0,
            500.0,
        )
        .with_headers(vec![self.create_header("Single Column")])
        .finish();

        let many_columns = Table::new(self.table_states[10].clone(), 800.0, 500.0)
            .with_headers(
                (1..=10)
                    .map(|i| {
                        self.create_header_with_width(
                            &format!("Col{}", i),
                            TableColumnWidth::Fixed(60.0),
                        )
                    })
                    .collect(),
            )
            .with_row_count(2)
            .with_row_render_fn(move |row_idx, _| {
                (1..=10)
                    .map(|i| {
                        Text::new(format!("R{}C{}", row_idx + 1, i), font_family, 14.0)
                            .with_color(ColorU::new(50, 50, 50, 255))
                            .finish()
                    })
                    .collect()
            })
            .finish();

        let tables_row = Flex::row()
            .with_spacing(20.0)
            .with_child(
                Flex::column()
                    .with_spacing(8.0)
                    .with_child(
                        Text::new_inline("Empty (headers only)", self.font_family, 12.0)
                            .with_color(ColorU::white())
                            .finish(),
                    )
                    .with_child(empty_table)
                    .finish(),
            )
            .with_child(
                Flex::column()
                    .with_spacing(8.0)
                    .with_child(
                        Text::new_inline("Single row", self.font_family, 12.0)
                            .with_color(ColorU::white())
                            .finish(),
                    )
                    .with_child(single_row)
                    .finish(),
            )
            .with_child(
                Flex::column()
                    .with_spacing(8.0)
                    .with_child(
                        Text::new_inline("Single column", self.font_family, 12.0)
                            .with_color(ColorU::white())
                            .finish(),
                    )
                    .with_child(single_column)
                    .finish(),
            )
            .finish();

        let scrollable = ClippedScrollable::horizontal(
            self.scroll_state_edge_cases.clone(),
            tables_row,
            ScrollbarWidth::Auto,
            ColorU::new(60, 60, 60, 255).into(),
            ColorU::new(80, 80, 80, 255).into(),
            ColorU::new(100, 100, 100, 255).into(),
        );

        Flex::column()
            .with_spacing(20.0)
            .with_child(self.render_demo_header(
                "Edge Cases",
                "Empty table, single row, single column, many columns. Scroll horizontally if needed.",
            ))
            .with_child(scrollable.finish())
            .with_child(
                Flex::column()
                    .with_spacing(8.0)
                    .with_child(
                        Text::new_inline("Many columns (10)", self.font_family, 12.0)
                            .with_color(ColorU::white())
                            .finish(),
                    )
                    .with_child(many_columns)
                    .finish(),
            )
            .finish()
    }

    fn render_demo_virtualized_varying_heights(&self) -> Box<dyn Element> {
        let row_count = 500;
        let font_family = self.font_family;

        let config = TableConfig {
            row_background: RowBackground::striped(
                ColorU::white(),
                ColorU::new(248, 250, 252, 255),
            ),
            ..TableConfig::default()
        };

        let table = Table::new(self.table_states[TOTAL_DEMOS - 2].clone(), 800.0, 500.0)
            .with_headers(vec![
                self.create_header_with_width("#", TableColumnWidth::Fixed(60.0)),
                self.create_header_with_width("Description", TableColumnWidth::Flex(1.0)),
                self.create_header_with_width("Price", TableColumnWidth::Fixed(100.0)),
            ])
            .with_row_count(row_count)
            .with_row_render_fn(move |row_idx, _app| {
                let description = match row_idx % 5 {
                    0 => format!("Row {} - Short", row_idx),
                    1 => format!("Row {} - This is a medium length description that will wrap to multiple lines in a constrained column", row_idx),
                    2 => format!("Row {} - Very long description here. Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam.", row_idx),
                    3 => format!("Row {} - Another short one", row_idx),
                    4 => format!("Row {} - Medium text here with some more content to make it wrap and create varying row heights throughout the virtualized table.", row_idx),
                    _ => unreachable!(),
                };
                vec![
                    Text::new(format!("{}", row_idx), font_family, 14.0).with_color(ColorU::new(50, 50, 50, 255)).finish(),
                    Text::new(description, font_family, 14.0).with_color(ColorU::new(50, 50, 50, 255)).finish(),
                    Text::new(format!("${}.{:02}", row_idx * 10, (row_idx * 17) % 100), font_family, 14.0).with_color(ColorU::new(50, 50, 50, 255)).finish(),
                ]
            })
            .with_config(config)
            .finish_scrollable();

        let scrollable = Scrollable::vertical(
            self.scroll_state_varying_heights.clone(),
            table,
            ScrollbarWidth::Auto,
            ColorU::new(60, 60, 60, 255).into(),
            ColorU::new(80, 80, 80, 255).into(),
            ColorU::new(100, 100, 100, 255).into(),
        );

        let selectable = SelectableArea::new(
            self.selection_handle_varying_heights.clone(),
            |selection_args, _, _| {
                if let Some(text) = &selection_args.selection {
                    println!("Varying heights selection: {:?}", text);
                }
            },
            scrollable.finish(),
        );

        Flex::column()
            .with_spacing(20.0)
            .with_child(self.render_demo_header(
                "Virtualized with Varying Heights",
                "500 rows with different content lengths. Rows have intrinsic heights based on text wrapping. Try scrolling and selecting text across rows.",
            ))
            .with_child(
                ConstrainedBox::new(
                    Container::new(selectable.finish())
                        .with_uniform_padding(10.0)
                        .finish(),
                )
                .with_height(500.0)
                .finish(),
            )
            .finish()
    }

    fn render_demo_fixed_header_mixed_columns(&self) -> Box<dyn Element> {
        let row_count = 500;
        let font_family = self.font_family;

        let config = TableConfig {
            fixed_header: true,
            row_background: RowBackground::striped(
                ColorU::white(),
                ColorU::new(248, 250, 252, 255),
            ),
            ..TableConfig::default()
        };

        let table = Table::new(self.table_states[TOTAL_DEMOS - 1].clone(), 800.0, 500.0)
            .with_headers(vec![
                self.create_header_with_width("#", TableColumnWidth::Fixed(50.0)),
                self.create_header_with_width("ID", TableColumnWidth::Intrinsic),
                self.create_header_with_width("Description", TableColumnWidth::Flex(1.0)),
                self.create_header_with_width("Status", TableColumnWidth::Intrinsic),
                self.create_header_with_width("Price", TableColumnWidth::Fraction(0.12)),
            ])
            .with_row_count(row_count)
            .with_row_render_fn(move |row_idx, _app| {
                let description = match row_idx % 5 {
                    0 => format!("Row {} - Short", row_idx),
                    1 => format!("Row {} - This is a medium length description that will wrap to multiple lines in a constrained column", row_idx),
                    2 => format!("Row {} - Very long description here. Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam.", row_idx),
                    3 => format!("Row {} - Another short one", row_idx),
                    4 => format!("Row {} - Medium text here with some more content to make it wrap and create varying row heights throughout the virtualized table.", row_idx),
                    _ => unreachable!(),
                };
                let (status, status_color) = match row_idx % 4 {
                    0 => ("Active", ColorU::new(34, 139, 34, 255)),
                    1 => ("Pending", ColorU::new(255, 165, 0, 255)),
                    2 => ("Complete", ColorU::new(30, 144, 255, 255)),
                    3 => ("Inactive", ColorU::new(128, 128, 128, 255)),
                    _ => unreachable!(),
                };
                vec![
                    Text::new(format!("{}", row_idx), font_family, 14.0).with_color(ColorU::new(50, 50, 50, 255)).finish(),
                    Text::new(format!("Item-{:04}", row_idx), font_family, 14.0).with_color(ColorU::new(50, 50, 50, 255)).finish(),
                    Text::new(description, font_family, 14.0).with_color(ColorU::new(50, 50, 50, 255)).finish(),
                    Text::new(status.to_string(), font_family, 14.0).with_color(status_color).finish(),
                    Text::new(format!("${}.{:02}", row_idx * 10, (row_idx * 17) % 100), font_family, 14.0).with_color(ColorU::new(50, 50, 50, 255)).finish(),
                ]
            })
            .with_config(config)
            .finish_scrollable();

        let scrollable = Scrollable::vertical(
            self.scroll_state_fixed_header_mixed_columns.clone(),
            table,
            ScrollbarWidth::Auto,
            ColorU::new(60, 60, 60, 255).into(),
            ColorU::new(80, 80, 80, 255).into(),
            ColorU::new(100, 100, 100, 255).into(),
        );

        let selectable = SelectableArea::new(
            self.selection_handle_fixed_header_mixed_columns.clone(),
            |selection_args, _, _| {
                if let Some(text) = &selection_args.selection {
                    println!("Fixed header mixed columns selection: {:?}", text);
                }
            },
            scrollable.finish(),
        );

        Flex::column()
            .with_spacing(20.0)
            .with_child(self.render_demo_header(
                "Fixed Header with Mixed Column Types",
                "500 rows with fixed header. Columns: Fixed(50), Intrinsic, Flex(1), Intrinsic, Fraction(0.12). Try scrolling - header stays fixed.",
            ))
            .with_child(
                ConstrainedBox::new(
                    Container::new(selectable.finish())
                        .with_uniform_padding(10.0)
                        .finish(),
                )
                .with_height(500.0)
                .finish(),
            )
            .finish()
    }
}

impl Entity for TableSampleView {
    type Event = ();
}

impl View for TableSampleView {
    fn ui_name() -> &'static str {
        "TableSampleView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        let demo_content = match self.current_demo {
            0 => self.render_demo_column_widths(),
            1 => self.render_demo_virtualized(),
            2 => self.render_demo_virtualized_fixed_header(),
            3 => self.render_demo_selection(),
            4 => self.render_demo_selection_virtualized(),
            5 => self.render_demo_text_wrapping(),
            6 => self.render_demo_mixed_elements(),
            7 => self.render_demo_padding(),
            8 => self.render_demo_banding(),
            9 => self.render_demo_theming(),
            10 => self.render_demo_edge_cases(),
            11 => self.render_demo_virtualized_varying_heights(),
            12 => self.render_demo_fixed_header_mixed_columns(),
            _ => self.render_demo_column_widths(),
        };

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(warpui::elements::CrossAxisAlignment::Stretch)
                .with_child(
                    Container::new(demo_content)
                        .with_uniform_padding(20.0)
                        .finish(),
                )
                .with_child(Box::new(Empty::new()))
                .finish(),
        )
        .with_background(Fill::Solid(ColorU::new(40, 44, 52, 255)))
        .finish()
    }
}

impl TypedActionView for TableSampleView {
    type Action = SampleAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SampleAction::NextDemo => {
                self.current_demo = (self.current_demo + 1) % TOTAL_DEMOS;

                // Trigger capture and auto-advance if in capture mode
                if self.capture_config.capture_screenshots {
                    ctx.spawn(
                        async {
                            // Wait for render to complete
                            warpui::r#async::Timer::after(std::time::Duration::from_millis(300))
                                .await;
                        },
                        |view, _, ctx| {
                            view.capture_current_demo(ctx);
                        },
                    );
                }
            }
            SampleAction::PreviousDemo => {
                self.current_demo = if self.current_demo == 0 {
                    TOTAL_DEMOS - 1
                } else {
                    self.current_demo - 1
                };
            }
        }
        ctx.notify();
    }
}

fn save_frame_as_png(frame: &CapturedFrame, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut writer = std::io::BufWriter::new(file);

    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        &mut writer,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::NoFilter,
    );

    encoder.write_image(
        &frame.data,
        frame.width,
        frame.height,
        image::ColorType::Rgba8.into(),
    )?;

    Ok(())
}
