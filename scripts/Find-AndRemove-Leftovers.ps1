<#
Find-AndRemove-Leftovers.ps1
Enhanced version: preview save/load, GUI selection, whitelist/blacklist, zip backup, logs
Place under scripts/ in repo
Usage examples:
  # Preview and save results
  .\scripts\Find-AndRemove-Leftovers.ps1 -Name "phanthywork" -Preview -SavePreview "C:\Users\51139\Downloads\preview-phanthywork.json" -UseGui

  # Apply from saved preview (no re-scan) and delete
  .\scripts\Find-AndRemove-Leftovers.ps1 -ApplyFrom "C:\Users\51139\Downloads\preview-phanthywork.json" -Delete -AutoZipBackup

Notes:
 - Run as Administrator for best coverage.
 - By default Preview mode does not delete. Use -Delete to perform deletion.
 - -UseGui enables Out-GridView selection (Windows desktop required).
#>

param(
  [string] $Name = "",
  [switch] $Preview = $true,
  [switch] $Delete,
  [switch] $Deep,
  [string] $BackupRoot = "",
  [string] $SavePreview = "",
  [string] $ApplyFrom = "",
  [string] $WhitelistPath = "",
  [string] $BlacklistPath = "",
  [switch] $UseGui,
  [switch] $AutoZipBackup
)

function Log-Info { param($m) Write-Output ("[INFO]  $(Get-Date -Format o)  $m") }
function Log-Warn { param($m) Write-Warning ("[WARN]  $(Get-Date -Format o)  $m") }
function Log-Err  { param($m) Write-Error ("[ERR]   $(Get-Date -Format o)  $m") }

