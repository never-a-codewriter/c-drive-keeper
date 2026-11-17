<# Default preview. Apply requires explicit IDs and a backup on another drive. #>
[CmdletBinding()]
param([switch]$Apply,[string[]]$Only,[string]$BackupRoot,[string]$ReportPath,
    [ValidateRange(1,3650)][int]$MinAgeDays=7,[switch]$Deep,[switch]$IncludeRecycleBin,[switch]$IncludeOtherDrives)
$ErrorActionPreference='Stop'
. "$PSScriptRoot\cleanup-core.ps1"
$known=Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders'
$local=[Environment]::ExpandEnvironmentVariables($known.'Local AppData')
$roaming=[Environment]::ExpandEnvironmentVariables($known.AppData)
if (-not $local -or -not $roaming -or $local -match '#!sandbox') { throw 'Cannot resolve real AppData.' }
$catalog=@(
    @{Id='temp';Path=(Join-Path $local 'Temp')},
    @{Id='crash-dumps';Path=(Join-Path $local 'CrashDumps')},
    @{Id='installer-temp';Path=(Join-Path $local 'SquirrelTemp')},
    @{Id='amd-cache';Path=(Join-Path $local 'AMD\DX9Cache')}
)
foreach($app in @('Kiro','Code')) {
    foreach($folder in @('Cache','Code Cache','GPUCache','CachedData','CachedExtensionVSIXs','logs')) {
        $catalog+=@{Id=($app.ToLower()+'-'+($folder -replace ' ','-').ToLower());Path=(Join-Path $roaming "$app\$folder")}
    }
}
if($IncludeOtherDrives) {
    foreach($pair in @(@('pip-cache','PIP_CACHE_DIR'),@('uv-cache','UV_CACHE_DIR'))) {
        $cache=[Environment]::GetEnvironmentVariable($pair[1],'User')
        if($cache -and [IO.Path]::IsPathRooted($cache) -and -not $cache.StartsWith($env:SystemDrive,[StringComparison]::OrdinalIgnoreCase)) {
            $catalog+=@{Id=$pair[0];Path=$cache}
        }
    }
    if(Get-Command npm.cmd -ErrorAction SilentlyContinue) {
        $cache=(& npm.cmd config get cache 2>$null | Select-Object -Last 1)
        if($LASTEXITCODE -eq 0 -and $cache -and [IO.Path]::IsPathRooted($cache) -and -not $cache.StartsWith($env:SystemDrive,[StringComparison]::OrdinalIgnoreCase)) {
            $catalog+=@{Id='npm-cache';Path=$cache}
        }
    }
}
$Only=@($Only | ForEach-Object {$_ -split ','} | Where-Object {$_})
foreach($id in $Only) {if($catalog.Id -notcontains $id){throw "Unknown ID: $id"}}
if($Apply -and (-not $Only -or -not $BackupRoot)){throw 'Apply requires -Only and -BackupRoot.'}
if($Deep){throw 'Deep cleanup is separate: use Windows Disk Cleanup.'}
if(-not $ReportPath){$ReportPath=Join-Path (Get-Location) ('cleanup-'+(Get-Date -Format 'yyyyMMdd-HHmmss-fff')+'.json')}
$report=[IO.Path]::GetFullPath($ReportPath)
if(Test-Path -LiteralPath $report){throw 'Report exists; use a new path.'}
@{Status='Started'} | ConvertTo-Json | Set-Content -LiteralPath $report -Encoding UTF8
$results=@()
$before=(Get-PSDrive -Name $env:SystemDrive.TrimEnd(':')).Free
foreach($entry in $catalog){
    if($Only -and $Only -notcontains $entry.Id){continue}
    $result=Invoke-CacheCleanup -Root $entry.Path -Id $entry.Id -Apply:$Apply -BackupRoot $BackupRoot -MinAgeDays $MinAgeDays
    $results+=$result
    $results | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $report -Encoding UTF8
    $result | Select-Object Id,Path,Status,EligibleBytes,RemovedBytes | Format-List
    foreach($message in $result.Errors){Write-Warning $message}
}
if($IncludeRecycleBin) {
    if($Apply) {
        try {
            Clear-RecycleBin -Force -ErrorAction Stop
            $results += @{Id='recycle-bin';Status='Completed';Note='All drives; irreversible; no backup'}
        } catch {
            $results += @{Id='recycle-bin';Status='Failed';Errors=@($_.Exception.Message)}
            Write-Warning $_.Exception.Message
        }
    } else {
        Write-Warning 'Apply will empty Recycle Bin on all drives. Size unknown; no backup.'
    }
    $results | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $report -Encoding UTF8
}
$after=(Get-PSDrive -Name $env:SystemDrive.TrimEnd(':')).Free
Write-Host "Report: $report"
Write-Host "Free space delta (includes other applications): $($after-$before) bytes"
Write-Host 'Logical sizes are not hard-link-deduplicated physical usage. Applications and system files are excluded. Recycle Bin requires -IncludeRecycleBin.'
if(@($results | Where-Object {$_.Status -in @('Failed','Partial')}).Count){exit 1}
