use crate::core::logger;
use crate::core::proc_mgr::ProcMgr;
use anyhow::{anyhow, Context, Result};
use crossbeam_channel::{bounded, Sender};
use std::fs;
use std::io::ErrorKind;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::history::{HistoryManager, MigrationRecord};

pub struct Mover;

impl Mover {
    fn ensure_target_root(target_root: &Path) -> Result<()> {
        if target_root.exists() && !target_root.is_dir() {
            return Err(anyhow!(
                "目标根目录不是文件夹：{:?}\n建议：\n- 请选择一个新的目标根目录（例如 D:\\WindowsClear\\AppData）\n- 或者删除/更名该路径后再重试",
                target_root
            ));
        }
        if target_root.exists() {
            return Ok(());
        }
        match fs::create_dir_all(target_root) {
            Ok(_) => Ok(()),
            Err(e) => {
                let code = e.raw_os_error().unwrap_or(0);
                Err(anyhow!(
                    "无法创建目标根目录：{:?}\n系统错误：{} (os error {})\n建议：\n- 确认目标盘符存在且可写（例如 D: / G:）\n- 尝试手动创建该目录看是否报权限\n- 检查是否被安全软件/受控文件夹访问拦截\n- 避免把目标根目录放在受保护目录下（建议 D:\\WindowsClear\\AppData）",
                    target_root,
                    e,
                    code
                ))
            }
        }
    }

    fn is_safe_to_skip_on_lock(src_path: &Path) -> bool {
        let Some(file_name) = src_path.file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        if file_name.eq_ignore_ascii_case("LOCK") {
            let lower = src_path.to_string_lossy().to_lowercase();
            return lower.contains("\\leveldb\\");
        }
        false
    }

    /// 迁移文件夹并创建链接
    #[allow(clippy::too_many_arguments)]
    pub fn move_and_link<F>(
        source: &Path,
        target_root: &Path,
        progress_cb: F,
        pause_signal: Arc<AtomicBool>,
        parallelism: usize,
        auto_kill: bool,
        batch_id: String,
        batch_timestamp: u64,
    ) -> Result<(u64, MigrationRecord)>
    where
        F: Fn(u64) + Clone + Send + Sync + 'static,
    {
        logger::log(&format!(
            "move_and_link start source={:?} target_root={:?}",
            source, target_root
        ));
        if !source.exists() {
            return Err(anyhow!("源路径不存在: {:?}", source));
        }

        let folder_name = source.file_name().ok_or_else(|| anyhow!("无效的源路径"))?;

        let target_path = target_root.join(folder_name);
        let target_partial = target_root.join(format!("{}.partial", folder_name.to_string_lossy()));

        // 1. 确保目标根目录存在
        Self::ensure_target_root(target_root)?;

        // 注意：断点续传允许目标路径存在
        if target_path.exists() && !target_path.is_dir() {
            return Err(anyhow!("目标路径已存在且不是目录: {:?}", target_path));
        }
        if target_partial.exists() {
            let _ = fs::remove_dir_all(&target_partial);
        }

        if target_path.exists() {
            logger::log(&format!(
                "target exists, incremental sync source={:?} target={:?}",
                source, target_path
            ));
            Self::copy_dir_all_auto(
                source,
                &target_path,
                progress_cb.clone(),
                pause_signal.clone(),
                parallelism,
            )
            .context("增量复制失败")?;
            match Self::place_junction_with_backup(source, &target_path, auto_kill) {
                Ok(lb) => {
                    logger::log(&format!("move_and_link done via incremental {:?}", source));

                    let record = MigrationRecord {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: folder_name.to_string_lossy().to_string(),
                        source_path: source.to_path_buf(),
                        target_path: target_path.clone(),
                        size_bytes: crate::core::scanner::Scanner::get_dir_size(&target_path),
                        timestamp: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                        batch_id: Some(batch_id.clone()),
                        batch_timestamp: Some(batch_timestamp),
                    };
                    HistoryManager::add_record(record.clone());

                    return Ok((lb, record));
                }
                Err(e) => {
                    logger::log(&format!("root junction failed: {}", e));
                    let (ok, total, _) = Self::link_child_dirs(source, &target_path, auto_kill);
                    if ok > 0 {
                        return Err(anyhow!(
                            "根目录创建软链接失败（{}），但已对 {}/{} 个子目录创建软链接（见日志）",
                            e,
                            ok,
                            total
                        ));
                    }
                    return Err(e);
                }
            }
        }

        if fs::rename(source, &target_path).is_ok() {
            logger::log(&format!("rename success {:?} -> {:?}", source, target_path));
            Self::create_junction(source, &target_path).map_err(|e| {
                logger::log(&format!("create_junction failed after rename: {}", e));
                let _ = Self::move_dir_back(&target_path, source);
                anyhow!("创建链接失败: {}。已尝试恢复文件。", e)
            })?;
            logger::log(&format!("move_and_link done via rename {:?}", source));

            let record = MigrationRecord {
                id: uuid::Uuid::new_v4().to_string(),
                name: folder_name.to_string_lossy().to_string(),
                source_path: source.to_path_buf(),
                target_path: target_path.clone(),
                size_bytes: crate::core::scanner::Scanner::get_dir_size(&target_path),
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                batch_id: Some(batch_id.clone()),
                batch_timestamp: Some(batch_timestamp),
            };
            HistoryManager::add_record(record.clone());

            return Ok((0, record));
        }
        logger::log(&format!(
            "rename failed {:?} -> {:?}, fallback to staged copy",
            source, target_path
        ));

        fs::create_dir_all(&target_partial).context("无法创建临时目标目录")?;
        if let Err(e) = Self::copy_dir_all_auto(
            source,
            &target_partial,
            progress_cb.clone(),
            pause_signal.clone(),
            parallelism,
        ) {
            logger::log(&format!("copy_dir_all failed: {}", e));
            let _ = fs::remove_dir_all(&target_partial);
            return Err(e.context("文件复制失败"));
        }

        fs::rename(&target_partial, &target_path).context("整理目标目录失败")?;
        logger::log(&format!(
            "rename partial ok {:?} -> {:?}",
            target_partial, target_path
        ));

        match Self::place_junction_with_backup(source, &target_path, auto_kill) {
            Ok(lb) => {
                logger::log(&format!("move_and_link done via staged copy {:?}", source));

                let record = MigrationRecord {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: folder_name.to_string_lossy().to_string(),
                    source_path: source.to_path_buf(),
                    target_path: target_path.clone(),
                    size_bytes: crate::core::scanner::Scanner::get_dir_size(&target_path),
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    batch_id: Some(batch_id.clone()),
                    batch_timestamp: Some(batch_timestamp),
                };
                HistoryManager::add_record(record.clone());

                Ok((lb, record))
            }
            Err(e) => {
                logger::log(&format!("root junction failed: {}", e));
                let (ok, total, _) = Self::link_child_dirs(source, &target_path, auto_kill);
                if ok > 0 {
                    return Err(anyhow!(
                        "根目录创建软链接失败（{}），但已对 {}/{} 个子目录创建软链接（见日志）",
                        e,
                        ok,
                        total
                    ));
                }
                Err(e)
            }
        }
    }

