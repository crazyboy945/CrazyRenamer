#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod explorer;
mod hook;
mod rename;
mod tray;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{
    Align, Color32, FontData, FontDefinitions, FontFamily, FontId, Layout, Margin, RichText, Stroke,
    TextStyle,
};
use rename::{
    DateFormat, DatePosition, Entry, PlannedRename, RenameFilter, RenameMode, RenamePlan, RenameRule,
};

const APP_TITLE: &str = "CrazyRenamer";
const ICON_RGBA: &[u8] = include_bytes!("../assets/icon_64.rgba");

const BG: Color32 = Color32::from_rgb(0xF3, 0xF6, 0xFB);
const CARD: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const SURFACE_SOFT: Color32 = Color32::from_rgb(0xF6, 0xF8, 0xFC);
const BORDER: Color32 = Color32::from_rgb(0xE2, 0xE8, 0xF1);
const TEXT: Color32 = Color32::from_rgb(0x1F, 0x2A, 0x3C);
const MUTED: Color32 = Color32::from_rgb(0x64, 0x74, 0x8B);
const FAINT: Color32 = Color32::from_rgb(0x93, 0xA1, 0xB5);
const ACCENT: Color32 = Color32::from_rgb(0x4C, 0x6C, 0xF0);
const ACCENT_DARK: Color32 = Color32::from_rgb(0x39, 0x55, 0xCF);
const ACCENT_SOFT: Color32 = Color32::from_rgb(0xEC, 0xF1, 0xFF);
const ACCENT_SOFT_HOVER: Color32 = Color32::from_rgb(0xE1, 0xE9, 0xFF);
const SUCCESS: Color32 = Color32::from_rgb(0x16, 0x7C, 0x4A);
const SUCCESS_SOFT: Color32 = Color32::from_rgb(0xE8, 0xF8, 0xEF);
const DANGER: Color32 = Color32::from_rgb(0xC0, 0x30, 0x3C);
const DANGER_SOFT: Color32 = Color32::from_rgb(0xFD, 0xEF, 0xF0);
const WARN: Color32 = Color32::from_rgb(0xA9, 0x63, 0x0A);
const WARN_SOFT: Color32 = Color32::from_rgb(0xFF, 0xF6, 0xE6);
const INPUT_BG: Color32 = Color32::from_rgb(0xF7, 0xF9, 0xFD);
const ZEBRA: Color32 = Color32::from_rgb(0xF8, 0xFA, 0xFD);
const DISABLED_BG: Color32 = Color32::from_rgb(0xE4, 0xEA, 0xF3);

