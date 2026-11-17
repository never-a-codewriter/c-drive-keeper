#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex};

use crate::core::config::AppConfig;
use crate::core::history::{HistoryManager, MigrationRecord};
use crate::core::i18n::{self, Language};
use crate::core::logger;
use crate::core::mover::Mover;
use crate::core::proc_mgr::ProcMgr;
use crate::core::scanner::{ScanResult, Scanner};

mod core;
mod cleanup;

// Actual implementation with channels
enum AppEvent {
    // Scan events
    ScanProgress(usize, usize, String), // current, total, folder name
    ScanComplete(Vec<ScanResult>),
    ScanError(String),

    // Move events
    MoveStart(u64),               // Total bytes to move
    MoveProgressBytes(u64),       // Bytes moved in this chunk (incremental)
    MoveTaskComplete(usize, u64), // Index of task completed, left behind bytes
    MoveComplete,
    MoveError(String),

    // Restore events
    RestoreStart(Vec<MigrationRecord>), // Batch being restored
    RestoreProgressBytes(u64),          // Bytes restored in this chunk
    RestoreComplete,                    // Restore finished
    RestoreError(String),
}

struct CompletionStats {
    moved_folders: Vec<String>,
    moved_count: usize,
    freed_space: u64,
    duration: Duration,
}

#[derive(PartialEq)]
enum Tab {
    ScanAndMove,
    Cleanup,
    History,
}

pub struct App {
    cleanup_panel: cleanup::CleanupPanel,
    rx: std::sync::mpsc::Receiver<AppEvent>,
    tx: std::sync::mpsc::Sender<AppEvent>,

    config: AppConfig,
    scan_results: Vec<ScanResult>,
    selected_items: std::collections::HashSet<usize>,
    completed_tasks: std::collections::HashSet<usize>, // New: Track completed tasks
    current_move_indices: Vec<usize>,
    is_scanning: bool,

    current_tab: Tab,
    history_records: Vec<MigrationRecord>,

    target_root: PathBuf,

    is_processing: bool,
    is_paused: bool,
    processing_type: ProcessingType,

    status_msg: String,
    last_error: String,
    move_current_task: String,
    move_error_count: u32,

    // Scan Progress
    scan_current: usize,
    scan_total: usize,
    scan_current_item: String,

    // Move Progress
    move_total_bytes: u64,
    move_current_bytes: u64,
    move_actual_freed_bytes: u64,
    move_start_time: Option<Instant>,
    move_speed_bps: f64,
    move_remaining_secs: u64,
    pause_signal: Arc<AtomicBool>,

    auto_kill: bool,
    parallel_copy: bool,
    parallelism: usize,
    lang: Language,

    show_completion_dialog: bool,
    completion_stats: Option<CompletionStats>,
}

#[derive(PartialEq)]
enum ProcessingType {
    None,
    Scanning,
    Moving,
    Restoring,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_custom_fonts(&cc.egui_ctx);

