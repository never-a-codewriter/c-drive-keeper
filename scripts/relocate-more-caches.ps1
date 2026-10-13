<#
.SYNOPSIS
  补充迁移：把 pip、bun、yarn、pnpm、Playwright、Puppeteer、HuggingFace、Go、NuGet、Deno、Electron
  的缓存都指到非系统盘（默认 D:\cache）。默认只预览，加 -Apply 才修改。
.DESCRIPTION
  只设置“用户级”环境变量（pnpm 用它自己的全局配置），不需要管理员。
  - 已经指向非系统盘的设置会保留，不覆盖。
  - 加 -MoveOld 时，会把 C 盘旧位置里的缓存搬到新位置（用 robocopy /MOVE），不会直接删除。
  - 不碰 npm / uv（之前已经迁好了），也不碰任何软件本体和 Python 安装。
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\relocate-more-caches.ps1
  powershell -ExecutionPolicy Bypass -File .\scripts\relocate-more-caches.ps1 -Apply
  powershell -ExecutionPolicy Bypass -File .\scripts\relocate-more-caches.ps1 -Apply -MoveOld
#>
param(
    [string]$Root = 'D:\cache',
    [switch]$Apply,
    [switch]$MoveOld
)

$ErrorActionPreference = 'Stop'
$SysDrive = $env:SystemDrive
$Root = $Root.TrimEnd('\')

$driveRoot = [System.IO.Path]::GetPathRoot($Root)
if (-not $driveRoot -or -not (Test-Path -LiteralPath $driveRoot)) {
    Write-Host "目标盘不存在：$Root" -ForegroundColor Red; exit 1
}
if ($Root.StartsWith($SysDrive, [StringComparison]::OrdinalIgnoreCase)) {
    Write-Host "目标路径 $Root 还在系统盘上，请换一个盘，例如 -Root D:\cache" -ForegroundColor Red; exit 1
}

function Test-Cmd([string]$Name) { return [bool](Get-Command $Name -ErrorAction SilentlyContinue) }

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

function Test-OnSystemDrive([string]$Path) {
    return ($Path -and $Path.StartsWith($SysDrive, [StringComparison]::OrdinalIgnoreCase))
}

# ---------- 迁移清单 ----------
# Var: 用户环境变量名；Pnpm: 用 pnpm 自己的配置；Old: C 盘上的默认位置；When: 什么情况下才处理
$items = @(
    @{ Name = 'pip 缓存';           Var = 'PIP_CACHE_DIR';           Dir = 'pip';           Old = "$env:LOCALAPPDATA\pip\Cache";            When = $true },
    @{ Name = 'bun 安装缓存';       Var = 'BUN_INSTALL_CACHE_DIR';   Dir = 'bun';           Old = "$env:USERPROFILE\.bun\install\cache";    When = $true },
    @{ Name = 'yarn 缓存';          Var = 'YARN_CACHE_FOLDER';       Dir = 'yarn';          Old = "$env:LOCALAPPDATA\Yarn\Cache";           When = $true },
    @{ Name = 'pnpm 存储';          Pnpm = $true;                    Dir = 'pnpm-store';    Old = "$env:LOCALAPPDATA\pnpm\store";           When = (Test-Cmd 'pnpm.cmd') },
    @{ Name = 'Playwright 浏览器';  Var = 'PLAYWRIGHT_BROWSERS_PATH'; Dir = 'ms-playwright'; Old = "$env:LOCALAPPDATA\ms-playwright";        When = $true },
    @{ Name = 'Puppeteer 浏览器';   Var = 'PUPPETEER_CACHE_DIR';     Dir = 'puppeteer';     Old = "$env:USERPROFILE\.cache\puppeteer";      When = $true },
    @{ Name = 'HuggingFace 模型';   Var = 'HF_HOME';                 Dir = 'huggingface';   Old = "$env:USERPROFILE\.cache\huggingface";    When = $true },
    @{ Name = 'Electron 下载缓存';  Var = 'ELECTRON_CACHE';          Dir = 'electron';      Old = "$env:LOCALAPPDATA\electron\Cache";       When = $true },
    @{ Name = 'Go 模块缓存';        Var = 'GOMODCACHE';              Dir = 'go\mod';        Old = "$env:USERPROFILE\go\pkg\mod";            When = (Test-Cmd 'go') },
    @{ Name = 'Go 编译缓存';        Var = 'GOCACHE';                 Dir = 'go\build';      Old = "$env:LOCALAPPDATA\go-build";             When = (Test-Cmd 'go') },
    @{ Name = 'NuGet 包缓存';       Var = 'NUGET_PACKAGES';          Dir = 'nuget';         Old = "$env:USERPROFILE\.nuget\packages";       When = (Test-Cmd 'dotnet') },
    @{ Name = 'Deno 缓存';          Var = 'DENO_DIR';                Dir = 'deno';          Old = "$env:LOCALAPPDATA\deno";                 When = (Test-Cmd 'deno') }
)

# ---------- 计划 ----------
if ($Apply) { Write-Host "`n=== 执行迁移（-Apply）| 目标 $Root ===" -ForegroundColor Cyan }
else        { Write-Host "`n=== 预览模式：只显示计划，不修改（要执行请加 -Apply）| 目标 $Root ===" -ForegroundColor Cyan }
Write-Host ""

$plan = @()
foreach ($it in $items) {
    if (-not $it.When) { continue }
    $target = Join-Path $Root $it.Dir

    if ($it.Pnpm) {
        $current = (& pnpm.cmd config get store-dir 2>$null | Select-Object -Last 1)
        if ($current -eq 'undefined') { $current = '' }
    } else {
        $current = [Environment]::GetEnvironmentVariable($it.Var, 'User')
    }

    $action = 'set'
    if ($current -and ($current.TrimEnd('\') -ieq $target)) { $action = 'same' }
    elseif ($current -and -not (Test-OnSystemDrive $current)) { $action = 'keep' }

    $oldSize = Get-DirSize $it.Old
    $plan += [pscustomobject]@{ Item = $it; Target = $target; Current = $current; Action = $action; OldSize = $oldSize }

    $label = if ($it.Pnpm) { 'pnpm store-dir' } else { $it.Var }
    switch ($action) {
        'same' { Write-Host ("  [已完成] {0,-18} {1} = {2}" -f $it.Name, $label, $target) -ForegroundColor DarkGray }
        'keep' { Write-Host ("  [保留]   {0,-18} {1} 已经是 {2}（不在系统盘，不改）" -f $it.Name, $label, $current) -ForegroundColor DarkGray }
        'set'  { Write-Host ("  [将设置] {0,-18} {1} = {2}" -f $it.Name, $label, $target) -ForegroundColor Green }
    }
    if ($oldSize -gt 0) {
        $hint = if ($MoveOld) { '将搬到新位置' } else { '加 -MoveOld 可搬走' }
        Write-Host ("           C 盘旧缓存 {0}：{1}（{2}）" -f (Format-Size $oldSize), $it.Old, $hint) -ForegroundColor Yellow
    }
}

$skipped = @('pnpm.cmd', 'go', 'dotnet', 'deno') | Where-Object { -not (Test-Cmd $_) }
if ($skipped) {
    Write-Host ("`n  未检测到，已跳过：{0}" -f (($skipped -replace '\.cmd$', '') -join ', ')) -ForegroundColor DarkGray
}

if (-not $Apply) {
    Write-Host "`n确认无误后，加 -Apply 再运行一次；想顺便把 C 盘旧缓存搬过去，再加 -MoveOld。`n" -ForegroundColor Cyan
    exit 0
}

# ---------- 执行 ----------
function Send-EnvironmentChange {
    try {
        if (-not ('CDriveKeeper.Native2' -as [type])) {
            Add-Type -Namespace CDriveKeeper -Name Native2 -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true, CharSet = System.Runtime.InteropServices.CharSet.Auto)]
public static extern System.IntPtr SendMessageTimeout(System.IntPtr hWnd, uint Msg, System.UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out System.UIntPtr lpdwResult);
'@
        }
        $r = [UIntPtr]::Zero
        [void][CDriveKeeper.Native2]::SendMessageTimeout([IntPtr]0xffff, 0x1A, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$r)
    } catch {
        Write-Host "（未能广播环境变量变更，请注销再登录一次）" -ForegroundColor Yellow
    }
}

Write-Host ""
foreach ($p in $plan) {
    $it = $p.Item
    try {
        if ($p.Action -eq 'set') {
            New-Item -ItemType Directory -Path $p.Target -Force | Out-Null
            if ($it.Pnpm) {
                $ErrorActionPreference = 'Continue'
                & pnpm.cmd config set store-dir $p.Target --global 2>&1 | Out-Null
                $ErrorActionPreference = 'Stop'
            } else {
                [Environment]::SetEnvironmentVariable($it.Var, $p.Target, 'User')
                Set-Item -Path "Env:$($it.Var)" -Value $p.Target
            }
            Write-Host ("  已设置 {0} -> {1}" -f $it.Name, $p.Target) -ForegroundColor Green
        }

        # 只有新位置确实是我们设的这个目录时才搬，避免搬到别处
        if ($MoveOld -and $p.OldSize -gt 0 -and $p.Action -ne 'keep') {
            if ($it.Old.Length -lt 12) { continue }   # 防御：拒绝过短路径
            New-Item -ItemType Directory -Path $p.Target -Force | Out-Null
            Write-Host ("  正在搬运 {0}（{1}）..." -f $it.Name, (Format-Size $p.OldSize))
            & robocopy.exe $it.Old $p.Target /E /MOVE /R:1 /W:1 /NFL /NDL /NJH /NJS /NP | Out-Null
            if ($LASTEXITCODE -ge 8) {
                Write-Host ("  {0} 有部分文件没搬走（可能正被占用），关掉相关程序后再运行一次即可" -f $it.Name) -ForegroundColor Yellow
            } else {
                Write-Host ("  已搬走 {0}" -f $it.Name) -ForegroundColor Green
            }
        }
    } catch {
        Write-Host ("  {0} 出错：{1}" -f $it.Name, $_.Exception.Message) -ForegroundColor Red
        $ErrorActionPreference = 'Stop'
    }
}

Send-EnvironmentChange

Write-Host "`n=== 完成 ===" -ForegroundColor Cyan
Write-Host "接下来："
Write-Host "  1. 关掉所有终端、IDE、AI 客户端（包括托盘图标）再重新打开，新设置才会生效"
Write-Host "  2. 运行 scripts\audit.ps1 验证；以后想撤销，在“编辑账户的环境变量”里删掉对应变量即可`n"
