use eframe::egui;
use anyhow::{Context,Result};
use std::{fs,path::{Path,PathBuf},process::Command,os::windows::process::CommandExt,sync::{mpsc,Arc,atomic::AtomicBool},time::{SystemTime,UNIX_EPOCH}};
use crate::core::{plan::{self,Finding,Action},mover::Mover};

fn run_script(root:&Path,name:&str,source:&[u8],args:&[String])->Result<PathBuf>{
    fs::create_dir_all(root)?;let script=root.join(name);fs::write(&script,source)?;
    fs::write(root.join("cleanup-core.ps1"),include_bytes!("../scripts/cleanup-core.ps1"))?;
    let log=fs::File::create(root.join("console.log"))?;
    let status=Command::new("powershell.exe").creation_flags(0x08000000).args(["-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-File"]).arg(script).args(args).current_dir(root).stdout(log.try_clone()?).stderr(log).status()?;
    if !status.success(){anyhow::bail!("脚本未全部成功，详情：{}",root.join("console.log").display());}Ok(root.join("report.json"))
}
fn json_rows(path:&Path)->Result<Vec<serde_json::Value>>{let b=fs::read(path)?;let v:serde_json::Value=serde_json::from_slice(b.strip_prefix(&[239,187,191]).unwrap_or(&b))?;Ok(match v{serde_json::Value::Array(a)=>a,v=>vec![v]})}
fn scan(root:&Path,other:bool,progress:impl Fn(String))->Result<Vec<Finding>>{
    let mut args=vec!["-ReportPath".into(),root.join("report.json").display().to_string()];if other{args.push("-IncludeOtherDrives".into());}
    // Preserve partial reports even when some cache directories were inaccessible.
    let status=run_script(root,"clean.ps1",include_bytes!("../scripts/clean.ps1"),&args);
    let caches=json_rows(&root.join("report.json")).with_context(||format!("预览失败：{:?}",status.err()))?;
    let mut roots=Vec::new();
    for row in &caches {
        let p=PathBuf::from(row["Path"].as_str().unwrap_or(""));
        if row["Id"]=="temp" {if let Some(parent)=p.parent(){roots.push(parent.to_path_buf());}}
        if row["Id"]=="kiro-cache" {if let Some(parent)=p.parent().and_then(|p|p.parent()){roots.push(parent.to_path_buf());}}
    }
    let mut rows=Vec::new();
    for root in roots {
        match fs::read_dir(&root){
            Ok(children)=>for child in children {
                let child=match child{Ok(c)=>c,Err(e)=>{rows.push(Finding{path:root.clone(),label:"读取失败".into(),logical_bytes:0,clean_bytes:0,clean_ids:vec![],cache_snapshots:vec![],reason:e.to_string(),errors:vec![e.to_string()],action:Action::Keep,can_migrate:false});continue;}};
                let path=child.path();if !path.is_dir(){continue;}
                progress(format!("正在扫描：{}",path.display()));
                progress(format!("正在扫描：{}",path.display()));
            let (bytes,errors)=plan::inspect_tree(&path);
                let name=child.file_name().to_string_lossy().into_owned();
                let deny=["microsoft","packages","temp","programs","connecteddevicesplatform"].contains(&name.to_lowercase().as_str());
                rows.push(Finding{path,label:name,logical_bytes:bytes,clean_bytes:0,clean_ids:vec![],cache_snapshots:vec![],reason:if deny{"系统/共享组件，仅提供已知缓存清理"}else{"应用数据；迁移前需关闭对应软件，设置与数据整体保留"}.into(),errors,action:Action::Keep,can_migrate:!deny});
            },
            Err(e)=>rows.push(Finding{path:root,label:"目录访问失败".into(),logical_bytes:0,clean_bytes:0,clean_ids:vec![],cache_snapshots:vec![],reason:e.to_string(),errors:vec![e.to_string()],action:Action::Keep,can_migrate:false}),
        }
    }
    if let Some(home)=directories::UserDirs::new().map(|u|u.home_dir().to_path_buf()) {
        for name in [".cache",".kiro",".vscode",".codex",".copilot",".workbuddy-ai"] {
            let path=home.join(name);if !path.exists(){continue;}
            progress(format!("正在扫描：{}",path.display()));
            let (bytes,errors)=plan::inspect_tree(&path);
            rows.push(Finding{path,label:name.into(),logical_bytes:bytes,clean_bytes:0,clean_ids:vec![],cache_snapshots:vec![],reason:"工具运行环境/扩展；不能按缓存名直接删除。请先退出对应工具。".into(),errors,action:Action::Keep,can_migrate:!matches!(name,".codex"|".cache")});
        }
    }
    for cache in caches {
        let id=cache["Id"].as_str().unwrap_or("").to_string();let path=PathBuf::from(cache["Path"].as_str().unwrap_or(""));if id.is_empty()||!path.is_absolute(){continue;}
        let bytes=cache["EligibleBytes"].as_u64().unwrap_or(0);
        let errs:Vec<String>=cache["Errors"].as_array().map(|a|a.iter().map(|v|v.as_str().unwrap_or("未知错误").to_string()).collect()).unwrap_or_default();
        if let Some(row)=rows.iter_mut().find(|r|plan::overlaps(&r.path,&path)&&plan::key(&path).starts_with(&plan::key(&r.path))){row.clean_bytes+=bytes;row.clean_ids.push(id);row.cache_snapshots.push(cache.clone());row.errors.extend(errs);}
        else {rows.push(Finding{path,label:id.clone(),logical_bytes:bytes,clean_bytes:bytes,clean_ids:vec![id],cache_snapshots:vec![cache.clone()],reason:"已知缓存；默认只清理 7 天前文件".into(),errors:errs,action:Action::Keep,can_migrate:false});}
    }
    rows.sort_by_key(|r|std::cmp::Reverse(r.logical_bytes));fs::write(root.join("findings.json"),serde_json::to_vec_pretty(&rows)?)?;Ok(rows)
}
pub fn free_c()->Option<u64>{unsafe {let p:Vec<u16>=crate::core::config::system_volume().as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();let mut free=0;let ok=windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(windows::core::PCWSTR(p.as_ptr()),Some(&mut free),None,None).as_bool();if ok{Some(free)}else{None}}}
fn execute(rows:Vec<Finding>,dest:&Path,other:bool,recycle:bool,run:&Path,progress:mpsc::Sender<Message>)->Result<String>{
    plan::validate(&rows,dest)?;fs::create_dir_all(run)?;
    let before=free_c();let mut outcomes=Vec::new();
    fs::write(run.join("plan.json"),serde_json::to_vec_pretty(&rows)?)?;
    for (index,row) in rows.iter().filter(|r|r.action!=Action::Keep).enumerate(){
        let _=progress.send(Message::Progress(format!("正在执行第 {} 项：{}（{:?}）",index+1,row.path.display(),row.action)));
        let action_dir=run.join(format!("action-{index}"));fs::create_dir_all(&action_dir)?;
        fs::write(action_dir.join("started.json"),serde_json::to_vec_pretty(row)?)?;
        let result=match row.action {
            Action::Clean=>{
                fs::write(action_dir.join("approved.json"),serde_json::to_vec_pretty(&row.cache_snapshots)?)?;
                let mut args=vec!["-PreviewPath".into(),action_dir.join("approved.json").display().to_string(),"-Apply".into(),"-Only".into(),row.clean_ids.join(","),"-BackupRoot".into(),dest.join("cleanup-backups").display().to_string(),"-ReportPath".into(),action_dir.join("report.json").display().to_string()];if other{args.push("-IncludeOtherDrives".into());}
                run_script(&action_dir,"clean.ps1",include_bytes!("../scripts/clean.ps1"),&args).map(|_|"已清理；逐文件备份见 report.json".to_string())
            },
            Action::Migrate=>{
                // Target is unique for each action; no existing application folder is overwritten.
                let target=dest.join("migrated").join(uuid::Uuid::new_v4().to_string());
                let tx=progress.clone();let copied=Arc::new(std::sync::atomic::AtomicU64::new(0));let total=row.logical_bytes;
                Mover::move_and_link(&row.path,&target,move|bytes|{let n=copied.fetch_add(bytes,std::sync::atomic::Ordering::Relaxed)+bytes;let _=tx.send(Message::Progress(format!("迁移复制进度 {:.1} / {:.1} MB；复制后将校验内容",n as f64/1048576.,total as f64/1048576.)));},Arc::new(AtomicBool::new(false)),1,false,run.file_name().unwrap().to_string_lossy().into(),SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()).map(|(retained,record)|format!("迁移完成，原盘保留 {} 字节；还原目标：{}",retained,record.target_path.display()))
            },Action::Keep=>unreachable!(),
        };
        outcomes.push(serde_json::json!({"path":row.path,"action":row.action,"success":result.is_ok(),"detail":match result{Ok(s)=>s,Err(e)=>format!("{e:#}")}}));
        fs::write(run.join("results.json"),serde_json::to_vec_pretty(&outcomes)?)?;
    }
    if recycle {
        let r=run_script(&run.join("recycle"),"recycle.ps1",b"$ErrorActionPreference='Stop'; Clear-RecycleBin -Force",&[]);
        outcomes.push(serde_json::json!({"action":"RecycleBin","success":r.is_ok(),"detail":r.err().map(|e|e.to_string())}));
    }
    let after=free_c();let summary=serde_json::json!({"before_free_c":before,"after_free_c":after,"delta_bytes":before.zip(after).map(|(b,a)|a as i128-b as i128),"note":"磁盘实际变化包含其他应用活动；逻辑大小不等于物理释放量","outcomes":outcomes});
    fs::write(run.join("summary.json"),serde_json::to_vec_pretty(&summary)?)?;
    Ok(format!("执行结束；成功 {} / {}。C 盘实际变化：{}。报告：{}",outcomes.iter().filter(|o|o["success"]==true).count(),outcomes.len(),before.zip(after).map(|(b,a)|format!("{:.1} MB",(a as f64-b as f64)/1048576.)).unwrap_or("未知".into()),run.display()))
}

