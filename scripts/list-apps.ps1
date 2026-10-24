<#
.SYNOPSIS
  只读列出当前用户的商店应用和注册的桌面程序，便于检查可手动卸载的应用。
.DESCRIPTION
  Win+R 输入 shell:appsfolder 可查看 Windows 注册的应用入口，但不会显示应用占用大小。
  本脚本将当前用户的 Appx 包和卸载注册表中的桌面程序按类别列出；不会卸载或修改任何内容。
  Appx 安装目录大小只是估算值，可能因 WindowsApps 权限、共享文件或组件存储而不完整。
.PARAMETER Top
  每类最多显示多少个（默认 20）。
.PARAMETER Name
  只显示名称中包含该关键字的应用。
.PARAMETER IncludeSystem
  同时显示标记为系统或不可移除的商店应用。
.PARAMETER ExportCsv
  导出完整筛选结果为 CSV。
.PARAMETER Open
  完成后打开 shell:appsfolder，供用户手动检查应用卸载选项。
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\list-apps.ps1
  powershell -ExecutionPolicy Bypass -File .\scripts\list-apps.ps1 -Top 50 -Open
  powershell -ExecutionPolicy Bypass -File .\scripts\list-apps.ps1 -Name xbox -IncludeSystem
  powershell -ExecutionPolicy Bypass -File .\scripts\list-apps.ps1 -ExportCsv "$env:USERPROFILE\Downloads\apps.csv"
#>
param(
    [ValidateRange(1, 10000)]
    [int]$Top = 20,
    [string]$Name,
    [switch]$IncludeSystem,
    [string]$ExportCsv,
    [switch]$Open
)

$ErrorActionPreference = 'Continue'
$script:sizeCache = @{}

