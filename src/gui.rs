use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use anyhow::Result;
use eframe::egui;
use rfd::FileDialog;

use crate::processor::process_files;
use crate::theme::{apply_adaptive_theme, setup_cjk_fonts};
use crate::types::{LogEntry, LogLevel, RunOptions, WorkerMessage};

pub fn launch_gui() -> Result<()> {
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "FileCopyer GUI",
        native_options,
        Box::new(|cc| {
            setup_cjk_fonts(&cc.egui_ctx);
            Ok(Box::new(FileCopyerApp::default()))
        }),
    )
    .map_err(|err| anyhow::anyhow!("GUI 启动失败: {err}"))
}

struct FileCopyerApp {
    source: String,
    target: String,
    template: String,
    time_source: crate::types::TimeSource,
    mv: bool,
    dry_run: bool,
    running: bool,
    logs: Vec<LogEntry>,
    summary: Option<String>,
    rx: Option<Receiver<WorkerMessage>>,
    last_dark_mode: Option<bool>,
    show_logs: bool,
}

impl Default for FileCopyerApp {
    fn default() -> Self {
        Self {
            source: String::new(),
            target: String::new(),
            template: "{YYYY}-{MM}-{DD}".to_string(),
            time_source: crate::types::TimeSource::Auto,
            mv: false,
            dry_run: false,
            running: false,
            logs: Vec::new(),
            summary: None,
            rx: None,
            last_dark_mode: None,
            show_logs: false,
        }
    }
}

impl eframe::App for FileCopyerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dark_mode = ctx.style().visuals.dark_mode;
        if self.last_dark_mode != Some(dark_mode) {
            apply_adaptive_theme(ctx, dark_mode);
            self.last_dark_mode = Some(dark_mode);
        }

        self.poll_worker();

        let accent = if dark_mode {
            egui::Color32::from_rgb(90, 170, 255)
        } else {
            egui::Color32::from_rgb(20, 110, 210)
        };
        let card_bg = if dark_mode {
            egui::Color32::from_rgb(30, 36, 48)
        } else {
            egui::Color32::from_rgb(244, 247, 252)
        };

        let panel_fill = ctx.style().visuals.panel_fill;

        egui::TopBottomPanel::top("controls_panel")
            .resizable(false)
            .frame(
                egui::Frame::default()
                    .fill(panel_fill)
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new("FileCopyer")
                            .size(28.0)
                            .strong()
                            .color(accent),
                    );
                    ui.label("照片/视频自动归档");
                });
                ui.label("选择源目录和目标目录，按日期自动分类。支持浅色/深色主题自动适配。");
                ui.add_space(8.0);

                egui::Frame::group(ui.style())
                    .fill(card_bg)
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        Self::draw_path_picker(ui, "源目录", &mut self.source, self.running);
                        Self::draw_path_picker(ui, "目标目录", &mut self.target, self.running);

                        ui.horizontal(|ui| {
                            ui.label("目录模板");
                            ui.add_sized(
                                [ui.available_width(), 30.0],
                                egui::TextEdit::singleline(&mut self.template),
                            );
                        });
                        ui.small("变量: {YYYY} {MM} {DD}，示例: {YYYY}/{MM}/{DD}");

                        ui.horizontal(|ui| {
                            ui.label("时间来源");
                            egui::ComboBox::from_id_salt("time_source")
                                .selected_text(self.time_source.label())
                                .show_ui(ui, |ui| {
                                    for source in crate::types::TimeSource::all() {
                                        ui.selectable_value(
                                            &mut self.time_source,
                                            source,
                                            source.label(),
                                        );
                                    }
                                });
                        });

                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.mv, "移动文件（默认复制）");
                            ui.checkbox(&mut self.dry_run, "仅预演（dry-run）");
                        });
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let start_enabled = !self.running;
                    let button = egui::Button::new(
                        egui::RichText::new("开始整理")
                            .strong()
                            .color(egui::Color32::WHITE),
                    )
                    .fill(accent)
                    .min_size(egui::vec2(120.0, 36.0));

                    if ui.add_enabled(start_enabled, button).clicked() {
                        self.start_task();
                    }

                    if self.running {
                        ui.colored_label(accent, "处理中...");
                    }
                });

                if let Some(summary) = &self.summary {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(summary).strong());
                }
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(panel_fill)
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(ctx, |ui| {
                if !self.show_logs {
                    return;
                }

                egui::Frame::group(ui.style())
                    .fill(card_bg)
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("执行日志").strong());
                        self.draw_log_summary(ui, dark_mode);
                        ui.add_space(4.0);

                        let log_area_height = (ui.available_height() - 8.0).max(140.0);
                        egui::ScrollArea::vertical()
                            .id_salt("log_scroll")
                            .auto_shrink([false, false])
                            .stick_to_bottom(true)
                            .max_height(log_area_height)
                            .scroll_bar_visibility(
                                egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                            )
                            .show(ui, |ui| {
                                if self.logs.is_empty() {
                                    ui.label("暂无日志，点击“开始整理”后会显示处理过程。");
                                } else {
                                    for entry in &self.logs {
                                        self.draw_log_row(ui, entry, dark_mode);
                                    }
                                }
                            });
                    });
            });
    }
}