    fn link_child_dirs(source: &Path, target: &Path, auto_kill: bool) -> (usize, usize, u64) {
        let mut total = 0usize;
        let mut ok = 0usize;
        let mut left_behind = 0u64;

        let Ok(entries) = fs::read_dir(source) else {
            return (0, 0, 0);
        };

        for entry in entries.flatten() {
            let child = entry.path();
            if let Ok(meta) = fs::symlink_metadata(&child) {
                if meta.file_type().is_symlink() {
                    continue;
                }
            }
            if !child.is_dir() {
                continue;
            }
            total += 1;
            let target_child = target.join(entry.file_name());
            if !target_child.is_dir() {
                continue;
            }
            match Self::place_junction_with_backup(&child, &target_child, auto_kill) {
                Ok(lb) => {
                    ok += 1;
                    left_behind += lb;
                    logger::log(&format!(
                        "child junction ok {:?} -> {:?} left_behind {}",
                        child, target_child, lb
                    ));
                }
                Err(e) => {
                    logger::log(&format!("child junction failed {:?}: {}", child, e));
                }
            }
        }

        (ok, total, left_behind)
    }

    fn copy_dir_all_auto<F>(
        src: &Path,
        dst: &Path,
        progress_cb: F,
        pause_signal: Arc<AtomicBool>,
        parallelism: usize,
    ) -> Result<()>
    where
        F: Fn(u64) + Clone + Send + Sync + 'static,
    {
        if parallelism <= 1 {
            return Self::copy_dir_all_sequential(src, dst, progress_cb, pause_signal);
        }
        Self::copy_dir_all_parallel(src, dst, progress_cb, pause_signal, parallelism)
    }

