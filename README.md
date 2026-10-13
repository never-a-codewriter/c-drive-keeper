# c-drive-keeper

**Windows C 盘空间管家：先体检，再预览，最后才清理。** 专门应对 AI 客户端、MCP 工具、Python/Node 开发环境把 C 盘慢慢塞满的问题。

[English](README.en.md)

## Update on 2026/10/14
执行完所有脚本后，还需要手动操作：

1.系统自带的磁盘清理（通常能清出最多空间）：

按 Win+R，输入 cleanmgr，回车，选 C 盘。

🔜 点 “清理系统文件”，再选一次 C 盘。

🔜 勾上：Windows 更新清理、临时文件、传递优化文件、设备驱动程序包。“下载”那一项不要勾。

🔜 点确定。


2.几个软件内部的清理（可选）：

- Chrome / 夸克：按 Ctrl+Shift+Delete，只勾“缓存的图片和文件”。
- QQ / 微信：在设置 → 存储空间里清理缓存，或者把存储位置改到 D 盘。
- GitHub Desktop：打开 %LOCALAPPDATA%\GitHubDesktop，旧的 app-x.x.x 文件夹只留最新版本那个。

===================================================================================


## 为什么做这个

装了 Cherry Studio、Codex、Cursor、各种 MCP 工具之后，C 盘经常几个月内少掉几十 GB。原因很固定：

- 这类软件的数据目录默认在 `C:\Users\用户名\AppData` 和 `C:\Users\用户名\.xxx`，**不跟着安装路径走**，装在 D 盘也一样。
- 它们各自下载一份 Node、Python、uv、bun，重复占用。
- uv、npm、pip 的缓存默认都写在 C 盘，而且只增不减。
- 卸载软件不会清掉用户数据，残留一直留着。

本项目把"查、清、迁"三件事做成三个脚本，外加一份踩坑文档。

## 快速开始

需要 Windows 10/11 和自带的 PowerShell 5.1，不需要安装任何东西。

下载或 `git clone` 本项目后，在 PowerShell 里依次运行（路径按实际位置调整）。由于很多系统默认禁止运行 `.ps1`，下面统一用 `-ExecutionPolicy Bypass`，它只对这一次运行有效，不会改动系统设置。

```powershell
# 1. 体检：看空间花在哪里（只读，什么都不会改）
powershell -ExecutionPolicy Bypass -File .\scripts\audit.ps1

# 2. 预览能清多少（默认不删除）
powershell -ExecutionPolicy Bypass -File .\scripts\clean.ps1

# 3. 确认无误后，真正清理
powershell -ExecutionPolicy Bypass -File .\scripts\clean.ps1 -Apply

# 4. 把 uv / pip / npm 的缓存迁到 D 盘（先预览，再加 -Apply）
powershell -ExecutionPolicy Bypass -File .\scripts\relocate-dev-caches.ps1 -Root D:\cache
powershell -ExecutionPolicy Bypass -File .\scripts\relocate-dev-caches.ps1 -Root D:\cache -Apply

# 5.（可选）把 bun / yarn / pnpm / Playwright / HuggingFace 等其他缓存也迁过去，-MoveOld 顺便搬走 C 盘旧缓存
powershell -ExecutionPolicy Bypass -File .\scripts\relocate-more-caches.ps1 -Root D:\cache
powershell -ExecutionPolicy Bypass -File .\scripts\relocate-more-caches.ps1 -Root D:\cache -Apply -MoveOld
```

## 脚本说明

| 脚本 | 做什么 | 会修改系统吗 |
|---|---|---|
| `scripts/audit.ps1` | 显示 C 盘剩余空间、常见大户（休眠文件、Downloads、Temp 等）、AppData 和用户目录隐藏文件夹的 Top N，并检查 uv/npm/pip 缓存和 node/python 是否还指向 C 盘 | 否，只读 |
| `scripts/clean.ps1` | 清理用户 Temp、系统 Temp、崩溃转储、uv/npm/pip 缓存、回收站；加 `-Deep` 额外跑 DISM 组件清理 | 默认否（预览），加 `-Apply` 才删除 |
| `scripts/relocate-dev-caches.ps1` | 设置 `UV_CACHE_DIR`、`PIP_CACHE_DIR`、npm 的 cache 和 prefix 到指定目录，并把 npm 全局目录加入 PATH；加 `-UseSystemPythonOnly` 让 uv 只用系统里那一份 Python | 默认否（预览），加 `-Apply` 才修改 |
| `scripts/relocate-more-caches.ps1` | 用用户环境变量把 pip、bun、yarn、Playwright、Puppeteer、HuggingFace、Electron 的缓存指到指定目录；检测到 pnpm / Go / .NET / Deno 时一并处理。已指向非系统盘的设置不覆盖；加 `-MoveOld` 用 robocopy 把 C 盘旧缓存搬过去（搬，不删） | 默认否（预览），加 `-Apply` 才修改 |