fn main() -> eframe::Result {
    let icon = egui::IconData {
        rgba: ICON_RGBA.to_vec(),
        width: 64,
        height: 64,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_TITLE)
            .with_icon(icon)
            .with_inner_size([1080.0, 740.0])
            .with_min_inner_size([900.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        APP_TITLE,
        options,
        Box::new(|cc| Ok(Box::new(RenamerApp::new(cc)))),
    )
}

struct RenamerApp {
    folder: Option<PathBuf>,
    folder_input: String,
    folder_error: Option<String>,
    entries: Vec<Entry>,
    available_extensions: Vec<String>,
    selected_extensions: BTreeSet<String>,
    rename_files: bool,
    rename_dirs: bool,
    skipped_non_utf8: usize,
    rule: RenameRule,
    plan: Option<RenamePlan>,
    plan_error: Option<String>,
    status: String,
    status_is_error: bool,
    apply_failures: Vec<String>,
    hook: hook::HookHandle,
    tray: tray::TrayHandle,
    icon: egui::TextureHandle,
    hidden_to_tray: bool,
    skip_minimize_hide: bool,
    quitting: bool,
    show_focus_at: Option<Instant>,
    focus_attempts: u8,
}

impl RenamerApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_chinese_fonts(&cc.egui_ctx);
        configure_style(&cc.egui_ctx);
        explorer::init_com();
        let hook = hook::start(cc.egui_ctx.clone());
        let tray = tray::start(cc.egui_ctx.clone());
        let icon = cc.egui_ctx.load_texture(
            "app-icon",
            egui::ColorImage::from_rgba_unmultiplied([64, 64], ICON_RGBA),
            egui::TextureOptions::LINEAR,
        );
        Self {
            folder: None,
            folder_input: String::new(),
            folder_error: None,
            entries: Vec::new(),
            available_extensions: Vec::new(),
            selected_extensions: BTreeSet::new(),
            rename_files: true,
            rename_dirs: false,
            skipped_non_utf8: 0,
            rule: RenameRule::default(),
            plan: None,
            plan_error: None,
            status: "准备就绪。可在资源管理器中点击鼠标中键，或在上方载入文件夹。".to_owned(),
            status_is_error: false,
            apply_failures: Vec::new(),
            hook,
            tray,
            icon,
            hidden_to_tray: false,
            skip_minimize_hide: false,
            quitting: false,
            show_focus_at: None,
            focus_attempts: 0,
        }
    }

    fn process_middle_clicks(&mut self, ctx: &egui::Context) {
        let mut latest = None;
        while let Ok(hwnd) = self.hook.receiver.try_recv() {
            latest = Some(hwnd);
        }
        let Some(hwnd) = latest else {
            return;
        };
        self.show_window(ctx);
        if hwnd == 0 {
            self.set_error("无法识别资源管理器窗口。".to_owned());
            return;
        }
        let foreground = windows::Win32::Foundation::HWND(hwnd as *mut std::ffi::c_void);
        match explorer::active_folder(foreground) {
            Ok(path) => self.load_folder(path),
            Err(error) => self.set_error(format!("无法载入资源管理器中的文件夹：{error}")),
        }
    }

    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        self.hidden_to_tray = true;
        self.skip_minimize_hide = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.hidden_to_tray = false;
        self.skip_minimize_hide = true;
        self.focus_attempts = 3;
        self.show_focus_at = Some(Instant::now() + Duration::from_millis(140));
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint_after(Duration::from_millis(140));
    }

    fn process_tray(&mut self, ctx: &egui::Context) {
        while let Ok(command) = self.tray.receiver.try_recv() {
            match command {
                tray::TrayCommand::Show => self.show_window(ctx),
                tray::TrayCommand::Exit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn handle_window_state(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.viewport().close_requested())
            && !self.quitting
            && self.tray.is_ready()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_to_tray(ctx);
        }

        let minimized = ctx.input(|input| input.viewport().minimized == Some(true));
        if minimized {
            if !self.hidden_to_tray && !self.skip_minimize_hide && self.tray.is_ready() {
                self.hide_to_tray(ctx);
            }
        } else if !self.hidden_to_tray {
            self.skip_minimize_hide = false;
        }

        if let Some(due) = self.show_focus_at {
            if Instant::now() >= due {
                self.show_focus_at = None;
                hook::focus_main_window(APP_TITLE);
                if self.focus_attempts > 1 {
                    self.focus_attempts -= 1;
                    self.show_focus_at = Some(Instant::now() + Duration::from_millis(140));
                    ctx.request_repaint_after(Duration::from_millis(140));
                } else {
                    self.focus_attempts = 0;
                }
            }
        }
    }

    fn load_folder(&mut self, path: PathBuf) {
        let mut skipped = 0;
        match rename::list_entries(&path, &mut skipped) {
            Ok(entries) => {
                self.folder = Some(path.clone());
                self.folder_input = path.display().to_string();
                self.folder_error = None;
                self.entries = entries;
                self.skipped_non_utf8 = skipped;
                self.refresh_available_extensions(true);
                self.apply_failures.clear();
                self.rebuild_plan();
                let (files, dirs) = self.counts();
                self.status = if self.entries.is_empty() {
                    format!("已载入 {}：文件夹为空。", path.display())
                } else {
                    format!(
                        "已载入 {}：{} 个文件、{} 个文件夹。",
                        path.display(),
                        files,
                        dirs
                    )
                };
                self.status_is_error = false;
            }
            Err(error) => {
                let message = format!("载入失败：{error}");
                self.folder_error = Some(message.clone());
                self.set_error(message);
            }
        }
    }

    fn counts(&self) -> (usize, usize) {
        let files = self.entries.iter().filter(|entry| !entry.is_dir).count();
        (files, self.entries.len() - files)
    }

    fn refresh_available_extensions(&mut self, reset: bool) {
        let previous: BTreeSet<String> = self.available_extensions.iter().cloned().collect();
        let available: BTreeSet<String> = self
            .entries
            .iter()
            .filter(|entry| !entry.is_dir)
            .map(|entry| rename::extension_key(&entry.name))
            .collect();
        let available: Vec<String> = available.into_iter().collect();
        if reset {
            self.selected_extensions = available.iter().cloned().collect();
        } else {
            self.selected_extensions
                .retain(|extension| available.contains(extension));
            for extension in &available {
                if !previous.contains(extension) {
                    self.selected_extensions.insert(extension.clone());
                }
            }
        }
        self.available_extensions = available;
    }

    fn refresh_files(&mut self) {
        let Some(folder) = self.folder.clone() else {
            return;
        };
        let mut skipped = 0;
        match rename::list_entries(&folder, &mut skipped) {
            Ok(entries) => {
                self.entries = entries;
                self.skipped_non_utf8 = skipped;
                self.folder_error = None;
                self.refresh_available_extensions(false);
                self.rebuild_plan();
                let (files, dirs) = self.counts();
                self.status = format!(
                    "已刷新 {}：{} 个文件、{} 个文件夹。",
                    folder.display(),
                    files,
                    dirs
                );
                self.status_is_error = false;
            }
            Err(error) => self.set_error(format!("刷新失败：{error}")),
        }
    }

    fn rebuild_plan(&mut self) {
        self.plan = None;
        self.plan_error = None;
        let Some(folder) = &self.folder else {
            return;
        };
        let filter = RenameFilter {
            files: self.rename_files,
            dirs: self.rename_dirs,
            extensions: Some(self.selected_extensions.iter().cloned().collect()),
        };
        let (plan, error) =
            rename::make_preview_plan(folder, &self.entries, &filter, &self.rule);
        self.plan = Some(plan);
        self.plan_error = error;
    }

    fn apply_current_plan(&mut self) {
        let Some(folder) = self.folder.clone() else {
            return;
        };
        let Some(plan) = self.plan.clone() else {
            return;
        };
        if !plan.can_apply() {
            return;
        }
        let report = rename::apply_plan(&folder, &plan);
        self.apply_failures = report.failures;
        let mut message = format!("已成功更名 {} 个项目。", report.renamed);
        if !self.apply_failures.is_empty() {
            message.push_str(&format!(" {} 个失败，详见下方。", self.apply_failures.len()));
            self.status_is_error = true;
        } else {
            self.status_is_error = false;
        }
        self.status = message;

        let mut skipped = 0;
        match rename::list_entries(&folder, &mut skipped) {
            Ok(entries) => {
                self.entries = entries;
                self.skipped_non_utf8 = skipped;
                self.refresh_available_extensions(false);
            }
            Err(error) => {
                self.set_error(format!("更名完成，但刷新文件列表失败：{error}"));
            }
        }
        self.rebuild_plan();
    }

    fn set_error(&mut self, message: String) {
        self.status = message;
        self.status_is_error = true;
    }

    fn load_from_input(&mut self) {
        let trimmed = self.folder_input.trim().trim_matches('"').trim();
        if trimmed.is_empty() {
            let message = "请输入文件夹路径，或点击“浏览…”选择文件夹。".to_owned();
            self.folder_error = Some(message.clone());
            self.set_error(message);
            return;
        }
        self.load_folder(PathBuf::from(trimmed));
    }

    fn browse_for_folder(&mut self, frame: &eframe::Frame) {
        let dialog = rfd::FileDialog::new()
            .set_title("选择要批量更名的文件夹")
            .set_parent(&*frame);
        if let Some(path) = dialog.pick_folder() {
            self.load_folder(path);
        }
    }

    fn filter_editor(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            accent_mark(ui);
            ui.label(RichText::new("范围与筛选").size(15.0).strong().color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                badge(ui, "仅第一层", FAINT, SURFACE_SOFT);
            });
        });
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("范围").size(12.0).color(MUTED));
            changed |= ui.checkbox(&mut self.rename_files, "文件").changed();
            changed |= ui.checkbox(&mut self.rename_dirs, "文件夹").changed();
            if !self.rename_files && !self.rename_dirs {
                ui.label(RichText::new("请至少选择一项").size(12.0).color(WARN));
            }
        });

        if self.rename_files {
            ui.add_space(10.0);
            let extensions = self.available_extensions.clone();
            ui.horizontal(|ui| {
                ui.label(RichText::new("扩展名").size(12.0).color(MUTED));
                if !extensions.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "{} / {}",
                            self.selected_extensions.len(),
                            extensions.len()
                        ))
                        .size(11.0)
                        .color(FAINT),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if !extensions.is_empty() {
                        if text_button(ui, "清空").clicked() {
                            self.selected_extensions.clear();
                            changed = true;
                        }
                        if text_button(ui, "全选").clicked() {
                            self.selected_extensions =
                                extensions.iter().cloned().collect();
                            changed = true;
                        }
                    }
                });
            });
            ui.add_space(5.0);
            if extensions.is_empty() {
                let empty_text = if self.folder.is_some() {
                    "当前文件夹中没有文件"
                } else {
                    "载入文件夹后可选择扩展名"
                };
                ui.label(RichText::new(empty_text).size(12.0).color(FAINT));
            } else {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    for extension in &extensions {
                        let selected = self.selected_extensions.contains(extension);
                        let label = if extension.is_empty() {
                            "无扩展名".to_owned()
                        } else {
                            format!(".{extension}")
                        };
                        if chip(ui, selected, &label).clicked() {
                            if selected {
                                self.selected_extensions.remove(extension);
                            } else {
                                self.selected_extensions.insert(extension.clone());
                            }
                            changed = true;
                        }
                    }
                });
                if self.selected_extensions.is_empty() {
                    ui.add_space(8.0);
                    notice(ui, "未选择扩展名，文件将被全部跳过", WARN, WARN_SOFT);
                }
            }
        }

        ui.add_space(7.0);
        let hint = if self.rename_files {
            "仅处理勾选的扩展名；文件夹仅第一层。"
        } else {
            "仅处理第一层子文件夹的名称。"
        };
        ui.label(RichText::new(hint).size(11.5).color(FAINT));
        changed
    }

    fn rule_editor(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            accent_mark(ui);
            ui.label(RichText::new("更名规则").size(15.0).strong().color(TEXT));
        });

        if let Some(error) = &self.plan_error {
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(format!("! {error}")).size(11.5).color(DANGER),
                )
                .truncate(),
            )
            .on_hover_text(error);
        }
        ui.add_space(10.0);

        let gap = 4.0;
        let segment_width = ((ui.available_width() - gap * 4.0) / 5.0).floor();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for mode in RenameMode::ALL {
                if segment_button(
                    ui,
                    self.rule.mode == mode,
                    mode.short_label(),
                    segment_width,
                )
                .on_hover_text(mode.label())
                .clicked()
                {
                    self.rule.mode = mode;
                    changed = true;
                }
            }
        });
        ui.add_space(12.0);

        match self.rule.mode {
            RenameMode::Replace => {
                ui.columns(2, |columns| {
                    changed |= text_field(
                        &mut columns[0],
                        "查找内容",
                        &mut self.rule.find,
                        "要被替换的字串",
                    );
                    changed |= text_field(
                        &mut columns[1],
                        "替换为",
                        &mut self.rule.replace,
                        "留空表示删除",
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    changed |= ui
                        .checkbox(
                            &mut self.rule.keep_extension,
                            RichText::new("保留扩展名").size(13.0),
                        )
                        .changed();
                    changed |= ui
                        .checkbox(
                            &mut self.rule.ignore_case,
                            RichText::new("忽略大小写").size(13.0),
                        )
                        .changed();
                    if self.plan_error.is_none() {
                        ui.add_space(6.0);
                        caption(ui, "只修改文件主文件名；文件夹名称按整体处理。");
                    }
                });
            }
            RenameMode::Regex => {
                ui.columns(2, |columns| {
                    changed |= text_field(
                        &mut columns[0],
                        "正则表达式",
                        &mut self.rule.find,
                        r"例如 ^IMG_(\d+)$",
                    );
                    changed |= text_field(
                        &mut columns[1],
                        "替换为",
                        &mut self.rule.replace,
                        "支持 $1 捕获组",
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    changed |= ui
                        .checkbox(
                            &mut self.rule.keep_extension,
                            RichText::new("保留扩展名").size(13.0),
                        )
                        .changed();
                    changed |= ui
                        .checkbox(
                            &mut self.rule.ignore_case,
                            RichText::new("忽略大小写").size(13.0),
                        )
                        .changed();
                    if self.plan_error.is_none() {
                        ui.add_space(6.0);
                        caption(ui, "Rust regex 语法；文件夹名称按整体匹配。");
                    }
                });
            }
            RenameMode::Numbering => {
                changed |= text_field(ui, "模板", &mut self.rule.template, "例如 {name}_{n}");
                ui.columns(3, |columns| {
                    changed |= number_field(&mut columns[0], "起始", &mut self.rule.start, 0, 1_000_000);
                    changed |= number_field(&mut columns[1], "步长", &mut self.rule.step, 1, 1_000_000);
                    changed |= number_field(&mut columns[2], "位数", &mut self.rule.digits, 1, 12);
                });
                if self.plan_error.is_none() {
                    ui.add_space(6.0);
                    caption(ui, "{n} 为编号，{name} 为原名称；文件保留扩展名。");
                }
            }
            RenameMode::Date => {
                let gap = 6.0;
                ui.columns(2, |columns| {
                    field_label(&mut columns[0], "格式");
                    columns[0].add_space(8.0);
                    let width = ((columns[0].available_width() - gap) / 2.0).floor();
                    columns[0].horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for format in DateFormat::ALL {
                            if segment_button(
                                ui,
                                self.rule.date_format == format,
                                format.label(),
                                width,
                            )
                            .clicked()
                            {
                                self.rule.date_format = format;
                                changed = true;
                            }
                        }
                    });

                    field_label(&mut columns[1], "位置");
                    columns[1].add_space(8.0);
                    let width = ((columns[1].available_width() - gap) / 2.0).floor();
                    columns[1].horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for position in DatePosition::ALL {
                            if segment_button(
                                ui,
                                self.rule.date_position == position,
                                position.label(),
                                width,
                            )
                            .clicked()
                            {
                                self.rule.date_position = position;
                                changed = true;
                            }
                        }
                    });
                });
                if self.plan_error.is_none() {
                    ui.add_space(8.0);
                    caption(ui, "将今天的日期添加到名称；文件保留扩展名。");
                }
            }
            RenameMode::Affix => {
                ui.columns(2, |columns| {
                    changed |= text_field(
                        &mut columns[0],
                        "前缀",
                        &mut self.rule.prefix,
                        "添加在名称开头",
                    );
                    changed |= text_field(
                        &mut columns[1],
                        "后缀",
                        &mut self.rule.suffix,
                        "添加在扩展名之前",
                    );
                });
                if self.plan_error.is_none() {
                    ui.add_space(4.0);
                    caption(ui, "前缀和后缀可同时使用；文件保留扩展名，文件夹按完整名称处理。");
                }
            }
        }
        changed
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            accent_mark(ui);
            ui.label(RichText::new("更名预览").size(15.0).strong().color(TEXT));
            if self.plan_error.is_some() {
                badge(ui, "规则未就绪", WARN, WARN_SOFT);
            } else if let Some(plan) = &self.plan {
                if plan.error_count > 0 {
                    badge(
                        ui,
                        &format!("{} 个问题", plan.error_count),
                        DANGER,
                        DANGER_SOFT,
                    );
                } else if plan.changed_count > 0 {
                    badge(
                        ui,
                        &format!("{} 项待更名", plan.changed_count),
                        SUCCESS,
                        SUCCESS_SOFT,
                    );
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.skipped_non_utf8 > 0 {
                    badge(
                        ui,
                        &format!("已忽略 {} 个名称", self.skipped_non_utf8),
                        WARN,
                        WARN_SOFT,
                    );
                }
                badge(ui, "仅第一层", FAINT, SURFACE_SOFT);
            });
        });

        if let Some(folder) = &self.folder {
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(folder.display().to_string())
                        .size(11.0)
                        .monospace()
                        .color(FAINT),
                )
                .truncate(),
            );
        }
        ui.add_space(12.0);

        if self.folder.is_none() {
            self.empty_preview(
                ui,
                "从一个文件夹开始",
                "在资源管理器中打开目标文件夹并点击鼠标中键，也可以使用上方的路径输入框或“浏览…”按钮。",
                true,
            );
            return;
        }

        if self.entries.is_empty() {
            self.empty_preview(
                ui,
                "这个文件夹是空的",
                "没有可处理的文件或文件夹。回到资源管理器切换目录，或点击上方“刷新”重新扫描。",
                false,
            );
            return;
        }

        let Some(plan) = &self.plan else {
            self.empty_preview(
                ui,
                "规则尚未就绪",
                self.plan_error.as_deref().unwrap_or("请检查更名规则。"),
                false,
            );
            return;
        };

        if plan.rows.is_empty() {
            self.empty_preview(
                ui,
                "没有符合筛选条件的项目",
                "请在左侧调整处理范围，或重新勾选扩展名。",
                false,
            );
            return;
        }

        let spacing = 10.0;
        let status_width = 168.0;
        let arrow_width = 22.0;
        let name_width = ((ui.available_width() - status_width - arrow_width - spacing * 3.0)
            / 2.0)
            .max(80.0);

        egui::Frame::new()
            .fill(SURFACE_SOFT)
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(Margin::symmetric(8, 7))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = spacing;
                    table_cell(
                        ui,
                        name_width,
                        16.0,
                        RichText::new("原名称").size(11.5).strong().color(MUTED),
                        false,
                    );
                    ui.add_sized([arrow_width, 16.0], egui::Label::new(""));
                    table_cell(
                        ui,
                        name_width,
                        16.0,
                        RichText::new("新名称").size(11.5).strong().color(MUTED),
                        false,
                    );
                    table_cell(
                        ui,
                        status_width,
                        16.0,
                        RichText::new("状态").size(11.5).strong().color(MUTED),
                        false,
                    );
                });
            });
        ui.add_space(5.0);

        let table_height = ui.available_height().max(60.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(table_height)
            .show_rows(ui, 32.0, plan.rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
                for index in range {
                    preview_row(
                        ui,
                        &plan.rows[index],
                        index,
                        name_width,
                        arrow_width,
                        status_width,
                        spacing,
                    );
                }
            });
    }

    fn empty_preview(&self, ui: &mut egui::Ui, title: &str, detail: &str, show_icon: bool) {
        let height = ui.available_height();
        let show_icon = show_icon && height > 150.0;
        let offset = (height * 0.08).clamp(4.0, 30.0);
        ui.add_space(offset);
        ui.vertical_centered(|ui| {
            if show_icon {
                ui.add(
                    egui::Image::new(&self.icon)
                        .fit_to_exact_size(egui::vec2(50.0, 50.0))
                        .corner_radius(egui::CornerRadius::same(13)),
                );
                ui.add_space(10.0);
            }
            ui.label(RichText::new(title).size(16.5).strong().color(TEXT));
            ui.add_space(6.0);
            ui.allocate_ui_with_layout(
                egui::vec2(440.0, 0.0),
                Layout::top_down(Align::Center),
                |ui| {
                    ui.add(egui::Label::new(RichText::new(detail).size(12.0).color(MUTED)).wrap());
                },
            );
            if show_icon {
                ui.add_space(10.0);
                badge(ui, "仅处理第一层 · 不递归 · 关闭后驻留托盘", FAINT, SURFACE_SOFT);
            }
        });
    }
}

