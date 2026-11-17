# C Drive Keeper Desktop — 本地合并版

将 c-drive-keeper 的 PowerShell 清理后端接入 WindowsClear 的 Rust/egui 桌面界面。

## 功能入口

- 扫描与迁移：保留 WindowsClear 的目录扫描、跨盘迁移、目录联接和进度显示。
- 迁移记录：保留上游记录及还原功能。
- 缓存清理：新增页签，预览、勾选清理 ID、选择是否包含其他盘缓存和回收站，然后确认执行。使用同一份 scripts/clean.ps1 后端，无重复删除实现。
- 报告和备份写入界面所选目标根目录的 cleanup-reports / cleanup-backups。每次操作独立目录，普通文件先校验备份；回收站不备份。
- 清理进程不显示终端窗口，界面通过进程退出码和 JSON 报告显示结果。清理运行时阻止界面启动迁移。
- 清理脚本嵌入 EXE；分发 EXE 无需额外复制脚本。PowerShell 5.1 由 Windows 提供。

## 构建

在安装了 Rust MSVC 工具链及 Visual Studio C++ Build Tools 的 Windows 环境：

```powershell
cargo check --all-targets
cargo build --release
powershell -NoProfile -ExecutionPolicy Bypass -File tests/cleanup-smoke.ps1
```

程序输出 target/release/c-drive-keeper-desktop.exe。仓库包含 Windows CI 配置，但本地未上传或触发 CI。

## 本次修正

WindowsClear 扫描缓存判断从“未过期或目录时间相同”改为“未过期且目录时间相同”，避免目录时间不变时永久复用旧数据。10 分钟缓存窗口内，深层文件变化仍可能不立即体现。

## 当前验证范围与限制

PowerShell 临时目录测试与语法检查通过。当前主机没有 cargo/rustc，桌面源码尚未编译和 GUI 实测，不能当作已验证的成品 EXE。

迁移及还原算法沿用 WindowsClear，本次没有重新实现其事务性、内容校验或并发写入保护。迁移仍需关闭对应软件；不要迁移系统目录、正在运行的本程序或不理解用途的目录。自动结束进程默认关闭。未在真实用户数据上执行迁移测试。

清理说明及命令行兼容性变化见 LOCAL-CHANGES.md。原上游 README 保留作参考，以此文件及 LOCAL-CHANGES.md 为合并版说明。

## 来源与许可证

- c-drive-keeper: https://github.com/never-a-codewriter/c-drive-keeper
  基线提交 c455feeedb8e94b5ae8fdfe66b2eb30c46c1ef0e，许可证见 LICENSE。
- WindowsClear: https://github.com/tanaer/WindowsClear
  基线提交 1642e7d972fe2f46f5c51052893518dee8efb314，导入 src 与 Cargo.toml；许可证完整保存在 licenses/WindowsClear-MIT.txt。
- 新增 src/cleanup.rs，修改 src/main.rs、扫描缓存条件及包名。未导入上游广告、赞助链接或构建日志。
