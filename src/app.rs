use eframe::egui::{
    self, Align, Align2, Color32, FontId, Frame, Layout, Margin, RichText, Stroke, Vec2,
};
use egui_extras::{Column, TableBuilder};
use find_dev_data::{
    export, format_bytes,
    rules::{self, Category},
    scanner::{self, Event, Finding, Progress, ScanError, ScanOptions},
};
use serde::{Deserialize, Serialize};
use std::{
    env,
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::Duration,
};

const INK: Color32 = Color32::from_rgb(29, 39, 49);
const MUTED: Color32 = Color32::from_rgb(116, 126, 135);
const BG: Color32 = Color32::from_rgb(246, 248, 249);
const LINE: Color32 = Color32::from_rgb(227, 232, 235);
const NAVY: Color32 = Color32::from_rgb(24, 35, 45);
const MINT: Color32 = Color32::from_rgb(106, 226, 183);
const ACCENT: Color32 = Color32::from_rgb(16, 127, 100);

fn category_color(category: Category) -> Color32 {
    match category {
        Category::Dependencies => Color32::from_rgb(72, 120, 209),
        Category::Build => Color32::from_rgb(185, 126, 54),
        Category::Cache => Color32::from_rgb(143, 102, 193),
        Category::Models => ACCENT,
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    root: String,
    include_volumes: bool,
    allocated: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            root: "/".into(),
            include_volumes: false,
            allocated: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Sort {
    Size,
    Name,
    Tool,
}

pub struct App {
    prefs: Preferences,
    home: Option<PathBuf>,
    findings: Vec<Finding>,
    receiver: Option<Receiver<Event>>,
    cancel: Arc<AtomicBool>,
    progress: Progress,
    warnings: Vec<ScanError>,
    category: Option<Category>,
    query: String,
    minimum: u64,
    sort: Sort,
    descending: bool,
    selected: Option<PathBuf>,
    has_scanned: bool,
    was_cancelled: bool,
    stopping: bool,
    scan_failed: bool,
    show_warnings: bool,
    show_help: bool,
    notice: Option<String>,
    scanned_root: Option<PathBuf>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        cc.egui_ctx.global_style_mut(|style| {
            style.visuals.override_text_color = Some(INK);
            style.visuals.panel_fill = BG;
            style.visuals.window_fill = Color32::WHITE;
            style.visuals.selection.bg_fill = Color32::from_rgb(218, 241, 233);
            style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
            style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(239, 243, 245);
            style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(239, 243, 245);
            style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
            style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(225, 236, 234);
            style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(225, 236, 234);
            style.spacing.item_spacing = Vec2::new(10.0, 10.0);
            style.spacing.button_padding = Vec2::new(12.0, 8.0);
            style.spacing.interact_size.y = 32.0;
            style
                .text_styles
                .insert(egui::TextStyle::Body, FontId::proportional(14.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, FontId::proportional(13.0));
            style
                .text_styles
                .insert(egui::TextStyle::Small, FontId::proportional(11.0));
        });
        let prefs = cc
            .storage
            .and_then(|s| eframe::get_value(s, "preferences"))
            .unwrap_or_default();
        Self {
            prefs,
            home: env::var_os("HOME").map(PathBuf::from),
            findings: Vec::new(),
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Progress::default(),
            warnings: Vec::new(),
            category: None,
            query: String::new(),
            minimum: 0,
            sort: Sort::Size,
            descending: true,
            selected: None,
            has_scanned: false,
            was_cancelled: false,
            stopping: false,
            scan_failed: false,
            show_warnings: false,
            show_help: false,
            notice: None,
            scanned_root: None,
        }
    }

    fn running(&self) -> bool {
        self.receiver.is_some()
    }
    fn bytes(&self, item: &Finding) -> u64 {
        if self.prefs.allocated {
            item.allocated_bytes
        } else {
            item.logical_bytes
        }
    }

    fn start(&mut self, ctx: &egui::Context) {
        if self.running() {
            return;
        }
        let root = self.home.as_ref().map_or_else(
            || PathBuf::from(&self.prefs.root),
            |home| rules::expand_home(Path::new(&self.prefs.root), home),
        );
        if !root.is_dir() {
            self.notice = Some("Choose an existing folder to scan.".into());
            return;
        }
        self.findings.clear();
        self.selected = None;
        self.warnings.clear();
        self.progress = Progress::default();
        self.cancel = Arc::new(AtomicBool::new(false));
        self.was_cancelled = false;
        self.stopping = false;
        self.scan_failed = false;
        self.has_scanned = true;
        self.notice = None;
        self.scanned_root = Some(root.clone());
        let options = ScanOptions {
            root,
            home: self.home.clone(),
            include_volumes: self.prefs.include_volumes,
        };
        let (sender, receiver) = mpsc::channel();
        let cancel = self.cancel.clone();
        let ctx = ctx.clone();
        let spawn = std::thread::Builder::new()
            .name("disk-scanner".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    scanner::scan(options, cancel, |event| {
                        let _ = sender.send(event);
                        ctx.request_repaint();
                    })
                }));
                let message = match result {
                    Ok(Ok(())) => return,
                    Ok(Err(error)) => error.to_string(),
                    Err(_) => {
                        "The scanner stopped unexpectedly. You can start another scan.".into()
                    }
                };
                let _ = sender.send(Event::Failed(message));
                ctx.request_repaint();
            });
        match spawn {
            Ok(_) => self.receiver = Some(receiver),
            Err(error) => {
                self.scan_failed = true;
                self.notice = Some(format!("Could not start scanner: {error}"));
            }
        }
    }

    fn receive(&mut self) {
        let mut finished = false;
        if let Some(receiver) = &self.receiver {
            // A bounded per-frame drain keeps very fast discovery from delaying interaction.
            for _ in 0..500 {
                match receiver.try_recv() {
                    Ok(Event::Found(finding)) => {
                        scanner::merge_finding(&mut self.findings, finding)
                    }
                    Ok(Event::Progress(progress)) => self.progress = progress,
                    Ok(Event::Warning(error)) => self.warnings.push(error),
                    Ok(Event::Finished {
                        cancelled,
                        progress,
                    }) => {
                        self.progress = progress;
                        self.was_cancelled = cancelled;
                        finished = true;
                        break;
                    }
                    Ok(Event::Failed(message)) => {
                        self.scan_failed = true;
                        self.notice = Some(message);
                        finished = true;
                        break;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
        }
        if finished {
            self.receiver = None;
            self.stopping = false;
        }
    }

    fn visible(&self) -> Vec<usize> {
        let query = self.query.to_lowercase();
        let mut indices: Vec<_> = self
            .findings
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                self.category.is_none_or(|cat| item.category == cat)
                    && self.bytes(item) >= self.minimum
                    && (query.is_empty()
                        || item.path.to_string_lossy().to_lowercase().contains(&query)
                        || item.tool.to_lowercase().contains(&query)
                        || item.category.label().to_lowercase().contains(&query))
            })
            .map(|(index, _)| index)
            .collect();
        indices.sort_by(|&a, &b| {
            let (a, b) = (&self.findings[a], &self.findings[b]);
            let order = match self.sort {
                Sort::Size => self.bytes(a).cmp(&self.bytes(b)),
                Sort::Name => a.path.cmp(&b.path),
                Sort::Tool => a.tool.cmp(&b.tool),
            }
            .then_with(|| a.path.cmp(&b.path));
            if self.descending {
                order.reverse()
            } else {
                order
            }
        });
        indices
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("navigation")
            .exact_size(218.0)
            .resizable(false)
            .frame(Frame::new().fill(NAVY).inner_margin(Margin::same(20)))
            .show_inside(ui, |ui| {
                ui.visuals_mut().override_text_color = Some(Color32::from_rgb(216, 225, 231));
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    Frame::new()
                        .fill(MINT)
                        .corner_radius(10)
                        .inner_margin(Margin::symmetric(10, 8))
                        .show(ui, |ui| {
                            ui.label(RichText::new("fd").size(22.0).strong().color(NAVY));
                        });
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Find Dev Data")
                                .strong()
                                .size(16.0)
                                .color(Color32::WHITE),
                        );
                        ui.label(
                            RichText::new("A little disk clarity.")
                                .size(11.0)
                                .color(Color32::from_rgb(139, 158, 171)),
                        );
                    });
                });
                ui.add_space(40.0);
                ui.label(
                    RichText::new("LIBRARY")
                        .size(10.0)
                        .strong()
                        .color(Color32::from_rgb(124, 146, 160)),
                );
                ui.add_space(8.0);
                self.nav_item(ui, None, "All findings");
                for cat in Category::ALL {
                    self.nav_item(ui, Some(cat), cat.label());
                }
                ui.add_space(28.0);
                ui.separator();
                ui.add_space(15.0);
                ui.label(
                    RichText::new("SIZE DISPLAY")
                        .size(10.0)
                        .strong()
                        .color(Color32::from_rgb(124, 146, 160)),
                );
                ui.add_space(3.0);
                ui.scope(|ui| {
                    ui.visuals_mut().selection.bg_fill = ACCENT;
                    ui.radio_value(&mut self.prefs.allocated, false, "File size");
                    ui.radio_value(&mut self.prefs.allocated, true, "Space on disk");
                });
                ui.add_space(5.0);
                ui.label(
                    RichText::new("Allocated space may differ for sparse or compressed files.")
                        .size(11.0)
                        .color(Color32::from_rgb(139, 158, 171)),
                );
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("About & scan access").color(MINT))
                                .frame(false),
                        )
                        .clicked()
                    {
                        self.show_help = true;
                    }
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Read-only by design")
                            .size(12.0)
                            .color(Color32::from_rgb(205, 218, 226)),
                    );
                    ui.label(
                        RichText::new("Your files stay yours.")
                            .size(11.0)
                            .color(Color32::from_rgb(139, 158, 171)),
                    );
                });
            });
    }

    fn nav_item(&mut self, ui: &mut egui::Ui, category: Option<Category>, title: &str) {
        let count = self
            .findings
            .iter()
            .filter(|f| category.is_none_or(|c| c == f.category))
            .count();
        let selected = self.category == category;
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                true,
                selected,
                format!("{title}, {count} folders"),
            )
        });
        if selected || response.hovered() {
            ui.painter().rect_filled(
                rect,
                7.0,
                if selected {
                    Color32::from_rgb(43, 64, 71)
                } else {
                    Color32::from_rgb(34, 47, 58)
                },
            );
        }
        let color = category.map(category_color).unwrap_or(MINT);
        ui.painter()
            .circle_filled(rect.left_center() + Vec2::new(13.0, 0.0), 3.5, color);
        ui.painter().text(
            rect.left_center() + Vec2::new(27.0, 0.0),
            Align2::LEFT_CENTER,
            title,
            FontId::proportional(13.0),
            if selected {
                MINT
            } else {
                Color32::from_rgb(201, 212, 220)
            },
        );
        ui.painter().text(
            rect.right_center() - Vec2::new(10.0, 0.0),
            Align2::RIGHT_CENTER,
            count.to_string(),
            FontId::proportional(11.0),
            Color32::from_rgb(139, 158, 171),
        );
        if response.clicked() {
            self.category = category;
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("STORAGE EXPLORER")
                        .size(10.0)
                        .strong()
                        .color(ACCENT),
                );
                ui.label(
                    RichText::new("Room for your next idea.")
                        .size(29.0)
                        .strong(),
                );
                ui.label(
                    RichText::new("Find the dependencies, builds and models taking up your disk.")
                        .color(MUTED)
                        .size(13.0),
                );
            });
        });
        ui.add_space(18.0);
        Frame::new().fill(Color32::WHITE).stroke(Stroke::new(1.0, LINE)).corner_radius(10)
            .inner_margin(Margin::same(14)).show(ui, |ui| {
                let running = self.running();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Scan location").strong().size(12.0));
                    ui.add_enabled_ui(!running, |ui| {
                        egui::ComboBox::from_id_salt("scan_scope").selected_text(if self.prefs.root == "/" { "Entire disk" } else if self.home.as_ref().is_some_and(|p| p == Path::new(&self.prefs.root)) { "Home folder" } else { "Custom folder" })
                            .width(135.0).show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.prefs.root, "/".into(), "Entire disk");
                                if let Some(home) = &self.home { ui.selectable_value(&mut self.prefs.root, home.to_string_lossy().into_owned(), "Home folder"); }
                                if ui.selectable_label(false, "Choose folder…").clicked()
                                    && let Some(path) = rfd::FileDialog::new().set_title("Choose a folder to scan").pick_folder()
                                { self.prefs.root = path.to_string_lossy().into_owned(); }
                            });
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if running {
                            if ui.add_enabled(!self.stopping, egui::Button::new(if self.stopping { "Stopping…" } else { "Stop scan" })).clicked() {
                                self.cancel.store(true, Ordering::Relaxed); self.stopping = true;
                            }
                        } else if ui.add(egui::Button::new(RichText::new(if self.has_scanned { "Scan again" } else { "Start scan" }).color(Color32::WHITE).strong()).fill(ACCENT).min_size(Vec2::new(116.0, 35.0))).clicked() {
                            self.start(ui.ctx());
                        }
                        ui.add_enabled_ui(!running, |ui| { ui.checkbox(&mut self.prefs.include_volumes, "External volumes").on_hover_text("Include mounted disks under /Volumes. Network disks may take longer."); });
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&self.prefs.root).monospace().size(11.0).color(MUTED));
                    if self.prefs.root == "/" { ui.label(RichText::new("•  Full Disk Access recommended").size(11.0).color(MUTED)); }
                });
            });
    }

    fn stats(&self, ui: &mut egui::Ui) {
        let total: u64 = self.findings.iter().map(|f| self.bytes(f)).sum();
        let builds: u64 = self
            .findings
            .iter()
            .filter(|f| f.category == Category::Build)
            .map(|f| self.bytes(f))
            .sum();
        let models: u64 = self
            .findings
            .iter()
            .filter(|f| f.category == Category::Models)
            .map(|f| self.bytes(f))
            .sum();
        ui.columns(4, |cols| {
            for (i, (label, value, caption, color)) in [
                (
                    "TOTAL FOUND",
                    format_bytes(total),
                    "across all categories",
                    ACCENT,
                ),
                (
                    "BUILD ARTIFACTS",
                    format_bytes(builds),
                    "compiled along the way",
                    category_color(Category::Build),
                ),
                (
                    "LOCAL AI",
                    format_bytes(models),
                    "models & model caches",
                    ACCENT,
                ),
                (
                    "FOLDERS",
                    self.findings.len().to_string(),
                    "individual locations",
                    category_color(Category::Dependencies),
                ),
            ]
            .into_iter()
            .enumerate()
            {
                Frame::new()
                    .fill(Color32::WHITE)
                    .stroke(Stroke::new(1.0, LINE))
                    .corner_radius(9)
                    .inner_margin(Margin::same(15))
                    .show(&mut cols[i], |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(RichText::new(label).size(10.0).strong().color(MUTED));
                        ui.label(RichText::new(value).size(25.0).strong().color(color));
                        ui.label(RichText::new(caption).size(11.0).color(MUTED));
                    });
            }
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, visible: &[usize]) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(self.category.map(Category::label).unwrap_or("All findings"))
                    .size(18.0)
                    .strong(),
            );
            ui.label(
                RichText::new(format!("{} folders", visible.len()))
                    .size(12.0)
                    .color(MUTED),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(!visible.is_empty(), egui::Button::new("Export CSV"))
                    .clicked()
                {
                    self.export(visible);
                }
                egui::ComboBox::from_id_salt("minimum_size")
                    .selected_text(match self.minimum {
                        0 => "All sizes",
                        104_857_600 => "100 MiB +",
                        _ => "1 GiB +",
                    })
                    .width(92.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.minimum, 0, "All sizes");
                        ui.selectable_value(&mut self.minimum, 104_857_600, "100 MiB +");
                        ui.selectable_value(&mut self.minimum, 1_073_741_824, "1 GiB +");
                    });
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Search paths or tools…")
                        .desired_width(180.0),
                );
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
                    search.request_focus();
                }
            });
        });
    }

    fn sort_header(&mut self, ui: &mut egui::Ui, sort: Sort, label: &str) {
        let suffix = if self.sort == sort {
            if self.descending { " [desc]" } else { " [asc]" }
        } else {
            ""
        };
        if ui
            .add(
                egui::Button::new(
                    RichText::new(format!("{label}{suffix}"))
                        .size(11.0)
                        .strong()
                        .color(MUTED),
                )
                .frame(false),
            )
            .clicked()
        {
            if self.sort == sort {
                self.descending = !self.descending;
            } else {
                self.sort = sort;
                self.descending = sort == Sort::Size;
            }
        }
    }

    fn table(&mut self, ui: &mut egui::Ui, visible: &[usize]) {
        let ctx = ui.ctx().clone();
        let total: u64 = visible.iter().map(|&i| self.bytes(&self.findings[i])).sum();
        let height = ui.available_height();
        TableBuilder::new(ui)
            .id_salt("findings")
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::remainder().at_least(220.0))
            .column(Column::initial(155.0).range(115.0..=220.0))
            .column(Column::initial(108.0).range(90.0..=150.0))
            .column(Column::exact(72.0))
            .min_scrolled_height(120.0)
            .max_scroll_height(height - 34.0)
            .header(33.0, |mut row| {
                row.col(|ui| self.sort_header(ui, Sort::Name, "FOLDER / PATH"));
                row.col(|ui| self.sort_header(ui, Sort::Tool, "TOOL"));
                row.col(|ui| self.sort_header(ui, Sort::Size, "SIZE"));
                row.col(|ui| {
                    ui.label(RichText::new("ACTION").size(11.0).color(MUTED));
                });
            })
            .body(|body| {
                body.rows(64.0, visible.len(), |mut row| {
                    let item = &self.findings[visible[row.index()]];
                    let path = item.path.clone();
                    row.set_selected(self.selected.as_ref() == Some(&path));
                    row.col(|ui| {
                        ui.vertical(|ui| {
                            ui.add_space(9.0);
                            let name = path
                                .file_name()
                                .unwrap_or(path.as_os_str())
                                .to_string_lossy();
                            ui.horizontal(|ui| {
                                let (dot, _) =
                                    ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                                ui.painter().circle_filled(
                                    dot.center(),
                                    3.0,
                                    category_color(item.category),
                                );
                                ui.add(
                                    egui::Label::new(RichText::new(name).strong().size(13.0))
                                        .truncate(),
                                );
                                if !item.complete {
                                    ui.label(
                                        RichText::new("partial")
                                            .color(Color32::from_rgb(180, 110, 25))
                                            .size(10.0),
                                    );
                                }
                            });
                            ui.add(
                                egui::Label::new(
                                    RichText::new(short_path(&path, self.home.as_deref()))
                                        .size(11.0)
                                        .color(MUTED),
                                )
                                .truncate(),
                            )
                            .on_hover_text(path.to_string_lossy());
                        });
                    });
                    row.col(|ui| {
                        ui.vertical(|ui| {
                            ui.add_space(9.0);
                            ui.label(
                                RichText::new(&item.tool)
                                    .size(12.0)
                                    .color(category_color(item.category)),
                            );
                            ui.label(RichText::new(item.category.label()).size(10.0).color(MUTED));
                        });
                    });
                    row.col(|ui| {
                        ui.vertical(|ui| {
                            ui.add_space(9.0);
                            ui.label(
                                RichText::new(format_bytes(self.bytes(item)))
                                    .strong()
                                    .size(14.0),
                            );
                            let share = if total > 0 {
                                self.bytes(item) as f32 / total as f32
                            } else {
                                0.0
                            };
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::new(76.0, 3.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 2.0, LINE);
                            let fill = egui::Rect::from_min_size(
                                rect.min,
                                Vec2::new(rect.width() * share, 3.0),
                            );
                            ui.painter()
                                .rect_filled(fill, 2.0, category_color(item.category));
                        });
                    });
                    let mut reveal = false;
                    row.col(|ui| {
                        reveal = ui
                            .small_button("Reveal")
                            .on_hover_text("Show in Finder")
                            .clicked();
                    });
                    let response = row.response();
                    if response.clicked() {
                        self.selected = Some(path.clone());
                    }
                    let mut copy = false;
                    response.context_menu(|ui| {
                        if ui.button("Show in Finder").clicked() {
                            reveal = true;
                            ui.close();
                        }
                        if ui.button("Copy full path").clicked() {
                            copy = true;
                            ui.close();
                        }
                    });
                    if copy {
                        ctx.copy_text(path.to_string_lossy().into_owned());
                    }
                    if reveal {
                        self.reveal(&path);
                    }
                });
            });
    }

    fn empty(&self, ui: &mut egui::Ui) {
        ui.add_space(35.0);
        ui.vertical_centered(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(62.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(rect.center(), 30.0, Color32::from_rgb(224, 241, 234));
            let lens = rect.center() - Vec2::splat(3.0);
            ui.painter()
                .circle_stroke(lens, 12.0, Stroke::new(2.5, ACCENT));
            ui.painter().line_segment(
                [lens + Vec2::splat(9.0), lens + Vec2::splat(18.0)],
                Stroke::new(3.0, ACCENT),
            );
            ui.add_space(14.0);
            ui.label(
                RichText::new(if self.running() {
                    "Looking for the usual suspects…"
                } else if self.has_scanned {
                    "No matching folders"
                } else {
                    "Big projects leave a footprint."
                })
                .size(20.0)
                .strong(),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(if self.running() {
                    "Results appear here as each folder is measured."
                } else if self.has_scanned {
                    "Try another location or clear your filters."
                } else {
                    "Start a scan to see where your development data lives."
                })
                .color(MUTED),
            );
            if !self.has_scanned {
                ui.add_space(22.0);
                ui.label(
                    RichText::new("Node.js  ·  Rust  ·  Swift  ·  Xcode  ·  Python")
                        .size(12.0)
                        .color(MUTED),
                );
                ui.label(
                    RichText::new("LM Studio  ·  Hugging Face  ·  Ollama  ·  oMLX  ·  MLX")
                        .size(12.0)
                        .color(MUTED),
                );
            }
        });
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .frame(
                Frame::new()
                    .fill(Color32::WHITE)
                    .stroke(Stroke::new(1.0, LINE))
                    .inner_margin(Margin::symmetric(24, 12)),
            )
            .show_inside(ui, |ui| {
                if let Some(path) = &self.selected
                    && let Some(item) = self.findings.iter().find(|f| &f.path == path)
                {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&item.tool).strong().size(12.0));
                        ui.label(
                            RichText::new(format!(
                                "{} files  ·  {} file size  ·  {} on disk",
                                item.files,
                                format_bytes(item.logical_bytes),
                                format_bytes(item.allocated_bytes)
                            ))
                            .size(11.0)
                            .color(MUTED),
                        );
                        if ui.small_button("Copy path").clicked() {
                            ui.ctx().copy_text(path.to_string_lossy().into_owned());
                        }
                    });
                    ui.label(
                        RichText::new(format!(
                            "{} · {} symlinks skipped · {} read errors",
                            item.detail, item.symlinks_skipped, item.errors
                        ))
                        .size(11.0)
                        .color(MUTED),
                    );
                    ui.separator();
                }
                ui.horizontal(|ui| {
                    if self.running() {
                        ui.spinner();
                    } else {
                        let (dot, _) =
                            ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::hover());
                        ui.painter().circle_filled(dot.center(), 3.5, ACCENT);
                    }
                    let status = if self.running() {
                        if self.progress.measuring {
                            "Measuring folder"
                        } else {
                            "Discovering folders"
                        }
                    } else if !self.has_scanned {
                        "Ready when you are"
                    } else if self.was_cancelled {
                        "Scan stopped · results are partial"
                    } else if self.scan_failed {
                        "Scan needs attention"
                    } else {
                        "Scan complete"
                    };
                    ui.label(RichText::new(status).size(12.0).strong());
                    if self.has_scanned {
                        ui.label(
                            RichText::new(format!(
                                "{} directories  ·  {} files measured  ·  {}s",
                                self.progress.directories,
                                self.progress.files_measured,
                                self.progress.elapsed.as_secs()
                            ))
                            .size(11.0)
                            .color(MUTED),
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self.progress.errors > 0
                            && ui
                                .small_button(format!("{} access issues", self.progress.errors))
                                .clicked()
                        {
                            self.show_warnings = true;
                        }
                    });
                });
                if self.running() {
                    ui.add(
                        egui::Label::new(
                            RichText::new(self.progress.current.to_string_lossy())
                                .monospace()
                                .size(10.0)
                                .color(MUTED),
                        )
                        .truncate(),
                    );
                } else if let Some(root) = &self.scanned_root {
                    ui.label(
                        RichText::new(format!("Scanned {}", root.display()))
                            .size(10.0)
                            .color(MUTED),
                    );
                }
            });
    }

    fn export(&mut self, visible: &[usize]) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Export visible findings")
            .set_file_name("dev-data.csv")
            .add_filter("CSV", &["csv"])
            .save_file()
        {
            let findings: Vec<_> = visible.iter().map(|&i| &self.findings[i]).collect();
            match File::create(&path)
                .and_then(|file| export::write_csv(BufWriter::new(file), &findings))
            {
                Ok(()) => {
                    self.notice = Some(format!(
                        "Exported {} folders to {}",
                        visible.len(),
                        path.display()
                    ))
                }
                Err(error) => self.notice = Some(format!("Could not export: {error}")),
            }
        }
    }

    fn reveal(&mut self, path: &Path) {
        #[cfg(target_os = "macos")]
        let result = Command::new("/usr/bin/open").arg("-R").arg(path).spawn();
        #[cfg(not(target_os = "macos"))]
        let result = Command::new("xdg-open").arg(path).spawn();
        if let Err(error) = result {
            self.notice = Some(format!("Could not reveal folder: {error}"));
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        egui::Window::new("About & scan access").open(&mut self.show_help).resizable(false).default_width(480.0).show(ctx, |ui| {
            ui.heading("Find Dev Data");
            ui.label("A local, read-only view of developer artifacts and downloaded AI models. Nothing is uploaded or deleted.");
            ui.separator();
            ui.label(RichText::new("Scanning your whole Mac").strong());
            ui.label("Choose Entire disk to search /. Hidden folders are included. External volumes are optional. macOS may deny access to protected folders.");
            ui.label("For a more complete scan, add Find Dev Data in System Settings → Privacy & Security → Full Disk Access, then restart the app. When launched with cargo run, grant access to your terminal.");
            if ui.button("Open Full Disk Access settings").clicked() {
                let _ = Command::new("/usr/bin/open").arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles").spawn();
            }
            ui.separator();
            ui.label(RichText::new("What the numbers mean").strong());
            ui.label("File size is the sum of regular file lengths. Space on disk uses filesystem allocated blocks. Hard links count once within each row. APFS clones, snapshots, and hard links shared across rows mean totals are estimates, not guaranteed reclaimable space.");
            ui.label("Nested matches are rolled into their outer folder. Symlinks are not followed; configured store roots are resolved once. Shared stores such as Ollama and Hugging Face appear once. MLX servers commonly reuse the Hugging Face cache.");
            ui.label("Generic model folders are identified by GGUF, safetensors, ONNX, or PyTorch weight filenames. The entire containing folder is measured. Custom model stores can be found by choosing their folder. Environment overrides apply when inherited at launch; Finder does not inherit your shell configuration.");
        });
        egui::Window::new("Scan access issues").open(&mut self.show_warnings).default_size([650.0, 390.0]).show(ctx, |ui| {
            ui.label("Some folders or files could not be read. Their sizes may be partial. Full Disk Access can help with macOS-protected folders.");
            ui.label(RichText::new(format!("Showing the first {} of {} issues.", self.warnings.len(), self.progress.errors)).color(MUTED));
            egui::ScrollArea::vertical().show(ui, |ui| {
                for issue in &self.warnings {
                    ui.label(RichText::new(issue.path.to_string_lossy()).monospace().size(11.0));
                    ui.label(RichText::new(&issue.message).size(11.0).color(MUTED));
                    ui.separator();
                }
            });
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.receive();
        if self.running() {
            ui.ctx().request_repaint_after(Duration::from_millis(120));
        }
        self.sidebar(ui);
        self.footer(ui);
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(BG)
                    .inner_margin(Margin::symmetric(26, 24)),
            )
            .show_inside(ui, |ui| {
                self.header(ui);
                ui.add_space(16.0);
                self.stats(ui);
                ui.add_space(20.0);
                let visible = self.visible();
                self.toolbar(ui, &visible);
                ui.add_space(8.0);
                if let Some(notice) = self.notice.clone() {
                    Frame::new()
                        .fill(Color32::from_rgb(230, 239, 242))
                        .corner_radius(6)
                        .inner_margin(10)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(notice).size(12.0));
                                if ui.small_button("Dismiss").clicked() {
                                    self.notice = None;
                                }
                            });
                        });
                }
                if visible.is_empty() {
                    self.empty(ui);
                } else {
                    self.table(ui, &visible);
                }
            });
        self.dialogs(ui.ctx());
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "preferences", &self.prefs);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn short_path(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home
        && let Ok(rest) = path.strip_prefix(home)
    {
        return format!("~/{}", rest.display());
    }
    path.to_string_lossy().into_owned()
}
