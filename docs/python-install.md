# 把 Python 和 Node.js 装到非系统盘，并且只留一份

目标：Node.js、Python、npm 缓存、npm 全局包、uv 缓存全部在 D 盘，C 盘不再被开发环境占用。

下文以 `D:\DevTools` 存放程序、`D:\cache` 存放缓存为例，路径建议全部用英文。

## 1. Node.js

在"设置 -> 应用"里卸载 C 盘那份，重新运行安装包，在选择路径那一步改成 `D:\DevTools\nodejs`。

## 2. 缓存和全局包目录

运行本项目的脚本，一步搞定 npm、pip、uv 三者：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\relocate-dev-caches.ps1 -Root D:\cache -Apply -UseSystemPythonOnly
```

之前用 `npm install -g` 装过的命令需要重新安装。

## 3. Python

### 方案 A：传统 exe 安装包（能直接选路径）

python.org 的传统安装包会在 **3.16 起停止发布**，在此之前（例如 3.14、3.15）仍然有。下载 "Windows installer (64-bit)"，然后：

1. 第一页选 **Customize installation**，不要点 Install Now。
2. 第三页（Advanced Options）勾上 **Add Python to environment variables**，路径改成 `D:\DevTools\Python314`。

### 方案 B：新版 Python 安装管理器（pymanager）

新版管理器**没有选路径的安装界面**。它只是个小工具，Python 是之后用 `py install` 命令装的，路径通过配置文件指定：

```powershell
New-Item -ItemType Directory "$env:APPDATA\Python" -Force
'{ "install_dir": "D:\\DevTools\\Python" }' | Set-Content "$env:APPDATA\Python\pymanager.json" -Encoding ascii
py install 3.14
```

注意事项：

- 下载页上的 **MSIX** 是普通用户该选的；MSI 是给管理员批量部署的，官方说明使用前要先看文档。
- 配置文件必须在装 Python **之前**写好。已经装在 C 盘的，先 `py list` 查看，再 `py uninstall 版本号` 卸载后重装。
- 如果 `py install` 报"拒绝访问"或"找不到文件"，见 [排错文档](troubleshooting.md)，这时直接改用方案 A 更省事。

### 版本选择

刚发布的新大版本（例如 3.15）可能有一批带编译扩展的包还没有预编译包。如果主要用来跑 AI 客户端和 MCP 工具，建议先用上一个稳定版（例如 3.14）。

## 4. 让 uv 只用这一份 Python

`relocate-dev-caches.ps1 -UseSystemPythonOnly` 已经设置了：

- `UV_PYTHON_PREFERENCE=only-system`：只使用系统里已安装的 Python
- `UV_PYTHON_DOWNLOADS=never`：不再自己下载 Python

uv 之前自己下载的 Python 在 `C:\Users\用户名\.local\share\uv\python` 或 `%APPDATA%\uv\python`，确认用不到后删掉。

## 5. 关掉商店的假 Python

"设置 -> 应用 -> 应用执行别名"，把 `python.exe` 和 `python3.exe` 两项**关闭**。否则输入 `python` 可能弹出 Microsoft Store，或者抢在真 Python 前面。

## 6. 验证

关掉所有终端，**新开**一个 PowerShell，并先切到别的目录（当前目录里同名文件会干扰 `where.exe`）：

```powershell
cd D:\
where.exe node
where.exe python
npm.cmd config get cache
npm.cmd config get prefix
echo $env:UV_CACHE_DIR
uv python find
```

预期：`node` 和 `python` 都指向 D 盘，npm 的两项分别是 `D:\cache\npm` 和 `D:\cache\npm-global`，`uv python find` 输出 D 盘那份 Python。也可以直接运行 `scripts\audit.ps1`，最后一部分会检查这些。

最后，把 AI 客户端和 IDE **完全退出再打开**（包括托盘图标），它们才会读到新的 PATH 和环境变量。
