<#
.SYNOPSIS
  只读体检：看 C 盘空间花在哪、开发环境是否还在往 C 盘写。不会修改或删除任何东西。
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\audit.ps1
  powershell -ExecutionPolicy Bypass -File .\scripts\audit.ps1 -Top 15
  powershell -ExecutionPolicy Bypass -File .\scripts\audit.ps1 -Quick
#>
param(
    [int]$Top = 10,
    [switch]$Quick    # 跳过最慢的 AppData / 隐藏文件夹扫描
)

$ErrorActionPreference = 'SilentlyContinue'
$SysDrive = $env:SystemDrive   # 通常是 C:

# ---------- 工具函数 ----------
function Get-DirSize([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return [int64]0 }
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

function Show-TopFolders($Dirs, [int]$N) {
    $Dirs |
        ForEach-Object { [pscustomobject]@{ Name = $_.Name; Bytes = (Get-DirSize $_.FullName) } } |
        Sort-Object Bytes -Descending |
        Select-Object -First $N |
        ForEach-Object { [pscustomobject]@{ '文件夹' = $_.Name; '大小' = (Format-Size $_.Bytes) } } |
        Format-Table -AutoSize
}

function Test-OnSystemDrive([string]$Path) {
    return ($Path -and $Path.StartsWith($SysDrive, [StringComparison]::OrdinalIgnoreCase))
}

function Show-Check([string]$Label, [string]$Value, [bool]$Ok, [string]$Hint) {
    if ($Ok) {
        Write-Host ("  [OK]   {0}: {1}" -f $Label, $Value) -ForegroundColor Green
    } else {
        Write-Host ("  [注意] {0}: {1}" -f $Label, $Value) -ForegroundColor Yellow
        if ($Hint) { Write-Host ("         -> {0}" -f $Hint) -ForegroundColor DarkYellow }
    }
}

# ---------- 1. 总览 ----------
Write-Host "`n=== 1. $SysDrive 盘总览 ===" -ForegroundColor Cyan
$drive = Get-PSDrive -Name $SysDrive.TrimEnd(':')
$freeGB = [math]::Round($drive.Free / 1GB, 1)
$totalGB = [math]::Round(($drive.Used + $drive.Free) / 1GB, 1)
Write-Host "剩余 $freeGB GB / 共 $totalGB GB"
if ($freeGB -lt 30) {
    Write-Host "剩余空间低于 30 GB，建议尽快清理。" -ForegroundColor Yellow
}

# ---------- 2. 已知的大户 ----------
Write-Host "`n=== 2. 常见大户 ===" -ForegroundColor Cyan
$known = @(
    @{ Name = '休眠文件 hiberfil.sys'; Path = "$SysDrive\hiberfil.sys"; IsFile = $true;  Tip = '管理员 CMD: powercfg -h off（会关闭休眠和快速启动）' },
    @{ Name = '旧系统 Windows.old';    Path = "$SysDrive\Windows.old";  IsFile = $false; Tip = '磁盘清理 -> 清理系统文件 -> 以前的 Windows 安装' },
    @{ Name = '用户临时文件 Temp';     Path = $env:TEMP;                IsFile = $false; Tip = '用 scripts\clean.ps1 清理' },
    @{ Name = '下载 Downloads';        Path = "$env:USERPROFILE\Downloads"; IsFile = $false; Tip = '手动翻一遍，安装包最占地方' },
    @{ Name = '桌面 Desktop';          Path = [Environment]::GetFolderPath('Desktop'); IsFile = $false; Tip = '手动翻一遍' }
)
foreach ($k in $known) {
    if (-not (Test-Path -LiteralPath $k.Path -ErrorAction SilentlyContinue)) {
        if ($k.IsFile) {
            $item = Get-Item -LiteralPath $k.Path -Force -ErrorAction SilentlyContinue
            if (-not $item) { continue }
        } else { continue }
    }
    if ($k.IsFile) {
        $size = (Get-Item -LiteralPath $k.Path -Force).Length
    } else {
        $size = Get-DirSize $k.Path
    }
    Write-Host ("  {0,-24} {1,10}   {2}" -f $k.Name, (Format-Size $size), $k.Tip)
}

# ---------- 3. AppData 与隐藏文件夹 ----------
if (-not $Quick) {
    Write-Host "`n=== 3. AppData\Local 最大的 $Top 个文件夹（较慢，请耐心等待）===" -ForegroundColor Cyan
    Show-TopFolders (Get-ChildItem -LiteralPath $env:LOCALAPPDATA -Directory -Force) $Top

    Write-Host "=== 4. AppData\Roaming 最大的 $Top 个文件夹 ===" -ForegroundColor Cyan
    Show-TopFolders (Get-ChildItem -LiteralPath $env:APPDATA -Directory -Force) $Top

    Write-Host "=== 5. 用户目录下的隐藏文件夹（.xxx，AI 工具常放这里）===" -ForegroundColor Cyan
    Show-TopFolders (Get-ChildItem -LiteralPath $env:USERPROFILE -Directory -Force |
                     Where-Object { $_.Name -like '.*' }) $Top
} else {
    Write-Host "`n（已跳过第 3~5 项：AppData 与隐藏文件夹扫描）" -ForegroundColor DarkGray
}

# ---------- 6. 开发环境是否还在往 C 盘写 ----------
Write-Host "=== 6. 开发环境检查 ===" -ForegroundColor Cyan

# uv
if (Get-Command uv -ErrorAction SilentlyContinue) {
    $uvDir = (& uv cache dir 2>$null | Select-Object -First 1)
    Show-Check 'uv 缓存' $uvDir (-not (Test-OnSystemDrive $uvDir)) '运行 scripts\relocate-dev-caches.ps1 迁移'
}

# npm
if (Get-Command npm.cmd -ErrorAction SilentlyContinue) {
    $npmCache  = (& npm.cmd config get cache  2>$null | Select-Object -Last 1)
    $npmPrefix = (& npm.cmd config get prefix 2>$null | Select-Object -Last 1)
    Show-Check 'npm 缓存'   $npmCache  (-not (Test-OnSystemDrive $npmCache))  '运行 scripts\relocate-dev-caches.ps1 迁移'
    Show-Check 'npm 全局包' $npmPrefix (-not (Test-OnSystemDrive $npmPrefix)) '运行 scripts\relocate-dev-caches.ps1 迁移'
}

# pip
if (Get-Command python -ErrorAction SilentlyContinue) {
    $pipDir = if ($env:PIP_CACHE_DIR) { $env:PIP_CACHE_DIR } else { "$env:LOCALAPPDATA\pip\Cache" }
    Show-Check 'pip 缓存' $pipDir (-not (Test-OnSystemDrive $pipDir)) '运行 scripts\relocate-dev-caches.ps1 迁移'
}

# node / python 实际用的是哪一份
foreach ($cmd in 'node', 'python') {
    $paths = @(& where.exe $cmd 2>$null)
    if ($paths.Count -eq 0) { continue }
    Write-Host ("  {0} 的查找顺序（越靠前越优先）:" -f $cmd)
    foreach ($p in $paths) {
        $warn = ''
        if ($p -like '*\WindowsApps\python*.exe') { $warn = '  <- 商店的假 Python 别名，建议在"应用执行别名"里关闭' }
        elseif ($p -like '*\venv\*' -or $p -like '*\.venv\*') { $warn = '  <- 虚拟环境出现在 PATH 里，通常不该如此' }
        Write-Host ("    {0}{1}" -f $p, $warn)
    }
}

Write-Host "`n体检结束。这个脚本是只读的，没有改动任何东西。" -ForegroundColor Cyan
Write-Host "下一步：先用 scripts\clean.ps1 预览可清理的内容。`n"
