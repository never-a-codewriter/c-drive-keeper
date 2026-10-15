# c-drive-keeper

**Keep your Windows C: drive from filling up: audit first, preview next, clean last.** Built for the way AI desktop clients, MCP tools and Python/Node toolchains quietly eat system-drive space.

[中文](README.md)

## What's new (2026-10-16)

Added `scripts/tweak-registry.ps1`: 7 safe, reversible registry tweaks. Like the other scripts it **previews by default and only writes with `-Apply`**; original values are backed up first and can be restored exactly with one command.

| Id | What it does | Admin |
|---|---|---|
| `menu-delay` | Cascading menu delay 400ms → 100ms | No |
| `no-auto-reboot` | No automatic restart after updates while a user is signed in (may be partly ignored on Home) | Yes |
| `taskbar-preview` | Near-instant taskbar thumbnail previews | No |
| `cortana` | Disable Cortana (largely removed in Win11, usually no effect) | Yes |
| `show-ext` | Show file extensions (spot fakes like `xxx.pdf.exe`) | No |
| `no-recent-docs` | Stop tracking recently opened files | No |
| `fast-shutdown` | Service shutdown wait 5000ms → 2000ms (some services may not finish saving) | Yes |

```powershell
# Preview: show current vs target values, change nothing
powershell -ExecutionPolicy Bypass -File .\scripts\tweak-registry.ps1

# Apply all (use an elevated PowerShell, otherwise the 3 HKLM items are skipped)
powershell -ExecutionPolicy Bypass -File .\scripts\tweak-registry.ps1 -Apply -RestartExplorer

# Pick or skip items
powershell -ExecutionPolicy Bypass -File .\scripts\tweak-registry.ps1 -Apply -Only show-ext,menu-delay
powershell -ExecutionPolicy Bypass -File .\scripts\tweak-registry.ps1 -Apply -Skip fast-shutdown,cortana

# Roll back exactly to the previous state (the path is printed after -Apply)
powershell -ExecutionPolicy Bypass -File .\scripts\tweak-registry.ps1 -RestoreFrom "$env:USERPROFILE\Downloads\backups\registry_<timestamp>\original-values.json" -Apply

# Or reset to Windows defaults
powershell -ExecutionPolicy Bypass -File .\scripts\tweak-registry.ps1 -Restore -Apply
```

The backup folder `Downloads\backups\registry_<timestamp>` contains `original-values.json` (for rollback), `.reg` exports of the touched keys (double-click to import manually) and `tweak.log`.

> These tweaks don't free disk space; they're a small quality-of-life bonus. Only the values listed above are touched.

## What's new (2026-10-15)

A new, enhanced leftover cleaner has been added: `scripts/Find-AndRemove-Leftovers.ps1`.

Features:

- Keyword search: find uninstall entries, caches, startup items, services and registry keys related to a program name
- Preview mode: default is read-only and only shows candidate items
- Save/Load results: save scan results to JSON so you can apply deletions later without re-scanning
- Interactive selection: `-UseGui` uses Out-GridView on Windows desktop to let you pick items to delete
- Whitelist / Blacklist: ignore known-safe paths or force-include specific patterns
- Backup and zip: backups are created before deletion; `-AutoZipBackup` creates a ZIP archive
- Logging: detailed operation logs for rollback and auditing

This script is suitable for cleaning leftover files and settings after uninstalling apps, especially caches and logs from AI/office/development tools.

---

## Update on 2026/10/14

After running the scripts, do some manual steps for best results:

1) Windows Disk Cleanup (usually recovers the most space)

- Press Win+R, run `cleanmgr`, choose C:\
- Click "Clean up system files", choose C:\ again
- Select: Windows Update Cleanup, Temporary Files, Delivery Optimization Files, Device Driver Packages (do NOT select "Downloads")
- Click OK

2) In-app cleanups (optional)

- Chrome / other browsers: Ctrl+Shift+Delete and clear cached images and files only
- QQ / WeChat: clear cache from settings or move storage to D:\
- GitHub Desktop: remove older `app-x.x.x` folders under `%LOCALAPPDATA%\GitHubDesktop`, keep only the latest

---

## Why this project

Installing Cherry Studio, Codex, Cursor and many MCP tools often makes the C: drive shrink by tens of GB over months. Typical causes:

- Many apps store data under `C:\Users\<user>\AppData` or `C:\Users\<user>\.<name>` and don't follow install location
- Multiple tools may download their own copies of Node/Python/uv/bun etc., duplicating storage
- Package manager caches (uv/npm/pip) default to C: and grow over time
- Uninstallers usually leave user data behind

This project implements "inspect, clean, relocate" workflows with scripts and documentation.

## New script: Find and remove uninstall leftovers

