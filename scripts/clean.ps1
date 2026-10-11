<#
.SYNOPSIS
  清理 C 盘上"安全可再生"的内容：临时文件、崩溃转储、uv/npm/pip 缓存、回收站。
  默认只预览（显示每一项能释放多少），加 -Apply 才真正删除。
.DESCRIPTION
  不会碰：浏览器数据、Downloads、Desktop、聊天记录、WinSxS、Windows\Installer、System32。
.PARAMETER Apply
  真正执行清理。不加这个参数时只预览。
.PARAMETER Deep
  额外执行 DISM 系统组件清理（需要管理员，可能要几分钟）。
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\clean.ps1            # 预览
  powershell -ExecutionPolicy Bypass -File .\scripts\clean.ps1 -Apply     # 执行
  powershell -ExecutionPolicy Bypass -File .\scripts\clean.ps1 -Apply -Deep
#>
param(
    [switch]$Apply,
    [switch]$Deep
)

$ErrorActionPreference = 'SilentlyContinue'
$SysDrive = $env:SystemDrive

# ---------- 工具函数 ----------
function Get-DirSize([string]$Path) {
    if (-not $Path -or -not (Test-Path -LiteralPath $Path)) { return [int64]0 }
    $sum = (Get-ChildItem -LiteralPath $Path -Recurse -Force -File -ErrorAction SilentlyContinue |
            Measure-Object -Property Length -Sum).Sum
    if ($null -eq $sum) { return [int64]0 }
    return [int64]$sum
}

function Format-Size([int64]$Bytes) {
    if ($Bytes -ge 1GB) { return ('{0:N2} GB' -f ($Bytes / 1GB)) }
    if ($Bytes -ge 1MB) { return ('{0:N0} MB' -f ($Bytes / 1MB)) }
    return ('{0:N0} KB' -f ($Bytes / 1KB))
}

function Get-FreeGB { return [math]::Round((Get-PSDrive -Name $SysDrive.TrimEnd(':')).Free / 1GB, 2) }

# 只清空目录"里面的内容"，不删目录本身；拒绝处理过短的路径（防止误删根目录）
function Clear-DirContents([string]$Path) {
    if (-not $Path -or $Path.Length -lt 8) { return }
    if (-not (Test-Path -LiteralPath $Path)) { return }
    Remove-Item -Path (Join-Path $Path '*') -Recurse -Force -ErrorAction SilentlyContinue
}

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
           ).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

$script:totalBytes = [int64]0
$script:reportedPaths = @()

function Report([string]$Name, [string]$Path, [int64]$Bytes, [string]$Note) {
    $script:totalBytes += $Bytes
    if ($Path) { $script:reportedPaths += $Path }
    $line = "  {0,-22} {1,10}   {2}" -f $Name, (Format-Size $Bytes), $Path
    if ($Note) { $line += "  ($Note)" }
    Write-Host $line
}

# ---------- 开始 ----------
$before = Get-FreeGB
if ($Apply) {
    Write-Host "`n=== 执行清理（-Apply）| $SysDrive 剩余 $before GB ===" -ForegroundColor Cyan
} else {
    Write-Host "`n=== 预览模式：只统计，不删除（要执行请加 -Apply）| $SysDrive 剩余 $before GB ===" -ForegroundColor Cyan
}
if (-not $isAdmin) {
    Write-Host "提示：当前不是管理员，系统 Temp 和 DISM 会被跳过。" -ForegroundColor Yellow
}
Write-Host ""

# 1. 用户 Temp
$p = $env:TEMP
Report '用户 Temp' $p (Get-DirSize $p) ''
if ($Apply) { Clear-DirContents $p }

# 2. 系统 Temp（需管理员）
$p = "$env:SystemRoot\Temp"
if ($isAdmin) {
    Report '系统 Temp' $p (Get-DirSize $p) ''
    if ($Apply) { Clear-DirContents $p }
} else {
    Write-Host ("  {0,-22} {1,10}   {2}" -f '系统 Temp', '-', '跳过（需要管理员）') -ForegroundColor DarkGray
}

# 3. 崩溃转储
$p = "$env:LOCALAPPDATA\CrashDumps"
Report '崩溃转储' $p (Get-DirSize $p) ''
if ($Apply) { Clear-DirContents $p }