impl eframe::App for RenamerApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_window_state(ctx);
        self.process_tray(ctx);
        self.process_middle_clicks(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let mut load_requested = false;
        let mut browse_requested = false;
        let mut refresh_requested = false;

        egui::Panel::top("header")
            .frame(egui::Frame::new().fill(CARD).inner_margin(Margin {
                left: 22,
                right: 22,
                top: 14,
                bottom: 12,
            }))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Image::new(&self.icon)
                            .fit_to_exact_size(egui::vec2(36.0, 36.0))
                            .corner_radius(egui::CornerRadius::same(9)),
                    );
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(APP_TITLE).size(19.0).strong().color(TEXT));
                        ui.add_space(1.0);
                        ui.label(
                            RichText::new("资源管理器批量更名工具 · 关闭后驻留托盘")
                                .size(11.0)
                                .color(FAINT),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        hook_status_pill(ui, &self.hook);
                        ui.add_space(6.0);
                        let mut enabled = self.hook.enabled.load(Ordering::Relaxed);
                        if ui
                            .checkbox(&mut enabled, RichText::new("中键触发").size(12.0))
                            .on_hover_text(
                                "启用后，在资源管理器文件夹内点击中键即可载入。\n关闭或最小化窗口时隐藏到托盘，右键托盘图标可退出。",
                            )
                            .changed()
                        {
                            self.hook.enabled.store(enabled, Ordering::Relaxed);
                        }
                    });
                });
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    let field_width = (ui.available_width() - 238.0).max(140.0);
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.folder_input)
                            .desired_width(field_width)
                            .hint_text(
                                RichText::new(r"输入文件夹路径，例如 C:\Users\me\Pictures")
                                    .size(12.0)
                                    .color(FAINT),
                            )
                            .background_color(INPUT_BG)
                            .text_color(TEXT)
                            .margin(Margin::symmetric(10, 7)),
                    );
                    let enter_pressed = response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter))
                        && !self.folder_input.trim().is_empty();
                    if response.changed() {
                        self.folder_error = None;
                    }
                    let load_clicked = primary_button(ui, "载入", 64.0, true).clicked();
                    let browse_clicked = secondary_button(ui, "浏览…", 82.0).clicked();
                    let refresh_clicked = secondary_button(ui, "刷新", 60.0).clicked();
                    if enter_pressed
                        || (load_clicked && !self.folder_input.trim().is_empty())
                    {
                        load_requested = true;
                    } else if browse_clicked
                        || (load_clicked && self.folder_input.trim().is_empty())
                    {
                        browse_requested = true;
                    }
                    if refresh_clicked && self.folder.is_some() {
                        refresh_requested = true;
                    }
                });

                if let Some(error) = &self.folder_error {
                    ui.add_space(8.0);
                    notice(ui, error, DANGER, DANGER_SOFT);
                }

                let hook_failed = self.hook.enabled.load(Ordering::Relaxed)
                    && self.hook.status.load(Ordering::Relaxed) == hook::STATUS_FAILED;
                if hook_failed {
                    let error = self
                        .hook
                        .error
                        .lock()
                        .ok()
                        .and_then(|error| error.clone())
                        .unwrap_or_else(|| "未知错误".to_owned());
                    ui.add_space(7.0);
                    notice(ui, &format!("中键监听失败：{error}"), DANGER, DANGER_SOFT);
                }
                if self.tray.status() == tray::STATUS_FAILED {
                    let error = self.tray.error().unwrap_or_else(|| "未知错误".to_owned());
                    ui.add_space(7.0);
                    notice(
                        ui,
                        &format!("托盘不可用：{error}（关闭窗口将直接退出）"),
                        DANGER,
                        DANGER_SOFT,
                    );
                }
            });

        if browse_requested {
            self.browse_for_folder(frame);
        }
        if load_requested {
            self.load_from_input();
        }
        if refresh_requested {
            self.refresh_files();
        }

        let mut rules_changed = false;
        egui::Panel::top("settings")
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(BG).inner_margin(Margin {
                left: 20,
                right: 20,
                top: 14,
                bottom: 8,
            }))
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let filter_width = (ui.available_width() * 0.31).clamp(286.0, 336.0);
                    let mut filter_rect = egui::Rect::NOTHING;
                    let mut rule_rect = egui::Rect::NOTHING;
                    let filter_idx = ui.painter().add(egui::Shape::Noop);
                    ui.vertical(|ui| {
                        ui.set_width(filter_width);
                        let inner = card_content_frame().show(ui, |ui| {
                            rules_changed |= self.filter_editor(ui);
                        });
                        filter_rect = inner.response.rect;
                    });
                    let rule_idx = ui.painter().add(egui::Shape::Noop);
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        let inner = card_content_frame().show(ui, |ui| {
                            rules_changed |= self.rule_editor(ui);
                        });
                        rule_rect = inner.response.rect;
                    });
                    let max_height = filter_rect.height().max(rule_rect.height());
                    paint_card_background(
                        ui,
                        filter_idx,
                        egui::Rect::from_min_size(
                            filter_rect.min,
                            egui::vec2(filter_rect.width(), max_height),
                        ),
                    );
                    paint_card_background(
                        ui,
                        rule_idx,
                        egui::Rect::from_min_size(
                            rule_rect.min,
                            egui::vec2(rule_rect.width(), max_height),
                        ),
                    );
                });
            });

        if rules_changed {
            self.rebuild_plan();
        }

        let (can_apply, disabled_reason) = if self.folder.is_none() {
            (false, "请先载入文件夹".to_owned())
        } else if self.plan_error.is_some() {
            (false, "请先修正规则".to_owned())
        } else if let Some(plan) = &self.plan {
            (
                plan.can_apply(),
                if plan.rows.is_empty() {
                    "当前筛选条件下没有可处理的项目"
                } else if plan.changed_count == 0 {
                    "没有需要更改的名称"
                } else if plan.error_count > 0 {
                    "请先解决预览中的问题"
                } else {
                    "按预览执行更名"
                }
                .to_owned(),
            )
        } else {
            (false, "规则尚未就绪".to_owned())
        };

        let mut apply_requested = false;
        egui::Panel::bottom("actions")
            .frame(egui::Frame::new().fill(CARD).inner_margin(Margin {
                left: 20,
                right: 20,
                top: 12,
                bottom: 12,
            }))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(if self.folder.is_some() {
                                    format!("共 {} 个项目", self.entries.len())
                                } else {
                                    "尚未载入文件夹".to_owned()
                                })
                                .size(13.0)
                                .strong()
                                .color(TEXT),
                            );
                            if self.plan_error.is_some() {
                                badge(ui, "规则未就绪", WARN, WARN_SOFT);
                            } else if let Some(plan) = &self.plan {
                                if plan.changed_count > 0 {
                                    ui.label(
                                        RichText::new(format!("将更名 {} 个", plan.changed_count))
                                            .size(12.0)
                                            .color(SUCCESS),
                                    );
                                }
                                if plan.error_count > 0 {
                                    ui.label(
                                        RichText::new(format!("{} 个问题", plan.error_count))
                                            .size(12.0)
                                            .color(DANGER),
                                    );
                                }
                                if plan.changed_count == 0 && plan.error_count == 0 {
                                    ui.label(RichText::new("无需更改").size(12.0).color(FAINT));
                                }
                            }
                        });
                        ui.add_space(3.0);
                        ui.horizontal(|ui| {
                            let dot_color = if self.status_is_error {
                                DANGER
                            } else if self.folder.is_some() {
                                SUCCESS
                            } else {
                                FAINT
                            };
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(8.0, 8.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().circle_filled(rect.center(), 3.0, dot_color);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&self.status).size(11.0).color(
                                        if self.status_is_error { DANGER } else { FAINT },
                                    ),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&self.status);
                        });
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary_button(ui, "应用更名", 132.0, can_apply).clicked() {
                            apply_requested = true;
                        }
                        if !can_apply {
                            ui.add_space(6.0);
                            ui.label(RichText::new(&disabled_reason).size(11.0).color(FAINT));
                        }
                    });
                });

                if !self.apply_failures.is_empty() {
                    ui.add_space(6.0);
                    egui::CollapsingHeader::new(format!(
                        "{} 个项目未能更名",
                        self.apply_failures.len()
                    ))
                    .default_open(true)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(90.0).show(ui, |ui| {
                            for failure in &self.apply_failures {
                                ui.colored_label(
                                    DANGER,
                                    RichText::new(failure).size(11.0),
                                );
                            }
                        });
                    });
                }
            });

        if apply_requested {
            self.apply_current_plan();
            ui.ctx().request_repaint();
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(Margin {
                left: 20,
                right: 20,
                top: 8,
                bottom: 8,
            }))
            .show(ui, |ui| {
                let card_height = ui.available_height();
                card_frame().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height((card_height - 28.0).max(120.0));
                    self.preview(ui);
                });
            });
    }
}

