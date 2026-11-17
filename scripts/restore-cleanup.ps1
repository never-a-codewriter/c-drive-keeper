[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$ReportPath,[switch]$Apply,[string]$OutputPath)
$ErrorActionPreference='Stop'
. "$PSScriptRoot\cleanup-core.ps1"
$rows=@(Get-Content -LiteralPath $ReportPath -Raw -Encoding UTF8 | ConvertFrom-Json)
$results=@()
foreach($row in $rows){foreach($f in $row.Files){
    $status='Preview';$errorText=$null
    try {
        if(-not [IO.Path]::IsPathRooted($f.Source) -or -not [IO.Path]::IsPathRooted($f.Backup)){throw 'Absolute paths required'}
        Assert-PlainPath $f.Source; Assert-PlainPath $f.Backup
        if(Test-Path -LiteralPath $f.Source){throw 'Source now exists: refusing overwrite'}
        if((Get-FileHash -LiteralPath $f.Backup -Algorithm SHA256).Hash -ne $f.SHA256){throw 'Backup hash mismatch'}
        if($Apply){New-Item -ItemType Directory -Path (Split-Path -Parent $f.Source) -Force | Out-Null
            # CreateNew prevents overwriting a file created after the preview/check.
            $inputStream=[IO.File]::OpenRead($f.Backup)
            try{$outputStream=[IO.File]::Open($f.Source,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
                try{$inputStream.CopyTo($outputStream);$outputStream.Flush()}finally{$outputStream.Dispose()}
            }finally{$inputStream.Dispose()}
            if((Get-FileHash -LiteralPath $f.Source -Algorithm SHA256).Hash -ne $f.SHA256){throw 'Restore verification failed; backup retained'}
            $status='Restored'
        }
    }catch{$status='SkippedOrFailed';$errorText=$_.Exception.Message}
    $results+=[pscustomobject]@{Source=$f.Source;Backup=$f.Backup;Status=$status;Error=$errorText}
}}
if($OutputPath){$results | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $OutputPath -Encoding UTF8}
$results | Format-List
if(@($results | Where-Object Status -eq 'SkippedOrFailed').Count){exit 1}
