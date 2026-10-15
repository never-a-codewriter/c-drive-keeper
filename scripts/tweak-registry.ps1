<#
.SYNOPSIS
  几个安全、可回滚的注册表小优化：菜单更快、更新不自动重启、显示扩展名等。
  默认只预览（显示每一项的当前值和目标值），加 -Apply 才真正修改。
.DESCRIPTION
  修改前会自动备份到 Downloads\backups\registry_时间戳：
    - original-values.json：每一项修改前的原值（含"原本不存在"），供 -RestoreFrom 精确回滚
    - *.reg：相关注册表项的完整导出，也可以双击手动导入
    - tweak.log：操作日志
  HKLM 下的项目需要管理员；非管理员运行时会跳过这些项，其余照常处理。
.PARAMETER Apply
  真正写入注册表。不加时只预览。
.PARAMETER Only
  只处理指定的项目（用 Id，逗号分隔）。可选 Id 见下方列表或预览输出。
.PARAMETER Skip
  跳过指定的项目（用 Id，逗号分隔）。
.PARAMETER Restore
  把所选项目恢复成 Windows 默认值（同样需要配合 -Apply 才执行）。
.PARAMETER RestoreFrom
  从某次备份的 original-values.json 精确恢复到修改前的状态（同样需要 -Apply）。
.PARAMETER RestartExplorer
  完成后重启资源管理器，让"显示扩展名"等界面设置立即生效。
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\tweak-registry.ps1                          # 预览
  powershell -ExecutionPolicy Bypass -File .\scripts\tweak-registry.ps1 -Apply -RestartExplorer  # 执行
  powershell -ExecutionPolicy Bypass -File .\scripts\tweak-registry.ps1 -Apply -Skip fast-shutdown
  powershell -ExecutionPolicy Bypass -File .\scripts\tweak-registry.ps1 -Restore -Apply          # 恢复默认
  powershell -ExecutionPolicy Bypass -File .\scripts\tweak-registry.ps1 -RestoreFrom "$env:USERPROFILE\Downloads\backups\registry_20261016_011700\original-values.json" -Apply
#>
param(
    [switch]$Apply,
    [string[]]$Only,
    [string[]]$Skip,
    [switch]$Restore,
    [string]$RestoreFrom,
    [switch]$RestartExplorer
)

$ErrorActionPreference = 'Stop'

# 用 -File 调用时 "-Only a,b" 会被当成一个字符串，这里统一按逗号拆开并校验
$ValidIds = @('menu-delay', 'no-auto-reboot', 'taskbar-preview', 'cortana', 'show-ext', 'no-recent-docs', 'fast-shutdown')
function Split-Ids([string[]]$List) {
    $ids = @($List | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
    $bad = @($ids | Where-Object { $ValidIds -notcontains $_ })
    if ($bad) {
        Write-Host ("未知的项目 Id：{0}`n可选：{1}" -f ($bad -join ', '), ($ValidIds -join ', ')) -ForegroundColor Red
        exit 1
    }
    return $ids
}
$Only = Split-Ids $Only
$Skip = Split-Ids $Skip

# ---------- 优化项定义 ----------
# Default = $null 表示 Windows 默认不存在这个值，恢复默认时会删除它
$Tweaks = @(
    @{ Id = 'menu-delay';      Name = '菜单弹出更快';         Key = 'HKCU:\Control Panel\Desktop'
       Value = 'MenuShowDelay';                 Kind = 'String'; Target = '100';  Default = '400'
       Note  = '多级菜单展开延迟 400ms -> 100ms，注销或重启后生效' },
    @{ Id = 'no-auto-reboot';  Name = '更新不自动重启';       Key = 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU'
       Value = 'NoAutoRebootWithLoggedOnUsers'; Kind = 'DWord';  Target = 1;      Default = $null
       Note  = '有用户登录时不自动重启；家庭版可能不完全生效' },
    @{ Id = 'taskbar-preview'; Name = '任务栏预览更快';       Key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced'
       Value = 'ExtendedUIHoverTime';           Kind = 'DWord';  Target = 1;      Default = $null
       Note  = '鼠标悬停缩略图几乎无延迟' },
    @{ Id = 'cortana';         Name = '禁用 Cortana';         Key = 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\Windows Search'
       Value = 'AllowCortana';                  Kind = 'DWord';  Target = 0;      Default = $null
       Note  = 'Win11 已基本移除 Cortana，多半无实际效果' },
    @{ Id = 'show-ext';        Name = '显示文件扩展名';       Key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced'
       Value = 'HideFileExt';                   Kind = 'DWord';  Target = 0;      Default = 1
       Note  = '防止 xxx.pdf.exe 这类伪装文件' },
    @{ Id = 'no-recent-docs';  Name = '不记录最近使用的文件'; Key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced'
       Value = 'Start_TrackDocs';               Kind = 'DWord';  Target = 0;      Default = 1
       Note  = '开始菜单/跳转列表不再显示最近文件' },
    @{ Id = 'fast-shutdown';   Name = '关机更快';             Key = 'HKLM:\SYSTEM\CurrentControlSet\Control'
       Value = 'WaitToKillServiceTimeout';      Kind = 'String'; Target = '2000'; Default = '5000'
       Note  = '服务关闭等待 5000ms -> 2000ms；个别服务可能来不及保存数据' }
)

# ---------- 工具函数 ----------
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
           ).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

