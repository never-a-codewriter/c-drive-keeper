# 常见问题与排错

## 运行 `.ps1` 提示"禁止运行脚本"

PowerShell 默认执行策略会拦截 `.ps1`。本项目的所有示例都用下面的形式，只对这一次运行有效：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\audit.ps1
```

想一劳永逸，可以只对当前用户放宽（会弹出确认，输入 Y）：

```powershell
Set-ExecutionPolicy RemoteSigned -Scope CurrentUser
```

## `npm` 提示"无法加载文件 npm.ps1，因为在此系统上禁止运行脚本"

同一个原因：PowerShell 里的 `npm` 实际调用的是 `npm.ps1`。临时办法是改用 `npm.cmd`，例如 `npm.cmd cache clean --force`。本项目的脚本内部已经统一使用 `npm.cmd`，不受影响。

## 用 `Measure-Object Length` 统计文件夹大小时一直报红字

空文件夹里没有文件，`Measure-Object` 找不到 `Length` 属性。统计大小时给 `Get-ChildItem` 加上 `-File`，只统计文件即可。本项目的脚本已经处理了这一点。

## `py install` 报"程序 py.exe 无法运行：系统无法访问此文件 / 拒绝访问"，`pymanager.exe` 提示找不到文件

这是新版 Python 安装管理器的 MSIX 包注册不完整：包装上了，但 `WindowsApps` 下的别名坏了，直接运行包目录里的 `pymanager.exe` 也会被拒绝访问。按顺序尝试：

1. 查看包状态：`Get-AppxPackage *Python* | Format-List Name, Version, Status, InstallLocation`
2. "设置 -> 应用 -> 已安装的应用 -> Python install manager -> 高级选项"，先点"修复"，再点"重置"，然后重开 PowerShell。
3. 卸载后重装：`Get-AppxPackage *Python* | Remove-AppxPackage`，然后 `winget install 9NQ7512CXL7T`。
4. 还不行就放弃管理器，改用传统 exe 安装包，见 [python-install.md](python-install.md) 的方案 A。

可能的原因包括系统版本偏旧、应用部署服务异常，或安全软件拦截了 WindowsApps 下的程序，不必深究。

## `where.exe python` 第一行是 `venv\Scripts\python.exe`，运行 `python` 报"No Python at ...Python38..."

PATH 里残留了一个虚拟环境的 `Scripts` 目录，而创建它的那份 Python 已经被卸载，虚拟环境成了"孤儿"。处理办法：

1. "系统属性 -> 高级 -> 环境变量"，在用户和系统的 Path 里删掉该虚拟环境的 `Scripts` 路径。
2. 把虚拟环境文件夹先重命名为 `venv.bak`，用几天确认没有影响再删。
3. 以后在项目目录里用 `uv venv` 新建，不要把虚拟环境加进全局 PATH。

## `where.exe python` 最前面出现当前目录下的一个同名文件

`where.exe` 会先查当前目录。切到别的目录再测，例如 `cd D:\`。如果是 0 字节的空文件，可以直接删除。

## 输入 `python` 弹出 Microsoft Store

商店的"应用执行别名"在作怪。"设置 -> 应用 -> 应用执行别名"，关闭 `python.exe` 和 `python3.exe`。

## 修改环境变量后，AI 客户端还是用旧的路径

环境变量只在进程启动时读取。把终端、AI 客户端和 IDE **完全退出**（包括托盘图标）再重新打开。必要时注销一次或重启。

## 清理之后 C 盘剩余空间没有变化

回收站和 DISM 释放的空间有时要稍后才刷新。`hiberfil.sys` 需要重启之后才会真正消失。

## 卸载杀毒或"优化"软件之后 C 盘还是很大

这类软件常留下很大的残留（组件、日志、更新包）。卸载后检查 `C:\ProgramData`、`C:\Program Files`、`AppData\Roaming` 和 `AppData\Local` 里有没有同名目录，确认不再需要后，用安全软件自带的"文件粉碎"或手动删除。不建议再用一键优化和注册表清理工具，它们往往是残留的来源。
