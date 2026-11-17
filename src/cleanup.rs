use eframe::egui;
use std::{path::PathBuf, process::{Child, Command}, os::windows::process::CommandExt};

#[derive(Default)]
pub struct CleanupPanel {
    child: Option<Child>,
    report: PathBuf,
    rows: Vec<serde_json::Value>,
    selected: std::collections::HashSet<String>,
    other_drives: bool,
    recycle: bool,
    confirm: bool,
    status: String,
}
impl CleanupPanel {
    fn start(&mut self, apply: bool, destination: &std::path::Path) -> anyhow::Result<()> {
        let run = destination.join("cleanup-reports").join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&run)?;
        std::fs::write(run.join("clean.ps1"), include_bytes!("../scripts/clean.ps1"))?;
        std::fs::write(run.join("cleanup-core.ps1"), include_bytes!("../scripts/cleanup-core.ps1"))?;
        let preview_path = self.report.clone();
        self.report = run.join("report.json");
        let log = std::fs::File::create(run.join("console.log"))?;
        let mut cmd = Command::new("powershell.exe");
        cmd.creation_flags(0x08000000).args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(run.join("clean.ps1")).arg("-ReportPath").arg(&self.report)
            .stdout(log.try_clone()?).stderr(log);
        if self.other_drives { cmd.arg("-IncludeOtherDrives"); }
        if self.recycle { cmd.arg("-IncludeRecycleBin"); }
        if apply {
            cmd.arg("-PreviewPath").arg(preview_path).arg("-Apply").arg("-Only").arg(self.selected.iter().cloned().collect::<Vec<_>>().join(","))
                .arg("-BackupRoot").arg(destination.join("cleanup-backups"));
        }
        self.child = Some(cmd.spawn()?);
        self.status = if apply { "正在备份、校验和清理" } else { "正在预览，不删除文件" }.into();
        self.confirm = false;
        Ok(())
    }
    pub fn busy(&self) -> bool { self.child.is_some() }
    pub fn show(&mut self, ui: &mut egui::Ui, destination: &std::path::Path, blocked: bool) {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(exit)) => {
                    self.child = None;
                    self.rows.clear();
                    match std::fs::read(&self.report).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&b)).ok()) {
                        Some(serde_json::Value::Array(rows)) => self.rows = rows,
                        Some(row) if row.get("Id").is_some() => self.rows.push(row),
                        _ => {}
                    }
                    self.selected.clear();
                    self.status = format!("{}；报告：{}", if exit.success() { "完成" } else { "未全部完成，请查看报告及 console.log" }, self.report.display());
                }
                Ok(None) => { ui.ctx().request_repaint_after(std::time::Duration::from_millis(200)); }
                Err(e) => { self.status = e.to_string(); }
            }
        }
        ui.heading("缓存清理 / Cache cleanup");
        ui.label(format!("报告和备份位置：{}（在迁移页选择目标根目录）", destination.display()));
        ui.label("默认保留最近 7 天文件。大小为逻辑字节；备份留在同一盘不会净释放该盘空间。");
        ui.add_enabled_ui(!self.busy() && !blocked, |ui| {
            let changed = ui.checkbox(&mut self.other_drives, "包括 D 盘等非系统盘开发缓存").changed()
                | ui.checkbox(&mut self.recycle, "同时清空所有盘回收站（不可恢复）").changed();
            if changed { self.rows.clear(); self.selected.clear(); self.confirm = false; }
            if ui.button("扫描 / 重新预览").clicked() {
                self.rows.clear(); self.selected.clear();
                if let Err(e) = self.start(false, destination) { self.status = e.to_string(); }
            }
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                for row in &self.rows {
                    let id = row["Id"].as_str().unwrap_or("").to_string();
                    if id.is_empty() || id == "recycle-bin" { continue; }
                    let eligible = row["EligibleBytes"].as_u64().unwrap_or(0);
                    let mut selected = self.selected.contains(&id);
                    if ui.checkbox(&mut selected, format!("{} — {:.1} MB — {}", id, eligible as f64 / 1048576.0, row["Status"].as_str().unwrap_or("Unknown"))).changed() {
                        if selected { self.selected.insert(id); } else { self.selected.remove(&id); }
                        self.confirm = false;
                    }
                    ui.small(row["Path"].as_str().unwrap_or(""));
                }
            });
            ui.checkbox(&mut self.confirm, "确认按当前选项清理；普通文件先备份，回收站不备份");
            if ui.add_enabled(self.confirm && !self.selected.is_empty(), egui::Button::new("执行所选清理")).clicked() {
                if let Err(e) = self.start(true, destination) { self.status = e.to_string(); }
            }
        });
        ui.label(&self.status);
    }
}


