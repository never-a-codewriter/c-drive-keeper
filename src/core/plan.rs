use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{path::{Path, PathBuf, Component}, fs, os::windows::fs::MetadataExt};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Action { Keep, Clean, Migrate }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finding {
    pub path: PathBuf,
    pub label: String,
    pub logical_bytes: u64,
    pub clean_bytes: u64,
    pub clean_ids: Vec<String>,
    pub cache_snapshots: Vec<serde_json::Value>,
    pub reason: String,
    pub errors: Vec<String>,
    pub action: Action,
    pub can_migrate: bool,
}
pub fn key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}
pub fn overlaps(a: &Path, b: &Path) -> bool {
    let a = key(a); let b = key(b);
    a == b || a.starts_with(&(b.clone()+"\\")) || b.starts_with(&(a+"\\"))
}
pub fn plain_path(path: &Path) -> Result<()> {
    if !path.is_absolute() || path.components().any(|c| matches!(c,Component::ParentDir)) { bail!("必须使用无 .. 的绝对路径：{}",path.display()); }
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(m) if m.file_attributes() & 0x400 != 0 => bail!("拒绝目录联接或重解析路径：{}",ancestor.display()),
            Ok(_) => {},
            Err(e) if e.kind()==std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(e).context(format!("无法检查路径：{}",ancestor.display())),
        }
    }
    Ok(())
}
pub fn validate(rows: &[Finding], destination: &Path) -> Result<()> {
    plain_path(destination)?;
    let chosen: Vec<_> = rows.iter().filter(|r| r.action!=Action::Keep).collect();
    if chosen.is_empty(){bail!("请先选择处理方案");}
    for (i,row) in chosen.iter().enumerate() {
        plain_path(&row.path)?;
        if !row.errors.is_empty(){bail!("扫描不完整，不能执行：{}",row.path.display());}
        if overlaps(&row.path,destination){bail!("备份/目标目录与源目录重叠：{}",row.path.display());}
        if row.action==Action::Migrate && !row.can_migrate {bail!("该目录不允许迁移");}
        if row.action==Action::Clean && row.clean_ids.is_empty(){bail!("没有可用清理规则");}
        for other in chosen.iter().skip(i+1){if overlaps(&row.path,&other.path){bail!("计划重复或父子路径冲突：{} / {}",row.path.display(),other.path.display());}}
    }
    Ok(())
}
pub fn inspect_tree(root: &Path) -> (u64, Vec<String>) {
    let mut bytes=0;let mut errors=Vec::new();
    if let Err(e)=plain_path(root){return (0,vec![e.to_string()]);}
    let iter=walkdir::WalkDir::new(root).follow_links(false).into_iter().filter_entry(|e| {
        fs::symlink_metadata(e.path()).map(|m|m.file_attributes() & 0x400 == 0).unwrap_or(true)
    });
    // Separate walk so excluded links are reported, rather than silently interpreted as empty.
    for entry in iter {
        match entry {
            Ok(e)=>match fs::symlink_metadata(e.path()) {
                Ok(m) if m.is_file()=>bytes+=m.len(),
                Ok(m) if m.is_dir()=> match fs::read_dir(e.path()) {
                    Ok(children)=>for child in children.flatten(){if fs::symlink_metadata(child.path()).map(|m|m.file_attributes() & 0x400 != 0).unwrap_or(false){errors.push(format!("已跳过链接：{}",child.path().display()));}},
                    Err(err)=>errors.push(err.to_string()),
                },
                Ok(_)=>{},Err(err)=>errors.push(err.to_string()),
            },
            Err(e)=>errors.push(e.to_string()),
        }
    }
    (bytes,errors)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn boundary_is_component_aware(){assert!(overlaps(Path::new("C:\\Apps\\Kiro"),Path::new("c:/apps/kiro/Cache")));assert!(!overlaps(Path::new("C:\\Apps\\Kiro"),Path::new("C:\\Apps\\Kiro2")));}
    #[test] fn rejects_relative_and_parent(){assert!(plain_path(Path::new("abc")).is_err());assert!(plain_path(Path::new("C:\\foo\\..\\bar")).is_err());}
}

#[cfg(test)] mod conflict_tests {
    use super::*;
    fn row(path:PathBuf)->Finding{Finding{path,label:"test".into(),logical_bytes:1,clean_bytes:1,clean_ids:vec!["temp".into()],cache_snapshots:vec![],reason:String::new(),errors:vec![],action:Action::Clean,can_migrate:true}}
    #[test] fn parent_child_plan_is_rejected(){let root=std::env::temp_dir().join(format!("keeper-plan-{}",uuid::Uuid::new_v4()));let child=root.join("child");fs::create_dir_all(&child).unwrap();let dest=root.with_extension("backup");assert!(validate(&[row(root.clone()),row(child)],&dest).is_err());assert!(validate(&[row(root.clone())],&dest).is_ok());fs::remove_dir_all(root).unwrap();}
    #[test] fn incomplete_scan_blocks_execution(){let root=std::env::temp_dir().join(format!("keeper-plan-{}",uuid::Uuid::new_v4()));let mut r=row(root.clone());r.errors.push("access denied".into());assert!(validate(&[r],&root.with_extension("backup")).is_err());}
    #[test] fn actual_windows_junction_is_refused(){let path=PathBuf::from("C:\\Documents and Settings");if let Ok(m)=fs::symlink_metadata(&path){if m.file_attributes() & 0x400 != 0 {assert!(plain_path(&path).is_err());assert!(!inspect_tree(&path).1.is_empty());}}}
}