# 4. uv 缓存
if (Get-Command uv -ErrorAction SilentlyContinue) {
    $p = (& uv cache dir 2>$null | Select-Object -First 1)
    Report 'uv 缓存' $p (Get-DirSize $p) ''
    if ($Apply) { & uv cache clean 2>$null | Out-Null }
}

# 5. npm 缓存（用 npm.cmd，绕开 ps1 执行策略限制）
if (Get-Command npm.cmd -ErrorAction SilentlyContinue) {
    $p = (& npm.cmd config get cache 2>$null | Select-Object -Last 1)
    Report 'npm 缓存' $p (Get-DirSize $p) ''
    if ($Apply) { & npm.cmd cache clean --force 2>$null | Out-Null }
}

# 6. pip 缓存
$p = if ($env:PIP_CACHE_DIR) { $env:PIP_CACHE_DIR } else { "$env:LOCALAPPDATA\pip\Cache" }
if (Test-Path -LiteralPath $p) {
    Report 'pip 缓存' $p (Get-DirSize $p) ''
    if ($Apply) { Clear-DirContents $p }
}

# 6b. 迁移缓存之后，C 盘默认位置里遗留的旧缓存
$reported = @()
foreach ($r in $script:reportedPaths) { $reported += $r.TrimEnd('\') }
$legacy = @(
    @{ Name = '旧 uv 缓存';  Path = "$env:LOCALAPPDATA\uv\cache" },
    @{ Name = '旧 npm 缓存'; Path = "$env:LOCALAPPDATA\npm-cache" },
    @{ Name = '旧 pip 缓存'; Path = "$env:LOCALAPPDATA\pip\Cache" }
)
foreach ($l in $legacy) {
    if (-not (Test-Path -LiteralPath $l.Path)) { continue }
    if ($reported -contains $l.Path.TrimEnd('\')) { continue }   # 已经在上面处理过
    Report $l.Name $l.Path (Get-DirSize $l.Path) '迁移前遗留'
    if ($Apply) { Clear-DirContents $l.Path }
}

# 7. 回收站
if ($Apply) {
    Clear-RecycleBin -Force -ErrorAction SilentlyContinue
    Write-Host ("  {0,-22} {1,10}" -f '回收站', '已清空')
} else {
    Write-Host ("  {0,-22} {1,10}   {2}" -f '回收站', '-', '（预览不统计大小）') -ForegroundColor DarkGray
}

# 8. 深度：DISM
if ($Deep) {
    if (-not $isAdmin) {
        Write-Host ("  {0,-22} {1,10}   {2}" -f 'DISM 组件清理', '-', '跳过（需要管理员）') -ForegroundColor Yellow
    } elseif ($Apply) {
        Write-Host "`n正在执行 DISM 系统组件清理（可能要几分钟）..." -ForegroundColor Green
        & DISM.exe /Online /Cleanup-Image /StartComponentCleanup
    } else {
        Write-Host ("  {0,-22} {1,10}   {2}" -f 'DISM 组件清理', '-', '（预览不执行，通常可释放 1~3 GB）') -ForegroundColor DarkGray
    }
}

# ---------- 汇总 ----------
Write-Host ""
if ($Apply) {
    $after = Get-FreeGB
    Write-Host ("=== 完成：{0} 剩余 {1} GB -> {2} GB（释放约 {3} GB）===" -f $SysDrive, $before, $after, [math]::Round($after - $before, 2)) -ForegroundColor Cyan
    Write-Host "（回收站和 DISM 释放的空间有时要稍后才刷新）" -ForegroundColor DarkGray
} else {
    Write-Host ("=== 预览合计：约可释放 {0}（不含回收站和 DISM）===" -f (Format-Size $script:totalBytes)) -ForegroundColor Cyan
    Write-Host "确认无误后，加 -Apply 再运行一次。" -ForegroundColor Cyan
}

Write-Host "`n脚本不会处理、需要你手动做的："
Write-Host "  - 浏览器：Ctrl+Shift+Delete，只勾“缓存的图片和文件”"
Write-Host "  - 翻一遍 Downloads 和 Desktop"
Write-Host "  - 磁盘清理 -> 清理系统文件（Windows 更新清理、旧驱动包）"
Write-Host ""
