use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanSource {
    pub enabled: bool,
    pub label: String,
    pub path: PathBuf,
    pub target_subdir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub version: u32,
    pub target_root: PathBuf,
    pub scan_sources: Vec<ScanSource>,
}

impl AppConfig {
    const VERSION: u32 = 1;

    pub fn config_path() -> Result<PathBuf> {
        let exe = std::env::current_exe().context("无法定位当前可执行文件路径")?;
        let dir = exe
            .parent()
            .ok_or_else(|| anyhow!("无法定位可执行文件目录"))?;
        Ok(dir.join("config.json"))
    }

    pub fn load_or_create() -> Result<Self> {
        let path = Self::config_path()?;
        if path.exists() {
            let raw = std::fs::read_to_string(&path).context("读取配置文件失败")?;
            match serde_json::from_str::<Self>(&raw) {
                Ok(mut cfg) => {
                    if cfg.version != Self::VERSION {
                        cfg.version = Self::VERSION;
                        let _ = cfg.save();
                    }
                    return Ok(cfg);
                }
                Err(_e) => {
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let backup = path.with_extension(format!("invalid_{}.json", ts));
                    let _ = std::fs::rename(&path, backup);
                    return Self::create_default_and_save();
                }
            }
        }
        Self::create_default_and_save()
    }

    fn create_default_and_save() -> Result<Self> {
        let cfg = Self::default_config();
        cfg.save()?;
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        let content = serde_json::to_string_pretty(self).context("序列化配置失败")?;
        std::fs::write(path, content).context("写入配置文件失败")?;
        Ok(())
    }

    pub fn default_config() -> Self {
        let target_root = Self::default_target_root();
        let mut scan_sources: Vec<ScanSource> = Vec::new();

        if let Ok(p) = std::env::var("LOCALAPPDATA") {
            scan_sources.push(ScanSource {
                enabled: true,
                label: "LocalAppData".to_string(),
                path: PathBuf::from(p),
                target_subdir: "Local".to_string(),
            });
        }
        if let Ok(p) = std::env::var("APPDATA") {
            scan_sources.push(ScanSource {
                enabled: true,
                label: "RoamingAppData".to_string(),
                path: PathBuf::from(p),
                target_subdir: "Roaming".to_string(),
            });
        }

        if let Ok(p) = std::env::var("ProgramFiles") {
            scan_sources.push(ScanSource {
                enabled: true,
                label: "Program Files".to_string(),
                path: PathBuf::from(p),
                target_subdir: "Program Files".to_string(),
            });
        }
        if let Ok(p) = std::env::var("ProgramFiles(x86)") {
            scan_sources.push(ScanSource {
                enabled: true,
                label: "Program Files (x86)".to_string(),
                path: PathBuf::from(p),
                target_subdir: "Program Files (x86)".to_string(),
            });
        }
        if let Ok(p) = std::env::var("ProgramData") {
            scan_sources.push(ScanSource {
                enabled: true,
                label: "ProgramData".to_string(),
                path: PathBuf::from(p),
                target_subdir: "ProgramData".to_string(),
            });
        }

        Self {
            version: Self::VERSION,
            target_root,
            scan_sources,
        }
    }

    fn default_target_root() -> PathBuf {
        let mut best_free: u64 = 0;
        let mut best_drive: Option<String> = None;
        for letter in b'C'..=b'Z' {
            let drive = format!("{}:\\", letter as char);
            let mut wide: Vec<u16> = drive.encode_utf16().collect();
            wide.push(0);
            unsafe {
                let dtype = GetDriveTypeW(PCWSTR(wide.as_ptr()));
                if dtype != 3 {
                    continue;
                }

                let mut free: u64 = 0;
                let mut total: u64 = 0;
                let mut total_free: u64 = 0;
                let ok = GetDiskFreeSpaceExW(
                    PCWSTR(wide.as_ptr()),
                    Some(&mut free),
                    Some(&mut total),
                    Some(&mut total_free),
                )
                .as_bool();
                if ok && free >= best_free {
                    best_free = free;
                    best_drive = Some(drive);
                }
            }
        }
        let base = best_drive.unwrap_or_else(|| "D:\\".to_string());
        PathBuf::from(base).join("Yugongyipan")
    }

    pub fn add_custom_scan_dir(&mut self, path: &Path) {
        let label = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Custom")
            .to_string();
        let mut target_subdir = label.clone();
        let mut i = 2u32;
        while self
            .scan_sources
            .iter()
            .any(|s| s.target_subdir.eq_ignore_ascii_case(&target_subdir))
        {
            target_subdir = format!("{}_{}", label, i);
            i += 1;
        }
        self.scan_sources.push(ScanSource {
            enabled: true,
            label,
            path: path.to_path_buf(),
            target_subdir,
        });
    }
}