impl FileCopyerApp {
    fn draw_log_row(&self, ui: &mut egui::Ui, entry: &LogEntry, dark_mode: bool) {
        let tag_color = match entry.level {
            LogLevel::Info => {
                if dark_mode {
                    egui::Color32::from_rgb(110, 180, 255)
                } else {
                    egui::Color32::from_rgb(30, 120, 220)
                }
            }
            LogLevel::Success => {
                if dark_mode {
                    egui::Color32::from_rgb(95, 220, 140)
                } else {
                    egui::Color32::from_rgb(40, 150, 80)
                }
            }
            LogLevel::Warn => {
                if dark_mode {
                    egui::Color32::from_rgb(255, 210, 110)
                } else {
                    egui::Color32::from_rgb(190, 120, 30)
                }
            }
            LogLevel::Error => {
                if dark_mode {
                    egui::Color32::from_rgb(255, 125, 125)
                } else {
                    egui::Color32::from_rgb(200, 45, 45)
                }
            }
            LogLevel::DryRun => {
                if dark_mode {
                    egui::Color32::from_rgb(190, 170, 255)
                } else {
                    egui::Color32::from_rgb(95, 70, 180)
                }
            }
        };

        ui.horizontal(|ui| {
            ui.monospace(
                egui::RichText::new(format!("[{}]", entry.timestamp))
                    .weak()
                    .color(ui.visuals().weak_text_color()),
            );
            ui.label(
                egui::RichText::new(format!("[{}]", entry.tag()))
                    .monospace()
                    .strong()
                    .color(tag_color),
            );
            ui.add(egui::Label::new(egui::RichText::new(&entry.message).monospace()).wrap());
        });
    }

    fn draw_log_summary(&self, ui: &mut egui::Ui, dark_mode: bool) {
        let mut success = 0usize;
        let mut warn = 0usize;
        let mut err = 0usize;
        let mut dry = 0usize;
        for entry in &self.logs {
            match entry.level {
                LogLevel::Success => success += 1,
                LogLevel::Warn => warn += 1,
                LogLevel::Error => err += 1,
                LogLevel::DryRun => dry += 1,
                LogLevel::Info => {}
            }
        }

        let text_color = if dark_mode {
            egui::Color32::from_rgb(210, 220, 235)
        } else {
            egui::Color32::from_rgb(60, 70, 85)
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(format!("总计 {}", self.logs.len())).color(text_color));
            ui.label(egui::RichText::new(format!("成功 {}", success)).color(text_color));
            ui.label(egui::RichText::new(format!("预演 {}", dry)).color(text_color));
            ui.label(egui::RichText::new(format!("警告 {}", warn)).color(text_color));
            ui.label(egui::RichText::new(format!("错误 {}", err)).color(text_color));
        });
    }

    fn draw_path_picker(ui: &mut egui::Ui, label: &str, value: &mut String, running: bool) {
        ui.horizontal(|ui| {
            ui.label(label);
            ui.add_sized(
                [ui.available_width() - 90.0, 30.0],
                egui::TextEdit::singleline(value),
            );
            if ui
                .add_enabled(!running, egui::Button::new("选择..."))
                .clicked()
                && let Some(path) = FileDialog::new().pick_folder()
            {
                *value = path.display().to_string();
            }
        });
    }

    fn start_task(&mut self) {
        self.show_logs = true;
        self.summary = None;

        let source = self.source.trim();
        let target = self.target.trim();
        if source.is_empty() || target.is_empty() {
            self.logs
                .push(LogEntry::new(LogLevel::Error, "源目录和目标目录都不能为空"));
            return;
        }

        let options = RunOptions {
            source: PathBuf::from(source),
            target: PathBuf::from(target),
            mv: self.mv,
            dry_run: self.dry_run,
            time_source: self.time_source,
            template: self.template.clone(),
        };

        let (tx, rx) = mpsc::channel::<WorkerMessage>();
        self.rx = Some(rx);
        self.running = true;
        self.logs.clear();

        thread::spawn(move || {
            let result = process_files(&options, |line| {
                let _ = tx.send(WorkerMessage::Log(line));
            })
            .map_err(|err| err.to_string());

            let _ = tx.send(WorkerMessage::Done(result));
        });
    }

    fn poll_worker(&mut self) {
        let Some(rx) = &self.rx else {
            return;
        };

        while let Ok(msg) = rx.try_recv() {
            match msg {
                WorkerMessage::Log(line) => self.logs.push(line),
                WorkerMessage::Done(result) => {
                    self.running = false;
                    match result {
                        Ok(summary) => {
                            self.summary = Some(format!(
                                "完成: 扫描 {}，处理 {}，跳过 {}，失败 {}。",
                                summary.scanned, summary.handled, summary.skipped, summary.failed
                            ));
                        }
                        Err(err) => {
                            self.summary = Some(format!("执行失败: {err}"));
                        }
                    }
                }
            }
        }
    }
}
