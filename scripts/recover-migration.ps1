[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$JournalPath,[switch]$Apply)
$ErrorActionPreference='Stop'
. "$PSScriptRoot\cleanup-core.ps1"
$j=Get-Content -LiteralPath $JournalPath -Raw -Encoding UTF8 | ConvertFrom-Json
$source=[IO.Path]::GetFullPath($j.source)
$backup=[IO.Path]::GetFullPath($j.source_backup)
Assert-PlainPath $source;Assert-PlainPath $backup
if((Split-Path -Parent $source) -ne (Split-Path -Parent $backup)){throw 'Backup must be beside original source'}
$prefix=(Split-Path -Leaf $source)+'.keeper-original-'
if(-not (Split-Path -Leaf $backup).StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase)){throw 'Not a recognized migration backup'}
if(Test-Path -LiteralPath $source){throw 'Original location already exists: refusing overwrite or junction removal'}
if(-not (Test-Path -LiteralPath $backup -PathType Container)){throw 'Original backup missing; inspect target copy manually'}
Write-Output "Restore original directory name: $backup -> $source; keep target copy untouched."
if($Apply){Move-Item -LiteralPath $backup -Destination $source -ErrorAction Stop
    if(-not(Test-Path -LiteralPath $source -PathType Container)){throw 'Recovery verification failed'}
    Write-Output 'Recovered original directory. Target copy was retained.'
}
