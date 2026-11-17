use anyhow::{bail, Context, Result};
use std::{fs, io::{Read, Write}, path::{Path,PathBuf}, sync::{Arc,atomic::{AtomicBool,Ordering}}, time::{Duration,SystemTime,UNIX_EPOCH}, os::windows::{fs::{MetadataExt,OpenOptionsExt},process::CommandExt}};
use crate::core::{history::{MigrationRecord,HistoryManager},plan};
pub struct Mover;
fn files(root:&Path)->Result<Vec<PathBuf>> {
    plan::plain_path(root)?;
    let mut out=Vec::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let e=entry?;let m=fs::symlink_metadata(e.path())?;
        if m.file_attributes() & 0x400 != 0 {bail!("迁移树包含链接：{}",e.path().display());}
        if m.file_attributes() & (0x4000 | 0x200 | 0x1000) != 0 {bail!("不支持加密、稀疏或离线文件：{}",e.path().display());}
        if m.is_file(){out.push(e.path().strip_prefix(root)?.to_path_buf());}
    }
    out.sort();Ok(out)
}
fn same(a:&Path,b:&Path)->Result<bool>{
    if fs::metadata(a)?.len()!=fs::metadata(b)?.len(){return Ok(false);}
    let mut a=fs::File::open(a)?;let mut b=fs::File::open(b)?;
    let mut x=[0u8;65536];let mut y=[0u8;65536];
    loop {let n=a.read(&mut x)?;if n==0{return Ok(true);}b.read_exact(&mut y[..n])?;if x[..n]!=y[..n]{return Ok(false);}}
}
fn verify(a:&Path,b:&Path)->Result<()> {
    let left=files(a)?;let right=files(b)?;
    if left!=right {bail!("文件清单发生变化；保留两侧数据");}
    for rel in left {if !same(&a.join(&rel),&b.join(&rel))?{bail!("内容校验失败：{}",rel.display());}}
    Ok(())
}
fn copy<F:Fn(u64)>(a:&Path,b:&Path,cb:&F,pause:&AtomicBool)->Result<()> {
    let entries=files(a)?;fs::create_dir(b)?;
    for e in walkdir::WalkDir::new(a).follow_links(false){let e=e?;if e.file_type().is_dir(){fs::create_dir_all(b.join(e.path().strip_prefix(a)?))?;}}
    for rel in entries {
        while pause.load(Ordering::SeqCst){std::thread::sleep(Duration::from_millis(100));}
        let from=a.join(&rel);let to=b.join(rel);plan::plain_path(&from)?;
        let mut input=fs::OpenOptions::new().read(true).share_mode(1).open(&from)?;
        let mut output=fs::OpenOptions::new().write(true).create_new(true).open(&to)?;
        let bytes=std::io::copy(&mut input,&mut output)?;output.sync_all()?;cb(bytes);
    }
    verify(a,b)
}
fn reject_named_streams(source:&Path)->Result<()> {
    let result=std::process::Command::new("powershell.exe").creation_flags(0x08000000)
        .args(["-NoProfile","-NonInteractive","-Command","$ErrorActionPreference='Stop'; $streams=@(Get-ChildItem -LiteralPath $env:KEEPER_SOURCE -Recurse -Force -File | ForEach-Object {Get-Item -LiteralPath $_.FullName -Stream *} | Where-Object {$_.Stream -ne ':$DATA'}); if($streams.Count){Write-Error 'Named data streams found; migration refused'}"])
        .env("KEEPER_SOURCE",source).output()?;
    if !result.status.success(){bail!("无法确认 NTFS 数据流安全，迁移取消：{}",String::from_utf8_lossy(&result.stderr));}Ok(())
}
fn journal(root:&Path,stage:&str,source:&Path,target:&Path,backup:&Path)->Result<()> {
    let p=root.join(format!("{}.{}.json",uuid::Uuid::new_v4(),stage));
    let mut f=fs::File::create(p)?;f.write_all(serde_json::to_string_pretty(&serde_json::json!({"stage":stage,"source":source,"target":target,"source_backup":backup}))?.as_bytes())?;f.sync_all()?;Ok(())
}
fn link(source:&Path,target:&Path)->Result<()> {
    let out=std::process::Command::new("powershell.exe").creation_flags(0x08000000)
        .args(["-NoProfile","-NonInteractive","-Command","$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:KEEPER_LINK -Target $env:KEEPER_TARGET | Out-Null"])
        .env("KEEPER_LINK",source).env("KEEPER_TARGET",target).output()?;
    if !out.status.success(){bail!("创建联接失败：{}",String::from_utf8_lossy(&out.stderr));}
    if fs::canonicalize(source)?!=fs::canonicalize(target)?{bail!("联接目标核验失败");}Ok(())
}
// Delete only a verified original copy, never traverse links or unknown additions.
fn discard_verified(original:&Path,target:&Path)->Result<u64> {
    verify(original,target)?;let mut retained=0;
    for rel in files(original)? {let p=original.join(&rel);plan::plain_path(&p)?;let _guard=fs::OpenOptions::new().read(true).share_mode(1|4).open(&p)?;if !same(&p,&target.join(&rel))?{bail!("源文件再次变化，保留：{}",p.display());}if fs::remove_file(&p).is_err(){retained+=fs::metadata(&p)?.len();}}
    let mut dirs:Vec<_>=walkdir::WalkDir::new(original).follow_links(false).into_iter().collect::<std::result::Result<Vec<_>,_>>()?;
    dirs.sort_by_key(|e|std::cmp::Reverse(e.depth()));for e in dirs{if e.file_type().is_dir(){let _=fs::remove_dir(e.path());}}
    Ok(retained)
}
fn stage_move<F:Fn(u64),L:Fn(&Path,&Path)->Result<()>>(source:&Path,target:&Path,backup:&Path,log_root:&Path,cb:&F,pause:&AtomicBool,create_link:L)->Result<()> {
    if target.exists() || backup.exists(){bail!("目标或源备份已存在，拒绝覆盖");}
    fs::rename(source,backup).context("源目录被占用或无权限；尚未移动内容")?;
    let result=(||->Result<()> {
        journal(log_root,"source-renamed",source,target,backup)?;
        copy(backup,target,cb,pause)?;
        journal(log_root,"verified",source,target,backup)?;
        if source.exists(){bail!("应用重新创建了源目录，停止切换");}
        create_link(source,target)
    })();
    if let Err(e)=result {
        if !source.exists(){fs::rename(backup,source).context(format!("恢复源名称失败；原数据在 {}",backup.display()))?;}
        let _=journal(log_root,"failed-preserved",source,target,backup);
        return Err(e.context(format!("目标副本保留于 {}；原目录或备份保留于 {}",target.display(),backup.display())));
    }
    Ok(())
}
impl Mover {
    #[allow(clippy::too_many_arguments)]
    pub fn move_and_link<F>(source:&Path,target_root:&Path,cb:F,pause:Arc<AtomicBool>,_parallelism:usize,auto_kill:bool,batch_id:String,batch_timestamp:u64)->Result<(u64,MigrationRecord)>
    where F:Fn(u64)+Clone+Send+Sync+'static {
        if auto_kill {bail!("新版不自动结束进程，请先关闭对应软件");}
        plan::plain_path(source)?;plan::plain_path(target_root)?;
        if plan::overlaps(source,target_root){bail!("源和目标重叠");}
        if source.components().next()==target_root.components().next(){bail!("迁移目标必须在另一盘");}
        let source_name=plan::key(source);
        for variable in ["SystemRoot","ProgramFiles","ProgramFiles(x86)","ProgramData"] {if let Ok(root)=std::env::var(variable){if plan::overlaps(source,Path::new(&root)){bail!("系统/程序安装目录不支持直接迁移，请使用软件自己的迁移或安装功能");}}}
        if source.components().count()<4 || source_name.ends_with("\\appdata") || source_name.ends_with("\\local") || source_name.ends_with("\\roaming"){bail!("源范围过大");}
        if plan::overlaps(source,&std::env::current_exe()?){bail!("不能迁移正在运行的本程序");}
        let manifest=files(source)?;
        reject_named_streams(source)?;
        let needed:u64=manifest.iter().map(|p|fs::metadata(source.join(p)).map(|m|m.len())).collect::<std::io::Result<Vec<_>>>()?.iter().sum();
        let volume=target_root.ancestors().last().context("无目标盘")?;
        let wide:Vec<u16>=volume.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();let mut available=0;
        unsafe{if !windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(windows::core::PCWSTR(wide.as_ptr()),Some(&mut available),None,None).as_bool(){bail!("无法读取目标盘空间");}}
        if available<needed.saturating_add(64*1024*1024){bail!("目标盘空间不足");}
        if !crate::core::proc_mgr::ProcMgr::check_locking_processes_dir(source)?.is_empty(){bail!("目录仍被程序使用，请关闭软件后重试");}
        let target=target_root.join(source.file_name().context("无效源目录")?);
        if target.exists(){bail!("目标已存在，拒绝覆盖：{}",target.display());}
        fs::create_dir_all(target_root)?;
        let backup=source.with_file_name(format!("{}.keeper-original-{}",source.file_name().unwrap().to_string_lossy(),uuid::Uuid::new_v4()));
        journal(target_root,"prepared",source,&target,&backup)?;
        stage_move(source,&target,&backup,target_root,&cb,&pause,link)?;
        journal(target_root,"linked",source,&target,&backup)?;
        let bytes=files(&target)?.iter().map(|p|fs::metadata(target.join(p)).map(|m|m.len())).collect::<std::io::Result<Vec<_>>>()?.iter().sum();
        let record=MigrationRecord{id:uuid::Uuid::new_v4().to_string(),name:source.file_name().unwrap().to_string_lossy().into(),source_path:source.into(),target_path:target.clone(),size_bytes:bytes,timestamp:SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),batch_id:Some(batch_id),batch_timestamp:Some(batch_timestamp)};
        HistoryManager::add_record(record.clone())?;
        let retained=discard_verified(&backup,&target).unwrap_or_else(|_|crate::core::plan::inspect_tree(&backup).0);
        journal(target_root,"completed",source,&target,&backup)?;
        Ok((retained,record))
    }
    pub fn restore_migration<F>(record:&MigrationRecord,cb:F,pause:Arc<AtomicBool>,_parallelism:usize,auto_kill:bool)->Result<()> where F:Fn(u64)+Clone+Send+Sync+'static {
        if auto_kill{bail!("请手动关闭对应软件");}
        let source=&record.source_path;let target=&record.target_path;
        if fs::symlink_metadata(source)?.file_attributes() & 0x400 == 0 {bail!("源位置不是联接，拒绝覆盖");}
        if fs::canonicalize(source)?!=fs::canonicalize(target)? {bail!("联接指向已改变，拒绝还原");}
        if !crate::core::proc_mgr::ProcMgr::check_locking_processes_dir(target)?.is_empty(){bail!("目标仍被占用");}
        let stage=source.with_file_name(format!("keeper-restore-{}",uuid::Uuid::new_v4()));
        journal(target.parent().context("目标缺少父目录")?,"restore-started",source,target,&stage)?;
        reject_named_streams(target)?;copy(target,&stage,&cb,&pause)?;verify(target,&stage)?;
        fs::remove_dir(source).context("解除联接失败；两份数据均保留")?;
        if let Err(e)=fs::rename(&stage,source){let _=link(source,target);return Err(e.into());}
        // Keep the D: copy intentionally: restoration never deletes the only recovery copy.
        HistoryManager::remove_record(&record.id)?;
        journal(target.parent().unwrap(),"restored-target-retained",source,target,&stage)?;Ok(())
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn changed_content_and_extra_files_are_detected(){let root=std::env::temp_dir().join(format!("keeper-test-{}",uuid::Uuid::new_v4()));fs::create_dir(&root).unwrap();let a=root.join("a");let b=root.join("b");fs::create_dir(&a).unwrap();fs::write(a.join("x"),b"one").unwrap();copy(&a,&b,&|_|{},&AtomicBool::new(false)).unwrap();fs::write(b.join("x"),b"two").unwrap();assert!(verify(&a,&b).is_err());fs::write(b.join("x"),b"one").unwrap();fs::write(b.join("extra"),b"extra").unwrap();assert!(verify(&a,&b).is_err());assert_eq!(fs::read(a.join("x")).unwrap(),b"one");fs::remove_dir_all(root).unwrap();}
}



