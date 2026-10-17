//! The window: a searchable program list, one Remove button, an activity log.
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use eframe::egui::{self, Align, Color32, Key, Layout, RichText, Sense};
use egui_extras::{Column, TableBuilder};

use app_remover::disk::{folder_size_kb, program_drive};
use app_remover::engine::{backup_base, default_choice, remove_program, Options};
use app_remover::model::{Confidence, Leftover, Program, RegKey};
use app_remover::registry::list_programs;

use crate::Backend;

enum Msg {
    Log(String),
    Review(Vec<Leftover>, Sender<Vec<usize>>),
    Done,
    /// Folder size measured in the background for a program the registry has no size for.
    Size(RegKey, u32),
}

/// Sends `Done` even if the worker panics, so the window never stays "busy".
struct DoneGuard(Sender<Msg>, egui::Context);

impl Drop for DoneGuard {
    fn drop(&mut self) {
        let _ = self.0.send(Msg::Done);
        self.1.request_repaint();
    }
}

struct Review {
    items: Vec<Leftover>,
    checked: Vec<bool>,
    reply: Sender<Vec<usize>>,
}

#[derive(Clone, Copy, PartialEq)]
enum SortBy {
    Size,
    Name,
    Drive,
    Publisher,
    Version,
}

impl SortBy {
    const ALL: [SortBy; 5] = [SortBy::Size, SortBy::Name, SortBy::Drive, SortBy::Publisher, SortBy::Version];

    fn label(self) -> &'static str {
        match self {
            SortBy::Size => "Size",
            SortBy::Name => "Name",
            SortBy::Drive => "Drive",
            SortBy::Publisher => "Publisher",
            SortBy::Version => "Version",
        }
    }

    /// Biggest first is what you want for sizes; A-Z / C: first for the rest.
    fn default_descending(self) -> bool {
        self == SortBy::Size
    }
}

/// `None` = all drives, `Some(None)` = programs whose drive is unknown.
type DriveFilter = Option<Option<String>>;

pub struct App {
    b: Backend,
    ctx: egui::Context,
    programs: Vec<Program>,
    /// Drive of each program, parallel to `programs`.
    drives: Vec<Option<String>>,
    /// Sizes measured from the install folder when the registry has none.
    measured: HashMap<RegKey, u32>,
    measuring: HashSet<RegKey>,
    drive_filter: DriveFilter,
    filter: String,
    selected: Option<usize>,
    sort: SortBy,
    descending: bool,
    silent: bool,
    review: bool,
    force: bool,
    log: Vec<String>,
    busy: Option<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    confirm: Option<usize>,
    pending_review: Option<Review>,
    first_frame: bool,
}

fn size_text(kb: u32) -> String {
    size_text_u64(u64::from(kb))
}

fn size_text_u64(kb: u64) -> String {
    let mb = kb as f64 / 1024.0;
    match kb {
        0 => String::new(),
        _ if mb >= 1024.0 => format!("{:.1} GB", mb / 1024.0),
        _ if mb >= 1.0 => format!("{mb:.0} MB"),
        _ => format!("{kb} KB"),
    }
}

fn programs_text(n: usize) -> String {
    if n == 1 {
        "1 program".into()
    } else {
        format!("{n} programs")
    }
}

fn drive_label(d: &Option<String>) -> &str {
    d.as_deref().unwrap_or("unknown")
}

impl App {
    fn new(b: Backend, ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        let mut app = App {
            b,
            ctx,
            programs: Vec::new(),
            drives: Vec::new(),
            measured: HashMap::new(),
            measuring: HashSet::new(),
            drive_filter: None,
            filter: String::new(),
            selected: None,
            sort: SortBy::Size,
            descending: true,
            silent: true,
            review: false,
            force: false,
            log: Vec::new(),
            busy: None,
            tx,
            rx,
            confirm: None,
            pending_review: None,
            first_frame: true,
        };
        if app.b.demo {
            app.log.push("DEMO MODE: made-up programs, nothing on this PC will be changed.".into());
        }
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        let keep = self.selected.and_then(|i| self.programs.get(i)).map(|p| p.key.clone());
        self.programs = list_programs(self.b.reg.as_ref());
        self.drives = self.programs.iter().map(program_drive).collect();
        self.selected = keep.and_then(|k| self.programs.iter().position(|p| p.key == k));
        self.measure_missing_sizes();
    }

