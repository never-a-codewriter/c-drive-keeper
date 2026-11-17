use crate::core::config::ScanSource;
use anyhow::Result;
use directories::ProjectDirs;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub path: PathBuf,
    pub name: String,
    pub size_bytes: u64,
    pub label: String,
    pub target_subdir: String,
    pub parent_total_size: u64,
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    path: PathBuf,
    size: u64,
    modified_time: u64, // UNIX timestamp
    scanned_time: u64,  // UNIX timestamp
}

#[derive(Serialize, Deserialize)]
struct ScanCache {
    version: u32,
    created_at: u64,
    roots: Vec<PathBuf>,
    entries: Vec<CacheEntry>,
}

pub struct Scanner;

impl Scanner {
    const CACHE_VERSION: u32 = 3;
    const CACHE_TTL_SECS: u64 = 10 * 60;

    /// 获取目录最后修改时间（取本身元数据）
    fn get_modified_time(path: &PathBuf) -> u64 {
        if let Ok(metadata) = std::fs::metadata(path) {
            if let Ok(modified) = metadata.modified() {
                return modified
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
            }
        }
        0
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn cache_path() -> Option<PathBuf> {
        let proj = ProjectDirs::from("com", "tanaer", "WindowsClear")?;
        let dir = proj.cache_dir().to_path_buf();
        let _ = std::fs::create_dir_all(&dir);
        Some(dir.join("scan_cache.json"))
    }

    /// 加载缓存
    fn load_cache() -> Option<ScanCache> {
        let cache_path = Self::cache_path()?;
        if !cache_path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(cache_path).ok()?;
        let cache: ScanCache = serde_json::from_str(&content).ok()?;
        if cache.version != Self::CACHE_VERSION {
            return None;
        }
        Some(cache)
    }

    /// 保存缓存
    fn save_cache(roots: Vec<PathBuf>, all_entries: Vec<CacheEntry>) {
        let Some(cache_path) = Self::cache_path() else {
            return;
        };
        let cache = ScanCache {
            version: Self::CACHE_VERSION,
            created_at: Self::now_secs(),
            roots,
            entries: all_entries,
        };
        if let Ok(content) = serde_json::to_string(&cache) {
            let _ = std::fs::write(cache_path, content);
        }
    }

    /// 计算指定目录的大小（递归）
    pub fn get_dir_size(path: &PathBuf) -> u64 {
        WalkDir::new(path)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.metadata().ok())
            .filter(|metadata| metadata.is_file())
            .map(|metadata| metadata.len())
            .sum()
    }

    /// 检查路径是否是符号链接或 Junction Point
    pub fn is_symlink_or_junction(path: &PathBuf) -> bool {
        // 在 Windows 上，fs::symlink_metadata 可以检测 symlink 和 reparse points (junctions)
        if let Ok(metadata) = std::fs::symlink_metadata(path) {
            return metadata.file_type().is_symlink();
        }
        false
    }

    /// 扫描并返回占用超过 10% 的文件夹
    ///
    /// `progress_cb`: 回调函数，参数为 (已完成数量, 总数量, 当前正在处理的文件夹名称)
    pub fn scan_large_folders<F>(
        scan_sources: &[ScanSource],
        progress_cb: F,
    ) -> Result<Vec<ScanResult>>
    where
        F: Fn(usize, usize, String) + Sync + Send + Clone,
    {
        let mut results = Vec::new();

        // 1. 收集所有一级子目录，以便计算总任务数
        let mut all_targets = Vec::new();

        let active_sources: Vec<ScanSource> =
            scan_sources.iter().filter(|s| s.enabled).cloned().collect();
        let roots: Vec<PathBuf> = active_sources.iter().map(|s| s.path.clone()).collect();

        for src in &active_sources {
            let root = &src.path;
            if !root.exists() {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        // 过滤掉已经是软链接的文件夹
                        if !Self::is_symlink_or_junction(&path) {
                            all_targets.push((
                                path,
                                src.label.clone(),
                                src.target_subdir.clone(),
                                root.clone(),
                            ));
                        }
                    }
                }
            }
        }

        let total_count = all_targets.len();
        let finished_count = Arc::new(AtomicUsize::new(0));

        let now = Self::now_secs();

        let (cache_map, cache_is_fresh): (std::collections::HashMap<PathBuf, (u64, u64)>, bool) =
            if let Some(cache) = Self::load_cache() {
                let fresh = cache.created_at.saturating_add(Self::CACHE_TTL_SECS) >= now
                    && cache.roots == roots;
                (
                    cache
                        .entries
                        .into_iter()
                        .map(|e| (e.path, (e.size, e.modified_time)))
                        .collect(),
                    fresh,
                )
            } else {
                (std::collections::HashMap::new(), false)
            };

        // 2. 并行处理
        let sizes: Vec<(PathBuf, String, String, PathBuf, u64, u64)> = all_targets
            .into_par_iter()
            .map(|(path, label, target_subdir, parent)| {
                // 上报进度
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                let current_mtime = Self::get_modified_time(&path);
                let size = if let Some((cached_size, cached_mtime)) = cache_map.get(&path) {
                    if cache_is_fresh && *cached_mtime == current_mtime {
                        *cached_size
                    } else {
                        Self::get_dir_size(&path)
                    }
                } else {
                    Self::get_dir_size(&path)
                };

                let finished = finished_count.fetch_add(1, Ordering::SeqCst) + 1;
                progress_cb(finished, total_count, name);

                (path, label, target_subdir, parent, size, current_mtime)
            })
            .collect();

        // 3. 按父目录聚合计算 total_size 并筛选
        let mut parent_sizes = std::collections::HashMap::new();
        for (_, _, _, parent, size, _) in &sizes {
            *parent_sizes.entry(parent.clone()).or_insert(0) += size;
        }

        let mut cache_entries: Vec<CacheEntry> = Vec::new();

        for (path, label, target_subdir, parent, size, mtime) in sizes {
            cache_entries.push(CacheEntry {
                path: path.clone(),
                size,
                modified_time: mtime,
                scanned_time: now,
            });

            let total_size = *parent_sizes.get(&parent).unwrap_or(&0);
            if total_size == 0 {
                continue;
            }

            let threshold = (total_size as f64 * 0.1) as u64;
            if size > threshold {
                results.push(ScanResult {
                    name: path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    path,
                    size_bytes: size,
                    label,
                    target_subdir,
                    parent_total_size: total_size,
                });
            }
        }

        // 按大小降序排列
        results.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));

        // Save new cache (cache all folders, not only top results)
        Self::save_cache(roots, cache_entries);

        Ok(results)
    }
}