function Get-RegValue([string]$Key, [string]$Name) {
    # 返回 @{ Exists; Kind; Data }
    if (-not (Test-Path -LiteralPath $Key)) { return @{ Exists = $false; Kind = $null; Data = $null } }
    $item = Get-Item -LiteralPath $Key
    if ($item.GetValueNames() -notcontains $Name) { return @{ Exists = $false; Kind = $null; Data = $null } }
    return @{ Exists = $true; Kind = $item.GetValueKind($Name).ToString(); Data = $item.GetValue($Name) }
}

function Set-RegValue([string]$Key, [string]$Name, [string]$Kind, $Data) {
    if (-not (Test-Path -LiteralPath $Key)) { New-Item -Path $Key -Force | Out-Null }
    New-ItemProperty -LiteralPath $Key -Name $Name -PropertyType $Kind -Value $Data -Force | Out-Null
}

function Remove-RegValue([string]$Key, [string]$Name) {
    if ((Get-RegValue $Key $Name).Exists) {
        Remove-ItemProperty -LiteralPath $Key -Name $Name -Force
    }
}

function Format-Val($v) {
    if (-not $v.Exists) { return '(不存在)' }
    return [string]$v.Data
}

function Test-Same($cur, $want) {
    if ($null -eq $want) { return -not $cur.Exists }
    return $cur.Exists -and ([string]$cur.Data -eq [string]$want)
}

function Test-NeedAdmin($t) { return $t.Key -like 'HKLM:*' }

function ConvertTo-RegExePath([string]$Key) {
    return ($Key -replace '^HKCU:', 'HKCU' -replace '^HKLM:', 'HKLM')
}

$script:logFile = $null
function Write-Log([string]$Msg) {
    if ($script:logFile) { Add-Content -LiteralPath $script:logFile -Value ("{0}  {1}" -f (Get-Date -Format s), $Msg) -Encoding UTF8 }
}

# ---------- 从备份精确回滚 ----------
if ($RestoreFrom) {
    if (-not (Test-Path -LiteralPath $RestoreFrom)) { Write-Host "找不到备份文件：$RestoreFrom" -ForegroundColor Red; exit 1 }
    $saved = Get-Content -LiteralPath $RestoreFrom -Raw -Encoding UTF8 | ConvertFrom-Json
    $mode = if ($Apply) { '执行回滚（-Apply）' } else { '预览回滚：不修改（要执行请加 -Apply）' }
    Write-Host "`n=== $mode | 来源 $RestoreFrom ===`n" -ForegroundColor Cyan
    $script:logFile = Join-Path (Split-Path -Parent $RestoreFrom) 'tweak.log'
    foreach ($s in $saved) {
        if (($s.Key -like 'HKLM:*') -and -not $isAdmin) {
            Write-Host ("  {0,-20} 跳过（需要管理员）" -f $s.Name) -ForegroundColor DarkGray; continue
        }
        $cur = Get-RegValue $s.Key $s.Value
        $back = if ($s.Existed) { [string]$s.Data } else { '(不存在)' }
        Write-Host ("  {0,-20} {1,-10} -> {2}" -f $s.Name, (Format-Val $cur), $back)
        if ($Apply) {
            if ($s.Existed) { Set-RegValue $s.Key $s.Value $s.Kind $s.Data } else { Remove-RegValue $s.Key $s.Value }
            Write-Log "ROLLBACK $($s.Key)\$($s.Value) -> $back"
        }
    }
    if (-not $isAdmin) { Write-Host "`n提示：HKLM 下的项目需要以管理员身份运行 PowerShell 才能回滚。" -ForegroundColor Yellow }
    if ($Apply -and $RestartExplorer) { Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue; Start-Process explorer.exe }
    Write-Host ""
    exit 0
}

# ---------- 选出要处理的项目 ----------
$selected = $Tweaks | Where-Object { (-not $Only -or $Only -contains $_.Id) -and -not ($Skip -contains $_.Id) }
if (-not $selected) { Write-Host '没有选中任何项目。' -ForegroundColor Yellow; exit 0 }

$action = if ($Restore) { '恢复默认值' } else { '应用优化' }
if ($Apply) {
    Write-Host "`n=== 执行：$action（-Apply）===" -ForegroundColor Cyan
} else {
    Write-Host "`n=== 预览模式：$action，只显示不修改（要执行请加 -Apply）===" -ForegroundColor Cyan
}
if (-not $isAdmin) {
    Write-Host "提示：当前不是管理员，HKLM 下的项目（更新重启、Cortana、关机）会被跳过。" -ForegroundColor Yellow
}
Write-Host ""
Write-Host ("  {0,-16} {1,-20} {2,-10} {3,-10} {4}" -f 'Id', '项目', '当前', '目标', '说明')

