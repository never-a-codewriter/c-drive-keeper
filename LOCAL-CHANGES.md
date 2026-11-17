# 本地改进版：用法变更

本文件优先于旧 README 中 clean.ps1 的示例。其他迁移、残留、注册表脚本尚未改造。

- 默认预览，显式 -Only 选择清理 ID；-Apply 必须提供 -BackupRoot。
- 清理范围增加 SquirrelTemp、AMD DX9Cache、Kiro/Code 缓存及日志。
- 默认只处理 7 天前文件，保留目录；拒绝目录链接，不跨链接递归。
- 每个文件复制备份、SHA256 校验后删除；失败保留并记入 JSON。运行中断时备份仍在，但最后一个项目的报告可能不完整。
- 读取失败显示 Failed/Partial，不当成零字节。目录大小为逻辑大小，不能直接等同可释放物理空间，尤其是硬链接。
- -IncludeRecycleBin 保留顺带清空所有盘回收站功能（不可恢复，不做备份）。
- -IncludeOtherDrives 将非系统盘的 npm、用户级 uv/pip 缓存加入候选；仍需 -Only 选择 ID。
- 备份不能位于待清理目录内。建议备份到另一盘；D 盘缓存备份仍放 D 盘时，不会净释放 D 盘空间。
- -Deep 不再混合执行系统修改，系统组件清理使用 Windows 自带工具。

```powershell
# 全部候选预览（包括 D 盘缓存和回收站提示）
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/clean.ps1 -IncludeOtherDrives -IncludeRecycleBin
# 按预览显示的 ID 执行
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/clean.ps1 -Apply -Only temp,installer-temp,kiro-cache,npm-cache -IncludeOtherDrives -IncludeRecycleBin -BackupRoot D:\cleanup-backups
# 测试：只在 tests 下生成临时文件
powershell -NoProfile -ExecutionPolicy Bypass -File tests/cleanup-smoke.ps1
```

JSON 的 Files 项记录 Source、Backup、SHA256。恢复时先关闭对应应用，把备份复制回 Source；不要覆盖后来产生的新文件。

测试覆盖预览不删除、文件年龄、含方括号的文件名、备份路径边界、备份校验与删除、近期文件保留。此主机禁止创建 junction，链接拒绝分支仅做代码检查；尚未验证管理员、回收站实际清空、锁定文件及并发改写场景。不要将此版本视为对所有软件残留的自动判定器。