fn configure_style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Light);
    let mut style = (*ctx.style_of(egui::Theme::Light)).clone();
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = BG;
    visuals.window_fill = CARD;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.window_corner_radius = egui::CornerRadius::same(12);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 20,
        spread: 0,
        color: Color32::from_black_alpha(24),
    };
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 14,
        spread: 0,
        color: Color32::from_black_alpha(18),
    };
    visuals.faint_bg_color = ZEBRA;
    visuals.extreme_bg_color = INPUT_BG;
    visuals.text_edit_bg_color = Some(INPUT_BG);
    visuals.hyperlink_color = ACCENT;
    visuals.warn_fg_color = WARN;
    visuals.error_fg_color = DANGER;
    visuals.selection.bg_fill = ACCENT_SOFT_HOVER;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = CARD;
    widgets.noninteractive.weak_bg_fill = CARD;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    widgets.noninteractive.corner_radius = egui::CornerRadius::same(8);
    widgets.inactive.bg_fill = CARD;
    widgets.inactive.weak_bg_fill = SURFACE_SOFT;
    widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    widgets.inactive.corner_radius = egui::CornerRadius::same(8);
    widgets.hovered.bg_fill = ACCENT_SOFT;
    widgets.hovered.weak_bg_fill = ACCENT_SOFT_HOVER;
    widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xC8, 0xD5, 0xF4));
    widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    widgets.hovered.corner_radius = egui::CornerRadius::same(8);
    widgets.active.bg_fill = ACCENT_SOFT_HOVER;
    widgets.active.weak_bg_fill = ACCENT_SOFT_HOVER;
    widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    widgets.active.fg_stroke = Stroke::new(1.0, ACCENT_DARK);
    widgets.active.corner_radius = egui::CornerRadius::same(8);
    widgets.open.bg_fill = ACCENT_SOFT;
    widgets.open.weak_bg_fill = ACCENT_SOFT_HOVER;
    widgets.open.bg_stroke = Stroke::new(1.0, BORDER);
    widgets.open.fg_stroke = Stroke::new(1.0, TEXT);
    widgets.open.corner_radius = egui::CornerRadius::same(8);

    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.scroll.foreground_color = true;
    style.spacing.scroll.floating_width = 4.0;
    style.spacing.scroll.dormant_handle_opacity = 0.35;
    style.spacing.scroll.active_handle_opacity = 0.6;
    style.spacing.scroll.interact_handle_opacity = 1.0;
    style.animation_time = 0.12;
    style.text_styles = BTreeMap::from([
        (TextStyle::Small, FontId::proportional(11.0)),
        (TextStyle::Body, FontId::proportional(13.0)),
        (TextStyle::Button, FontId::proportional(12.5)),
        (TextStyle::Heading, FontId::proportional(20.0)),
        (TextStyle::Monospace, FontId::monospace(12.0)),
    ]);

    ctx.set_style_of(egui::Theme::Light, style.clone());
    ctx.set_global_style(style);
}

fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(Margin::symmetric(15, 14))
        .shadow(egui::epaint::Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: Color32::from_black_alpha(10),
        })
}

fn card_content_frame() -> egui::Frame {
    egui::Frame::new().inner_margin(Margin::symmetric(15, 14))
}

fn paint_card_background(ui: &egui::Ui, idx: egui::layers::ShapeIdx, rect: egui::Rect) {
    let rect = rect.expand(1.0);
    let corner = egui::CornerRadius::same(12);
    let shadow = egui::Shadow {
        offset: [0, 2],
        blur: 8,
        spread: 0,
        color: Color32::from_black_alpha(10),
    };
    let shapes = egui::Shape::Vec(vec![
        egui::Shape::Rect(shadow.as_shape(rect, corner)),
        egui::Shape::Rect(egui::epaint::RectShape::new(
            rect,
            corner,
            CARD,
            Stroke::new(1.0, BORDER),
            egui::StrokeKind::Inside,
        )),
    ]);
    ui.painter().set(idx, shapes);
}

fn accent_mark(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(3.0, 14.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(2), ACCENT);
    ui.add_space(2.0);
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(MUTED));
}

fn caption(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(11.5).color(FAINT)).wrap());
}

fn inline_label(ui: &mut egui::Ui, text: &str, width: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::hover());
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(12.0),
        MUTED,
    );
}

