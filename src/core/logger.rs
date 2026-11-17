use directories::ProjectDirs;
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_FILE: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

fn log_path() -> Option<PathBuf> {
    let proj = ProjectDirs::from("com", "tanaer", "WindowsClear")?;
    let dir = proj.cache_dir().join("logs");
    let _ = create_dir_all(&dir);
    Some(dir.join("windowsclear.log"))
}

pub fn init() {
    if LOG_FILE.get().is_some() {
        return;
    }
    let Some(path) = log_path() else {
        return;
    };
    let Ok(file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = LOG_FILE.set(Mutex::new(file));
}

pub fn log(message: &str) {
    init();
    let Some(lock) = LOG_FILE.get() else {
        return;
    };
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if let Ok(mut file) = lock.lock() {
        let _ = writeln!(file, "[{}] {}", ts, message.replace('\n', "\\n"));
        let _ = file.flush();
    }
}

pub fn log_file_path_string() -> String {
    log_path()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}