enum Message{Progress(String),Scanned(Result<Vec<Finding>>),Done(Result<String>)}
pub struct Workflow{rows:Vec<Finding>,rx:Option<mpsc::Receiver<Message>>,other:bool,recycle:bool,confirmed:bool,status:String,scan_dest:PathBuf,history:Vec<PathBuf>,keyword:String}
impl Default for Workflow{fn default()->Self{Self{rows:vec![],rx:None,other:false,recycle:false,confirmed:false,status:"先扫描，再为每个目录选择保留、清理或迁移".into(),scan_dest:PathBuf::new(),history:vec![],keyword:String::new()}}}
impl Workflow{
    pub fn busy(&self)->bool{self.rx.is_some()}
    pub fn show(&mut self,ui:&mut egui::Ui,dest:&Path,blocked:bool){
        if let Some(rx)=&self.rx {match rx.try_iter().last().map(Ok).unwrap_or_else(||rx.try_recv()){Ok(Message::Progress(text))=>{self.status=text;ui.ctx().request_repaint();},Ok(message)=>{self.rx=None;match message{Message::Progress(_)=>{},Message::Scanned(Ok(rows))=>{self.rows=rows;self.status="扫描完成。逻辑大小不去重硬链接；有读取错误的项目禁用执行。".into();},Message::Scanned(Err(e))=>self.status=format!("扫描失败：{e:#}"),Message::Done(result)=>{self.status=result.unwrap_or_else(|e|format!("执行失败：{e:#}"));self.rows.clear();}}},Err(mpsc::TryRecvError::Disconnected)=>{self.rx=None;self.status="后台任务中断；请检查计划和阶段记录".into();},Err(_)=>ui.ctx().request_repaint_after(std::time::Duration::from_millis(200))}}
        ui.heading("空间体检与处理计划");
        #[link(name="shell32")] extern "system" { fn IsUserAnAdmin()->i32; }
        ui.label(if unsafe {IsUserAnAdmin()!=0} {"当前权限：管理员"} else {"当前权限：普通用户；受保护项目会显示访问错误，可关闭后右键以管理员运行"});ui.label(format!("目标及备份：{}",dest.display()));
        ui.label("同一目录只选一种方案；缓存清理保留近期文件，迁移整体保留应用数据。");
        if self.scan_dest!=dest && !self.rows.is_empty(){self.rows.clear();self.confirmed=false;}
        ui.add_enabled_ui(!self.busy()&&!blocked,|ui|{
            if ui.checkbox(&mut self.other,"纳入非系统盘开发缓存").changed(){self.rows.clear();self.confirmed=false;}
            ui.horizontal(|ui|{if ui.button("重新扫描空间与缓存").clicked(){let (tx,rx)=mpsc::channel();self.rx=Some(rx);self.rows.clear();self.confirmed=false;self.scan_dest=dest.into();let root=dest.join("operations").join(format!("scan-{}",uuid::Uuid::new_v4()));let other=self.other;self.status="正在读取实际目录及清理规则…".into();std::thread::spawn(move||{let _=tx.send(Message::Scanned(scan(&root,other,|text|{let _=tx.send(Message::Progress(text));})));});}
                if ui.button("刷新操作记录").clicked(){self.history=fs::read_dir(dest.join("operations")).into_iter().flatten().flatten().map(|e|e.path()).filter(|p|p.join("plan.json").exists()).collect();}
            });
            egui::ScrollArea::vertical().id_source("unified-findings").max_height(340.).show(ui,|ui|{
                for (index,row) in self.rows.iter_mut().enumerate(){ui.push_id(index,|ui|{ui.group(|ui|{
                    ui.label(format!("{}   总占用 {:.1} MB / 可清缓存 {:.1} MB",row.label,row.logical_bytes as f64/1048576.,row.clean_bytes as f64/1048576.));ui.small(row.path.display().to_string());ui.small(&row.reason);if row.errors.len()>2{ui.collapsing(format!("查看全部 {} 条读取提示",row.errors.len()),|ui|{for e in &row.errors{ui.small(e);}});}
                    if !row.errors.is_empty(){ui.colored_label(egui::Color32::YELLOW,format!("扫描不完整：{}",row.errors.iter().take(2).cloned().collect::<Vec<_>>().join("；")));}
                    ui.horizontal(|ui|{let mut changed=ui.selectable_value(&mut row.action,Action::Keep,"保留").changed();
                        ui.add_enabled_ui(row.errors.is_empty(),|ui|{
                            ui.add_enabled_ui(row.clean_bytes>0,|ui|{changed|=ui.selectable_value(&mut row.action,Action::Clean,"清缓存").changed();});
                            ui.add_enabled_ui(row.can_migrate,|ui|{changed|=ui.selectable_value(&mut row.action,Action::Migrate,"迁移整个目录").changed();});
                        });if changed{self.confirmed=false;}
                    });
                });});}
            });
            if ui.checkbox(&mut self.recycle,"计划完成后清空所有盘回收站（无备份）").changed(){self.confirmed=false;}
            let selected=self.rows.iter().filter(|r|r.action!=Action::Keep).count();ui.label(format!("计划共 {} 项；迁移前关闭对应软件。",selected));
            let check=plan::validate(&self.rows,dest);if selected>0{if let Err(e)=&check{ui.colored_label(egui::Color32::RED,e.to_string());}}
            ui.checkbox(&mut self.confirmed,"我已核对计划、关闭相关软件；接受所选清理与迁移");
            if ui.add_enabled(self.confirmed&&check.is_ok(),egui::Button::new("执行统一计划")).clicked(){let (tx,rx)=mpsc::channel();self.rx=Some(rx);self.confirmed=false;let rows=self.rows.clone();let dest=dest.to_path_buf();let other=self.other;let recycle=self.recycle;self.status="执行中：先记录计划，再逐项处理；请勿关闭程序".into();std::thread::spawn(move||{let run=dest.join("operations").join(format!("run-{}",uuid::Uuid::new_v4()));let result=execute(rows,&dest,other,recycle,&run,tx.clone());let _=tx.send(Message::Done(result));});}
            ui.collapsing("操作记录与恢复",|ui|{
                if ui.button("恢复中断迁移：选择阶段 JSON（仅在原路径缺失时）").clicked(){
                    if let Some(report)=rfd::FileDialog::new().add_filter("迁移阶段日志",&["json"]).pick_file(){
                        let root=dest.join("operations").join(format!("recover-{}",uuid::Uuid::new_v4()));let(tx,rx)=mpsc::channel();self.rx=Some(rx);
                        std::thread::spawn(move||{let args=vec!["-JournalPath".into(),report.display().to_string(),"-Apply".into()];let result=run_script(&root,"recover-migration.ps1",include_bytes!("../scripts/recover-migration.ps1"),&args).map(|_|format!("中断恢复完成：{}",root.display()));let _=tx.send(Message::Done(result));});
                    }
                }
                if ui.button("选择清理报告并恢复备份（不覆盖现有文件）").clicked(){
                    if let Some(report)=rfd::FileDialog::new().add_filter("清理 JSON 报告",&["json"]).pick_file(){
                        let root=dest.join("operations").join(format!("restore-{}",uuid::Uuid::new_v4()));let(tx,rx)=mpsc::channel();self.rx=Some(rx);
                        std::thread::spawn(move||{let args=vec!["-ReportPath".into(),report.display().to_string(),"-Apply".into(),"-OutputPath".into(),root.join("restore-result.json").display().to_string()];let result=run_script(&root,"restore-cleanup.ps1",include_bytes!("../scripts/restore-cleanup.ps1"),&args).map(|_|format!("恢复完成，原备份保留：{}",root.display()));let _=tx.send(Message::Done(result));});
                    }
                }for path in &self.history{ui.label(path.display().to_string());if let Ok(text)=fs::read_to_string(path.join("summary.json")){ui.collapsing("查看执行结果",|ui|{ui.label(text);});}else{ui.colored_label(egui::Color32::YELLOW,"未完成记录：检查 started.json、迁移阶段日志和两侧数据");}}ui.label("迁移还原使用“迁移记录”页；清理备份的源路径和校验值在 report.json 中。不要直接覆盖新文件。");});
            ui.collapsing("原项目工具：预览诊断",|ui|{
                ui.label("保留应用清单、缓存重定向及残留搜索；此处只预览，不卸载或修改设置。");ui.text_edit_singleline(&mut self.keyword);
                let tools:[(&str,&str,&[u8]);4]=[("应用清单","list-apps.ps1",include_bytes!("../scripts/list-apps.ps1")),("开发缓存位置方案","relocate-dev-caches.ps1",include_bytes!("../scripts/relocate-dev-caches.ps1")),("其他缓存位置方案","relocate-more-caches.ps1",include_bytes!("../scripts/relocate-more-caches.ps1")),("按输入的软件名查残留","Find-AndRemove-Leftovers.ps1",include_bytes!("../scripts/Find-AndRemove-Leftovers.ps1"))];
                for (label,name,bytes) in tools{if ui.button(label).clicked(){if name.starts_with("Find-")&&self.keyword.trim().is_empty(){self.status="先输入已卸载软件的名称".into();continue;}let args=if name.starts_with("Find-"){vec!["-Name".into(),self.keyword.clone(),"-Preview".into()]}else if name.starts_with("relocate-"){vec!["-Root".into(),dest.join("cache").display().to_string()]}else{vec![]};let root=dest.join("operations").join(format!("diagnostic-{}",uuid::Uuid::new_v4()));let (tx,rx)=mpsc::channel();self.rx=Some(rx);std::thread::spawn(move||{let result=run_script(&root,name,bytes,&args).map(|_|format!("诊断完成：{}",root.join("console.log").display()));let _=tx.send(Message::Done(result));});}}
            });
        });ui.separator();ui.label(&self.status);
    }
}








