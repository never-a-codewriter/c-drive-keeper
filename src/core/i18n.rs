use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Chinese,
    English,
}

pub struct I18n {
    lang: Language,
    en_map: HashMap<&'static str, &'static str>,
}

impl I18n {
    fn new() -> Self {
        let mut en_map = HashMap::new();
        // UI Labels
        en_map.insert("AppData Mover", "AppData Mover");
        en_map.insert("扫描大文件夹", "Scan Large Folders");
        en_map.insert("正在初始化扫描...", "Initializing scan...");
        en_map.insert("执行迁移", "Move Folders");
        en_map.insert(
            "请先勾选需要迁移的文件夹",
            "Please select folders to move first",
        );
        en_map.insert("目标根目录:", "Target Root:");
        en_map.insert("选择...", "Browse...");
        en_map.insert("扫描目录:", "Scan folders:");
        en_map.insert("添加...", "Add...");
        en_map.insert(
            "自动结束占用进程 (慎用)",
            "Auto-kill locking processes (Use with caution)",
        );
        en_map.insert("打开日志", "Open Log");
        en_map.insert(
            "使用步骤：1 扫描大文件夹 → 2 勾选目录 → 3 执行迁移",
            "Steps: 1 Scan → 2 Select → 3 Move",
        );
        en_map.insert("并发传输", "Parallel copy");
        en_map.insert(
            "建议 SSD 硬盘使用，机械硬盘可能变慢或卡顿。",
            "Recommended for SSD. HDD may become slower or stutter.",
        );
        en_map.insert(
            "正在全盘扫描，请耐心等待...",
            "Full scanning, please wait...",
        );
        en_map.insert("准备就绪", "Ready");

        // Progress Messages
        en_map.insert("正在扫描: {} ({}/{})", "Scanning: {} ({}/{})");
        en_map.insert(
            "扫描完成，共找到 {} 个大文件夹",
            "Scan complete, found {} large folders",
        );
        en_map.insert("扫描出错: {}", "Scan error: {}");
        en_map.insert(
            "准备迁移... 总大小: {}",
            "Preparing to move... Total size: {}",
        );
        en_map.insert(
            "正在迁移... {:.1}% - {}/s - 剩余约 {} ",
            "Moving... {:.1}% - {}/s - ETA {} ",
        );
        en_map.insert("所有迁移任务完成", "All tasks completed");
        en_map.insert("错误: {}", "Error: {}");
        en_map.insert("速度: {}/s", "Speed: {}/s");
        en_map.insert("剩余时间: {}s", "Time left: {}s");
        en_map.insert("剩余时间: {}", "Time left: {}");

        en_map.insert("暂停", "Pause");
        en_map.insert("继续", "Resume");
        en_map.insert("已暂停", "Paused");

        Self {
            lang: Language::Chinese, // Default
            en_map,
        }
    }

    pub fn set_lang(&mut self, lang: Language) {
        self.lang = lang;
    }

    pub fn t<'a>(&self, key: &'a str) -> &'a str {
        match self.lang {
            Language::Chinese => key,
            Language::English => {
                if let Some(val) = self.en_map.get(key) {
                    val
                } else {
                    key
                }
            }
        }
    }
}

static I18N: OnceLock<Mutex<I18n>> = OnceLock::new();

pub fn get_i18n() -> std::sync::MutexGuard<'static, I18n> {
    I18N.get_or_init(|| Mutex::new(I18n::new())).lock().unwrap()
}

pub fn t(key: &str) -> String {
    get_i18n().t(key).to_string()
}