fn text_field(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) -> bool {
    let response = ui.horizontal(|ui| {
        inline_label(ui, label, 68.0);
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(ui.available_width())
                .hint_text(RichText::new(hint).size(12.0).color(FAINT))
                .background_color(INPUT_BG)
                .text_color(TEXT)
                .margin(Margin::symmetric(10, 7)),
        )
    });
    ui.add_space(7.0);
    response.inner.changed()
}

fn number_field<Num: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Num,
    min: Num,
    max: Num,
) -> bool {
    ui.horizontal(|ui| {
        inline_label(ui, label, 40.0);
        number_box(ui, value, min, max)
    })
    .inner
}

fn number_box<Num: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    value: &mut Num,
    min: Num,
    max: Num,
) -> bool {
    egui::Frame::new()
        .fill(INPUT_BG)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.add_sized(
                [ui.available_width(), 18.0],
                egui::DragValue::new(value).speed(1.0).range(min..=max),
            )
        })
        .inner
        .changed()
}

fn segment_button(
    ui: &mut egui::Ui,
    selected: bool,
    label: &str,
    width: f32,
) -> egui::Response {
    let size = egui::vec2(width.max(44.0), 30.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let fill = if selected {
            ACCENT
        } else if hovered {
            ACCENT_SOFT_HOVER
        } else {
            SURFACE_SOFT
        };
        let stroke = if selected {
            Stroke::new(1.0, ACCENT)
        } else if hovered {
            Stroke::new(1.0, Color32::from_rgb(0xC8, 0xD5, 0xF4))
        } else {
            Stroke::new(1.0, BORDER)
        };
        let text_color = if selected { Color32::WHITE } else { TEXT };
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(8), fill);
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(8),
            stroke,
            egui::StrokeKind::Inside,
        );
        let galley =
            ui.painter()
                .layout_no_wrap(label.to_owned(), FontId::proportional(12.5), text_color);
        ui.painter().galley(
            egui::pos2(
                rect.center().x - galley.size().x / 2.0,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            text_color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn chip(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
    let font = FontId::proportional(11.5);
    let text_color = if selected { ACCENT_DARK } else { MUTED };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, text_color);
    let size = egui::vec2(galley.size().x + 20.0, 25.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let fill = if selected {
            ACCENT_SOFT
        } else if hovered {
            SURFACE_SOFT
        } else {
            CARD
        };
        let stroke = if selected {
            Stroke::new(1.0, ACCENT)
        } else if hovered {
            Stroke::new(1.0, Color32::from_rgb(0xC8, 0xD5, 0xF4))
        } else {
            Stroke::new(1.0, BORDER)
        };
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(12), fill);
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(12),
            stroke,
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(
            egui::pos2(
                rect.center().x - galley.size().x / 2.0,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            text_color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn text_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Label::new(RichText::new(label).size(11.5).color(ACCENT))
            .sense(egui::Sense::click()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn status_pill(
    ui: &mut egui::Ui,
    label: &str,
    fg: Color32,
    bg: Color32,
    dot: Option<Color32>,
) -> egui::Response {
    let font = FontId::proportional(11.0);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, fg);
    let dot_space = if dot.is_some() { 9.0 } else { 0.0 };
    let size = egui::vec2(galley.size().x + 18.0 + dot_space, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(11), bg);
        let mut x = rect.left() + 9.0;
        if let Some(dot_color) = dot {
            ui.painter()
                .circle_filled(egui::pos2(x + 2.5, rect.center().y), 2.5, dot_color);
            x += 9.0;
        }
        ui.painter().galley(
            egui::pos2(x, rect.center().y - galley.size().y / 2.0),
            galley,
            fg,
        );
    }
    response
}

fn badge(ui: &mut egui::Ui, label: &str, fg: Color32, bg: Color32) -> egui::Response {
    status_pill(ui, label, fg, bg, None)
}

fn primary_button(
    ui: &mut egui::Ui,
    label: &str,
    width: f32,
    enabled: bool,
) -> egui::Response {
    let size = egui::vec2(width, 32.0);
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let hovered = enabled && response.hovered();
        let fill = if !enabled {
            DISABLED_BG
        } else if hovered {
            ACCENT_DARK
        } else {
            ACCENT
        };
        let text_color = if enabled {
            Color32::WHITE
        } else {
            MUTED
        };
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(8), fill);
        let galley = ui.painter().layout_no_wrap(
            label.to_owned(),
            FontId::proportional(12.5),
            text_color,
        );
        ui.painter().galley(
            egui::pos2(
                rect.center().x - galley.size().x / 2.0,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            text_color,
        );
    }
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

fn secondary_button(ui: &mut egui::Ui, label: &str, width: f32) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(12.5))
            .min_size(egui::vec2(width, 32.0))
            .corner_radius(egui::CornerRadius::same(8)),
    )
}

