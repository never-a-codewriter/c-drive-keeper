use crate::core::{config::ScanSource,plan};
use anyhow::{Context,Result};
use serde::{Deserialize,Serialize};
use std::path::PathBuf;
#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct ScanResult {pub path:PathBuf,pub name:String,pub size_bytes:u64,pub label:String,pub target_subdir:String,pub parent_total_size:u64}
pub struct Scanner;
impl Scanner {
    pub fn scan_large_folders<F>(sources:&[ScanSource],progress:F)->Result<Vec<ScanResult>> where F:Fn(usize,usize,String)+Sync+Send+Clone {
        let mut targets=Vec::new();
        for source in sources.iter().filter(|s|s.enabled){plan::plain_path(&source.path)?;for entry in std::fs::read_dir(&source.path).with_context(||format!("无法读取 {}",source.path.display()))?{let entry=entry?;if entry.path().is_dir(){targets.push((entry.path(),source));}}}
        let mut rows=Vec::new();let count=targets.len();
        for (index,(path,source)) in targets.into_iter().enumerate(){let (bytes,errors)=plan::inspect_tree(&path);if !errors.is_empty(){crate::core::logger::log(&format!("扫描不完整，不列为迁移候选：{} {:?}",path.display(),errors));continue;}
            let name=path.file_name().unwrap_or_default().to_string_lossy().to_string();progress(index+1,count,name.clone());
            rows.push(ScanResult{path,name,size_bytes:bytes,label:source.label.clone(),target_subdir:source.target_subdir.clone(),parent_total_size:0});
        }
        // Always fresh: directory mtime does not capture changes to descendant files.
        for index in 0..rows.len(){rows[index].parent_total_size=rows.iter().filter(|r|r.path.parent()==rows[index].path.parent()).map(|r|r.size_bytes).sum();}
        rows.sort_by_key(|r|std::cmp::Reverse(r.size_bytes));Ok(rows)
    }
}