Path: `scripts/Find-AndRemove-Leftovers.ps1`

Purpose:

- Given a program keyword (e.g. `phanthywork`, `Cursor`, `Code`, `Claude`), scans common leftover locations:
  - Uninstall registry keys
  - Related registry keys/values
  - `ProgramData` / `AppData` / `Temp` / `Cache` directories
  - Services and startup entries
- Default preview-only behavior — it only lists candidates
- Optionally save the preview to JSON and later run deletion against the saved preview (no re-scan)

### Basic usage

```powershell
# 1) Preview only (scan without deleting)
powershell -ExecutionPolicy Bypass -File .\scripts\Find-AndRemove-Leftovers.ps1 -Name "phanthywork" -Preview

# 2) Preview and save results (so you won't need to rescan later)
powershell -ExecutionPolicy Bypass -File .\scripts\Find-AndRemove-Leftovers.ps1 -Name "phanthywork" -Preview -SavePreview "C:\Users\51139\Downloads\preview-phanthywork.json"

# 3) Use GUI to select candidates to delete
powershell -ExecutionPolicy Bypass -File .\scripts\Find-AndRemove-Leftovers.ps1 -Name "phanthywork" -Preview -UseGui

# 4) Apply deletion using a saved preview (no re-scan) with automatic zip backup
powershell -ExecutionPolicy Bypass -File .\scripts\Find-AndRemove-Leftovers.ps1 -ApplyFrom "C:\Users\51139\Downloads\preview-phanthywork.json" -Delete -AutoZipBackup
```

### Whitelist / Blacklist files

You may create two text files with one regex per line (comments starting with `#` or `;`):

- `whitelist.txt`: patterns matched here are excluded from deletion
- `blacklist.txt`: patterns matched here are forced into deletion list

Example `whitelist.txt`:

```
# whitelist.txt
C:\Windows\\
C:\Program Files\\Common Files\\
C:\Users\\.*\\AppData\\Local\\Microsoft\\
```

Example `blacklist.txt`:

```
.*PhantyWork.*
.*phanthy-code-cli-nodejs.*
```

Use them like:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\Find-AndRemove-Leftovers.ps1 -Name "phanthywork" -Preview -WhitelistPath "C:\path\whitelist.txt" -BlacklistPath "C:\path\blacklist.txt"
```

### Backup & rollback

By default backups are written to:

```
$env:USERPROFILE\Downloads\backups\yyyyMMdd_HHmmss
```

If `-AutoZipBackup` is provided, the backup is also compressed to a `.zip` next to the backup folder. The script also writes an operation log (JSON) into the backup folder describing what was backed up and what was deleted.

## Quick start

Windows 10/11 with PowerShell 5.1 is required. No extra installs.

After cloning the repo, run the scripts (adjust paths to your environment). Many systems block running `.ps1`; the examples below use `-ExecutionPolicy Bypass`, which temporarily allows the current execution.

```powershell
# 1. Inspect: read-only overview of space usage
powershell -ExecutionPolicy Bypass -File .\scripts\audit.ps1

# 2. Preview possible cleanups (read-only)
powershell -ExecutionPolicy Bypass -File .\scripts\clean.ps1

# 3. Confirm and apply cleanups
powershell -ExecutionPolicy Bypass -File .\scripts\clean.ps1 -Apply

# 4. Move uv/pip/npm caches to D: (preview then -Apply)
powershell -ExecutionPolicy Bypass -File .\scripts\relocate-dev-caches.ps1 -Root D:\cache
powershell -ExecutionPolicy Bypass -File .\scripts\relocate-dev-caches.ps1 -Root D:\cache -Apply

# 5. (Optional) Move other caches and old C: caches
powershell -ExecutionPolicy Bypass -File .\scripts\relocate-more-caches.ps1 -Root D:\cache
powershell -ExecutionPolicy Bypass -File .\scripts\relocate-more-caches.ps1 -Root D:\cache -Apply -MoveOld

# 6. (New) Scan by program name and clean leftovers
powershell -ExecutionPolicy Bypass -File .\scripts\Find-AndRemove-Leftovers.ps1 -Name "phanthywork" -Preview -SavePreview "C:\Users\51139\Downloads\preview-phanthywork.json"
powershell -ExecutionPolicy Bypass -File .\scripts\Find-AndRemove-Leftovers.ps1 -ApplyFrom "C:\Users\51139\Downloads\preview-phanthywork.json" -Delete -AutoZipBackup