## 安全原则

- **默认预览。** 会删东西或改设置的脚本，不加 `-Apply` 就只显示计划。
- **只动可再生的内容。** 临时文件、崩溃转储、包管理器缓存、回收站，删掉后会自动重建。
- **永远不会碰：** 浏览器数据、Downloads、Desktop、聊天软件数据、`WinSxS`、`Windows\Installer`、`System32`、`DriverStore`。
- 清理目录时只清空里面的内容，不删目录本身，并拒绝处理过短的路径。
- 脚本都是普通文本，可以先打开看一遍再运行。

## 脚本之外：需要手动做的

这些要么有风险，要么需要你判断，所以没有做进脚本：

| 项目 | 做法 |
|---|---|
| 休眠文件 `hiberfil.sys`（通常 3GB 以上） | 管理员 CMD 运行 `powercfg -h off`。代价是没有休眠、关闭快速启动，台式机基本无影响 |
| `WinSxS`（常有 5~10GB） | 不要手动删。管理员运行 `DISM /Online /Cleanup-Image /StartComponentCleanup`，或用 `clean.ps1 -Apply -Deep` |
| 旧显卡驱动、Windows 更新缓存、`Windows.old` | 磁盘清理（`cleanmgr`）-> 清理系统文件 |
| 浏览器缓存 | 浏览器里 `Ctrl+Shift+Delete`，只勾缓存，别勾密码和 Cookie |
| 聊天软件缓存 | 在软件设置里清理或改存储路径，不要直接删数据文件夹 |
| Downloads、Desktop | 自己翻一遍，安装包最占地方 |
| 卸载软件后的残留 | 检查 `C:\Users\用户名\.软件名`、`AppData\Local\软件名`、`AppData\Roaming\软件名`、`C:\ProgramData\软件名` |

## 推荐的维护节奏

| 频率 | 做什么 | 耗时 |
|---|---|---|
| 每月 | `clean.ps1 -Apply`，清浏览器缓存，翻 Downloads 和 Desktop | 约 5 分钟 |
| 每季度 | `audit.ps1` 看有没有新的大户；`clean.ps1 -Apply -Deep`；磁盘清理的"清理系统文件"；卸载半年没用的软件 | 约 20 分钟 |
| 每次卸载软件后 | 检查上表最后一行的残留位置 | 1 分钟 |
| 剩余低于 30GB | 不要等下个月，当天做一次季度清理 | |

想让每月清理自动运行，可以用计划任务（路径里不要有空格）：

```powershell
schtasks /Create /SC MONTHLY /D 1 /ST 10:00 /TN "CDriveKeeper" /TR "powershell.exe -NoProfile -ExecutionPolicy Bypass -File D:\tools\c-drive-keeper\scripts\clean.ps1 -Apply"
```

计划任务默认以普通权限运行，所以系统 Temp 和 DISM 会被跳过，其余照常清理。

## 文档

- [Python / Node 装到非系统盘](docs/python-install.md)：新版 Python 安装管理器怎么指定路径，以及怎么让 uv 只用一份 Python
- [常见问题与排错](docs/troubleshooting.md)：`npm.ps1 禁止运行脚本`、`py.exe 拒绝访问`、虚拟环境抢 PATH、`Measure-Object` 报错等

## 贡献

欢迎提 issue 和 PR，尤其是：

- 其他 AI 客户端的缓存和数据目录位置（Cursor、Zed、opencode、各种 MCP 运行环境等）
- 其他语言环境的包管理器缓存（Cargo、Go、Maven、Gradle、conda 等）
- 在不同 Windows 版本上的测试反馈

提交前请遵守安全原则：任何会删除内容的新功能，必须默认预览，并且只处理可再生的数据。

## 免责声明

清理操作可能不可逆。请先运行预览并阅读输出，重要数据请先备份。作者不对使用本项目造成的数据损失负责。

## 许可证

[MIT](LICENSE)
