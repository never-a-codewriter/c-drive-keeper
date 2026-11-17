$ErrorActionPreference='Stop'
$root=Join-Path $PSScriptRoot ('fixture-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
$source=Join-Path $root 'app';$backup=Join-Path $root 'app.keeper-original-test';$target=Join-Path $root 'partial-target'
New-Item -ItemType Directory -Path $backup,$target | Out-Null
[IO.File]::WriteAllText((Join-Path $backup 'important'),'original data')
[IO.File]::WriteAllText((Join-Path $target 'partial'),'partial copy')
$journal=Join-Path $root 'source-renamed.json'
@{stage='source-renamed';source=$source;source_backup=$backup;target=$target}|ConvertTo-Json|Set-Content $journal -Encoding UTF8
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot\..\scripts\recover-migration.ps1" -JournalPath $journal -Apply
if($LASTEXITCODE -ne 0 -or (Get-Content (Join-Path $source 'important') -Raw) -ne 'original data' -or -not(Test-Path (Join-Path $target 'partial'))){throw 'Interrupted migration recovery failed'}
$ErrorActionPreference='Continue'
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot\..\scripts\recover-migration.ps1" -JournalPath $journal -Apply *> $null
$ErrorActionPreference='Stop'
if($LASTEXITCODE -eq 0){throw 'Recovery overwrite guard failed'}
Write-Host "PASS: interrupted migration recovery, target preserved, overwrite refused. Fixture: $root"