function Get-DirSize([string]$Path) {
    if (-not $Path -or -not (Test-Path -LiteralPath $Path -PathType Container -ErrorAction SilentlyContinue)) {
        return [int64]-1
    }

    $key = $Path.TrimEnd('\')
    if ($script:sizeCache.ContainsKey($key)) { return $script:sizeCache[$key] }

    try {
        $sum = (Get-ChildItem -LiteralPath $Path -Recurse -Force -File -ErrorAction Stop |
            Measure-Object -Property Length -Sum).Sum
        if ($null -eq $sum) { $sum = [int64]0 }
        $script:sizeCache[$key] = [int64]$sum
        return [int64]$sum
    } catch {
        Write-Verbose "Unable to calculate directory size for '$Path': $($_.Exception.Message)"
        $script:sizeCache[$key] = [int64]-1
        return [int64]-1
    }
}

function Format-Size([int64]$Bytes) {
    if ($Bytes -lt 0) { return '-' }
    if ($Bytes -ge 1GB) { return ('{0:N2} GB' -f ($Bytes / 1GB)) }
    if ($Bytes -ge 1MB) { return ('{0:N0} MB' -f ($Bytes / 1MB)) }
    return ('{0:N0} KB' -f ($Bytes / 1KB))
}

function Get-Drive([string]$Path) {
    if ($Path -match '^([A-Za-z]:)') { return $Matches[1].ToUpperInvariant() }
    return '-'
}

function Test-NameMatch([string]$Text) {
    return (-not $Name -or ($Text -and $Text.IndexOf($Name, [StringComparison]::OrdinalIgnoreCase) -ge 0))
}

$apps = @()
$hiddenSystem = 0

Write-Host "`n=== 商店应用（当前用户）===" -ForegroundColor Cyan
$getAppx = Get-Command Get-AppxPackage -ErrorAction SilentlyContinue
if (-not $getAppx) {
    Write-Warning '当前 PowerShell 环境不支持 Get-AppxPackage，跳过商店应用。'
} else {
    $packages = Get-AppxPackage -ErrorAction SilentlyContinue |
        Where-Object { -not $_.IsFramework -and -not $_.IsResourcePackage -and -not $_.IsBundle }
    foreach ($package in $packages) {
        $isSystem = ($package.SignatureKind -eq 'System') -or ($package.NonRemovable -eq $true)
        $displayName = $package.Name
        $manifest = Get-AppxPackageManifest -Package $package.PackageFullName -ErrorAction SilentlyContinue
        $manifestName = [string]$manifest.Package.Properties.DisplayName
        if ($manifestName -and $manifestName -notlike 'ms-resource:*') { $displayName = $manifestName }
        if (-not (Test-NameMatch "$displayName $($package.Name)")) { continue }
        if ($isSystem -and -not $IncludeSystem) { $hiddenSystem++; continue }

        $source = switch ($package.SignatureKind) {
            'System'    { '系统组件' }
            'Store'     { '微软商店' }
            'Developer' { '侧载/开发者' }
            default     { [string]$package.SignatureKind }
        }
        $apps += [pscustomobject]@{
            '类型' = '商店应用'
            '名称' = $displayName
            '大小字节' = (Get-DirSize $package.InstallLocation)
            '盘' = (Get-Drive $package.InstallLocation)
            '来源' = $source
            '版本' = [string]$package.Version
            '发布者' = [string]$package.PublisherId
            '安装日期' = ''
            '标识' = [string]$package.PackageFullName
            '位置' = [string]$package.InstallLocation
        }
    }
}

Write-Host "`n=== 桌面程序（卸载注册表信息）===" -ForegroundColor Cyan
$uninstallKeys = @(
    'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall',
    'HKLM:\SOFTWARE\Wow6432Node\Microsoft\Windows\CurrentVersion\Uninstall',
    'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall'
)
$seen = @{}
foreach ($hive in $uninstallKeys) {
    $keys = Get-ChildItem -LiteralPath $hive -ErrorAction SilentlyContinue
    foreach ($key in $keys) {
        $entry = Get-ItemProperty -LiteralPath $key.PSPath -ErrorAction SilentlyContinue
        if (-not $entry.DisplayName) { continue }
        if ($entry.SystemComponent -eq 1 -or $entry.ParentKeyName -or $entry.ReleaseType) { continue }
        if (-not (Test-NameMatch ([string]$entry.DisplayName))) { continue }

        $identity = '{0}|{1}' -f $entry.DisplayName, $entry.DisplayVersion
        if ($seen.ContainsKey($identity)) { continue }
        $seen[$identity] = $true

        $location = [string]$entry.InstallLocation
        if (-not $location -and $entry.DisplayIcon) {
            $iconPath = ([string]$entry.DisplayIcon -split ',')[0].Trim('"')
            $location = Split-Path -Parent $iconPath
        }
        $size = if ($entry.EstimatedSize) { [int64]$entry.EstimatedSize * 1KB } else { [int64]-1 }
        $installDate = ''
        if ([string]$entry.InstallDate -match '^\d{8}$') {
            $installDate = '{0}-{1}-{2}' -f $entry.InstallDate.Substring(0, 4), $entry.InstallDate.Substring(4, 2), $entry.InstallDate.Substring(6, 2)
        }

        $apps += [pscustomobject]@{
            '类型' = '桌面程序'
            '名称' = [string]$entry.DisplayName
            '大小字节' = $size
            '盘' = (Get-Drive $location)
            '来源' = $(if ($hive -like 'HKCU:*') { '仅当前用户' } else { '所有用户' })
            '版本' = [string]$entry.DisplayVersion
            '发布者' = [string]$entry.Publisher
            '安装日期' = $installDate
            '标识' = [string]$key.PSChildName
            '位置' = $location
        }
    }
}

foreach ($type in @('商店应用', '桌面程序')) {
    $group = @($apps | Where-Object { $_.'类型' -eq $type } | Sort-Object '大小字节' -Descending)
    $knownSizes = @($group | Where-Object { $_.'大小字节' -ge 0 })
    $totalBytes = if ($knownSizes.Count) {
        [int64](($knownSizes | Measure-Object -Property '大小字节' -Sum).Sum)
    } else {
        [int64]-1
    }
    Write-Host ("`n{0}：{1} 个，已知大小合计 {2}；显示前 {3} 个" -f $type, $group.Count, (Format-Size $totalBytes), [math]::Min($Top, $group.Count)) -ForegroundColor Cyan
    $group | Select-Object -First $Top | ForEach-Object {
        Write-Host ("  {0,10}  {1,-3} {2,-10} {3}" -f (Format-Size $_.'大小字节'), $_.'盘', $_.'来源', $_.'名称')
    }
}

if ($hiddenSystem -gt 0) {
    Write-Host "`n另有 $hiddenSystem 个匹配的系统/不可移除商店应用已隐藏；加 -IncludeSystem 查看。" -ForegroundColor DarkGray
}

if ($ExportCsv) {
    try {
        $csvPath = [System.IO.Path]::GetFullPath($ExportCsv)
        $parent = Split-Path -Parent $csvPath
        if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
            throw "导出目录不存在：$parent"
        }
        $apps | Sort-Object '类型', @{ Expression = '大小字节'; Descending = $true } |
            Select-Object '类型', '名称', @{ N = '大小'; E = { Format-Size $_.'大小字节' } }, '大小字节', '盘', '来源', '版本', '发布者', '安装日期', '标识', '位置' |
            Export-Csv -LiteralPath $csvPath -NoTypeInformation -Encoding UTF8 -ErrorAction Stop
        Write-Host "`n完整清单已导出：$csvPath" -ForegroundColor Green
    } catch {
        Write-Error "导出 CSV 失败：$($_.Exception.Message)"
        exit 1
    }
}

Write-Host "`n此脚本只读取应用信息，不会卸载或修改任何内容。" -ForegroundColor Cyan
Write-Host "卸载前请确认应用用途；驱动、运行库（如 Visual C++、.NET、WebView2）不要随意卸载。"
Write-Host "Win+R 输入 shell:appsfolder 可查看应用入口；该窗口本身不显示占用大小。"

if ($Open) { Start-Process explorer.exe 'shell:AppsFolder' }