function Assert-Admin {
  if (-not ([bool]([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator))) {
    Log-Warn "Not running as Administrator. Some registry or system locations may be inaccessible."
  }
}

function New-BackupFolder {
  param([string] $root)
  if (-not $root) { $root = Join-Path -Path (Split-Path -Parent $PSCommandPath) -ChildPath "backups" }
  $stamp = (Get-Date).ToString("yyyyMMdd_HHmmss")
  $path = Join-Path $root $stamp
  New-Item -Path $path -ItemType Directory -Force | Out-Null
  return (Resolve-Path $path).Path
}

function Zip-Backup {
  param([string] $dir, [string] $zipPath)
  try {
    if (Test-Path $zipPath) { Remove-Item $zipPath -Force -ErrorAction SilentlyContinue }
    Compress-Archive -Path (Join-Path $dir '*') -DestinationPath $zipPath -Force -ErrorAction Stop
    return $true
  } catch {
    Log-Warn "Compress-Archive failed: $_"
    return $false
  }
}

function Find-UninstallRegistryKeys {
  param([string] $keyword)
  $hives = @(
    "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    "HKLM:\SOFTWARE\Wow6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    "HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"
  )
  $found = @()
  foreach ($h in $hives) {
    try {
      Get-ChildItem -Path $h -ErrorAction SilentlyContinue | ForEach-Object {
        $props = Get-ItemProperty -Path $_.PSPath -ErrorAction SilentlyContinue
        $display = $props.DisplayName
        if ($display -and ($display -match [regex]::Escape($keyword))) {
          $found += [PSCustomObject]@{
            Hive = $h
            KeyPath = $_.PSPath
            DisplayName = $display
            DisplayVersion = $props.DisplayVersion
            UninstallString = $props.UninstallString
          }
        }
      }
    } catch { }
  }
  return $found
}

function Find-RegistryByValueOrName {
  param([string] $keyword, [int] $max=200)
  $roots = @("HKLM:\SOFTWARE", "HKLM:\SOFTWARE\Wow6432Node", "HKCU:\SOFTWARE")
  $matches = @()
  foreach ($r in $roots) {
    try {
      # limit recursion depth: use Get-ChildItem with -Depth on PS Core; emulate depth in 5.1 by catching long recursions
      Get-ChildItem -Path $r -Recurse -ErrorAction SilentlyContinue -Force -Depth 3 | ForEach-Object {
        try {
          $p=Get-ItemProperty -Path $_.PSPath -ErrorAction SilentlyContinue
          foreach ($n in $p.PSObject.Properties) {
            if ($n.Value -and ($n.Value.ToString() -match [regex]::Escape($keyword) -or $n.Name -match [regex]::Escape($keyword))) {
              $matches += [PSCustomObject]@{ Path = $_.PSPath; Property = $n.Name; Value = $n.Value }
            }
          }
        } catch { }
      }
    } catch { }
    if ($matches.Count -gt $max) { break }
  }
  return $matches
}

function Find-FsLocations {
  param([string] $keyword, [switch] $deep)
  $common = @(
    "$env:ProgramFiles",
    "$env:ProgramFiles(x86)",
    "$env:ProgramData",
    "$env:LOCALAPPDATA",
    "$env:APPDATA",
    "C:\Users\Public\Desktop",
    "C:\Users\Public\Start Menu",
    "C:\Windows\Installer"
  )
  $found = @()
  foreach ($p in $common) {
    if (-not (Test-Path $p)) { continue }
    try {
      Get-ChildItem -Path $p -Force -ErrorAction SilentlyContinue -Recurse -Directory |
        Where-Object { $_.Name -match [regex]::Escape($keyword) } |
        ForEach-Object { $found += $_.FullName }
    } catch { }
  }

  if ($deep) {
    Log-Info "Deep search enabled: scanning C:\ (this may be slow)..."
    try {
      Get-ChildItem -Path C:\ -Force -ErrorAction SilentlyContinue -Recurse -Directory |
        Where-Object { $_.Name -match [regex]::Escape($keyword) } |
        ForEach-Object { $found += $_.FullName }
    } catch { }
  }

  return $found | Sort-Object -Unique
}

function Find-ServicesMatching {
  param([string] $keyword)
  $kw = [regex]::Escape($keyword)
  Get-CimInstance -ClassName Win32_Service -ErrorAction SilentlyContinue |
    Where-Object { ($_.Name -match $kw) -or ($_.DisplayName -match $kw) -or ($_.PathName -match $kw) } |
    Select-Object Name, DisplayName, State, StartMode, PathName
}

function Find-StartupEntries {
  param([string] $keyword)
  $paths = @(
    "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run",
    "HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run",
    "HKLM:\SOFTWARE\Wow6432Node\Microsoft\Windows\CurrentVersion\Run"
  )
  $kw = [regex]::Escape($keyword)
  $results = @()
  foreach ($p in $paths) {
    try {
      Get-ItemProperty -Path $p -ErrorAction SilentlyContinue | ForEach-Object {
        foreach ($n in $_.PSObject.Properties) {
          if ($n.Name -eq "PSPath") { continue }
          if ($n.Value -and ($n.Value.ToString() -match $kw -or $n.Name -match $kw)) {
            $results += [PSCustomObject]@{ Path = $p; Name = $n.Name; Value = $n.Value }
          }
        }
      }
    } catch { }
  }
  return $results
}

function Backup-RegistryKey {
  param([string] $regPath, [string] $backupDir)
  $fileName = (Join-Path $backupDir ("reg_" + ((Get-Date).ToString("yyyyMMdd_HHmmss")) + "_" + ([io.path]::GetRandomFileName() + ".reg")))
  $regExp = $regPath -replace "^HKLM:\\","HKLM\\" -replace "^HKCU:\\","HKCU\\"
  try {
    & reg export "$regExp" "$fileName" /y 2>$null
    if (Test-Path $fileName) { return $fileName } else { return $null }
  } catch { return $null }
}

function Backup-AndRemoveItems {
  param(
    [string[]] $pathsToRemove,
    [string] $backupDir,
    [switch] $actuallyDelete
  )
  $copied = @()
  foreach ($p in $pathsToRemove) {
    try {
      if (-not (Test-Path $p)) { continue }
      $leaf = (Split-Path $p -Leaf)
      $dest = Join-Path $backupDir $leaf
      if ((Get-Item $p).PSIsContainer) {
        Copy-Item -Path $p -Destination $dest -Recurse -Force -ErrorAction SilentlyContinue
      } else {
        Copy-Item -Path $p -Destination $dest -Force -ErrorAction SilentlyContinue
      }
      $copied += [PSCustomObject]@{ Source = $p; Backup = $dest }
      if ($actuallyDelete) {
        Remove-Item -Path $p -Recurse -Force -ErrorAction SilentlyContinue
      }
    } catch {
      Log-Warn "Backup or delete failed for $p : $_"
    }
  }
  return $copied
}

function Save-PreviewToFile {
  param([string] $path, [object] $obj)
  try {
    $json = $obj | ConvertTo-Json -Depth 8
    Set-Content -Path $path -Value $json -Encoding UTF8
    Log-Info "Preview saved to: $path"
    return $true
  } catch {
    Log-Warn "Unable to save preview to $path : $_"
    return $false
  }
}

function Load-PreviewFromFile {
  param([string] $path)
  if (-not (Test-Path $path)) { Throw "Preview file not found: $path" }
  try {
    $raw = Get-Content -Raw -Path $path -ErrorAction Stop
    $obj = $raw | ConvertFrom-Json
    return $obj
  } catch {
    Throw "Failed to load/parse preview file: $_"
  }
}

function Show-SelectionGui {
  param([array] $items, [string] $title = "Select items to delete")
  if (-not $UseGui) { return $items }
  try {
    # Out-GridView expects objects; return selected objects
    $sel = $items | Out-GridView -Title $title -PassThru
    if (-not $sel) { return @() }
    return $sel
  } catch {
    Log-Warn "GUI selection unavailable: $_. Falling back to all items."
    return $items
  }
}

# Load whitelist/blacklist
function Load-ListFromFile { param([string]$path) if(-not $path){ return @() } if(-not (Test-Path $path)){ Log-Warn "List file not found: $path"; return @() } return Get-Content -Path $path | Where-Object { $_ -and -not ($_ -match '^[#;]') } }

# Main flow
Assert-Admin

if ($ApplyFrom) {
  Log-Info "ApplyFrom mode: loading saved preview from $ApplyFrom"
  $preview = Load-PreviewFromFile -path $ApplyFrom
  # Restore lists
  $Name = $preview.Keyword
  $regUninst = $preview.Uninstall
  $regLoose = $preview.RegistryLoose
  $fs = @()
  if ($preview.Files) { $fs = $preview.Files }
  $services = $preview.Services
  $startup = $preview.Startup
  $backupDir = $preview.BackupDir
  if (-not $backupDir) { $backupDir = New-BackupFolder -root $BackupRoot }
  New-Item -Path $backupDir -ItemType Directory -Force | Out-Null
  Log-Info "Loaded preview for keyword: $Name. Loaded counts: Uninstall=$($regUninst.Count), RegistryLoose=$($regLoose.Count), Files=$($fs.Count), Services=$($services.Count), Startup=$($startup.Count)"
  # Continue to deletion flow below (skip rescanning)
} else {
  if (-not $Name) { 
    # If mandatory not provided, prompt
    $Name = Read-Host "Enter keyword to search for"
  }
  Log-Info "Searching keyword: $Name"

  if (-not $BackupRoot) { $BackupRoot = Join-Path $env:USERPROFILE "Downloads\backups" }
  $backupDir = New-BackupFolder -root $BackupRoot

  $regUninst = Find-UninstallRegistryKeys -keyword $Name
  $regLoose = Find-RegistryByValueOrName -keyword $Name
  $fs = Find-FsLocations -keyword $Name -deep:$Deep
  $services = Find-ServicesMatching -keyword $Name
  $startup = Find-StartupEntries -keyword $Name

  Log-Info "==== Uninstall registry keys ===="
  if ($regUninst.Count -eq 0) { Log-Info "No Uninstall keys found." } else { $regUninst | Format-Table -AutoSize }

  Log-Info "==== Registry loose matches ===="
  if ($regLoose.Count -eq 0) { Log-Info "No loose registry matches found." } else { $regLoose | Format-Table -AutoSize }

  Log-Info "==== Files/Folders (common paths) ===="
  if ($fs.Count -eq 0) { Log-Info "No common path matches found." } else { $fs | ForEach-Object { Write-Output $_ } }

  Log-Info "==== Services ===="
  if ($services.Count -eq 0) { Log-Info "No services found." } else { $services | Format-Table -AutoSize }

  Log-Info "==== Startup entries ===="
  if ($startup.Count -eq 0) { Log-Info "No startup entries found." } else { $startup | Format-Table -AutoSize }

  # Save preview if requested
  $previewObj = [PSCustomObject]@{
    Keyword = $Name
    Timestamp = (Get-Date).ToString("o")
    Uninstall = $regUninst
    RegistryLoose = $regLoose
    Files = $fs
    Services = $services
    Startup = $startup
    BackupDir = $backupDir
  }
  if ($SavePreview) { Save-PreviewToFile -path $SavePreview -obj $previewObj }
}

# If only preview requested and not ApplyFrom-delete, exit
if ($Preview -and -not $Delete -and -not $ApplyFrom) {
  Log-Info "Preview mode (no deletion). Backup folder created: $backupDir"
  if ($SavePreview) { Log-Info "Preview saved to: $SavePreview" }
  exit 0
}

# At this point either ApplyFrom was provided (we loaded lists) or Delete flag set after a real-time search
# Prepare whitelist/blacklist
$whitelist = Load-ListFromFile -path $WhitelistPath
$blacklist = Load-ListFromFile -path $BlacklistPath

# Build unified file list objects for GUI/selection
$fileObjs = @()
foreach ($f in $fs) { $fileObjs += [PSCustomObject]@{ Type='File'; Path=$f; Note='' } }
foreach ($r in $regLoose) { $fileObjs += [PSCustomObject]@{ Type='RegistryValue'; Path=$r.Path; Property=$r.Property; Note='' } }
foreach ($u in $regUninst) { $fileObjs += [PSCustomObject]@{ Type='UninstallKey'; Path=$u.KeyPath; DisplayName=$u.DisplayName; Note='' } }
foreach ($s in $services) { $fileObjs += [PSCustomObject]@{ Type='Service'; Name=$s.Name; DisplayName=$s.DisplayName; Path=$s.PathName } }
foreach ($st in $startup) { $fileObjs += [PSCustomObject]@{ Type='Startup'; Path=$st.Path; Name=$st.Name; Value=$st.Value } }

# Apply whitelist/blacklist rules: whitelist -> exclude from deletion, blacklist -> force include
$toDelete = @()
foreach ($item in $fileObjs) {
  $path = $null
  if ($item.PSObject.Properties.Match('Path')) { $path = $item.Path } elseif ($item.PSObject.Properties.Match('Name')) { $path = $item.Name }
  $skip = $false
  foreach ($w in $whitelist) { if ($path -and ($path -match $w)) { $skip = $true } }
  if ($skip) { continue }
  $force = $false
  foreach ($b in $blacklist) { if ($path -and ($path -match $b)) { $force = $true } }
  if ($force) { $toDelete += $item; continue }
  # default: include filesystem/registry matches; keep uninstall keys, services and startup items
  $toDelete += $item
}

if ($toDelete.Count -eq 0) { Log-Info "No items selected for deletion after applying whitelist/blacklist. Exiting."; exit 0 }

# If GUI requested, show selection and let user choose subset
if ($UseGui) {
  $sel = Show-SelectionGui -items $toDelete -title "Select items to delete (multi-select then OK)"
  if (-not $sel -or $sel.Count -eq 0) { Log-Info "No items selected in GUI. Exiting."; exit 0 }
  $toDelete = $sel
}

# Confirm action with user (unless ApplyFrom + Delete can skip prompt)
if (-not $Delete) {
  Log-Info "Deletion not requested (flag -Delete missing). Exiting without changes. Use -Delete to perform deletion."
  exit 0
}

$yn = Read-Host "Confirm execution: backup then delete the selected $($toDelete.Count) items? Type Y to confirm"
if ($yn -ne 'Y' -and $yn -ne 'y') { Log-Info "User cancelled. Exiting."; exit 1 }

# Perform deletion: 1) backup registry keys, 2) copy files/folders, 3) remove registry props/keys, files, services, startup
$logRecords = @()

# Ensure backup dir exists
if (-not (Test-Path $backupDir)) { New-Item -Path $backupDir -ItemType Directory -Force | Out-Null }

# Helper to record action
function Record { param($rec) $logRecords += $rec }

# 1) Export uninstall keys and registry keys for registry-value entries
foreach ($i in $toDelete | Where-Object { $_.Type -eq 'UninstallKey' }) {
  try {
    $bk = Backup-RegistryKey -regPath $i.Path -backupDir $backupDir
    Record @{ Item = $i; Action = 'BackupUninstallKey'; Backup = $bk }
    Remove-Item -Path $i.Path -Recurse -Force -ErrorAction SilentlyContinue
    Record @{ Item = $i; Action = 'RemovedUninstallKey' }
    Log-Info "Removed Uninstall key: $($i.Path)"
  } catch { Log-Warn "Failed to remove uninstall key $($i.Path): $_" }
}

foreach ($i in $toDelete | Where-Object { $_.Type -eq 'RegistryValue' }) {
  try {
    $bk = Backup-RegistryKey -regPath $i.Path -backupDir $backupDir
    Record @{ Item = $i; Action='BackupRegistryValue'; Backup=$bk }
    Remove-ItemProperty -Path $i.Path -Name $i.Property -ErrorAction SilentlyContinue
    Record @{ Item = $i; Action='RemovedRegistryValue' }
    Log-Info "Removed registry value $($i.Property) at $($i.Path)"
  } catch { Log-Warn "Failed to remove registry property $($i.Property) at $($i.Path): $_" }
}

# 2) Backup & delete filesystem items
$fsTo = $toDelete | Where-Object { $_.Type -eq 'File' }
$fsPaths = $fsTo | ForEach-Object { $_.Path }
$copied = Backup-AndRemoveItems -pathsToRemove $fsPaths -backupDir $backupDir -actuallyDelete
foreach ($c in $copied) { Record @{ Item = $c.Source; Action='BackupAndRemove'; Backup=$c.Backup } }

# 3) Services
foreach ($i in $toDelete | Where-Object { $_.Type -eq 'Service' }) {
  try {
    if ($i.State -ne 'Stopped') { Stop-Service -Name $i.Name -Force -ErrorAction SilentlyContinue }
    sc.exe delete $i.Name | Out-Null
    Record @{ Item = $i; Action='DeletedService' }
    Log-Info "Requested deletion of service: $($i.Name)"
  } catch { Log-Warn "Failed to delete service $($i.Name): $_" }
}

# 4) Startup entries
foreach ($i in $toDelete | Where-Object { $_.Type -eq 'Startup' }) {
  try {
    Remove-ItemProperty -Path $i.Path -Name $i.Name -ErrorAction SilentlyContinue
    Record @{ Item = $i; Action='RemovedStartup' }
    Log-Info "Removed startup entry $($i.Name) at $($i.Path)"
  } catch { Log-Warn "Failed to remove startup $($i.Name) at $($i.Path): $_" }
}

# Create log and optionally zip backup
$logFile = Join-Path $backupDir ("cleanup_log_" + ((Get-Date).ToString("yyyyMMdd_HHmmss")) + ".json")
try {
  $logRecords | ConvertTo-Json -Depth 8 | Set-Content -Path $logFile -Encoding UTF8
  Log-Info "Operation log written to: $logFile"
} catch { Log-Warn "Failed to write operation log: $_" }

if ($AutoZipBackup) {
  $zipPath = Join-Path (Split-Path $backupDir -Parent) (Split-Path $backupDir -Leaf) + ".zip"
  if (Zip-Backup -dir $backupDir -zipPath $zipPath) { Log-Info "Backup zipped to: $zipPath" } else { Log-Warn "Failed to create zip for backup dir: $backupDir" }
}

Log-Info "Deletion completed. Backup dir: $backupDir"

# Exit
exit 0
