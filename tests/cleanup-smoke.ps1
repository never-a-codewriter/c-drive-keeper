$ErrorActionPreference='Stop'
. "$PSScriptRoot\..\scripts\cleanup-core.ps1"
$fixture=Join-Path $PSScriptRoot ('fixture-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixture | Out-Null
$old=Join-Path $fixture 'old[1].txt'
[IO.File]::WriteAllText($old,'preserve me')
(Get-Item -LiteralPath $old).LastWriteTime=(Get-Date).AddDays(-10)
[IO.File]::WriteAllText((Join-Path $fixture 'new.txt'),'recent')
$r=Invoke-CacheCleanup -Root $fixture -Id test
if($r.Status -ne 'Preview' -or $r.EligibleBytes -ne 11 -or -not(Test-Path -LiteralPath $old)){throw 'Preview/age/literal path failed'}
$r=Invoke-CacheCleanup -Root $fixture -Id test -Apply -BackupRoot (Join-Path $fixture 'backup')
if($r.Status -ne 'Failed' -or -not(Test-Path -LiteralPath $old)){throw 'Backup guard failed'}
$backup=Join-Path $PSScriptRoot ('backup-'+[guid]::NewGuid().ToString('N'))
$r=Invoke-CacheCleanup -Root $fixture -Id test -Apply -BackupRoot $backup
if($r.Status -ne 'Completed' -or $r.RemovedBytes -ne 11 -or (Test-Path -LiteralPath $old) -or -not(Test-Path -LiteralPath (Join-Path $fixture 'new.txt'))){throw 'Verified cleanup failed'}
if((Get-Content -LiteralPath $r.Files[0].Backup -Raw) -ne 'preserve me'){throw 'Backup content failed'}
Remove-Item -LiteralPath $r.Files[0].Backup -Force
Remove-Item -LiteralPath (Split-Path -Parent $r.Files[0].Backup),$backup -Force
Remove-Item -LiteralPath (Join-Path $fixture 'new.txt') -Force
Remove-Item -LiteralPath $fixture -Force
Write-Host 'PASS: preview, age filter, literal filenames, nested backup rejection, verified backup/delete, recent-file preservation'