fn notice(ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) {
    egui::Frame::new()
        .fill(bg)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("!").size(12.0).strong().color(fg));
                ui.add(egui::Label::new(RichText::new(text).size(11.5).color(fg)).wrap());
            });
        });
}

fn hook_status_pill(ui: &mut egui::Ui, handle: &hook::HookHandle) {
    let enabled = handle.enabled.load(Ordering::Relaxed);
    let status = handle.status.load(Ordering::Relaxed);
    if !enabled {
        status_pill(ui, "中键已暂停", MUTED, SURFACE_SOFT, Some(FAINT));
    } else if status == hook::STATUS_RUNNING {
        status_pill(ui, "中键监听中", SUCCESS, SUCCESS_SOFT, Some(SUCCESS))
            .on_hover_text("在资源管理器的文件夹窗口内点击鼠标中键即可载入");
    } else if status == hook::STATUS_FAILED {
        let error = handle
            .error
            .lock()
            .ok()
            .and_then(|error| error.clone())
            .unwrap_or_else(|| "未知错误".to_owned());
        status_pill(ui, "中键监听失败", DANGER, DANGER_SOFT, Some(DANGER))
            .on_hover_text(error);
    } else {
        status_pill(ui, "中键监听启动中", MUTED, SURFACE_SOFT, Some(FAINT));
    }
}

