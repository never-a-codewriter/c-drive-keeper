use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationRecord {
    pub id: String,
    pub name: String,
    pub source_path: PathBuf,
    pub target_path: PathBuf,
    pub size_bytes: u64,
    pub timestamp: u64,
    pub batch_id: Option<String>,
    pub batch_timestamp: Option<u64>,
}

pub struct HistoryManager;

impl HistoryManager {
    fn history_file_path() -> Option<PathBuf> {
        let proj = ProjectDirs::from("com", "tanaer", "WindowsClear")?;
        let dir = proj.data_local_dir().to_path_buf();
        let _ = fs::create_dir_all(&dir);
        Some(dir.join("history.json"))
    }

    pub fn load_records() -> Vec<MigrationRecord> {
        let Some(path) = Self::history_file_path() else {
            return Vec::new();
        };
        if !path.exists() {
            return Vec::new();
        }
        match fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|_| Vec::new()),
            Err(_) => Vec::new(),
        }
    }

    pub fn save_records(records: &[MigrationRecord]) {
        let Some(path) = Self::history_file_path() else {
            return;
        };
        if let Ok(content) = serde_json::to_string_pretty(records) {
            let _ = fs::write(&path, content);
        }
    }

    pub fn add_record(record: MigrationRecord) {
        let mut records = Self::load_records();
        records.push(record);
        Self::save_records(&records);
    }

    pub fn remove_record(id: &str) {
        let mut records = Self::load_records();
        records.retain(|r| r.id != id);
        Self::save_records(&records);
    }
}
