# c-drive-keeper

**Keep your Windows C: drive from filling up: audit first, preview next, clean last.** Built for the way AI desktop clients, MCP tools and Python/Node toolchains quietly eat system-drive space.

[中文](README.md)

## Why

AI clients (Cherry Studio, Codex, Cursor, and similar) and their MCP runtimes tend to:

- store data under `C:\Users\<you>\AppData` and `C:\Users\<you>\.<tool>` regardless of where the app itself is installed,
- each download their own copy of Node, Python, uv or bun,
- leave uv, npm and pip caches on C: where they only grow,
- leave user data behind after uninstall.

This project turns "inspect, clean, relocate" into three scripts plus a troubleshooting guide.

## Quick start

Windows 10/11 with the built-in PowerShell 5.1. Nothing to install. Many systems block `.ps1` files by default, so the commands below use `-ExecutionPolicy Bypass`, which applies to that single run only.

```powershell
# 1. Audit (read-only)
powershell -ExecutionPolicy Bypass -File .\scripts\audit.ps1

# 2. Preview what can be cleaned (nothing is deleted)
powershell -ExecutionPolicy Bypass -File .\scripts\clean.ps1

# 3. Actually clean
powershell -ExecutionPolicy Bypass -File .\scripts\clean.ps1 -Apply

# 4. Move uv / pip / npm caches to another drive (preview, then -Apply)
powershell -ExecutionPolicy Bypass -File .\scripts\relocate-dev-caches.ps1 -Root D:\cache
powershell -ExecutionPolicy Bypass -File .\scripts\relocate-dev-caches.ps1 -Root D:\cache -Apply
```

## Scripts

| Script | What it does | Changes the system? |
|---|---|---|
| `scripts/audit.ps1` | Free space, common big items (hiberfil.sys, Downloads, Temp), top folders in AppData and hidden `.xxx` folders, and whether uv/npm/pip caches and node/python still point at C: | No, read-only |
| `scripts/clean.ps1` | Clears user/system Temp, crash dumps, uv/npm/pip caches and the Recycle Bin. `-Deep` also runs DISM component cleanup | Preview by default; deletes only with `-Apply` |
| `scripts/relocate-dev-caches.ps1` | Sets `UV_CACHE_DIR`, `PIP_CACHE_DIR`, npm `cache` and `prefix` under a root you choose, and adds the npm global dir to PATH. `-UseSystemPythonOnly` makes uv use the one installed Python instead of downloading its own | Preview by default; changes only with `-Apply` |

## Safety rules

- Anything that deletes or modifies settings is a preview unless you pass `-Apply`.
- Only regenerable data is touched: temp files, crash dumps, package-manager caches, Recycle Bin.
- Never touched: browser data, Downloads, Desktop, chat app data, `WinSxS`, `Windows\Installer`, `System32`, `DriverStore`.
- Directory cleanup empties contents only and refuses suspiciously short paths.

## Manual steps (deliberately not scripted)

`powercfg -h off` to drop hiberfil.sys; `DISM /Online /Cleanup-Image /StartComponentCleanup` for WinSxS; `cleanmgr` -> Clean up system files for old drivers and `Windows.old`; clear browser cache from the browser; clear chat-app caches from inside the app; review Downloads and Desktop; after uninstalling software, check `~\.<name>`, `AppData\Local\<name>`, `AppData\Roaming\<name>` and `ProgramData\<name>` for leftovers.

## Docs

- [Installing Python/Node off the system drive](docs/python-install.md)
- [Troubleshooting](docs/troubleshooting.md)

(The docs are currently in Chinese; translations are welcome.)

## Disclaimer

Cleanup can be irreversible. Read the preview output first and back up anything important. No warranty; see [LICENSE](LICENSE).

## License

[MIT](LICENSE)