fn table_cell(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    text: RichText,
    truncate: bool,
) {
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_width(width);
            let label = egui::Label::new(text);
            ui.add(if truncate { label.truncate() } else { label });
        },
    );
}

fn preview_row(
    ui: &mut egui::Ui,
    row: &PlannedRename,
    index: usize,
    name_width: f32,
    arrow_width: f32,
    status_width: f32,
    spacing: f32,
) {
    let kind = if row.is_dir { " · 文件夹" } else { "" };
    let (new_text, new_color) = if row.error.is_some() {
        (row.new_name.as_str(), DANGER)
    } else if row.changed {
        (row.new_name.as_str(), ACCENT_DARK)
    } else {
        ("（不变）", FAINT)
    };
    let (state_text, state_color) = if let Some(error) = &row.error {
        (format!("{error}{kind}"), DANGER)
    } else if row.changed {
        (format!("● 将更名{kind}"), SUCCESS)
    } else {
        (format!("● 名称不变{kind}"), FAINT)
    };
    let old_color = if row.changed || row.error.is_some() {
        TEXT
    } else {
        FAINT
    };

    let output = egui::Frame::new()
        .fill(if index % 2 == 0 { CARD } else { ZEBRA })
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = spacing;
                table_cell(
                    ui,
                    name_width,
                    22.0,
                    RichText::new(&row.old_name)
                        .size(12.5)
                        .monospace()
                        .color(old_color),
                    true,
                );
                ui.add_sized(
                    [arrow_width, 22.0],
                    egui::Label::new(RichText::new("→").size(12.0).color(FAINT)),
                );
                table_cell(
                    ui,
                    name_width,
                    22.0,
                    RichText::new(new_text)
                        .size(12.5)
                        .monospace()
                        .color(new_color),
                    true,
                );
                table_cell(
                    ui,
                    status_width,
                    22.0,
                    RichText::new(&state_text).size(11.0).color(state_color),
                    true,
                );
            });
        });

    if output.response.hovered() {
        ui.painter().rect_stroke(
            output.response.rect,
            egui::CornerRadius::same(6),
            Stroke::new(1.0, ACCENT_SOFT_HOVER),
            egui::StrokeKind::Inside,
        );
    }
    output.response.on_hover_text(format!(
        "{}{}\n→ {}\n{}",
        if row.is_dir { "[文件夹] " } else { "" },
        row.old_name,
        row.new_name,
        row.error.as_deref().unwrap_or(if row.changed {
            "将更名"
        } else {
            "名称不变"
        })
    ));
}

fn install_chinese_fonts(ctx: &egui::Context) {
    for path in [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\Deng.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
    ] {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let mut fonts = FontDefinitions::default();
        fonts
            .font_data
            .insert("cjk".to_owned(), std::sync::Arc::new(FontData::from_owned(bytes)));
        if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
            family.push("cjk".to_owned());
        }
        if let Some(family) = fonts.families.get_mut(&FontFamily::Monospace) {
            family.push("cjk".to_owned());
        }
        ctx.set_fonts(fonts);
        return;
    }
    eprintln!("warning: no Chinese font found; the UI may not render Chinese text");
}
