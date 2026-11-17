function Assert-PlainPath([string]$Path) {
    $current = [IO.Path]::GetFullPath($Path)
    while ($current) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force -ErrorAction Stop
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Link refused: $current" }
        }
        $current = Split-Path -Parent $current
    }
}
function Invoke-CacheCleanup {
    param([string]$Root,[string]$Id,[switch]$Apply,[string]$BackupRoot,[int]$MinAgeDays=7,[object[]]$ApprovedFiles,[switch]$UseSnapshot)
    $r = [ordered]@{Id=$Id;Path=$Root;Status='Preview';EligibleBytes=0L;RemovedBytes=0L;Errors=@();Files=@();Candidates=@()}
    try {
        $p = [IO.Path]::GetFullPath($Root).TrimEnd('\')
        if ($p.Length -lt 10) { throw 'Root too broad.' }
        Assert-PlainPath $p
        try { $item = Get-Item -LiteralPath $p -Force -ErrorAction Stop }
        catch [System.Management.Automation.ItemNotFoundException] { $r.Status='Missing'; return [pscustomobject]$r }
        if (-not $item.PSIsContainer) { throw 'Expected directory.' }
        if ($Apply) {
            $backup = [IO.Path]::GetFullPath($BackupRoot)
            if ($backup.TrimEnd('\').Equals($p,[StringComparison]::OrdinalIgnoreCase) -or $backup.StartsWith($p+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Backup must be outside target.' }
            Assert-PlainPath $backup
            $backup = Join-Path $backup ($Id+'-'+[guid]::NewGuid().ToString('N'))
            New-Item -ItemType Directory -Path $backup -Force -ErrorAction Stop | Out-Null
        }
        $queue = New-Object 'System.Collections.Generic.Queue[string]'
        $queue.Enqueue($p)
        $cutoff = (Get-Date).AddDays(-$MinAgeDays)
        while ($queue.Count) {
            $directory = $queue.Dequeue()
            try { Assert-PlainPath $directory; $children = @(Get-ChildItem -LiteralPath $directory -Force -ErrorAction Stop) }
            catch { $r.Errors += $_.Exception.Message; continue }
            foreach ($file in $children) {
                if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) { $r.Errors += "Link skipped: $($file.FullName)"; continue }
                if ($file.PSIsContainer) { $queue.Enqueue($file.FullName); continue }
                if ($file.LastWriteTime -gt $cutoff) { continue }
                if ($UseSnapshot) {
                    $approved = @($ApprovedFiles | Where-Object { $_.Path -eq $file.FullName })
                    if ($approved.Count -ne 1) { continue }
                    if ([int64]$approved[0].Bytes -ne $file.Length -or [string]$approved[0].ModifiedUtc -ne $file.LastWriteTimeUtc.ToString('o')) {
                        $r.Errors += "Changed since preview: $($file.FullName)"; continue
                    }
                }
                $r.Candidates += @{Path=$file.FullName;Bytes=$file.Length;ModifiedUtc=$file.LastWriteTimeUtc.ToString('o')}
                $r.EligibleBytes += $file.Length
                if (-not $Apply) { continue }
                try {
                    if (-not $file.FullName.StartsWith($p+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Outside target.' }
                    Assert-PlainPath $file.FullName
                    $destination = Join-Path $backup $file.FullName.Substring($p.Length+1)
                    New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force -ErrorAction Stop | Out-Null
                    Copy-Item -LiteralPath $file.FullName -Destination $destination -ErrorAction Stop
                    $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256 -ErrorAction Stop).Hash
                    if ($hash -ne (Get-FileHash -LiteralPath $destination -Algorithm SHA256 -ErrorAction Stop).Hash) { throw 'Backup mismatch.' }
                    $fresh = Get-Item -LiteralPath $file.FullName -Force -ErrorAction Stop
                    if ($fresh.Length -ne $file.Length -or $fresh.LastWriteTime -ne $file.LastWriteTime) { throw 'File changed during backup.' }
                    Assert-PlainPath $file.FullName
                    Remove-Item -LiteralPath $file.FullName -Force -ErrorAction Stop
                    $r.RemovedBytes += $file.Length
                    $r.Files += @{Source=$file.FullName;Backup=$destination;SHA256=$hash;Bytes=$file.Length}
                } catch { $r.Errors += "$($file.FullName): $($_.Exception.Message)" }
            }
        }
        if ($Apply) { $r.Status='Completed' }
    } catch { $r.Status='Failed'; $r.Errors += $_.Exception.Message }
    if ($r.Errors.Count -and $r.Status -ne 'Failed') { $r.Status='Partial' }
    [pscustomobject]$r
}