    /// Many programs don't record their size. Measure their install folder in the
    /// background (like Revo does) so sorting by size is meaningful.
    fn measure_missing_sizes(&mut self) {
        let jobs: Vec<(RegKey, String)> = self
            .programs
            .iter()
            .filter(|p| p.size_kb == 0 && !p.install_location.is_empty())
            .filter(|p| !self.measured.contains_key(&p.key) && !self.measuring.contains(&p.key))
            // never measure e.g. all of "C:\Program Files" because an entry points there
            .filter(|p| self.b.env.may_touch(&p.install_location))
            .filter(|p| Path::new(&p.install_location).is_dir())
            .map(|p| (p.key.clone(), p.install_location.clone()))
            .collect();
        if jobs.is_empty() {
            return;
        }
        self.measuring.extend(jobs.iter().map(|(k, _)| k.clone()));
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            for (key, loc) in jobs {
                let kb = u32::try_from(folder_size_kb(Path::new(&loc))).unwrap_or(u32::MAX);
                if tx.send(Msg::Size(key, kb)).is_err() {
                    return; // window closed
                }
                ctx.request_repaint();
            }
        });
    }

    /// Registry size, or the measured folder size; the flag says it was measured.
    fn size_of(&self, i: usize) -> (u32, bool) {
        let p = &self.programs[i];
        match (p.size_kb, self.measured.get(&p.key)) {
            (0, Some(&kb)) => (kb, true),
            (kb, _) => (kb, false),
        }
    }

    /// Indices into `programs` that match the search, in display order.
    fn visible(&self) -> Vec<usize> {
        let q = self.filter.trim().to_lowercase();
        let mut v: Vec<usize> = (0..self.programs.len())
            .filter(|&i| {
                let p = &self.programs[i];
                q.is_empty() || p.name.to_lowercase().contains(&q) || p.publisher.to_lowercase().contains(&q)
            })
            .filter(|&i| self.drive_filter.as_ref().is_none_or(|d| *d == self.drives[i]))
            .collect();
        let ps = &self.programs;
        let name = |i: usize| ps[i].name.to_lowercase();
        let primary = |a: usize, b: usize| -> Ordering {
            match self.sort {
                SortBy::Size => self.size_of(a).0.cmp(&self.size_of(b).0),
                SortBy::Name => name(a).cmp(&name(b)),
                SortBy::Publisher => ps[a].publisher.to_lowercase().cmp(&ps[b].publisher.to_lowercase()),
                SortBy::Version => ps[a].version.cmp(&ps[b].version),
                // unknown drive sorts after every lettered drive
                SortBy::Drive => {
                    (self.drives[a].is_none(), &self.drives[a]).cmp(&(self.drives[b].is_none(), &self.drives[b]))
                }
            }
        };
        v.sort_by(|&a, &b| {
            let o = primary(a, b);
            let o = if self.descending { o.reverse() } else { o };
            o.then_with(|| match self.sort {
                // within a drive: biggest first
                SortBy::Drive => self.size_of(b).0.cmp(&self.size_of(a).0),
                _ => Ordering::Equal,
            })
            .then_with(|| name(a).cmp(&name(b)))
        });
        v
    }

    fn drain_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Log(line) => self.log.push(line),
                Msg::Review(items, reply) => {
                    let checked = items.iter().map(|i| i.confidence == Confidence::High).collect();
                    self.pending_review = Some(Review { items, checked, reply });
                }
                Msg::Done => {
                    self.busy = None;
                    self.refresh();
                }
                Msg::Size(key, kb) => {
                    self.measuring.remove(&key);
                    self.measured.insert(key, kb);
                }
            }
        }
    }

    fn start_removal(&mut self, idx: usize, ctx: &egui::Context) {
        let program = self.programs[idx].clone();
        let opts = Options { silent: self.silent, force: self.force, dry_run: self.b.demo, ..Options::default() };
        let review = self.review;
        let (reg, sys, env) = (Arc::clone(&self.b.reg), Arc::clone(&self.b.sys), self.b.env.clone());
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        self.busy = Some(program.name.clone());
        self.log.push(format!("=== {} ===", program.name));

        std::thread::spawn(move || {
            let _done = DoneGuard(tx.clone(), ctx.clone());
            let log = |m: &str| {
                let _ = tx.send(Msg::Log(m.to_string()));
                ctx.request_repaint();
            };
            let choose = |items: &[Leftover]| -> Vec<usize> {
                if !review || items.is_empty() {
                    return default_choice(items, false);
                }
                let (reply_tx, reply_rx) = channel();
                let _ = tx.send(Msg::Review(items.to_vec(), reply_tx));
                ctx.request_repaint();
                reply_rx.recv().unwrap_or_default()
            };
            remove_program(&program, &env, reg.as_ref(), sys.as_ref(), &opts, &log, &choose, &backup_base());
        });
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, visible: &[usize]) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add_enabled(self.busy.is_none(), egui::Button::new("Refresh")).clicked() {
                    self.refresh();
                }
                let can_remove = self.busy.is_none() && self.selected.is_some();
                let remove = egui::Button::new(RichText::new("Remove").strong()).min_size(egui::vec2(90.0, 0.0));
                if ui.add_enabled(can_remove, remove).on_hover_text("Enter / double-click").clicked() {
                    self.confirm = self.selected;
                }
                let search = egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Search programs...")
                    .desired_width(ui.available_width());
                let resp = ui.add(search);
                // Typing always searches, even after clicking elsewhere (like Revo).
                let dialog_open = self.confirm.is_some() || self.pending_review.is_some();
                if !resp.has_focus() && !dialog_open {
                    let typed: String = ui.input(|i| {
                        i.events
                            .iter()
                            .filter_map(|e| match e {
                                egui::Event::Text(t) => Some(t.as_str()),
                                _ => None,
                            })
                            .collect()
                    });
                    // A leading space is ignored (it may just toggle a focused checkbox),
                    // spaces inside a search term are kept.
                    if !typed.trim().is_empty() || (!typed.is_empty() && !self.filter.is_empty()) {
                        self.filter.push_str(&typed);
                        resp.request_focus();
                        // Put the cursor after the text, so the next keystrokes append.
                        let mut state = egui::text_edit::TextEditState::load(ui.ctx(), resp.id).unwrap_or_default();
                        let end = egui::text::CCursor::new(self.filter.chars().count());
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                        state.store(ui.ctx(), resp.id);
                    }
                }
                if self.first_frame {
                    resp.request_focus();
                }
            })
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.silent, "Silent uninstall")
                .on_hover_text("Run the program's uninstaller without its wizard where possible");
            ui.checkbox(&mut self.review, "Review leftovers before deleting");
            ui.checkbox(&mut self.force, "Force remove (skip its uninstaller)");
            if self.b.demo {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new("DEMO").color(Color32::from_rgb(220, 120, 0)).strong());
                });
            }
        });
        ui.horizontal(|ui| self.sort_and_drive_bar(ui, visible));
        ui.add_space(4.0);
    }

    fn sort_and_drive_bar(&mut self, ui: &mut egui::Ui, visible: &[usize]) {
        ui.label("Sort by");
        egui::ComboBox::from_id_salt("sort").selected_text(self.sort.label()).width(100.0).show_ui(ui, |ui| {
            for key in SortBy::ALL {
                if ui.selectable_label(self.sort == key, key.label()).clicked() {
                    self.sort = key;
                    self.descending = key.default_descending();
                }
            }
        });
        let direction = match (self.sort, self.descending) {
            (SortBy::Size, true) => "largest first",
            (SortBy::Size, false) => "smallest first",
            (_, false) => "A-Z",
            (_, true) => "Z-A",
        };
        if ui.button(direction).on_hover_text("Reverse the order").clicked() {
            self.descending = !self.descending;
        }

        ui.add_space(16.0);
        ui.label("Drive");
        // Programs and total size per drive, over the whole list.
        let mut per_drive: Vec<(Option<String>, usize, u64)> = Vec::new();
        for i in 0..self.programs.len() {
            let kb = u64::from(self.size_of(i).0);
            match per_drive.iter_mut().find(|(d, ..)| *d == self.drives[i]) {
                Some(entry) => {
                    entry.1 += 1;
                    entry.2 += kb;
                }
                None => per_drive.push((self.drives[i].clone(), 1, kb)),
            }
        }
        per_drive.sort_by(|a, b| (a.0.is_none(), &a.0).cmp(&(b.0.is_none(), &b.0)));
        let current = match &self.drive_filter {
            None => "All drives".to_string(),
            Some(d) => drive_label(d).to_string(),
        };
        egui::ComboBox::from_id_salt("drive").selected_text(current).width(110.0).show_ui(ui, |ui| {
            if ui.selectable_label(self.drive_filter.is_none(), "All drives").clicked() {
                self.drive_filter = None;
            }
            for (drive, count, kb) in &per_drive {
                let text = format!("{}   {}, {}", drive_label(drive), programs_text(*count), size_text_u64(*kb));
                if ui.selectable_label(self.drive_filter.as_ref() == Some(drive), text).clicked() {
                    self.drive_filter = Some(drive.clone());
                }
            }
        });

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let total: u64 = visible.iter().map(|&i| u64::from(self.size_of(i).0)).sum();
            let count = if visible.len() == self.programs.len() {
                programs_text(visible.len())
            } else {
                format!("{} of {}", visible.len(), programs_text(self.programs.len()))
            };
            ui.label(if total == 0 { count } else { format!("{count}, {}", size_text_u64(total)) });
            if !self.measuring.is_empty() {
                ui.spinner().on_hover_text("Measuring the size of programs that don't report one");
            }
        });
    }

    fn log_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.strong("Activity");
            if let Some(name) = &self.busy {
                ui.spinner();
                ui.label(format!("removing {name}..."));
            }
        });
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink(false).show(ui, |ui| {
            for line in &self.log {
                let color = if line.contains("FAILED") || line.contains("WARNING") {
                    Some(Color32::from_rgb(220, 80, 60))
                } else {
                    None
                };
                let mut text = RichText::new(line).monospace();
                if let Some(c) = color {
                    text = text.color(c);
                }
                ui.label(text);
            }
        });
    }

    fn table(&mut self, ui: &mut egui::Ui, visible: &[usize], scroll_to: Option<usize>) {
        let mut clicked = None;
        let mut double = None;
        let mut sort_clicked = None;
        let headers = [
            (SortBy::Name, "Name"),
            (SortBy::Publisher, "Publisher"),
            (SortBy::Version, "Version"),
            (SortBy::Drive, "Drive"),
            (SortBy::Size, "Size"),
        ];

        let mut tb = TableBuilder::new(ui)
            .striped(true)
            .sense(Sense::click())
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::remainder().at_least(180.0).clip(true))
            .column(Column::initial(220.0).at_least(80.0).clip(true).resizable(true))
            .column(Column::initial(110.0).at_least(60.0).clip(true).resizable(true))
            .column(Column::initial(60.0).at_least(50.0))
            .column(Column::initial(80.0).at_least(60.0));
        if let Some(row) = scroll_to.and_then(|sel| visible.iter().position(|&i| i == sel)) {
            tb = tb.scroll_to_row(row, None);
        }
        tb.header(24.0, |mut header| {
            for (key, title) in headers {
                header.col(|ui| {
                    let arrow = match (self.sort == key, self.descending) {
                        (true, false) => " ^",
                        (true, true) => " v",
                        _ => "",
                    };
                    let label =
                        egui::Label::new(RichText::new(format!("{title}{arrow}")).strong()).sense(Sense::click());
                    if ui.add(label).clicked() {
                        sort_clicked = Some(key);
                    }
                });
            }
        })
        .body(|body| {
            body.rows(22.0, visible.len(), |mut row| {
                let idx = visible[row.index()];
                let p = &self.programs[idx];
                row.set_selected(self.selected == Some(idx));
                row.col(|ui| {
                    ui.label(&p.name);
                });
                row.col(|ui| {
                    ui.label(&p.publisher);
                });
                row.col(|ui| {
                    ui.label(&p.version);
                });
                row.col(|ui| {
                    ui.label(drive_label(&self.drives[idx]));
                });
                let (kb, measured) = self.size_of(idx);
                row.col(|ui| {
                    let label = ui.label(size_text(kb));
                    if measured {
                        label.on_hover_text("Measured from the install folder (the program doesn't report its size)");
                    }
                });
                let resp = row.response();
                if resp.clicked() {
                    clicked = Some(idx);
                }
                if resp.double_clicked() {
                    double = Some(idx);
                }
            });
        });

        if let Some(i) = clicked {
            self.selected = Some(i);
        }
        if let Some(i) = double {
            self.selected = Some(i);
            if self.busy.is_none() {
                self.confirm = Some(i);
            }
        }
        if let Some(key) = sort_clicked {
            if self.sort == key {
                self.descending = !self.descending;
            } else {
                self.sort = key;
                self.descending = key.default_descending();
            }
        }
    }

    /// `accept_enter` is false on the frame the dialog opens, so the Enter that
    /// opened it doesn't also confirm it.
    fn confirm_dialog(&mut self, ctx: &egui::Context, accept_enter: bool) {
        let Some(idx) = self.confirm else { return };
        let Some(p) = self.programs.get(idx) else {
            self.confirm = None;
            return;
        };
        let name = p.name.clone();
        let (mut go, mut cancel) = (false, false);
        let resp = egui::Modal::new(egui::Id::unique("confirm")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading(format!("Remove {name}?"));
            ui.add_space(6.0);
            if self.force {
                ui.label("Force mode: its uninstaller is skipped, the program is removed by hand.");
            } else {
                ui.label("Its uninstaller runs first (silently if possible). If it is missing or fails, the program is removed by hand.");
            }
            ui.label(if self.review {
                "You will see the leftovers before anything is deleted."
            } else {
                "Leftovers that clearly belong to it are deleted afterwards."
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Remove").strong()).clicked() {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
            if accept_enter && ui.input(|i| i.key_pressed(Key::Enter)) {
                go = true;
            }
        });
        if go {
            self.confirm = None;
            self.start_removal(idx, ctx);
        } else if cancel || resp.should_close() {
            self.confirm = None;
        }
    }

    fn review_dialog(&mut self, ctx: &egui::Context) {
        let Some(rv) = self.pending_review.as_mut() else { return };
        let (mut delete, mut skip) = (false, false);
        let resp = egui::Modal::new(egui::Id::unique("review")).show(ctx, |ui| {
            ui.set_width(760.0);
            ui.heading(format!("{} leftover(s) found", rv.items.len()));
            ui.label("Checked items will be deleted. Less certain matches are unchecked.");
            ui.horizontal(|ui| {
                if ui.small_button("All").clicked() {
                    rv.checked.iter_mut().for_each(|c| *c = true);
                }
                if ui.small_button("None").clicked() {
                    rv.checked.iter_mut().for_each(|c| *c = false);
                }
                if ui.small_button("Only certain").clicked() {
                    for (c, it) in rv.checked.iter_mut().zip(&rv.items) {
                        *c = it.confidence == Confidence::High;
                    }
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
                for (it, checked) in rv.items.iter().zip(rv.checked.iter_mut()) {
                    ui.horizontal(|ui| {
                        ui.checkbox(checked, "");
                        ui.add_sized([70.0, 18.0], egui::Label::new(RichText::new(it.kind.as_str()).weak()));
                        let mut reason = it.reason.clone();
                        if it.confidence == Confidence::Medium {
                            reason.push_str(" (less certain)");
                        }
                        ui.label(it.label()).on_hover_text(reason);
                    });
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Delete checked").strong()).clicked() {
                    delete = true;
                }
                if ui.button("Skip").clicked() {
                    skip = true;
                }
            });
        });
        if delete || skip || resp.should_close() {
            let rv = self.pending_review.take().expect("review is open");
            let picked = if delete { (0..rv.items.len()).filter(|&i| rv.checked[i]).collect() } else { Vec::new() };
            let _ = rv.reply.send(picked);
        }
    }

    /// Arrow keys move the selection, Enter / Delete asks to remove.
    fn keyboard(&mut self, ctx: &egui::Context, visible: &[usize]) -> Option<usize> {
        if self.confirm.is_some() || self.pending_review.is_some() {
            return None;
        }
        let (down, up, enter, del) = ctx.input(|i| {
            (
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Delete),
            )
        });
        if (enter || del) && self.busy.is_none() && self.selected.is_some_and(|s| visible.contains(&s)) {
            self.confirm = self.selected;
            return None;
        }
        if visible.is_empty() || !(down || up) {
            return None;
        }
        let pos = self.selected.and_then(|s| visible.iter().position(|&i| i == s));
        let next = match (pos, down) {
            (None, _) => 0,
            (Some(p), true) => (p + 1).min(visible.len() - 1),
            (Some(p), false) => p.saturating_sub(1),
        };
        self.selected = Some(visible[next]);
        self.selected
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.drain_messages();
        let visible = self.visible();
        let confirm_was_open = self.confirm.is_some();
        let scroll_to = self.keyboard(&ctx, &visible);

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui, &visible));
        egui::Panel::bottom("log").resizable(true).default_size(170.0).min_size(70.0).show(ui, |ui| self.log_panel(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            let visible = self.visible();
            if visible.is_empty() {
                ui.centered_and_justified(|ui| ui.label("No programs match the search."));
            } else {
                self.table(ui, &visible, scroll_to);
            }
        });

        self.confirm_dialog(&ctx, confirm_was_open);
        self.review_dialog(&ctx);
        self.first_frame = false;
    }
}

pub fn run(b: Backend) -> eframe::Result {
    let title = if b.demo { "App Remover (demo)" } else { "App Remover" };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([980.0, 660.0])
            .with_min_inner_size([640.0, 420.0])
            .with_icon(egui::IconData {
                rgba: include_bytes!("../assets/icon64.rgba").to_vec(),
                width: 64,
                height: 64,
            }),
        ..Default::default()
    };
    eframe::run_native(title, options, Box::new(move |cc| Ok(Box::new(App::new(b, cc.egui_ctx.clone())))))
}