        let config = AppConfig::load_or_create().unwrap_or_else(|_| AppConfig::default_config());
        let (tx, rx) = std::sync::mpsc::channel();
        Self {
            rx,
            tx,
            config: config.clone(),
            cleanup_panel: cleanup::CleanupPanel::default(),
            scan_results: Vec::new(),
            selected_items: std::collections::HashSet::new(),
            completed_tasks: std::collections::HashSet::new(),
            current_move_indices: Vec::new(),
            is_scanning: false,
            current_tab: Tab::ScanAndMove,
            history_records: HistoryManager::load_records(),
            target_root: config.target_root.clone(),
            is_processing: false,
            is_paused: false,
            processing_type: ProcessingType::None,
            status_msg: i18n::t("准备就绪"),
            last_error: String::new(),
            move_current_task: String::new(),
            move_error_count: 0,

            scan_current: 0,
            scan_total: 0,
            scan_current_item: String::new(),

            move_total_bytes: 0,
            move_current_bytes: 0,
            move_actual_freed_bytes: 0,
            move_start_time: None,
            move_speed_bps: 0.0,
            move_remaining_secs: 0,
            pause_signal: Arc::new(AtomicBool::new(false)),

            auto_kill: false,
            parallel_copy: true,
            parallelism: 6,
            lang: Language::Chinese,

            show_completion_dialog: false,
            completion_stats: None,
        }
    }

    fn persist_config(&mut self) {
        self.config.target_root = self.target_root.clone();
        let _ = self.config.save();
    }

    fn format_bytes(bytes: u64) -> String {
        const KB: u64 = 1024;
        const MB: u64 = KB * 1024;
        const GB: u64 = MB * 1024;

        if bytes >= GB {
            format!("{:.2} GB", bytes as f64 / GB as f64)
        } else if bytes >= MB {
            format!("{:.2} MB", bytes as f64 / MB as f64)
        } else if bytes >= KB {
            format!("{:.2} KB", bytes as f64 / KB as f64)
        } else {
            format!("{} B", bytes)
        }
    }

    fn format_duration(secs: u64) -> String {
        if secs < 60 {
            format!("{}s", secs)
        } else if secs < 3600 {
            format!("{}m {}s", secs / 60, secs % 60)
        } else {
            format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
        }
    }

    fn toggle_pause(&mut self) {
        if self.processing_type == ProcessingType::Moving {
            self.is_paused = !self.is_paused;
            self.pause_signal.store(self.is_paused, Ordering::SeqCst);
        }
    }

    fn show_history_tab(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        if self.history_records.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(i18n::t("暂无迁移记录"));
            });
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut restore_batch: Option<Vec<MigrationRecord>> = None;
            let mut delete_record_id: Option<String> = None;

            // Group by batch_id
            let mut groups: std::collections::BTreeMap<String, Vec<MigrationRecord>> =
                std::collections::BTreeMap::new();
            for r in &self.history_records {
                let key = r.batch_id.clone().unwrap_or_else(|| r.id.clone());
                groups.entry(key).or_default().push(r.clone());
            }

            // Sort groups by time (descending)
            let mut sorted_groups: Vec<(String, Vec<MigrationRecord>)> =
                groups.into_iter().collect();
            sorted_groups.sort_by(|a, b| {
                let t_a = a.1.first().map(|r| r.timestamp).unwrap_or(0);
                let t_b = b.1.first().map(|r| r.timestamp).unwrap_or(0);
                t_b.cmp(&t_a) // Newest first
            });

            for (_batch_key, records) in sorted_groups {
                ui.group(|ui| {
                    let total_size: u64 = records.iter().map(|r| r.size_bytes).sum();
                    let first_r = records.first().unwrap();
                    let timestamp = first_r.batch_timestamp.unwrap_or(first_r.timestamp);
                    let time = UNIX_EPOCH + Duration::from_secs(timestamp);
                    let datetime: chrono::DateTime<chrono::Local> = time.into();

                    ui.horizontal(|ui| {
                        if records.len() > 1 {
                            ui.heading(format!(
                                "{} ({} 项)",
                                i18n::t("多个迁移批次"),
                                records.len()
                            ));
                        } else {
                            ui.heading(&first_r.name);
                        }
                        ui.label(format!("({})", Self::format_bytes(total_size)));
                        ui.label(datetime.format("%Y-%m-%d %H:%M:%S").to_string());

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_enabled(
                                    !self.is_processing,
                                    egui::Button::new(i18n::t("一键还原本批次")),
                                )
                                .clicked()
                            {
                                restore_batch = Some(records.clone());
                            }
                        });
                    });

                    for record in &records {
                        ui.horizontal(|ui| {
                            ui.label(format!("📁 {}", record.name));
                            ui.label(format!("{} {:?}", i18n::t("从:"), record.source_path));
                            ui.label(format!("{} {:?}", i18n::t("到:"), record.target_path));
                            if ui
                                .add_enabled(
                                    !self.is_processing,
                                    egui::Button::new(i18n::t("清除记录")),
                                )
                                .on_hover_text(i18n::t("强制从列表中移除该记录（不还原文件）"))
                                .clicked()
                            {
                                delete_record_id = Some(record.id.clone());
                            }
                        });
                    }
                });
            }

            if let Some(id) = delete_record_id {
                HistoryManager::remove_record(&id);
                self.history_records = HistoryManager::load_records();
            }

            if let Some(batch) = restore_batch {
                self.start_restore_batch(batch, ctx);
            }
        });
    }

    fn start_restore_batch(&mut self, batch: Vec<MigrationRecord>, ctx: &egui::Context) {
        self.is_processing = true;
        self.processing_type = ProcessingType::Restoring;
        self.is_paused = false;
        self.pause_signal.store(false, Ordering::SeqCst);

        let tx = self.tx.clone();
        let ctx_clone = ctx.clone();
        let ctx_final_clone = ctx.clone();
        let pause_signal = self.pause_signal.clone();
        let parallelism = if self.parallel_copy {
            self.parallelism.max(1)
        } else {
            1
        };
        let auto_kill = self.auto_kill;

        thread::spawn(move || {
            tx.send(AppEvent::RestoreStart(batch.clone())).unwrap();

            for record in batch {
                let tx_clone = tx.clone();
                let ctx_inner = ctx_clone.clone();
                let progress_cb = move |bytes_delta| {
                    let _ = tx_clone.send(AppEvent::RestoreProgressBytes(bytes_delta));
                    ctx_inner.request_repaint();
                };

                while pause_signal.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(100));
                }

                match Mover::restore_migration(
                    &record,
                    progress_cb,
                    pause_signal.clone(),
                    parallelism,
                    auto_kill,
                ) {
                    Ok(_) => {}
                    Err(e) => {
                        tx.send(AppEvent::RestoreError(format!(
                            "未能还原全部分支，因为 {} 失败: {}",
                            record.name, e
                        )))
                        .unwrap();
                        ctx_final_clone.request_repaint();
                        return;
                    }
                }
            }
            tx.send(AppEvent::RestoreComplete).unwrap();
            ctx_final_clone.request_repaint();
        });
    }
}

fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    let font_path = "C:\\Windows\\Fonts\\msyh.ttc";
    let font_path_alt = "C:\\Windows\\Fonts\\simhei.ttf";

    let font_data = if std::path::Path::new(font_path).exists() {
        std::fs::read(font_path).ok()
    } else if std::path::Path::new(font_path_alt).exists() {
        std::fs::read(font_path_alt).ok()
    } else {
        None
    };

    if let Some(data) = font_data {
        fonts
            .font_data
            .insert("my_font".to_owned(), egui::FontData::from_owned(data));
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "my_font".to_owned());
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .insert(0, "my_font".to_owned());
        ctx.set_fonts(fonts);
    }
}

fn compute_target_parent(base_target: &Path, source_folder: &Path) -> PathBuf {
    let parent = source_folder.parent().unwrap_or(source_folder);
    let mut rel = PathBuf::new();
    for comp in parent.components() {
        match comp {
            Component::Prefix(_) | Component::RootDir => {}
            _ => rel.push(comp.as_os_str()),
        }
    }
    base_target.join(rel)
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Handle events
        while let Ok(event) = self.rx.try_recv() {
            match event {
                // --- SCAN EVENTS ---
                AppEvent::ScanProgress(current, total, name) => {
                    self.scan_current = current;
                    self.scan_total = total;
                    self.scan_current_item = name;

                    if self.lang == Language::English {
                        self.status_msg = format!(
                            "Scanning: {} ({}/{})",
                            self.scan_current_item, current, total
                        );
                    } else {
                        self.status_msg = format!(
                            "正在扫描: {} ({}/{})",
                            self.scan_current_item, current, total
                        );
                    }
                }
                AppEvent::ScanComplete(res) => {
                    self.is_scanning = false;
                    if self.processing_type == ProcessingType::Scanning {
                        self.is_processing = false;
                        self.processing_type = ProcessingType::None;
                    }
                    self.scan_results = res;
                    self.completed_tasks.clear(); // Reset completed tasks on new scan

                    if self.lang == Language::English {
                        self.status_msg = format!(
                            "Scan complete, found {} large folders",
                            self.scan_results.len()
                        );
                    } else {
                        self.status_msg =
                            format!("扫描完成，共找到 {} 个大文件夹", self.scan_results.len());
                    }
                }
                AppEvent::ScanError(e) => {
                    self.is_scanning = false;
                    if self.processing_type == ProcessingType::Scanning {
                        self.is_processing = false;
                        self.processing_type = ProcessingType::None;
                    }
                    self.status_msg =
                        format!("{}: {}", i18n::t("扫描出错: {}").replace("{}", ""), e);
                }

                // --- MOVE EVENTS ---
                AppEvent::MoveStart(total_bytes) => {
                    self.move_total_bytes = total_bytes;
                    self.move_current_bytes = 0;
                    self.move_actual_freed_bytes = 0;
                    self.move_start_time = Some(Instant::now());
                    self.is_paused = false;
                    self.pause_signal.store(false, Ordering::SeqCst);
                    self.last_error.clear();
                    self.move_current_task.clear();
                    self.move_error_count = 0;
                    self.move_speed_bps = 0.0;
                    self.move_remaining_secs = 0;
                    self.show_completion_dialog = false;
                    self.completion_stats = None;

                    if self.lang == Language::English {
                        self.status_msg = format!(
                            "Preparing to move... Total size: {}",
                            Self::format_bytes(total_bytes)
                        );
                    } else {
                        self.status_msg =
                            format!("准备迁移... 总大小: {}", Self::format_bytes(total_bytes));
                    }
                }
                AppEvent::MoveProgressBytes(bytes_delta) => {
                    self.move_current_bytes += bytes_delta;

                    // Calculate speed & ETA
                    if let Some(start_time) = self.move_start_time {
                        if !self.is_paused {
                            let elapsed = start_time.elapsed().as_secs_f64();
                            if elapsed > 0.5 {
                                self.move_speed_bps = self.move_current_bytes as f64 / elapsed;
                                if self.move_speed_bps > 0.0 {
                                    let remaining_bytes = self
                                        .move_total_bytes
                                        .saturating_sub(self.move_current_bytes);
                                    self.move_remaining_secs =
                                        (remaining_bytes as f64 / self.move_speed_bps) as u64;
                                }
                            }
                        }
                    }
                }
                AppEvent::MoveTaskComplete(idx, left_behind) => {
                    self.completed_tasks.insert(idx);
                    if let Some(res) = self.scan_results.get(idx) {
                        self.move_actual_freed_bytes += res.size_bytes.saturating_sub(left_behind);
                    }
                }
                AppEvent::MoveComplete => {
                    self.is_processing = false;
                    self.processing_type = ProcessingType::None;
                    self.is_paused = false;

                    // Reload history manually so ui updates
                    self.history_records = HistoryManager::load_records();

                    let mut moved_folders = Vec::new();
                    for idx in &self.current_move_indices {
                        if let Some(res) = self.scan_results.get(*idx) {
                            moved_folders.push(res.path.to_string_lossy().to_string());
                        }
                    }
                    let duration = self
                        .move_start_time
                        .map(|t| t.elapsed())
                        .unwrap_or_default();

                    self.completion_stats = Some(CompletionStats {
                        moved_folders,
                        moved_count: self.current_move_indices.len(),
                        freed_space: self.move_actual_freed_bytes,
                        duration,
                    });
                    self.show_completion_dialog = true;

                    if self.move_error_count > 0 {
                        let log_path = logger::log_file_path_string();
                        if self.lang == Language::English {
                            self.status_msg = format!(
                                "Finished with {} error(s). Log: {}",
                                self.move_error_count, log_path
                            );
                        } else {
                            self.status_msg = format!(
                                "迁移完成，但有 {} 个错误。日志: {}",
                                self.move_error_count, log_path
                            );
                        }
                    } else {
                        self.status_msg = i18n::t("所有迁移任务完成");
                    }
                    self.move_current_bytes = self.move_total_bytes;
                }
                AppEvent::MoveError(e) => {
                    self.last_error = e.clone();
                    self.move_error_count = self.move_error_count.saturating_add(1);
                    logger::log(&format!("move error: {}", e));
                    self.status_msg = format!("{}: {}", i18n::t("错误: {}").replace("{}", ""), e);
                }

                // --- RESTORE EVENTS ---
                AppEvent::RestoreStart(batch) => {
                    self.move_current_bytes = 0;
                    self.move_total_bytes = batch.iter().map(|r| r.size_bytes).sum();
                    self.move_start_time = Some(Instant::now());
                    self.is_paused = false;
                    self.pause_signal.store(false, Ordering::SeqCst);
                    self.last_error.clear();
                    self.move_error_count = 0;
                    self.move_speed_bps = 0.0;
                    self.move_remaining_secs = 0;

                    if self.lang == Language::English {
                        self.status_msg = "Restoring...".to_string();
                    } else {
                        self.status_msg = "正在还原...".to_string();
                    }
                }
                AppEvent::RestoreProgressBytes(bytes_delta) => {
                    self.move_current_bytes += bytes_delta;

                    // Calculate speed & ETA
                    if let Some(start_time) = self.move_start_time {
                        if !self.is_paused {
                            let elapsed = start_time.elapsed().as_secs_f64();
                            if elapsed > 0.5 {
                                self.move_speed_bps = self.move_current_bytes as f64 / elapsed;
                                if self.move_speed_bps > 0.0 {
                                    let remaining_bytes = self
                                        .move_total_bytes
                                        .saturating_sub(self.move_current_bytes);
                                    self.move_remaining_secs =
                                        (remaining_bytes as f64 / self.move_speed_bps) as u64;
                                }
                            }
                        }
                    }
                }
                AppEvent::RestoreComplete => {
                    self.is_processing = false;
                    self.processing_type = ProcessingType::None;
                    self.is_paused = false;

                    // Reload history
                    self.history_records = HistoryManager::load_records();

                    if self.lang == Language::English {
                        self.status_msg = "Restore completed successfully.".to_string();
                    } else {
                        self.status_msg = "批次还原成功。".to_string();
                    }
                }
                AppEvent::RestoreError(e) => {
                    self.is_processing = false;
                    self.processing_type = ProcessingType::None;
                    self.is_paused = false;
                    self.last_error = e.clone();
                    logger::log(&format!("restore error: {}", e));
                    self.status_msg =
                        format!("{}: {}", i18n::t("还原失败: {}").replace("{}", ""), e);
                }
            }
        }

        // Force repaint if moving or restoring to update animation/progress bar smoothly
        if self.processing_type == ProcessingType::Moving
            || self.processing_type == ProcessingType::Restoring
        {
            ctx.request_repaint();
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.current_tab, Tab::ScanAndMove, i18n::t("扫描与迁移"));
                ui.selectable_value(&mut self.current_tab, Tab::History, i18n::t("迁移记录"));
            });
            ui.add_enabled_ui(!self.is_processing && !self.is_scanning, |ui| {
                ui.selectable_value(&mut self.current_tab, Tab::Cleanup, "缓存清理");
            });
            ui.separator();
            if self.current_tab == Tab::Cleanup || self.cleanup_panel.busy() {
                self.cleanup_panel.show(ui, &self.target_root, self.is_processing || self.is_scanning);
                return;
            }

            if self.current_tab == Tab::History {
                self.show_history_tab(ctx, ui);
                return;
            }

            ui.horizontal(|ui| {
                let btn_size = egui::vec2(130.0, 36.0);
                let txt_size = 18.0;

                let scan_btn_enabled = !self.is_scanning
                    && (!self.is_processing
                        || (self.processing_type == ProcessingType::Moving && self.is_paused));
                if ui
                    .add_enabled(
                        scan_btn_enabled,
                        egui::Button::new(
                            egui::RichText::new(i18n::t("扫描大文件夹")).size(txt_size),
                        )
                        .min_size(btn_size),
                    )
                    .clicked()
                {
                    self.is_scanning = true;
                    if self.processing_type != ProcessingType::Moving {
                        self.is_processing = true;
                        self.processing_type = ProcessingType::Scanning;
                        self.scan_results.clear();
                        self.selected_items.clear();
                        self.completed_tasks.clear();
                    }

                    self.status_msg = i18n::t("正在初始化扫描...");
                    self.scan_current = 0;
                    self.scan_total = 0;

                    let tx = self.tx.clone();
                    let ctx = ctx.clone();
                    let scan_sources = self.config.scan_sources.clone();
                    thread::spawn(move || {
                        let tx_clone = tx.clone();
                        let cb = move |current, total, name| {
                            let _ = tx_clone.send(AppEvent::ScanProgress(current, total, name));
                            ctx.request_repaint();
                        };

                        match Scanner::scan_large_folders(&scan_sources, cb) {
                            Ok(res) => {
                                let _ = tx.send(AppEvent::ScanComplete(res));
                            }
                            Err(e) => {
                                let _ = tx.send(AppEvent::ScanError(e.to_string()));
                            }
                        }
                    });
                }

                // Combined Move/Pause/Resume Button
                let move_btn_text =
                    if self.is_processing && self.processing_type == ProcessingType::Moving {
                        if self.is_paused {
                            i18n::t("继续")
                        } else {
                            i18n::t("暂停")
                        }
                    } else {
                        i18n::t("执行迁移")
                    };

                // Enable button if not processing OR if processing type is Moving (to allow pause/resume)
                let btn_enabled =
                    !self.is_processing || self.processing_type == ProcessingType::Moving;

                if ui
                    .add_enabled(
                        btn_enabled,
                        egui::Button::new(egui::RichText::new(move_btn_text).size(txt_size))
                            .min_size(btn_size),
                    )
                    .clicked()
                {
                    if self.is_processing && self.processing_type == ProcessingType::Moving {
                        // Handle Pause/Resume
                        self.toggle_pause();
                    } else {
                        // Handle Start Move
                        // Clone tasks
                        let tasks: Vec<(usize, ScanResult)> = self
                            .scan_results
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| self.selected_items.contains(i))
                            .map(|(i, r)| (i, r.clone()))
                            .collect();

                        if tasks.is_empty() {
                            self.status_msg = i18n::t("请先勾选需要迁移的文件夹");
                        } else {
                            self.is_processing = true;
                            self.processing_type = ProcessingType::Moving;
                            self.is_paused = false;
                            self.pause_signal.store(false, Ordering::SeqCst);
                            self.last_error.clear();
                            self.move_current_task.clear();
                            self.current_move_indices = tasks.iter().map(|(i, _)| *i).collect();

                            let tx = self.tx.clone();
                            let ctx = ctx.clone();
                            let target_base = self.target_root.clone();
                            let auto_kill = self.auto_kill;
                            let pause_signal = self.pause_signal.clone();
                            let parallelism = if self.parallel_copy {
                                self.parallelism.max(1)
                            } else {
                                1
                            };

                            // Calculate total size first
                            let total_bytes: u64 = tasks.iter().map(|(_, t)| t.size_bytes).sum();
                            let batch_id = uuid::Uuid::new_v4().to_string();
                            let batch_timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();

                            thread::spawn(move || {
                                tx.send(AppEvent::MoveStart(total_bytes)).unwrap();

                                let mut roots_to_check: std::collections::HashSet<PathBuf> =
                                    std::collections::HashSet::new();
                                for (_, task) in &tasks {
                                    roots_to_check.insert(compute_target_parent(
                                        &target_base,
                                        &task.path,
                                    ));
                                }
                                for root in roots_to_check {
                                    if root.exists() && !root.is_dir() {
                                        let _ = tx.send(AppEvent::MoveError(format!(
                                            "目标根目录不是文件夹：{:?}\n建议：选择一个新的目标根目录（例如 D:\\WindowsClear\\AppData），或删除/更名该路径后重试",
                                            root
                                        )));
                                        tx.send(AppEvent::MoveComplete).unwrap();
                                        ctx.request_repaint();
                                        return;
                                    }
                                    if !root.exists() {
                                        if let Err(e) = std::fs::create_dir_all(&root) {
                                            let code = e.raw_os_error().unwrap_or(0);
                                            let _ = tx.send(AppEvent::MoveError(format!(
                                                "无法创建目标根目录：{:?}\n系统错误：{} (os error {})\n建议：确认目标盘符存在且可写；尝试手动创建该目录；检查安全软件/受控文件夹访问拦截；建议使用 D:\\WindowsClear\\AppData 作为目标根目录",
                                                root,
                                                e,
                                                code
                                            )));
                                            tx.send(AppEvent::MoveComplete).unwrap();
                                            ctx.request_repaint();
                                            return;
                                        }
                                    }
                                }

                                for (idx, task) in tasks {
                                    let target_root = compute_target_parent(&target_base, &task.path);

                                    if auto_kill {
                                        if let Ok(pids) =
                                            ProcMgr::check_locking_processes_dir(&task.path)
                                        {
                                            if !pids.is_empty() {
                                                for pid in pids {
                                                    let _ = ProcMgr::kill_process(pid);
                                                }
                                            }
                                        }
                                    }

                                    let tx_clone = tx.clone();
                                    let progress_cb = move |bytes_delta| {
                                        let _ =
                                            tx_clone.send(AppEvent::MoveProgressBytes(bytes_delta));
                                    };

                                    match Mover::move_and_link(
                                        &task.path,
                                        &target_root,
                                        progress_cb,
                                        pause_signal.clone(),
                                        parallelism,
                                        auto_kill,
                                        batch_id.clone(),
                                        batch_timestamp,
                                    ) {
                                        Ok((lb, _record)) => {
                                            tx.send(AppEvent::MoveTaskComplete(idx, lb)).unwrap();
                                        }
                                        Err(e) => {
                                            tx.send(AppEvent::MoveError(format!(
                                                "{} 失败: {}",
                                                task.name, e
                                            )))
                                            .unwrap();
                                        }
                                    }
                                }
                                tx.send(AppEvent::MoveComplete).unwrap();
                                ctx.request_repaint();
                            });
                        }
                    }
                }

                if ui
                    .add_sized(
                        btn_size,
                        egui::Button::new(egui::RichText::new(i18n::t("打开日志")).size(txt_size)),
                    )
                    .clicked()
                {
                    let log_path = logger::log_file_path_string();
                    if !log_path.is_empty() {
                        let _ = std::process::Command::new("explorer")
                            .arg("/select,")
                            .arg(log_path)
                            .spawn();
                    }
                }

                let before = self.lang;
                egui::ComboBox::from_id_source("lang_select")
                    .selected_text(
                        egui::RichText::new(match self.lang {
                            Language::Chinese => "中文",
                            Language::English => "English",
                        })
                        .size(txt_size),
                    )
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.lang, Language::Chinese, "中文");
                        ui.selectable_value(&mut self.lang, Language::English, "English");
                    });
                if self.lang != before {
                    i18n::get_i18n().set_lang(self.lang);
                    if !self.is_processing {
                        self.status_msg = i18n::t("准备就绪");
                    }
                }
            });

            ui.label(i18n::t(
                "使用步骤：1 扫描大文件夹 → 2 勾选目录 → 3 执行迁移",
            ));

            ui.separator();

            ui.horizontal(|ui| {
                ui.label(i18n::t("目标根目录:"));
                let path_str = self.target_root.to_string_lossy();
                ui.label(path_str);
                if ui
                    .add_enabled(!self.is_processing, egui::Button::new(i18n::t("选择...")))
                    .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.target_root = path;
                        self.persist_config();
                    }
                }
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label(i18n::t("扫描目录:"));
                if ui
                    .add_enabled(!self.is_processing, egui::Button::new(i18n::t("添加...")))
                    .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.config.add_custom_scan_dir(&path);
                        let _ = self.config.save();
                    }
                }
            });
            let mut scan_sources_changed = false;
            egui::ScrollArea::vertical()
                .max_height(120.0)
                .show(ui, |ui| {
                    let mut remove_idx: Option<usize> = None;
                    for (idx, s) in self.config.scan_sources.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut s.enabled, "").changed() {
                                scan_sources_changed = true;
                            }
                            ui.label(&s.label);
                            ui.weak(s.path.to_string_lossy());
                            if ui
                                .add_enabled(!self.is_processing, egui::Button::new("×"))
                                .clicked()
                            {
                                remove_idx = Some(idx);
                            }
                        });
                    }
                    if let Some(idx) = remove_idx {
                        self.config.scan_sources.remove(idx);
                        scan_sources_changed = true;
                    }
                });
            if scan_sources_changed {
                let _ = self.config.save();
            }

            ui.checkbox(&mut self.auto_kill, i18n::t("自动结束占用进程 (慎用)"));
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.parallel_copy, i18n::t("并发传输"));
                ui.label("？")
                    .on_hover_text(i18n::t("建议 SSD 硬盘使用，机械硬盘可能变慢或卡顿。"));
                if self.parallel_copy {
                    ui.add(egui::Slider::new(&mut self.parallelism, 2..=16).text(""));
                }
            });

            ui.separator();

            if self.is_processing {
                ui.vertical(|ui| {
                    if self.processing_type == ProcessingType::Scanning {
                        // Scan Progress Bar
                        let progress = if self.scan_total > 0 {
                            self.scan_current as f32 / self.scan_total as f32
                        } else {
                            0.0
                        };
                        ui.add(
                            egui::ProgressBar::new(progress)
                                .text(format!("{}/{}", self.scan_current, self.scan_total)),
                        );

                        if self.lang == Language::English {
                            ui.label(format!("Analyzing: {}", self.scan_current_item));
                        } else {
                            ui.label(format!("正在分析: {}", self.scan_current_item));
                        }
                    } else if self.processing_type == ProcessingType::Moving {
                        // Move Progress Bar
                        let progress = if self.move_total_bytes > 0 {
                            self.move_current_bytes as f32 / self.move_total_bytes as f32
                        } else {
                            0.0
                        };
                        ui.add(egui::ProgressBar::new(progress).show_percentage());
                        ui.horizontal(|ui| {
                            ui.label(format!(
                                "{}/{}",
                                Self::format_bytes(self.move_current_bytes),
                                Self::format_bytes(self.move_total_bytes)
                            ));

                            if self.lang == Language::English {
                                ui.label(format!(
                                    "Speed: {}/s",
                                    Self::format_bytes(self.move_speed_bps as u64)
                                ));
                                ui.label(format!(
                                    "ETA: {}",
                                    Self::format_duration(self.move_remaining_secs)
                                ));
                            } else {
                                ui.label(format!(
                                    "速度: {}/s",
                                    Self::format_bytes(self.move_speed_bps as u64)
                                ));
                                ui.label(format!(
                                    "剩余: {}",
                                    Self::format_duration(self.move_remaining_secs)
                                ));
                            }
                        });
                    }
                });
            } else {
                ui.label(&self.status_msg);
            }

            ui.separator();

            // List
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (i, res) in self.scan_results.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let mut selected = self.selected_items.contains(&i);
                        if ui
                            .add_enabled(
                                !self.is_processing,
                                egui::Checkbox::new(&mut selected, ""),
                            )
                            .changed()
                        {
                            if selected {
                                self.selected_items.insert(i);
                            } else {
                                self.selected_items.remove(&i);
                            }
                        }

                        if ui
                            .link(format!(
                                "{} ({})",
                                res.name,
                                Self::format_bytes(res.size_bytes)
                            ))
                            .clicked()
                        {
                            let _ = std::process::Command::new("explorer")
                                .arg(res.path.to_string_lossy().to_string())
                                .spawn();
                        }
                        ui.weak(format!("[{}]", res.label));
                        if ui.link(res.path.to_string_lossy().to_string()).clicked() {
                            let _ = std::process::Command::new("explorer")
                                .arg(res.path.to_string_lossy().to_string())
                                .spawn();
                        }

                        // Show checkmark if completed
                        if self.completed_tasks.contains(&i) {
                            ui.label("✅");
                        }
                    });
                }
            });
        });

        if self.show_completion_dialog {
            egui::Window::new(if self.lang == Language::English {
                "Migration Completed"
            } else {
                "迁移成功"
            })
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                if let Some(stats) = &self.completion_stats {
                    ui.label(format!(
                        "{} {}",
                        if self.lang == Language::English {
                            "Moved folders:"
                        } else {
                            "成功移动"
                        },
                        stats.moved_count
                    ));
                    ui.label(format!(
                        "{} {}",
                        if self.lang == Language::English {
                            "Freed space:"
                        } else {
                            "释放了"
                        },
                        Self::format_bytes(stats.freed_space)
                    ));
                    ui.label(format!(
                        "{} {}",
                        if self.lang == Language::English {
                            "Duration:"
                        } else {
                            "耗时:"
                        },
                        Self::format_duration(stats.duration.as_secs())
                    ));

                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(140.0)
                        .show(ui, |ui| {
                            let max_show = 50usize;
                            for (i, p) in stats.moved_folders.iter().take(max_show).enumerate() {
                                ui.label(format!("{}. {}", i + 1, p));
                            }
                            if stats.moved_folders.len() > max_show {
                                ui.label(if self.lang == Language::English {
                                    format!("... and {} more", stats.moved_folders.len() - max_show)
                                } else {
                                    format!(
                                        "... 还有 {} 个未显示",
                                        stats.moved_folders.len() - max_show
                                    )
                                });
                            }
                        });

                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui
                            .button(if self.lang == Language::English {
                                "Save Log"
                            } else {
                                "保存日志"
                            })
                            .clicked()
                        {
                            let log_path = logger::log_file_path_string();
                            if !log_path.is_empty() {
                                let log_file = PathBuf::from(&log_path);
                                if log_file.exists() {
                                    if let Ok(exe_path) = std::env::current_exe() {
                                        if let Some(dir) = exe_path.parent() {
                                            let target_path = dir.join(format!(
                                                "migration_log_{}.txt",
                                                chrono::Local::now().format("%Y%m%d_%H%M%S")
                                            ));
                                            if std::fs::copy(&log_file, &target_path).is_ok() {
                                                let _ = std::process::Command::new("explorer")
                                                    .arg("/select,")
                                                    .arg(target_path)
                                                    .spawn();
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if ui
                            .button(if self.lang == Language::English {
                                "Close"
                            } else {
                                "关闭"
                            })
                            .clicked()
                        {
                            self.show_completion_dialog = false;
                        }
                    });
                }
            });
        }
    }
}

// Single instance check
struct SingleInstance {
    handle: HANDLE,
}

impl SingleInstance {
    fn new(name: &str) -> Option<Self> {
        unsafe {
            let mut wide_name: Vec<u16> = name.encode_utf16().collect();
            wide_name.push(0);

            let handle = CreateMutexW(None, true, PCWSTR(wide_name.as_ptr()));

            // Check for error
            if let Ok(h) = handle {
                if h.is_invalid() {
                    return None;
                }

                if GetLastError() == ERROR_ALREADY_EXISTS {
                    let _ = CloseHandle(h);
                    return None;
                }

                Some(Self { handle: h })
            } else {
                None
            }
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            if !self.handle.is_invalid() {
                let _ = ReleaseMutex(self.handle);
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    logger::init();
    // Check single instance
    let _single_instance = match SingleInstance::new("Global\\AppDataMover_SingleInstance_Mutex") {
        Some(s) => s,
        None => {
            // Already running
            // We could show a message box here, but for now just exit or print
            // Since we are GUI, exiting silently or logging is okay.
            // But let's just return Ok.
            return Ok(());
        }
    };

    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "AppData Mover",
        native_options,
        Box::new(|cc| Box::new(App::new(cc))),
    )
}

