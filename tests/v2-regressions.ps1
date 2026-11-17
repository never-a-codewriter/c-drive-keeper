$ErrorActionPreference='Stop'
. "$PSScriptRoot\..\scripts\cleanup-core.ps1"
$root=Join-Path $PSScriptRoot ('fixture-'+[guid]::NewGuid().ToString('N'))
$source=Join-Path $root 'source';$backup=Join-Path $root 'backup'
New-Item -ItemType Directory -Path $source | Out-Null
$old=Join-Path $source 'old.txt';[IO.File]::WriteAllText($old,'old');(Get-Item $old).LastWriteTime=(Get-Date).AddDays(-20)
$preview=Invoke-CacheCleanup -Root $source -Id test
[IO.File]::WriteAllText($old,'changed');(Get-Item $old).LastWriteTime=(Get-Date).AddDays(-20)
$new=Join-Path $source 'not-in-preview.txt';[IO.File]::WriteAllText($new,'new');(Get-Item $new).LastWriteTime=(Get-Date).AddDays(-20)
$r=Invoke-CacheCleanup -Root $source -Id test -Apply -BackupRoot $backup -UseSnapshot -ApprovedFiles $preview.Candidates
if($r.RemovedBytes -ne 0 -or -not(Test-Path $old) -or -not(Test-Path $new) -or $r.Status -ne 'Partial'){throw 'Snapshot guard failed'}
$preview=Invoke-CacheCleanup -Root $source -Id test
$lock=[IO.File]::Open($old,[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
try{$r=Invoke-CacheCleanup -Root $source -Id test -Apply -BackupRoot $backup -UseSnapshot -ApprovedFiles $preview.Candidates}finally{$lock.Dispose()}
if(-not(Test-Path $old) -or $r.Status -ne 'Partial'){throw 'Locked-file preservation failed'}
$report=Join-Path $root 'report.json';$r | ConvertTo-Json -Depth 6 | Set-Content $report -Encoding UTF8
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot\..\scripts\restore-cleanup.ps1" -ReportPath $report -Apply
if($LASTEXITCODE -ne 0 -or (Get-Content $new -Raw) -ne 'new'){throw 'Restore failed'}
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot\..\scripts\restore-cleanup.ps1" -ReportPath $report -Apply *> $null
if($LASTEXITCODE -eq 0 -or (Get-Content $new -Raw) -ne 'new'){throw 'Restore overwrite guard failed'}
Write-Host "PASS: changed/new files excluded, locked source preserved, verified restore, overwrite refused. Fixture retained: $root"
