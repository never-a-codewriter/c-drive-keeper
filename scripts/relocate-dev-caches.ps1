<#
.SYNOPSIS
  把 uv / pip / npm 的缓存和 npm 全局包目录迁到非系统盘，让它们以后不再往 C 盘写。
  默认只预览，加 -Apply 才真正修改。
.PARAMETER Root
  新的缓存根目录，默认 D:\cache。必须在非系统盘上。
.PARAMETER Apply
  真正执行修改。
.PARAMETER UseSystemPythonOnly
  额外设置 uv 只使用系统里已安装的 Python，不再自己下载（保证只有一份 Python）。
.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\relocate-dev-caches.ps1
  powershell -ExecutionPolicy Bypass -File .\scripts\relocate-dev-caches.ps1 -Root E:\cache -Apply
  powershell -ExecutionPolicy Bypass -File .\scripts\relocate-dev-caches.ps1 -Apply -UseSystemPythonOnly
#>
param(
    [string]$Root = 'D:\cache',
    [switch]$Apply,
    [switch]$UseSystemPythonOnly
)

$ErrorActionPreference = 'Stop'
$SysDrive = $env:SystemDrive

# ---------- 校验目标路径 ----------
$Root = $Root.TrimEnd('\')
$driveRoot = [System.IO.Path]::GetPathRoot($Root)
if (-not $driveRoot -or -not (Test-Path -LiteralPath $driveRoot)) {
    Write-Host "目标盘不存在：$Root" -ForegroundColor Red
    exit 1
}
if ($Root.StartsWith($SysDrive, [StringComparison]::OrdinalIgnoreCase)) {
    Write-Host "目标路径 $Root 仍然在系统盘 $SysDrive 上，这样迁移没有意义。请换一个盘，例如 -Root D:\cache" -ForegroundColor Red
    exit 1
}

$hasNpm = [bool](Get-Command npm.cmd -ErrorAction SilentlyContinue)
$npmGlobal = Join-Path $Root 'npm-global'

# ---------- 计划 ----------
$envVars = [ordered]@{
    'UV_CACHE_DIR'  = (Join-Path $Root 'uv')
    'PIP_CACHE_DIR' = (Join-Path $Root 'pip')
}
if ($UseSystemPythonOnly) {
    $envVars['UV_PYTHON_PREFERENCE'] = 'only-system'
    $envVars['UV_PYTHON_DOWNLOADS']  = 'never'
}

Write-Host ""
if ($Apply) {
    Write-Host "=== 执行迁移（-Apply）===" -ForegroundColor Cyan
} else {
    Write-Host "=== 预览模式：只显示计划，不修改（要执行请加 -Apply）===" -ForegroundColor Cyan
}
Write-Host "目标根目录: $Root`n"
Write-Host "将设置的用户环境变量："
foreach ($k in $envVars.Keys) { Write-Host ("  {0} = {1}" -f $k, $envVars[$k]) }
if ($hasNpm) {
    Write-Host "将设置的 npm 配置："
    Write-Host ("  cache  = {0}" -f (Join-Path $Root 'npm'))
    Write-Host ("  prefix = {0}（并把它加入用户 PATH）" -f $npmGlobal)
} else {
    Write-Host "未检测到 npm，跳过 npm 部分。" -ForegroundColor DarkGray
}

if (-not $Apply) {
    Write-Host "`n确认无误后，加 -Apply 再运行一次。`n" -ForegroundColor Cyan
    exit 0
}

# ---------- 执行 ----------
function Add-UserPath([string]$Dir) {
    # 直接读注册表原始值，保留 %USERPROFILE% 这类变量；写回时仍用 ExpandString 类型
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
    try {
        $raw = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        $parts = @($raw -split ';' | Where-Object { $_ })
        $norm = $Dir.TrimEnd('\')
        $exists = $parts | Where-Object { $_.TrimEnd('\') -ieq $norm }
        if ($exists) { return $false }
        $newValue = (($parts + $Dir) -join ';')
        $key.SetValue('Path', $newValue, [Microsoft.Win32.RegistryValueKind]::ExpandString)
        return $true
    } finally {
        $key.Close()
    }
}

function Send-EnvironmentChange {
    # 通知系统环境变量已变化，这样新打开的窗口不用注销就能读到
    try {
        if (-not ('CDriveKeeper.Native' -as [type])) {
            Add-Type -Namespace CDriveKeeper -Name Native -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true, CharSet = System.Runtime.InteropServices.CharSet.Auto)]
public static extern System.IntPtr SendMessageTimeout(System.IntPtr hWnd, uint Msg, System.UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out System.UIntPtr lpdwResult);
'@
        }
        $result = [UIntPtr]::Zero
        [void][CDriveKeeper.Native]::SendMessageTimeout([IntPtr]0xffff, 0x1A, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$result)
    } catch {
        Write-Host "（未能广播环境变量变更，请注销再登录一次，或重启电脑）" -ForegroundColor Yellow
    }
}

try {
    # 目录
    $dirs = @($envVars['UV_CACHE_DIR'], $envVars['PIP_CACHE_DIR'], (Join-Path $Root 'npm'), $npmGlobal)
    foreach ($d in $dirs) { New-Item -ItemType Directory -Path $d -Force | Out-Null }

    # 环境变量（用户级）
    foreach ($k in $envVars.Keys) {
        [Environment]::SetEnvironmentVariable($k, $envVars[$k], 'User')
        Set-Item -Path "Env:$k" -Value $envVars[$k]
        Write-Host ("已设置 {0}" -f $k) -ForegroundColor Green
    }

    # npm
    if ($hasNpm) {
        # npm 会把警告写到 stderr，在 Stop 模式下会被当成错误，所以这里临时放宽
        $ErrorActionPreference = 'Continue'
        & npm.cmd config set cache (Join-Path $Root 'npm') 2>&1 | Out-Null
        & npm.cmd config set prefix $npmGlobal 2>&1 | Out-Null
        $ErrorActionPreference = 'Stop'
        Write-Host "已设置 npm cache / prefix" -ForegroundColor Green
        if (Add-UserPath $npmGlobal) {
            $env:Path = "$env:Path;$npmGlobal"
            Write-Host "已把 $npmGlobal 加入用户 PATH" -ForegroundColor Green
        } else {
            Write-Host "$npmGlobal 已经在 PATH 里，跳过" -ForegroundColor DarkGray
        }
    }

    Send-EnvironmentChange
} catch {
    Write-Host "执行出错：$($_.Exception.Message)" -ForegroundColor Red
    exit 1
}

Write-Host "`n=== 完成 ===" -ForegroundColor Cyan
Write-Host "接下来："
Write-Host "  1. 关掉所有终端、AI 客户端和 IDE（包括托盘图标），重新打开，它们才会读到新设置"
Write-Host "  2. 之前用 npm 全局安装的命令需要重新安装（npm install -g xxx）"
Write-Host "  3. 旧缓存还留在 C 盘，运行 scripts\clean.ps1 -Apply 清掉"
Write-Host "  4. 运行 scripts\audit.ps1 验证`n"