#[cfg(test)] mod transaction_tests {
    use super::*;
    fn fixture()->(PathBuf,PathBuf,PathBuf,PathBuf){let root=std::env::temp_dir().join(format!("keeper-transaction-{}",uuid::Uuid::new_v4()));fs::create_dir(&root).unwrap();let source=root.join("source");fs::create_dir(&source).unwrap();fs::write(source.join("data"),b"important").unwrap();let target=root.join("target");let backup=root.join("backup");(root,source,target,backup)}
    #[test] fn link_failure_restores_source_and_keeps_verified_copy(){let(r,s,t,b)=fixture();let result=stage_move(&s,&t,&b,&r,&|_|{},&AtomicBool::new(false),|_,_|anyhow::bail!("injected link failure"));assert!(result.is_err());assert_eq!(fs::read(s.join("data")).unwrap(),b"important");assert_eq!(fs::read(t.join("data")).unwrap(),b"important");assert!(!b.exists());fs::remove_dir_all(r).unwrap();}
    #[test] fn destination_conflict_does_not_rename_source(){let(r,s,t,b)=fixture();fs::create_dir(&t).unwrap();let result=stage_move(&s,&t,&b,&r,&|_|{},&AtomicBool::new(false),|_,_|Ok(()));assert!(result.is_err());assert!(s.join("data").exists());assert!(!b.exists());fs::remove_dir_all(r).unwrap();}
    #[test] fn locked_file_failure_keeps_original(){let(r,s,t,b)=fixture();let lock=fs::OpenOptions::new().read(true).write(true).share_mode(0).open(s.join("data")).unwrap();let result=stage_move(&s,&t,&b,&r,&|_|{},&AtomicBool::new(false),|_,_|Ok(()));assert!(result.is_err());drop(lock);assert!(s.join("data").exists()||b.join("data").exists());fs::remove_dir_all(r).unwrap();}
    #[test] fn read_error_is_not_empty_success(){let r=std::env::temp_dir().join(format!("keeper-missing-{}",uuid::Uuid::new_v4()));assert!(files(&r).is_err());assert!(!plan::inspect_tree(&r).1.is_empty());}
}
