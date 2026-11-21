[CmdletBinding()]
param(
    [ValidateSet('Analyze','ComponentCleanup','UpdateCacheCleanup')][string]$Action='Analyze',
    [string]$ReportPath
)
$ErrorActionPreference='Stop'
function Test-Admin {
    $id=[Security.Principal.WindowsIdentity]::GetCurrent()
    return ([Security.Principal.WindowsPrincipal]::new($id)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}
function Get-TreeBytes([string]$Path) {
    if(-not(Test-Path -LiteralPath $Path)){return [int64]0}
    $errors=@();$sum=(Get-ChildItem -LiteralPath $Path -Force -File -Recurse -ErrorAction SilentlyContinue -ErrorVariable +errors | Measure-Object Length -Sum).Sum
    if($errors.Count){Write-Warning "部分文件无法读取：$Path ($($errors.Count))"}
    if($null -eq $sum){return [int64]0};return [int64]$sum
}
$win=$env:SystemRoot
$items=@(
    @{Name='WinSxS 组件存储';Path=(Join-Path $win 'WinSxS');Method='DISM /StartComponentCleanup';Risk='使用 Windows 组件服务清理可替换组件；不能直接删除目录。';Action='ComponentCleanup'},
    @{Name='Windows Update 下载缓存';Path=(Join-Path $win 'SoftwareDistribution\Download');Method='备份后清理下载缓存';Risk='更新包可在需要时重新下载；不会删除更新数据库或卸载记录。';Action='UpdateCacheCleanup'},
    @{Name='Windows 临时文件';Path=(Join-Path $win 'Temp');Method='仅分析';Risk='可能有正在使用的安装或系统文件。';Action='Analyze'},
    @{Name='Windows Installer 缓存';Path=(Join-Path $win 'Installer');Method='仅分析';Risk='卸载/修复 Office 等微软产品可能依赖；禁止自动删除。';Action='Analyze'},
    @{Name='Windows.old';Path=(Join-Path (Split-Path $win -Parent) 'Windows.old');Method='设置 > 系统 > 存储 > 临时文件';Risk='删除会失去回退到先前 Windows 版本的能力。';Action='Analyze'},
    @{Name='Microsoft Store 应用数据';Path=(Join-Path $env:LOCALAPPDATA 'Packages');Method='按应用设置逐个管理';Risk='可能包含用户数据；不自动清理。';Action='Analyze'}
)
if($Action -eq 'ComponentCleanup'){
    if(-not(Test-Admin)){throw 'WinSxS 组件清理需要管理员权限。请关闭程序后以管理员身份运行。'}
    $dism=Join-Path $win 'System32\Dism.exe'
    $p=Start-Process -FilePath $dism -ArgumentList '/Online','/Cleanup-Image','/StartComponentCleanup' -Wait -PassThru -NoNewWindow
    if($p.ExitCode -ne 0){throw "DISM 组件清理失败，退出码 $($p.ExitCode)"}
    Write-Output 'DISM 组件清理完成。不会对 WinSxS 目录执行文件删除。'
    exit 0
}
if($Action -eq 'UpdateCacheCleanup'){
    if(-not(Test-Admin)){throw 'Windows Update 缓存清理需要管理员权限。请关闭程序后以管理员身份运行。'}
    $root=Join-Path $env:SystemDrive 'CDriveKeeper-SystemBackups'
    $backup=Join-Path $root ('WindowsUpdate-'+(Get-Date -Format 'yyyyMMdd-HHmmss'))
    $serviceWasRunning=(Get-Service wuauserv).Status -eq 'Running'
    $bitsWasRunning=(Get-Service BITS).Status -eq 'Running'
    try {
        Stop-Service wuauserv -Force -ErrorAction Stop
        Stop-Service BITS -Force -ErrorAction SilentlyContinue
        if(Test-Path -LiteralPath $items[1].Path){$entries=@(Get-ChildItem -LiteralPath $items[1].Path -Force);if($entries.Count){New-Item -ItemType Directory -Path $backup -Force|Out-Null;foreach($entry in $entries){Move-Item -LiteralPath $entry.FullName -Destination $backup -Force -ErrorAction Stop}}}
    } finally {
        if($bitsWasRunning){Start-Service BITS -ErrorAction SilentlyContinue}
        if($serviceWasRunning){Start-Service wuauserv -ErrorAction SilentlyContinue}
    }
    Write-Output "Windows Update 下载缓存已备份到：$backup。后续更新需要时可重新下载。"
    exit 0
}
$results=foreach($item in $items){$size=Get-TreeBytes $item.Path;$method=$item.Method
    [pscustomobject]@{Name=$item.Name;Path=$item.Path;Bytes=$size;GiB=[math]::Round($size/1GB,2);Method=$method;Risk=$item.Risk;Action=$item.Action}
}
$results | Format-Table Name,GiB,Method -AutoSize
foreach($item in $results){Write-Output "[$($item.Name)] $($item.GiB) GiB`n路径：$($item.Path)`n建议：$($item.Method)`n说明：$($item.Risk)"}
if($ReportPath){$results|ConvertTo-Json -Depth 4|Set-Content -LiteralPath $ReportPath -Encoding UTF8;Write-Output "报告：$ReportPath"}
