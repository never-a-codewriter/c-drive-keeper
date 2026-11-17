use serde::{Deserialize,Serialize};
use std::{fs,path::PathBuf,io::Write};
use anyhow::{Context,Result};
#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct MigrationRecord{pub id:String,pub name:String,pub source_path:PathBuf,pub target_path:PathBuf,pub size_bytes:u64,pub timestamp:u64,pub batch_id:Option<String>,pub batch_timestamp:Option<u64>}
pub struct HistoryManager;
impl HistoryManager{
    fn path()->Result<PathBuf>{let exe=std::env::current_exe()?;Ok(exe.parent().context("无程序目录")?.join("migration-history.json"))}
    fn read()->Result<Vec<MigrationRecord>>{let p=Self::path()?;match fs::read(&p){Ok(b)=>Ok(serde_json::from_slice(&b)?),Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ok(vec![]),Err(e)=>Err(e.into())}}
    pub fn load_records()->Vec<MigrationRecord>{Self::read().unwrap_or_else(|e|{crate::core::logger::log(&format!("迁移记录读取失败：{e}"));vec![]})}
    fn save(records:&[MigrationRecord])->Result<()>{
        let p=Self::path()?;let tmp=p.with_extension(format!("{}.tmp",uuid::Uuid::new_v4()));let mut file=fs::File::create(&tmp)?;file.write_all(&serde_json::to_vec_pretty(records)?)?;file.sync_all()?;drop(file);
        let from:Vec<u16>=tmp.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();let to:Vec<u16>=p.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
        unsafe{use windows::{core::PCWSTR,Win32::Storage::FileSystem::{MoveFileExW,MOVEFILE_REPLACE_EXISTING,MOVEFILE_WRITE_THROUGH}};if !MoveFileExW(PCWSTR(from.as_ptr()),PCWSTR(to.as_ptr()),MOVEFILE_REPLACE_EXISTING|MOVEFILE_WRITE_THROUGH).as_bool(){anyhow::bail!("无法保存迁移记录；临时记录保留于 {}",tmp.display());}}Ok(())
    }
    pub fn add_record(record:MigrationRecord)->Result<()>{let mut rows=Self::read()?;rows.push(record);Self::save(&rows)}
    pub fn remove_record(id:&str)->Result<()>{let mut rows=Self::read()?;rows.retain(|r|r.id!=id);Self::save(&rows)}
}
