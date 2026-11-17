# C Drive Keeper Desktop 0.2.1

## 路径适配

AppData 与公共系统目录从 Windows 读取；自动目标不依赖 D 盘。旧配置自动升级，换电脑后停用自定义扫描源。详见 PORTABILITY.md。

## 这次真正打通的流程

默认首页是“空间处理计划”。一次扫描把 AppData 应用目录、用户隐藏工具目录与缓存规则关联：每个目录展示逻辑总量、满足年龄条件的可清缓存、处理理由和访问错误，再选择“保留 / 清缓存 / 迁移整个目录”。同一目录只能选择一种方案。

执行前阻止重复和父子路径冲突、链接路径、源与备份目录重叠。清理绑定预览文件清单：预览后新增或变更的文件被跳过。执行逐项保存 plan.json、started.json、results.json 和 summary.json；区分逻辑大小与 C 盘实际可用空间变化，后者也受其他程序影响。

扫描取消了旧的目录时间缓存，每次重新读取。读取失败不能当成零；发现联接或访问错误的目录不允许整体处理。逻辑大小没有按硬链接或压缩文件去重，界面明确标明，不能把它理解成可释放物理空间。

## 清理与恢复

已知缓存包括临时目录、崩溃转储、SquirrelTemp、AMD DX9Cache，以及 Kiro / Code 的缓存、扩展安装包和日志。默认保留最近 7 天文件，普通文件复制到目标盘并验证 SHA256 后删除。D 盘开发缓存及回收站仍可选择；回收站没有备份。D 盘缓存备份仍放 D 盘不会净释放 D 盘空间。

“操作记录与恢复”中可选择 action-N/report.json 恢复清理备份。恢复先验证备份，不覆盖现在已存在的源文件，原备份保留。

## 迁移与失败处理

迁移要求关闭相应软件，拒绝自动结束进程、同盘迁移、系统安装目录及已存在目标。先检查目标可用空间；把源改名为同目录下的 .keeper-original-UUID，再复制到新目标，逐文件比较内容，最后创建和验证联接。不会在根联接失败后继续切换部分子目录。阶段日志存放于目标目录；完成前先记录可还原信息，再清理已核验的源副本。

失败时尽量恢复源名称；如果应用重新创建源位置或恢复失败，保留原始副本与目标副本并报告路径。程序崩溃后，若原路径缺失，可在恢复面板选择 *.prepared.json 或 *.source-renamed.json，恢复旁边的原始备份；存在源路径时拒绝覆盖。

“迁移记录”支持还原已完成迁移；还原复制并校验后解除联接，目标盘副本有意保留，不自动删除。记录保存在 EXE 旁的 migration-history.json。v0.1 上游的 AppData 历史文件不会自动导入，旧历史不要删除。

普通文件流的内容有校验；检测到加密、稀疏、离线文件或文件备用数据流时拒绝迁移。仍不是 NTFS 全保真备份，ACL、目录备用流、硬链接关系等没有完整保留。因此系统、共享组件等目录排除，活跃程序数据不应迁移。不能把单元测试视为所有第三方软件的兼容性保证。

## 保留的功能

高级“扫描与迁移”和“迁移记录”入口保留；缓存专用页保留。原项目的应用清单、开发缓存重定向方案、其他缓存重定向方案、按软件名查残留可在统一页运行诊断预览，输出 console.log。配置修改和残留删除没有自动批量接管，完整脚本随源码保留。不会自动卸载软件。

## 使用

解压便携包，运行 c-drive-keeper-desktop.exe。系统目录访问需要管理员时，关闭后右键“以管理员身份运行”；首页显示实际权限。先在高级迁移页选择目标根目录，再回首页扫描、选方案、核对并执行。执行中关闭窗口会被阻止，避免中途打断；强制终止或断电仍需通过阶段记录恢复。

## 验证与边界

- Rust 16 项测试：路径边界、父子计划冲突、实际 Windows 联接拒绝、读取失败、内容变化、目标冲突、锁定文件、联接失败保留/恢复原数据。
- PowerShell 5.1：预览、年龄过滤、备份边界、复制校验、预览后变更/新增文件保护、占用失败、恢复不覆盖、中断迁移恢复。
- 桌面实测：启动、中文界面、真实只读扫描、目录方案选择、未确认不可执行。未用真实用户应用数据测试跨盘迁移成功或管理员清空回收站。
- 阶段恢复测试使用隔离文件模拟中断；不宣称经过断电测试。

## 构建

Windows Rust MSVC + Visual Studio C++ Build Tools：

    cargo test --release --locked
    powershell -NoProfile -ExecutionPolicy Bypass -File tests/cleanup-smoke.ps1
    powershell -NoProfile -ExecutionPolicy Bypass -File tests/v2-regressions.ps1
    powershell -NoProfile -ExecutionPolicy Bypass -File tests/recovery-smoke.ps1
    cargo build --release --locked

CLI 清理变更：执行必须传入之前预览的 -PreviewPath。

    powershell -File scripts/clean.ps1 -ReportPath D:\preview.json
    powershell -File scripts/clean.ps1 -Apply -Only temp -PreviewPath D:\preview.json -BackupRoot D:\backups -ReportPath D:\result.json

## 来源

c-drive-keeper 基线 c455feeedb8e94b5ae8fdfe66b2eb30c46c1ef0e，MIT 见 LICENSE。
WindowsClear 基线 1642e7d972fe2f46f5c51052893518dee8efb314，MIT 见 licenses/WindowsClear-MIT.txt。
本地新版重写扫描、迁移与记录实现，增加统一计划和恢复入口，保留上游署名及许可证。