    fn copy_dir_all_sequential<F>(
        src: &Path,
        dst: &Path,
        progress_cb: F,
        pause_signal: Arc<AtomicBool>,
    ) -> Result<()>
    where
        F: Fn(u64) + Clone + Send + Sync,
    {
        fs::create_dir_all(dst)?;

        for entry in fs::read_dir(src)? {
            while pause_signal.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(100));
            }

            let entry = entry?;
            let file_type = entry.file_type()?;
            let src_path = entry.path();
            if file_type.is_symlink() {
                logger::log(&format!("skip symlink entry={:?}", src_path));
                continue;
            }
            let dst_path = dst.join(entry.file_name());

            if file_type.is_dir() {
                Self::copy_dir_all_sequential(
                    &src_path,
                    &dst_path,
                    progress_cb.clone(),
                    pause_signal.clone(),
                )?;
            } else if file_type.is_file() {
                let src_meta = entry.metadata().ok();
                let size = src_meta.as_ref().map(|m| m.len()).unwrap_or(0);
                let src_mtime = src_meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                if dst_path.exists() {
                    if let Ok(meta) = fs::metadata(&dst_path) {
                        let dst_mtime = meta
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        if meta.is_file() && meta.len() == size && dst_mtime == src_mtime {
                            progress_cb(size);
                            continue;
                        }
                    }
                }

                let mut last_err: Option<anyhow::Error> = None;
                for attempt in 0..3u32 {
                    match Self::copy_file_with_progress(
                        &src_path,
                        &dst_path,
                        progress_cb.clone(),
                        pause_signal.clone(),
                    ) {
                        Ok(_) => {
                            last_err = None;
                            break;
                        }
                        Err(e) => {
                            let raw = e
                                .root_cause()
                                .downcast_ref::<std::io::Error>()
                                .and_then(|ioe| ioe.raw_os_error());
                            let lockish = raw == Some(32) || raw == Some(33) || raw == Some(5);
                            if lockish && Self::is_safe_to_skip_on_lock(&src_path) {
                                logger::log(&format!(
                                    "skip locked file (safe): {:?} raw_os_error={:?}",
                                    src_path, raw
                                ));
                                progress_cb(size);
                                last_err = None;
                                break;
                            }
                            last_err = Some(e);
                            if attempt < 2 && lockish {
                                thread::sleep(Duration::from_millis(300));
                                continue;
                            }
                            break;
                        }
                    }
                }
                if let Some(e) = last_err {
                    return Err(e).with_context(|| format!("复制文件失败: {:?}", src_path));
                }
            }
        }

        Ok(())
    }

    fn copy_dir_all_parallel<F>(
        src: &Path,
        dst: &Path,
        progress_cb: F,
        pause_signal: Arc<AtomicBool>,
        parallelism: usize,
    ) -> Result<()>
    where
        F: Fn(u64) + Clone + Send + Sync + 'static,
    {
        fs::create_dir_all(dst)?;

        let cap = parallelism.saturating_mul(256).max(256);
        let (tx, rx) = bounded::<(std::path::PathBuf, std::path::PathBuf, u64, u64)>(cap);
        let error: Arc<std::sync::Mutex<Option<anyhow::Error>>> =
            Arc::new(std::sync::Mutex::new(None));
        let failed = Arc::new(AtomicBool::new(false));

        let mut workers = Vec::new();
        for _ in 0..parallelism {
            let rx = rx.clone();
            let progress_cb = progress_cb.clone();
            let pause_signal = pause_signal.clone();
            let error = error.clone();
            let failed = failed.clone();
            let handle = thread::spawn(move || {
                while let Ok((src_file, dst_file, size, src_mtime)) = rx.recv() {
                    if failed.load(Ordering::SeqCst) {
                        break;
                    }
                    while pause_signal.load(Ordering::SeqCst) {
                        thread::sleep(Duration::from_millis(100));
                    }

                    if let Some(parent) = dst_file.parent() {
                        let _ = fs::create_dir_all(parent);
                    }

                    if dst_file.exists() {
                        if let Ok(meta) = fs::metadata(&dst_file) {
                            let dst_mtime = meta
                                .modified()
                                .ok()
                                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            if meta.is_file() && meta.len() == size && dst_mtime == src_mtime {
                                progress_cb(size);
                                continue;
                            }
                        }
                    }

                    let mut last_err: Option<anyhow::Error> = None;
                    for attempt in 0..3u32 {
                        match Self::copy_file_with_progress(
                            &src_file,
                            &dst_file,
                            progress_cb.clone(),
                            pause_signal.clone(),
                        ) {
                            Ok(_) => {
                                last_err = None;
                                break;
                            }
                            Err(e) => {
                                let raw = e
                                    .root_cause()
                                    .downcast_ref::<std::io::Error>()
                                    .and_then(|ioe| ioe.raw_os_error());
                                let lockish = raw == Some(32) || raw == Some(33) || raw == Some(5);
                                if lockish && Self::is_safe_to_skip_on_lock(&src_file) {
                                    logger::log(&format!(
                                        "skip locked file (safe): {:?} raw_os_error={:?}",
                                        src_file, raw
                                    ));
                                    progress_cb(size);
                                    last_err = None;
                                    break;
                                }
                                last_err = Some(e);
                                if attempt < 2 && lockish {
                                    thread::sleep(Duration::from_millis(300));
                                    continue;
                                }
                                break;
                            }
                        }
                    }
                    if let Some(e) = last_err {
                        failed.store(true, Ordering::SeqCst);
                        if let Ok(mut slot) = error.lock() {
                            if slot.is_none() {
                                *slot = Some(e.context(format!("复制文件失败: {:?}", src_file)));
                            }
                        }
                        break;
                    }
                }
            });
            workers.push(handle);
        }

        Self::walk_and_send_tasks(src, dst, &tx, &pause_signal, &failed)?;
        drop(tx);

        for h in workers {
            let _ = h.join();
        }

        if let Ok(mut slot) = error.lock() {
            if let Some(e) = slot.take() {
                return Err(e);
            }
        }
        Ok(())
    }

    fn walk_and_send_tasks(
        src: &Path,
        dst: &Path,
        tx: &Sender<(std::path::PathBuf, std::path::PathBuf, u64, u64)>,
        pause_signal: &Arc<AtomicBool>,
        failed: &Arc<AtomicBool>,
    ) -> Result<()> {
        for entry in fs::read_dir(src)? {
            if failed.load(Ordering::SeqCst) {
                break;
            }
            while pause_signal.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(100));
            }

            let entry = entry?;
            let file_type = entry.file_type()?;
            let src_path = entry.path();
            if file_type.is_symlink() {
                logger::log(&format!("skip symlink entry={:?}", src_path));
                continue;
            }
            let dst_path = dst.join(entry.file_name());

            if file_type.is_dir() {
                let _ = fs::create_dir_all(&dst_path);
                Self::walk_and_send_tasks(&src_path, &dst_path, tx, pause_signal, failed)?;
            } else if file_type.is_file() {
                let src_meta = entry.metadata().ok();
                let size = src_meta.as_ref().map(|m| m.len()).unwrap_or(0);
                let src_mtime = src_meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                tx.send((src_path, dst_path, size, src_mtime))
                    .map_err(|_| anyhow!("任务队列已关闭"))?;
            }
        }
        Ok(())
    }

    fn place_junction_with_backup(source: &Path, target: &Path, auto_kill: bool) -> Result<u64> {
        let name = source
            .file_name()
            .ok_or_else(|| anyhow!("无效的源路径"))?
            .to_string_lossy()
            .to_string();
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let backup_root = std::env::temp_dir().join("WindowsClear").join("backups");
        let _ = fs::create_dir_all(&backup_root);
        let mut backup = backup_root.join(format!("{}.windowsclear_bak_{}", name, ts));
        if backup.exists() {
            for i in 1..1000u32 {
                let cand = backup_root.join(format!("{}.windowsclear_bak_{}_{}", name, ts, i));
                if !cand.exists() {
                    backup = cand;
                    break;
                }
            }
        }

        logger::log(&format!(
            "place_junction backup {:?} -> {:?}",
            source, backup
        ));
        if let Ok(meta) = fs::symlink_metadata(source) {
            if meta.file_type().is_symlink() {
                logger::log(&format!(
                    "source is symlink, remove and relink {:?}",
                    source
                ));
                let _ = fs::remove_dir(source);
                let _ = fs::remove_file(source);
                return Self::create_junction(source, target)
                    .context("创建链接失败")
                    .map(|_| 0);
            }
        }

        let mut rename_ok = false;
        let mut last_err: Option<std::io::Error> = None;

        for _ in 0..3 {
            match fs::rename(source, &backup) {
                Ok(_) => {
                    rename_ok = true;
                    break;
                }
                Err(e) => {
                    last_err = Some(e);
                    if let Some(err) = last_err.as_ref() {
                        if err.kind() == ErrorKind::CrossesDevices {
                            break;
                        }
                    }
                    thread::sleep(Duration::from_millis(200));
                }
            }
        }

        if !rename_ok {
            let raw = last_err.as_ref().and_then(|e| e.raw_os_error());
            let permission_denied = last_err
                .as_ref()
                .map(|e| e.kind() == ErrorKind::PermissionDenied)
                .unwrap_or(false)
                || raw == Some(5);
            if permission_denied {
                if let Some(parent) = source.parent() {
                    let mut in_parent = parent.join(format!("{}.windowsclear_bak_{}", name, ts));
                    if in_parent.exists() {
                        for i in 1..1000u32 {
                            let cand =
                                parent.join(format!("{}.windowsclear_bak_{}_{}", name, ts, i));
                            if !cand.exists() {
                                in_parent = cand;
                                break;
                            }
                        }
                    }
                    logger::log(&format!(
                        "backup permission denied, retry in parent {:?} -> {:?}",
                        source, in_parent
                    ));
                    backup = in_parent;
                    last_err = None;
                    for _ in 0..3 {
                        match fs::rename(source, &backup) {
                            Ok(_) => {
                                rename_ok = true;
                                break;
                            }
                            Err(e) => {
                                last_err = Some(e);
                                thread::sleep(Duration::from_millis(200));
                            }
                        }
                    }
                }
            }
        }

        if !rename_ok && auto_kill {
            let pids = ProcMgr::check_locking_processes_dir(source).unwrap_or_default();
            if !pids.is_empty() {
                logger::log(&format!("auto_kill before backup rename pids={:?}", pids));
                for pid in &pids {
                    let _ = ProcMgr::kill_process(*pid);
                }
                thread::sleep(Duration::from_millis(300));
                last_err = None;
                for _ in 0..3 {
                    match fs::rename(source, &backup) {
                        Ok(_) => {
                            rename_ok = true;
                            break;
                        }
                        Err(e) => {
                            last_err = Some(e);
                            thread::sleep(Duration::from_millis(200));
                        }
                    }
                }
            }
        }
        if !rename_ok {
            if let Some(err) = last_err.as_ref() {
                if err.kind() == ErrorKind::CrossesDevices {
                    let parent = source.parent().ok_or_else(|| anyhow!("无效的源路径"))?;
                    let mut in_parent = parent.join(format!("{}.windowsclear_bak_{}", name, ts));
                    if in_parent.exists() {
                        for i in 1..1000u32 {
                            let cand =
                                parent.join(format!("{}.windowsclear_bak_{}_{}", name, ts, i));
                            if !cand.exists() {
                                in_parent = cand;
                                break;
                            }
                        }
                    }
                    logger::log(&format!(
                        "backup crosses devices, retry in parent {:?} -> {:?}",
                        source, in_parent
                    ));
                    backup = in_parent;
                    last_err = None;
                    for _ in 0..3 {
                        match fs::rename(source, &backup) {
                            Ok(_) => {
                                rename_ok = true;
                                break;
                            }
                            Err(e) => {
                                last_err = Some(e);
                                thread::sleep(Duration::from_millis(200));
                            }
                        }
                    }
                }
            }
        }
        if !rename_ok {
            let pids = ProcMgr::check_locking_processes_dir(source).unwrap_or_default();
            logger::log(&format!(
                "backup rename failed source={:?} backup={:?} pids={:?}",
                source, backup, pids
            ));
            let detail = last_err
                .map(|e| e.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            return Err(anyhow!(
                "备份源目录失败: {}。可能被占用，相关进程 PID: {:?}",
                detail,
                pids
            ));
        }

        match Self::create_junction(source, target) {
            Ok(_) => {
                logger::log(&format!("junction created {:?} -> {:?}", source, target));
                let mut lb = 0;
                if let Err(e) = fs::remove_dir_all(&backup) {
                    logger::log(&format!("remove backup failed {:?}: {}", backup, e));
                    lb = crate::core::scanner::Scanner::get_dir_size(&backup);
                } else {
                    logger::log(&format!("remove backup ok {:?}", backup));
                }
                Ok(lb)
            }
            Err(e) => {
                logger::log(&format!("junction create failed: {}", e));
                let _ = fs::remove_dir_all(source);
                fs::rename(&backup, source).context("恢复源目录失败")?;
                Err(e)
            }
        }
    }

    fn copy_file_with_progress<F>(
        src: &Path,
        dst: &Path,
        progress_cb: F,
        pause_signal: Arc<AtomicBool>,
    ) -> Result<()>
    where
        F: Fn(u64) + Clone + Send + Sync,
    {
        let mut reader = BufReader::with_capacity(1024 * 1024, fs::File::open(src)?); // 1MB Buffer
        let mut writer = BufWriter::with_capacity(1024 * 1024, fs::File::create(dst)?);

        let mut buffer = [0u8; 1024 * 1024]; // 1MB Chunk
        loop {
            while pause_signal.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(100));
            }
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            writer.write_all(&buffer[..n])?;
            progress_cb(n as u64);
        }
        writer.flush()?;

        if let Ok(meta) = fs::metadata(src) {
            if let Ok(modified) = meta.modified() {
                let _ =
                    filetime::set_file_mtime(dst, filetime::FileTime::from_system_time(modified));
            }
        }

        Ok(())
    }

    // Fallback for simple copy if needed (not used in parallel logic)
    #[allow(dead_code)]
    fn move_dir_back(src: &Path, dst: &Path) -> Result<()> {
        if fs::rename(src, dst).is_ok() {
            return Ok(());
        }
        let dummy_signal = Arc::new(AtomicBool::new(false));
        Self::copy_dir_all_sequential(src, dst, |_| {}, dummy_signal)?;
        fs::remove_dir_all(src)?;
        Ok(())
    }

    fn create_junction(link: &Path, target: &Path) -> Result<()> {
        let output = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()?;

        if !output.status.success() {
            let err_msg = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("mklink 失败: {}", err_msg));
        }
        Ok(())
    }

    pub fn restore_migration<F>(
        record: &MigrationRecord,
        progress_cb: F,
        pause_signal: Arc<AtomicBool>,
        parallelism: usize,
        auto_kill: bool,
    ) -> Result<()>
    where
        F: Fn(u64) + Clone + Send + Sync + 'static,
    {
        logger::log(&format!(
            "restore_migration start for {:?}",
            record.source_path
        ));

        let source = &record.source_path;
        let target = &record.target_path;

        if !target.exists() {
            return Err(anyhow!(
                "移动目标文件 {} 已经不存在，无法自动还原。建议直接移除失效快捷方式",
                target.display()
            ));
        }

        if auto_kill {
            if let Ok(pids) = ProcMgr::check_locking_processes_dir(source) {
                for pid in pids {
                    let _ = ProcMgr::kill_process(pid);
                }
            }
        }

        // 1. Remove junction
        if source.exists() {
            if fs::symlink_metadata(source)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
            {
                // Remove junction carefully
                if let Err(e) = fs::remove_dir(source).or_else(|_| fs::remove_file(source)) {
                    return Err(anyhow!("解除原来的连接失败: {}", e));
                }
            } else if source.is_dir() {
                return Err(anyhow!(
                    "还原位置 {} 已包含非链接目录，为保护数据取消还原。请先手动清理",
                    source.display()
                ));
            }
        }

        // Make sure source parent exists
        if let Some(parent) = source.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).context("无法创建原路径父目录")?;
            }
        }

        // 2. Move files back
        if fs::rename(target, source).is_ok() {
            logger::log(&format!(
                "Restore rename success {:?} -> {:?}",
                target, source
            ));
            HistoryManager::remove_record(&record.id);
            return Ok(());
        }

        logger::log(&format!(
            "Restore rename failed {:?} -> {:?}, fallback to copy",
            target, source
        ));

        let partial_restore = source.with_extension("partial_restore");
        if partial_restore.exists() {
            let _ = fs::remove_dir_all(&partial_restore);
        }

        fs::create_dir_all(&partial_restore).context("无法创建临时还原目录")?;

        if let Err(e) = Self::copy_dir_all_auto(
            target,
            &partial_restore,
            progress_cb,
            pause_signal,
            parallelism,
        ) {
            logger::log(&format!("Restore copy failed: {}", e));
            let _ = fs::remove_dir_all(&partial_restore);
            return Err(e.context("文件复制回原路径失败"));
        }

        if let Err(e) = fs::rename(&partial_restore, source) {
            return Err(anyhow::Error::new(e).context("整理被还原的目录失败"));
        }

        logger::log(&format!(
            "Restore manual copy ok {:?} -> {:?}",
            target, source
        ));

        if let Err(e) = fs::remove_dir_all(target) {
            logger::log(&format!(
                "Failed to clean up target after restore {:?}: {}",
                target, e
            ));
            // Don't fail the whole restore if cleanup fails, the data is back in the correct spot
        }

        HistoryManager::remove_record(&record.id);
        Ok(())
    }
}