$todo = @()
foreach ($t in $selected) {
    $want = if ($Restore) { $t.Default } else { $t.Target }
    $wantText = if ($null -eq $want) { '(删除)' } else { [string]$want }
    if ((Test-NeedAdmin $t) -and -not $isAdmin) {
        Write-Host ("  {0,-16} {1,-20} {2,-10} {3,-10} {4}" -f $t.Id, $t.Name, '-', $wantText, '跳过（需要管理员）') -ForegroundColor DarkGray
        continue
    }
    $cur = Get-RegValue $t.Key $t.Value
    if (Test-Same $cur $want) {
        Write-Host ("  {0,-16} {1,-20} {2,-10} {3,-10} {4}" -f $t.Id, $t.Name, (Format-Val $cur), $wantText, '已是目标值') -ForegroundColor DarkGray
        continue
    }
    $note = if ($Restore) { '' } else { $t.Note }
    Write-Host ("  {0,-16} {1,-20} {2,-10} {3,-10} {4}" -f $t.Id, $t.Name, (Format-Val $cur), $wantText, $note)
    $todo += @{ T = $t; Cur = $cur; Want = $want }
}

Write-Host ""
if (-not $todo) { Write-Host '=== 全部已是目标值，无需修改 ===' -ForegroundColor Cyan; Write-Host ""; exit 0 }
if (-not $Apply) {
    Write-Host ("=== 预览合计：{0} 项将被修改。确认无误后，加 -Apply 再运行一次 ===" -f $todo.Count) -ForegroundColor Cyan
    Write-Host ""
    exit 0
}

# ---------- 备份 ----------
$stamp = Get-Date -Format 'yyyyMMdd_HHmmss'
$bk = Join-Path $env:USERPROFILE "Downloads\backups\registry_$stamp"
New-Item -ItemType Directory -Path $bk -Force | Out-Null
$script:logFile = Join-Path $bk 'tweak.log'
Write-Log "START $action, items: $(($todo | ForEach-Object { $_.T.Id }) -join ',')"

$original = foreach ($x in $todo) {
    [PSCustomObject]@{
        Id = $x.T.Id; Name = $x.T.Name; Key = $x.T.Key; Value = $x.T.Value
        Existed = $x.Cur.Exists; Kind = $(if ($x.Cur.Exists) { $x.Cur.Kind } else { $x.T.Kind }); Data = $x.Cur.Data
    }
}
$original | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $bk 'original-values.json') -Encoding UTF8

$i = 0
foreach ($k in ($todo | ForEach-Object { $_.T.Key } | Select-Object -Unique)) {
    $i++
    if (Test-Path -LiteralPath $k) {
        $file = Join-Path $bk ("{0}_{1}.reg" -f $i, ((Split-Path $k -Leaf) -replace '\s', ''))
        try { & reg.exe export (ConvertTo-RegExePath $k) $file /y 2>$null | Out-Null } catch { Write-Log "EXPORT-FAIL $k" }
    }
}
Write-Host "已备份到：$bk" -ForegroundColor Green

# ---------- 写入 ----------
$failed = 0
foreach ($x in $todo) {
    $t = $x.T
    try {
        if ($null -eq $x.Want) { Remove-RegValue $t.Key $t.Value } else { Set-RegValue $t.Key $t.Value $t.Kind $x.Want }
        Write-Host ("  [OK]   {0}" -f $t.Name)
        Write-Log "SET $($t.Key)\$($t.Value): $(Format-Val $x.Cur) -> $($x.Want)"
    } catch {
        $failed++
        Write-Host ("  [失败] {0}：{1}" -f $t.Name, $_.Exception.Message) -ForegroundColor Red
        Write-Log "FAIL $($t.Key)\$($t.Value): $($_.Exception.Message)"
    }
}

if ($RestartExplorer) {
    Write-Host "`n正在重启资源管理器..." -ForegroundColor Green
    Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 800
    if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) { Start-Process explorer.exe }
}

Write-Host ""
Write-Host ("=== 完成：{0} 项成功，{1} 项失败 ===" -f ($todo.Count - $failed), $failed) -ForegroundColor Cyan
if (-not $RestartExplorer) { Write-Host '扩展名、最近文件等界面设置：加 -RestartExplorer 或注销后生效。' -ForegroundColor DarkGray }
Write-Host '菜单延迟、更新、Cortana、关机相关设置：重启电脑后完全生效。' -ForegroundColor DarkGray
Write-Host "`n后悔了？精确回滚到修改前："
Write-Host ("  powershell -ExecutionPolicy Bypass -File .\scripts\tweak-registry.ps1 -RestoreFrom `"{0}`" -Apply" -f (Join-Path $bk 'original-values.json'))
Write-Host ""
Write-Log "END ok=$($todo.Count - $failed) fail=$failed"