# 7. (Optional, new) Registry tweaks: preview, then -Apply from an elevated PowerShell
powershell -ExecutionPolicy Bypass -File .\scripts\tweak-registry.ps1
powershell -ExecutionPolicy Bypass -File .\scripts\tweak-registry.ps1 -Apply -RestartExplorer
```

## Scripts at a glance

| Script | Purpose | Modifies system? |
|---|---|---|
| `scripts/audit.ps1` | Show C: free space and major folders (hiberfile, Downloads, Temp, AppData), check uv/npm/pip caches and versions | No |
| `scripts/clean.ps1` | Clean user temp, system temp, crash dumps, uv/npm/pip caches, recycle bin. Add `-Deep` to run DISM cleanup | Preview by default; `-Apply` to delete |
| `scripts/relocate-dev-caches.ps1` | Set `UV_CACHE_DIR`, `PIP_CACHE_DIR`, npm cache/prefix to the given directory and add npm global to PATH | Changes env & cache locations |
| `scripts/relocate-more-caches.ps1` | Move caches for bun/yarn/pnpm/Playwright/HuggingFace/Electron and others to the target, detect pnpm/Go/.NET/Deno | Changes env & cache locations |
| `scripts/Find-AndRemove-Leftovers.ps1` | Search by program name for uninstall leftovers, cache, logs, startup, services, registry. Supports preview/save/apply, GUI selection, whitelist/blacklist, backup zip | Preview by default; `-Delete` to delete |
| `scripts/tweak-registry.ps1` | 7 registry tweaks: faster menus, no auto-reboot after updates, faster taskbar previews, disable Cortana, show file extensions, no recent-files tracking, faster shutdown. `-Only` / `-Skip` to choose, `-RestoreFrom` for exact rollback, `-Restore` for Windows defaults | Preview by default; `-Apply` to write |

## Safety principles

- Preview by default. Scripts that delete or change settings will not act unless `-Apply` or `-Delete` is specified.
- Registry: only the explicitly listed values are changed. `tweak-registry.ps1` records each value's original state (including "did not exist") so it can be rolled back exactly.
- Only remove recreatable content (temporary files, cache, crash dumps, recycle bin).
- Never touch: browser data, Downloads, Desktop, chat app data, `WinSxS`, `Windows\Installer`, `System32`, `DriverStore`.
- Deletion is done by clearing contents, not removing high-level folders, and short paths are rejected.
- Scripts are plain text — read them before running.
- The script always backs up before deleting and writes logs for auditing and rollback.

## Manual actions (not scripted)

Some tasks require manual judgment:

| Item | Action |
|---|---|
| Hibernation file `hiberfil.sys` | Administrator CMD: `powercfg -h off` (disables hibernation and fast startup) |
| `WinSxS` | Use DISM: `DISM /Online /Cleanup-Image /StartComponentCleanup` or run `clean.ps1 -Apply -Deep` |
| Old GPU drivers / Windows update caches / `Windows.old` | Use Disk Cleanup (cleanmgr) -> Clean up system files |
| Browser cache | Clear via browser UI (Ctrl+Shift+Delete) — uncheck passwords and cookies |
| Chat app cache | Use app settings to clear or change storage location; avoid deleting app data folders directly |
| Downloads / Desktop | Manually review. Installer packages consume the most space |
| Leftovers after uninstall | Check: `C:\Users\<user>\.name`, `AppData\Local\name`, `AppData\Roaming\name`, `C:\ProgramData\name` |

## Maintenance cadence

| Frequency | Action | Time |
|---|---|---|
| Monthly | `clean.ps1 -Apply`, clear browser cache, review Downloads and Desktop | ~5 min |
| Quarterly | `audit.ps1`, `clean.ps1 -Apply -Deep`, Disk Cleanup -> Clean system files, uninstall unused apps | ~20 min |
| After uninstalling software | Check the leftover locations listed above | ~1 min |
| When free space < 30GB | Run a quarterly cleanup immediately | |

You can schedule monthly cleanups via Task Scheduler (avoid spaces in paths):

```powershell
schtasks /Create /SC MONTHLY /D 1 /ST 10:00 /TN "CDriveKeeper" /TR "powershell.exe -NoProfile -ExecutionPolicy Bypass -File D:\tools\c-drive-keeper\scripts\clean.ps1 -Apply"
```

## Docs

- [Python / Node: install to non-system drive](docs/python-install.md)
- [Troubleshooting](docs/troubleshooting.md)

## Contributing

PRs and issues welcome, especially for:

- Additional AI client cache/data locations (Cursor, Zed, opencode, MCP environments)
- More package manager caches (Cargo, Go, Maven, Gradle, conda)
- Cross-Windows-version testing and fixes

New deletion features must default to preview and only operate on recreatable data.

## Disclaimer

Cleaning operations may be irreversible. Always run preview, inspect results, and backup important data before deleting. The author is not responsible for data loss.

## License

[MIT](LICENSE)
